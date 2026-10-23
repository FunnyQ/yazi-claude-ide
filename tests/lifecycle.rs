mod common;

use common::{Client, assert_unauthorized};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use yazi_claude_ide::lock::{self, LockFile};

const TIMEOUT: Duration = Duration::from_secs(8);
// Poll rather than spin: eight tests in this file wait on spawned sidecars in
// parallel, and `yield_now` burns the cores those sidecars need to start.
const POLL: Duration = Duration::from_millis(5);

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn id(&self) -> u32 {
        self.child.as_ref().expect("child should be present").id()
    }

    fn wait(&mut self) -> std::process::Output {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let child = self.child.as_mut().expect("child should be present");
            if child.try_wait().expect("query child status").is_some() {
                return self
                    .child
                    .take()
                    .expect("child should be present")
                    .wait_with_output()
                    .expect("collect child output");
            }
            assert!(
                Instant::now() < deadline,
                "sidecar did not exit before timeout"
            );
            thread::sleep(POLL);
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

const FAST_LIVENESS_EXIT: (&str, &str) = ("100", "2");
const OUTLIVES_ITS_ASSERTIONS: (&str, &str) = ("2000", "100");

fn spawn_sidecar(config: &Path, yazi_id: &str, wants_liveness_exit: bool) -> ChildGuard {
    spawn_sidecar_labelled(config, yazi_id, wants_liveness_exit, None)
}

fn spawn_sidecar_labelled(
    config: &Path,
    yazi_id: &str,
    wants_liveness_exit: bool,
    label: Option<&str>,
) -> ChildGuard {
    let (poll_ms, failures) = if wants_liveness_exit {
        FAST_LIVENESS_EXIT
    } else {
        OUTLIVES_ITS_ASSERTIONS
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_yazi-claude-ide"));
    // README tells users to export YCI_IDE_LABEL, so the developer running the
    // suite may well have one. Every test states its own, or states none.
    match label {
        Some(label) => command.env("YCI_IDE_LABEL", label),
        None => command.env_remove("YCI_IDE_LABEL"),
    };
    let child = command
        .env("CLAUDE_CONFIG_DIR", config)
        .env("YAZI_ID", yazi_id)
        .env("YCI_POLL_MS", poll_ms)
        .env("YCI_FAILURES_BEFORE_GONE", failures)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if wants_liveness_exit {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .expect("spawn compiled sidecar binary");
    ChildGuard { child: Some(child) }
}

const FAKE_YA: &str = r#"#!/bin/sh
case "$1" in
sub)
  # One file per line, moved into place atomically, so a publish cannot race the
  # drain. The loop ends with the test's temp directory, which is what keeps a
  # killed sidecar from leaving this shell behind.
  i=0
  while [ "$i" -lt 600 ]; do
    [ -d "$YCI_TEST_BUS" ] || exit 0
    for line in "$YCI_TEST_BUS"/*.line; do
      [ -e "$line" ] || continue
      cat "$line"
      rm -f "$line"
    done
    i=$((i + 1))
    sleep 0.05
  done
  ;;
emit-to)
  # `emit-to <id> shell <command> --block` is J3; `emit-to <id> noop` is the
  # liveness probe and needs nothing but the exit status. The shell command is
  # recorded rather than run: the test plays yazi's pane, so the viewer runs when
  # the test says so and the race a real pane cannot have stays out of the suite.
  if [ "$3" = "shell" ]; then
    printf '%s' "$4" > "$YCI_TEST_SHELL.tmp"
    mv "$YCI_TEST_SHELL.tmp" "$YCI_TEST_SHELL"
  fi
  ;;
pub-to)
  # `pub-to 0 <kind> --json <body>`, the publish J4 rides.
  printf '%s,0,fake-ya,%s\n' "$3" "$5" > "$YCI_TEST_BUS/pub.tmp"
  mv "$YCI_TEST_BUS/pub.tmp" "$YCI_TEST_BUS/pub.line"
  ;;
esac
exit 0
"#;

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

async fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(contents) = fs::read_to_string(path) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        tokio::time::sleep(POLL).await;
    }
}

fn lock_files(config: &Path) -> Vec<PathBuf> {
    let dir = config.join("ide");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .collect();
    files.sort();
    files
}

fn wait_for_lock_count(config: &Path, count: usize) -> Vec<PathBuf> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let files = lock_files(config);
        if files.len() == count {
            return files;
        }
        assert!(
            Instant::now() < deadline,
            "expected {count} lock file(s), found {}",
            files.len()
        );
        thread::sleep(POLL);
    }
}

fn wait_for_one_lock(config: &Path) -> PathBuf {
    wait_for_lock_count(config, 1).pop().unwrap()
}

fn read_lock(path: &Path) -> LockFile {
    lock::read_lock(path.parent().expect("lock directory"), port_from(path))
        .expect("parse lock file")
}

fn port_from(path: &Path) -> u16 {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.parse().ok())
        .expect("lock file name should be a port")
}

fn signal(child: &ChildGuard, signal: libc::c_int) {
    // SAFETY: the guard owns this live child pid and the requested signals do not
    // require a pointer or shared-memory invariant.
    assert_eq!(unsafe { libc::kill(child.id() as libc::pid_t, signal) }, 0);
}

#[test]
fn a6_sigterm_removes_the_lock_file() {
    let temp = TempDir::new().unwrap();
    let mut child = spawn_sidecar(temp.path(), "lifecycle-sigterm", false);
    wait_for_one_lock(temp.path());
    signal(&child, libc::SIGTERM);
    assert!(child.wait().status.success());
    assert!(lock_files(temp.path()).is_empty());
}

#[test]
fn a6_sigint_removes_the_lock_file() {
    let temp = TempDir::new().unwrap();
    let mut child = spawn_sidecar(temp.path(), "lifecycle-sigint", false);
    wait_for_one_lock(temp.path());
    signal(&child, libc::SIGINT);
    assert!(child.wait().status.success());
    assert!(lock_files(temp.path()).is_empty());
}

#[test]
fn a6_liveness_exit_removes_the_lock_file() {
    let temp = TempDir::new().unwrap();
    let mut child = spawn_sidecar(temp.path(), "non-existent-lifecycle-yazi", true);
    wait_for_one_lock(temp.path());
    let output = child.wait();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("yazi-claude-ide: yazi is gone, exiting")
    );
    assert!(lock_files(temp.path()).is_empty());
}

#[test]
fn g4_two_sidecars_get_distinct_ports_and_tokens() {
    let temp = TempDir::new().unwrap();
    let _first = spawn_sidecar(temp.path(), "lifecycle-distinct-1", false);
    let _second = spawn_sidecar(temp.path(), "lifecycle-distinct-2", false);
    let files = wait_for_lock_count(temp.path(), 2);
    assert_ne!(files[0].file_name(), files[1].file_name());
    assert_ne!(
        read_lock(&files[0]).auth_token,
        read_lock(&files[1]).auth_token
    );
}

#[test]
fn g4_neither_sidecar_deletes_the_other_lock() {
    let temp = TempDir::new().unwrap();
    let mut first = spawn_sidecar(temp.path(), "lifecycle-owner-1", false);
    let second = spawn_sidecar(temp.path(), "lifecycle-owner-2", false);
    let files = wait_for_lock_count(temp.path(), 2);
    let first_pid = first.id();
    let first_path = files
        .iter()
        .find(|path| read_lock(path).pid == first_pid)
        .expect("first sidecar lock")
        .clone();
    let survivor_path = files.into_iter().find(|path| path != &first_path).unwrap();
    let survivor_contents = fs::read(&survivor_path).unwrap();
    signal(&first, libc::SIGTERM);
    assert!(first.wait().status.success());
    assert!(!first_path.exists());
    assert_eq!(fs::read(&survivor_path).unwrap(), survivor_contents);
    drop(second);
}

#[test]
fn a1_a3_a5_the_lock_file_the_binary_writes_is_well_formed() {
    let temp = TempDir::new().unwrap();
    let _child = spawn_sidecar(temp.path(), "lifecycle-lock-shape", false);
    let path = wait_for_one_lock(temp.path());
    let bytes = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 5);
    for field in [
        "pid",
        "workspaceFolders",
        "ideName",
        "transport",
        "authToken",
    ] {
        assert!(object.contains_key(field), "missing {field}");
    }
    let lock: LockFile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(lock.ide_name, "yazi");
    assert_eq!(lock.transport, "ws");
    assert_eq!(lock.auth_token.len(), 32);
    assert!(
        lock.auth_token
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
    assert_eq!(
        path.file_name().unwrap(),
        format!("{}.lock", port_from(&path)).as_str()
    );
}

#[test]
fn a3_the_binary_takes_its_ide_name_from_yci_ide_label() {
    let temp = TempDir::new().unwrap();
    let _child =
        spawn_sidecar_labelled(temp.path(), "lifecycle-ide-label", false, Some(" w41:t6 "));
    let lock = read_lock(&wait_for_one_lock(temp.path()));
    assert_eq!(lock.ide_name, "yazi (w41:t6)");
}

#[tokio::test]
async fn e1_the_running_binary_refuses_a_wrong_token() {
    let temp = TempDir::new().unwrap();
    let _child = spawn_sidecar(temp.path(), "lifecycle-wrong-token", false);
    let path = wait_for_one_lock(temp.path());
    assert_unauthorized(
        Client::connect_port(port_from(&path), "wrong-token")
            .await
            .err()
            .expect("wrong token must not upgrade"),
    );
}

#[tokio::test]
async fn initialize_and_tools_list_succeed_against_the_running_binary() {
    let temp = TempDir::new().unwrap();
    let _child = spawn_sidecar(temp.path(), "lifecycle-protocol", false);
    let path = wait_for_one_lock(temp.path());
    let lock = read_lock(&path);
    let mut client = Client::connect_port(port_from(&path), &lock.auth_token)
        .await
        .expect("authorized client should connect");
    let initialized = client
        .call(
            1,
            "initialize",
            Some(json!({"protocolVersion": "2099-01-01"})),
        )
        .await;
    assert_eq!(initialized["result"]["protocolVersion"], "2099-01-01");
    let listed = client.call(2, "tools/list", None).await;
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "getCurrentSelection",
            "getLatestSelection",
            "getWorkspaceFolders",
            "getOpenEditors"
        ]
    );
}

#[tokio::test]
async fn j1_j8_the_binary_opens_a_diff_and_answers_with_the_file_the_user_left() {
    let temp = TempDir::new().unwrap();
    let bus = temp.path().join("bus");
    let shell_command_file = temp.path().join("shell-command");
    let dollar_one_file = temp.path().join("dollar-one");
    let bin = temp.path().join("bin");
    fs::create_dir(&bus).unwrap();
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("ya"), FAKE_YA).unwrap();
    fs::set_permissions(
        bin.join("ya"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let user_file = temp.path().join("target.txt");
    fs::write(&user_file, "one\ntwo\n").unwrap();
    let tab_name = "✻ [Claude Code] target.txt (5c8bea) ⧉";

    let mut child = ChildGuard {
        child: Some(
            Command::new(env!("CARGO_BIN_EXE_yazi-claude-ide"))
                .env("CLAUDE_CONFIG_DIR", temp.path())
                .env("YAZI_ID", "lifecycle-diff")
                .env("YCI_POLL_MS", "2000")
                .env("YCI_FAILURES_BEFORE_GONE", "100")
                .env_remove("YCI_IDE_LABEL")
                .env("PATH", &path)
                .env("YCI_TEST_BUS", &bus)
                .env("YCI_TEST_SHELL", &shell_command_file)
                .env(
                    "YCI_DIFF_CMD",
                    format!(
                        "printf '%s' \"$1\" > '{}'\nprintf 'amended\\n' >> \"$2\"",
                        dollar_one_file.display()
                    ),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn compiled sidecar binary"),
        ),
    };

    let lock_path = wait_for_one_lock(temp.path());
    let lock = read_lock(&lock_path);
    let mut client = Client::connect_port(port_from(&lock_path), &lock.auth_token)
        .await
        .expect("authorized client should connect");
    client.raw(
        &json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "openDiff",
                "arguments": {
                    "old_file_path": user_file.to_str().unwrap(),
                    "new_file_path": user_file.to_str().unwrap(),
                    "new_file_contents": "one\nTWO\n",
                    "tab_name": tab_name,
                },
            },
        })
        .to_string(),
    );

    let shell_command = wait_for_file(&shell_command_file).await;
    let script = PathBuf::from(
        shell_command
            .strip_prefix("sh '")
            .and_then(|rest| rest.strip_suffix('\''))
            .expect("the blocking shell runs the generated script"),
    );
    let dir = script
        .parent()
        .expect("the script sits in its own directory");
    let copy = dir.join("target.txt");
    assert_eq!(fs::read_to_string(&copy).unwrap(), "one\nTWO\n");
    assert_eq!(mode_of(&copy), 0o600);
    assert_eq!(mode_of(&script), 0o600);
    assert_eq!(mode_of(dir), 0o700);
    assert!(client.silence(Duration::from_millis(300)).await.is_none());

    assert!(
        Command::new("sh")
            .arg("-c")
            .arg(&shell_command)
            .env("PATH", &path)
            .env("YCI_TEST_BUS", &bus)
            .status()
            .expect("run the generated script")
            .success()
    );

    let response = client.next(Duration::from_secs(8)).await.unwrap();
    assert_eq!(response["id"], 11);
    assert_eq!(response["result"]["content"][0]["text"], "FILE_SAVED");
    assert_eq!(
        response["result"]["content"][1]["text"],
        "one\nTWO\namended\n"
    );
    assert_eq!(
        fs::read_to_string(&dollar_one_file).unwrap(),
        user_file.to_str().unwrap()
    );
    assert!(!dir.exists());
    assert_eq!(fs::read_to_string(&user_file).unwrap(), "one\ntwo\n");

    signal(&child, libc::SIGTERM);
    let stderr = String::from_utf8_lossy(&child.wait().stderr).into_owned();
    assert!(stderr.contains(&format!(
        "yazi-claude-ide: diff {} ({tab_name})",
        user_file.display()
    )));
    assert!(stderr.contains("yazi-claude-ide: diff closed"));
    assert!(!stderr.contains("TWO"));
    assert!(!stderr.contains("amended"));
}

#[tokio::test]
async fn i10_the_binary_logs_the_selection_range_and_never_the_text() {
    let temp = TempDir::new().unwrap();
    let bus = temp.path().join("bus");
    let bin = temp.path().join("bin");
    fs::create_dir(&bus).unwrap();
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("ya"), FAKE_YA).unwrap();
    fs::set_permissions(
        bin.join("ya"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let user_file = temp.path().join("selected.txt");
    fs::write(&user_file, "one\nsecret-two\nsecret-three\n").unwrap();
    let text = "secret-two\nsecret-three\n";

    let mut child = ChildGuard {
        child: Some(
            Command::new(env!("CARGO_BIN_EXE_yazi-claude-ide"))
                .env("CLAUDE_CONFIG_DIR", temp.path())
                .env("YAZI_ID", "lifecycle-selection")
                .env("YCI_POLL_MS", "2000")
                .env("YCI_FAILURES_BEFORE_GONE", "100")
                .env_remove("YCI_IDE_LABEL")
                .env_remove("YCI_DIFF_CMD")
                .env("PATH", &path)
                .env("YCI_TEST_BUS", &bus)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn compiled sidecar binary"),
        ),
    };

    let lock_path = wait_for_one_lock(temp.path());
    let lock = read_lock(&lock_path);
    let mut client = Client::connect_port(port_from(&lock_path), &lock.auth_token)
        .await
        .expect("authorized client should connect");
    let body = json!({
        "yaziId": "lifecycle-selection",
        "url": user_file.to_str().unwrap(),
        "lineStart": 2,
        "lineEnd": 3,
        "charStart": 0,
        "charEnd": 0,
        "text": text,
    });
    fs::write(
        bus.join("selection.tmp"),
        format!("claude-editor-selection,0,fake-ya,{body}\n"),
    )
    .unwrap();
    fs::rename(bus.join("selection.tmp"), bus.join("selection.line")).unwrap();

    let deadline = Instant::now() + TIMEOUT;
    loop {
        let frame = client.next(TIMEOUT).await.expect("selection push");
        if frame["params"]["text"] == text {
            break;
        }
        assert!(Instant::now() < deadline, "the selection was never pushed");
    }

    signal(&child, libc::SIGTERM);
    let stderr = String::from_utf8_lossy(&child.wait().stderr).into_owned();
    assert!(stderr.contains(&format!(
        "yazi-claude-ide: selection {} L2-3",
        user_file.display()
    )));
    assert!(!stderr.contains("secret"));
}

#[test]
fn g4_the_startup_log_line_maps_the_yazi_id_to_its_port() {
    let temp = TempDir::new().unwrap();
    let mut child = ChildGuard {
        child: Some(
            Command::new(env!("CARGO_BIN_EXE_yazi-claude-ide"))
                .env("CLAUDE_CONFIG_DIR", temp.path())
                .env("YAZI_ID", "lifecycle-startup-line")
                .env("YCI_POLL_MS", "2000")
                .env("YCI_FAILURES_BEFORE_GONE", "100")
                .env_remove("YCI_IDE_LABEL")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn compiled sidecar binary"),
        ),
    };
    let port = port_from(&wait_for_one_lock(temp.path()));
    signal(&child, libc::SIGTERM);
    let stderr = String::from_utf8_lossy(&child.wait().stderr).into_owned();
    let lines: Vec<_> = stderr
        .lines()
        .filter(|line| line.contains("ws://127.0.0.1:"))
        .collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    assert!(
        lines[0].contains(&format!(
            "ws://127.0.0.1:{port} yazi=lifecycle-startup-line "
        )),
        "{stderr}"
    );
}

mod common;

use common::Client;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::Error;
use yazi_claude_ide::lock::LockFile;

const TIMEOUT: Duration = Duration::from_secs(8);

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
            thread::yield_now();
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

fn spawn_sidecar(config: &Path, yazi_id: &str, wants_liveness_exit: bool) -> ChildGuard {
    // Only the liveness test gets fast failure settings. Suite cleanup must not
    // "simplify" these into one constant or the other seven processes may exit
    // before their assertions finish.
    let (poll_ms, failures) = if wants_liveness_exit {
        ("100", "2")
    } else {
        ("2000", "100")
    };
    let child = Command::new(env!("CARGO_BIN_EXE_yazi-claude-ide"))
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
        thread::yield_now();
    }
}

fn wait_for_one_lock(config: &Path) -> PathBuf {
    wait_for_lock_count(config, 1).pop().unwrap()
}

fn read_lock(path: &Path) -> LockFile {
    serde_json::from_slice(&fs::read(path).expect("read lock file")).expect("parse lock file")
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

#[tokio::test]
async fn e1_the_running_binary_refuses_a_wrong_token() {
    let temp = TempDir::new().unwrap();
    let _child = spawn_sidecar(temp.path(), "lifecycle-wrong-token", false);
    let path = wait_for_one_lock(temp.path());
    let error = match Client::connect_port(port_from(&path), "wrong-token").await {
        Ok(_) => panic!("wrong token must not upgrade"),
        Err(error) => error,
    };
    match error {
        Error::Http(response) => assert_eq!(response.status(), 401),
        other => panic!("expected HTTP rejection, got {other}"),
    }
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

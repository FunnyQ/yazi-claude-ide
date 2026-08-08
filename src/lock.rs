use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockFile {
    pub pid: u32,
    pub workspace_folders: Vec<String>,
    pub ide_name: String,
    pub transport: String,
    pub auth_token: String,
}

pub fn new_auth_token() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("CSPRNG must produce an authentication token");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn lock_dir() -> PathBuf {
    lock_dir_from(|key| std::env::var(key).ok())
}

pub fn lock_dir_from(get: impl Fn(&str) -> Option<String>) -> PathBuf {
    let config = get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            get("HOME")
                .map(PathBuf::from)
                .or_else(std::env::home_dir)
                .unwrap_or_default()
                .join(".claude")
        });
    config.join("ide")
}

pub fn ide_name() -> String {
    ide_name_from(|key| std::env::var(key).ok())
}

/**
 * The picker's row label (A3). Two sidecars anchored on the same repository are
 * otherwise indistinguishable there — both rows carry `ideName` and the same
 * anchor — so the label is the only place a user can tell them apart.
 */
pub fn ide_name_from(get: impl Fn(&str) -> Option<String>) -> String {
    match get("YCI_IDE_LABEL").as_deref().map(str::trim) {
        Some(label) if !label.is_empty() => format!("yazi ({label})"),
        _ => "yazi".to_owned(),
    }
}

fn normalise(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    absolute.components().collect()
}

pub fn anchor_for(dir: &Path) -> PathBuf {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();

    if let Ok(output) = output
        && output.status.success()
        && let Ok(root) = String::from_utf8(output.stdout)
        && !root.trim().is_empty()
    {
        return normalise(Path::new(root.trim()));
    }

    // Not a repository, or no git on PATH. Either way the directory stands alone.
    normalise(dir)
}

pub fn workspace_folders(anchor: &Path, cursor: &Path) -> Vec<String> {
    let anchor = normalise(anchor);
    let cursor = normalise(cursor);
    let anchor = anchor.display().to_string();
    let cursor = cursor.display().to_string();
    if anchor == cursor {
        vec![anchor]
    } else {
        vec![anchor, cursor]
    }
}

fn lock_path(dir: &Path, port: u16) -> PathBuf {
    dir.join(format!("{port}.lock"))
}

pub fn write_lock(dir: &Path, port: u16, lock: &LockFile) -> io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // mkdir's mode is masked by umask, and the directory may already exist.
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;

    let path = lock_path(dir, port);
    // Written via rename so the CLI can never read a half-written lock file. When
    // the CLI re-reads is unmeasured, so it has to be safe at every instant.
    let temp = dir.join(format!("{port}.lock.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer(&mut file, lock).map_err(io::Error::other)?;
        file.flush()?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temp, &path)?;
        Ok(path.clone())
    })();
    if result.is_err() {
        // A failed publish must not leave a temporary discovery file behind.
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn read_lock(dir: &Path, port: u16) -> Option<LockFile> {
    serde_json::from_slice(&fs::read(lock_path(dir, port)).ok()?).ok()
}

pub fn remove_lock(dir: &Path, port: u16) {
    // Already gone. Removing a lock twice is not an error.
    let _ = fs::remove_file(lock_path(dir, port));
}

/**
 * Republish the folder pair, keeping pid, token, and everything else (B3, B4).
 * The caller owns which entry is the anchor: the anchor is only knowable from
 * yazi's first `cd` event, so the lock file cannot be the source of truth for it.
 */
pub fn update_folders(dir: &Path, port: u16, folders: Vec<String>) {
    let Some(mut lock) = read_lock(dir, port) else {
        return;
    };
    lock.workspace_folders = folders;
    // A concurrent shutdown may remove the lock before this best-effort republish.
    let _ = write_lock(dir, port, &lock);
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 performs the existence and permission check only; it
    // delivers nothing and cannot affect the target process.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    // EPERM means the process exists but belongs to another user — still alive,
    // and still owed its lock file.
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn reclaim_stale(dir: &Path) -> Vec<PathBuf> {
    reclaim_stale_with(dir, pid_alive)
}

/**
 * Remove lock files left behind by dead sidecars, and only those (A7).
 * An unparseable lock file is stale by definition — nothing can connect through it.
 */
pub fn reclaim_stale_with(dir: &Path, is_alive: impl Fn(u32) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".lock") else {
            continue;
        };
        if stem.is_empty() || !stem.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }

        let lock: Option<LockFile> = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        // Live sidecars, including other yazi instances, still own their locks.
        if lock.is_some_and(|lock| is_alive(lock.pid)) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
        // A failed unlink means another cleanup won the race, which is the desired outcome.
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, symlink};
    use tempfile::TempDir;

    fn lock(pid: u32, folders: Vec<String>) -> LockFile {
        LockFile {
            pid,
            workspace_folders: folders,
            ide_name: "yazi".into(),
            transport: "ws".into(),
            auth_token: "0123456789abcdef0123456789abcdef".into(),
        }
    }

    #[test]
    fn a1_the_lock_file_lives_at_config_ide_port_lock() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let config = temp.path().join("config");

        assert_eq!(
            lock_dir_from(|key| (key == "HOME").then(|| home.display().to_string())),
            home.join(".claude/ide")
        );
        assert_eq!(
            lock_dir_from(|key| match key {
                "CLAUDE_CONFIG_DIR" => Some(config.display().to_string()),
                "HOME" => Some(home.display().to_string()),
                _ => None,
            }),
            config.join("ide")
        );
        assert_eq!(
            lock_dir_from(|key| (key == "CLAUDE_CONFIG_DIR").then(String::new)),
            PathBuf::from("ide")
        );
        assert!(lock_dir().is_absolute());
        assert!(lock_dir().ends_with("ide"));
    }

    #[test]
    fn a1_the_file_name_is_the_bound_port() {
        let temp = TempDir::new().unwrap();
        let path = write_lock(temp.path(), 41234, &lock(1, vec![])).unwrap();
        assert_eq!(path.file_name().unwrap(), "41234.lock");
        assert!(path.is_file());
    }

    #[test]
    fn a2_directory_mode_is_0700_and_file_mode_is_0600() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("ide");
        let path = write_lock(&dir, 41234, &lock(1, vec![])).unwrap();

        assert_eq!(fs::metadata(dir).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn a3_the_lock_file_carries_exactly_the_five_fields() {
        let temp = TempDir::new().unwrap();
        let expected = lock(7, vec!["/repo".into()]);
        let path = write_lock(temp.path(), 4000, &expected).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let mut keys: Vec<_> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();

        assert_eq!(
            keys,
            [
                "authToken",
                "ideName",
                "pid",
                "transport",
                "workspaceFolders"
            ]
        );
        assert_eq!(json["ideName"], "yazi");
        assert_eq!(json["transport"], "ws");
        assert_eq!(read_lock(temp.path(), 4000), Some(expected));
    }

    #[test]
    fn a3_ide_name_is_yazi_unless_a_label_is_set() {
        let named = |value: Option<&str>| {
            ide_name_from(|key| match (key, value) {
                ("YCI_IDE_LABEL", Some(label)) => Some(label.to_owned()),
                _ => None,
            })
        };

        assert_eq!(named(None), "yazi");
        assert_eq!(named(Some("")), "yazi");
        assert_eq!(named(Some("   ")), "yazi");
        assert_eq!(named(Some("w41:t6")), "yazi (w41:t6)");
        assert_eq!(named(Some("  w41:t6  ")), "yazi (w41:t6)");
        // The label comes from YCI_IDE_LABEL alone — the sidecar knows no terminal
        // and no multiplexer, so a per-pane variable only counts once a user forwards it.
        assert_eq!(
            ide_name_from(|key| (key == "TERM_PROGRAM").then(|| "ghostty".to_owned())),
            "yazi"
        );
    }

    #[test]
    fn a4_auth_token_is_32_lowercase_hex_and_differs_per_call() {
        let first = new_auth_token();
        let second = new_auth_token();
        assert_eq!(first.len(), 32);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_ne!(first, second);
    }

    #[test]
    fn a6_remove_lock_deletes_the_file_and_tolerates_a_missing_one() {
        let temp = TempDir::new().unwrap();
        let path = write_lock(temp.path(), 4000, &lock(1, vec![])).unwrap();
        remove_lock(temp.path(), 4000);
        assert!(!path.exists());
        remove_lock(temp.path(), 4000);
    }

    #[test]
    fn a7_startup_reclaims_dead_pid_locks_and_spares_live_ones() {
        let temp = TempDir::new().unwrap();
        let live_pid = std::process::id();
        let dead = write_lock(temp.path(), 1111, &lock(999_999, vec![])).unwrap();
        let live = write_lock(temp.path(), 2222, &lock(live_pid, vec![])).unwrap();
        let unrelated = temp.path().join("not-a-lock.txt");
        fs::write(&unrelated, "keep").unwrap();

        assert_eq!(
            reclaim_stale_with(temp.path(), |pid| pid == live_pid),
            vec![dead.clone()]
        );
        assert!(!dead.exists());
        assert!(live.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn a7_an_unparseable_lock_file_is_reclaimed_rather_than_crashing_startup() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("2222.lock");
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(
            reclaim_stale_with(temp.path(), |_| true),
            vec![path.clone()]
        );
        assert!(!path.exists());
    }

    #[test]
    fn b1_the_pair_is_anchor_then_cursor() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cursor = repo.join("claude-ide.yazi");
        assert_eq!(
            workspace_folders(&repo, &cursor),
            vec![repo.display().to_string(), cursor.display().to_string()]
        );
    }

    #[test]
    fn b1_the_anchor_is_the_git_root_or_the_directory_itself() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(anchor_for(&repo.join("claude-ide.yazi")), repo);
        let temp = TempDir::new().unwrap();
        assert_eq!(anchor_for(temp.path()), temp.path());
    }

    #[test]
    fn b1_a_homedir_relative_anchor_stays_absolute() {
        let home = std::env::home_dir().unwrap();
        assert!(anchor_for(&home).is_absolute());
    }

    #[test]
    fn b2_an_anchor_equal_to_the_cursor_collapses_to_one_entry() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(workspace_folders(&repo, &repo).len(), 1);
    }

    #[test]
    fn b3_update_folders_republishes_the_pair_the_caller_computed() {
        let temp = TempDir::new().unwrap();
        write_lock(temp.path(), 4000, &lock(1, vec!["/repo".into()])).unwrap();
        let cases = [
            vec!["/repo".into(), "/repo/plugin".into()],
            vec!["/other".into(), "/other/plugin".into()],
            vec!["/other".into()],
        ];
        for folders in cases {
            update_folders(temp.path(), 4000, folders.clone());
            assert_eq!(
                read_lock(temp.path(), 4000).unwrap().workspace_folders,
                folders
            );
        }
    }

    #[test]
    fn b3_update_folders_on_a_lock_file_that_is_gone_is_a_no_op() {
        let temp = TempDir::new().unwrap();
        update_folders(temp.path(), 4000, vec!["/repo".into()]);
        assert_eq!(read_lock(temp.path(), 4000), None);
    }

    #[test]
    fn b4_the_rewrite_preserves_pid_and_auth_token_and_the_file_mode() {
        let temp = TempDir::new().unwrap();
        let original = lock(7, vec!["/repo".into()]);
        let path = write_lock(temp.path(), 4000, &original).unwrap();
        update_folders(temp.path(), 4000, vec!["/repo".into(), "/tmp".into()]);
        let updated = read_lock(temp.path(), 4000).unwrap();

        assert_eq!(updated.pid, original.pid);
        assert_eq!(updated.auth_token, original.auth_token);
        assert_eq!(updated.ide_name, original.ide_name);
        assert_eq!(updated.transport, original.transport);
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn b5_paths_are_absolutised_and_stripped_of_trailing_slashes() {
        let cwd = std::env::current_dir().unwrap();
        let cases = [
            (PathBuf::from("/repo/"), PathBuf::from("/repo")),
            (PathBuf::from("/repo/spike/"), PathBuf::from("/repo/spike")),
            (PathBuf::from("."), cwd.clone()),
            (PathBuf::from("spike"), cwd.join("spike")),
            (PathBuf::from("/"), PathBuf::from("/")),
            (PathBuf::from("/tmp/x/link"), PathBuf::from("/tmp/x/link")),
        ];
        for (input, expected) in cases {
            assert_eq!(
                workspace_folders(&input, &input),
                vec![expected.display().to_string()]
            );
        }
    }

    #[test]
    fn b5_symlinks_are_advertised_as_given_not_resolved() {
        let temp = TempDir::new().unwrap();
        let real = temp.path().join("real");
        let link = temp.path().join("link");
        fs::create_dir(&real).unwrap();
        symlink(&real, &link).unwrap();
        assert_eq!(
            workspace_folders(&link, &link),
            vec![link.display().to_string()]
        );
    }
}

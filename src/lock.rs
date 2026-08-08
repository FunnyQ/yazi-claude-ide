use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    todo!()
}

pub fn lock_dir() -> PathBuf {
    todo!()
}

pub fn lock_dir_from(_get: impl Fn(&str) -> Option<String>) -> PathBuf {
    todo!()
}

pub fn anchor_for(_dir: &Path) -> PathBuf {
    todo!()
}

pub fn workspace_folders(_anchor: &Path, _cursor: &Path) -> Vec<String> {
    todo!()
}

pub fn write_lock(_dir: &Path, _port: u16, _lock: &LockFile) -> std::io::Result<PathBuf> {
    todo!()
}

pub fn read_lock(_dir: &Path, _port: u16) -> Option<LockFile> {
    todo!()
}

pub fn remove_lock(_dir: &Path, _port: u16) {
    todo!()
}

pub fn update_folders(_dir: &Path, _port: u16, _folders: Vec<String>) {
    todo!()
}

pub fn reclaim_stale(_dir: &Path) -> Vec<PathBuf> {
    todo!()
}

pub fn reclaim_stale_with(_dir: &Path, _is_alive: impl Fn(u32) -> bool) -> Vec<PathBuf> {
    todo!()
}

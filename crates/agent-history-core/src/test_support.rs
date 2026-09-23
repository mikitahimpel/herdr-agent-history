//! Isolated helpers for synthetic parser, storage, and Git tests.
//!
//! Tests run inside a Git pre-push hook, and Git exports `GIT_DIR` and `GIT_INDEX_FILE`
//! to hooks. Every `git` invocation here therefore drops that inherited environment and
//! its user configuration, so a test repository can never be created, staged, or
//! committed in the repository under test.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct TempDir {
    path: PathBuf,
}
impl TempDir {
    pub fn new(prefix: &str) -> std::io::Result<Self> {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        for _ in 0..100 {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("agent-history-{prefix}-{pid}-{nonce}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate isolated temp directory",
        ))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn jsonl(&self, name: &str, records: &[&str]) -> std::io::Result<PathBuf> {
        let path = self.path.join(name);
        fs::write(&path, records.join("\n") + "\n")?;
        Ok(path)
    }
    pub fn database_path(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
    pub fn git_repository(&self) -> std::io::Result<PathBuf> {
        let repo = self.path.join("repo");
        fs::create_dir_all(&repo)?;
        let status = git_command(&repo).args(["init", "--quiet"]).status()?;
        if !status.success() {
            return Err(std::io::Error::other("git init failed"));
        }
        Ok(repo)
    }
}
/// A `git` invocation confined to `dir`, with the ambient Git environment, system and
/// global configuration, and committer identity all replaced by test-owned values.
pub fn git_command(dir: &Path) -> Command {
    let mut command = crate::git::git_command(dir);
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Agent History Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Agent History Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env_remove("GIT_REFLOG_ACTION");
    command
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

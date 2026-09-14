//! Isolated helpers for synthetic parser, storage, and Git tests.
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
        let path = std::env::temp_dir().join(format!(
            "agent-history-{prefix}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
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
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repo)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::other("git init failed"));
        }
        Ok(repo)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

//! Read-only Git context discovery for session indexing.

use crate::{GitContext, GitContextProvider, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// Discovers repository metadata without changing the repository or its worktrees.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitContextResolver;

impl GitContextProvider for GitContextResolver {
    fn context(&self, cwd: &Path) -> Result<GitContext> {
        let observed_at = SystemTime::now();
        let empty = || GitContext {
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            observed_at,
        };

        let Some(worktree) = git_value(cwd, ["rev-parse", "--show-toplevel"]) else {
            return Ok(empty());
        };
        let worktree =
            canonical_output_path(cwd, &worktree).unwrap_or_else(|| PathBuf::from(worktree));
        let Some(common_dir) = git_value(cwd, ["rev-parse", "--git-common-dir"]) else {
            return Ok(empty());
        };
        let common_dir =
            canonical_output_path(cwd, &common_dir).unwrap_or_else(|| PathBuf::from(common_dir));
        let repository_root = common_dir.parent().map(Path::to_path_buf);
        let repository = repository_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let branch = git_value(cwd, ["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let commit = git_value(cwd, ["rev-parse", "--verify", "HEAD"]);

        Ok(GitContext {
            repository,
            repository_root,
            worktree: Some(worktree),
            branch,
            commit,
            observed_at,
        })
    }
}

fn git_value<const N: usize>(cwd: &Path, args: [&str; N]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn canonical_output_path(cwd: &Path, value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    path.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    fn repository(temp: &TempDir) -> PathBuf {
        let repo = temp.path().join("repo with spaces [safe]");
        fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "--quiet"]);
        run(&repo, &["config", "user.email", "test@example.invalid"]);
        run(&repo, &["config", "user.name", "Agent History Test"]);
        fs::write(repo.join("file"), "content").unwrap();
        run(&repo, &["add", "file"]);
        run(&repo, &["commit", "--quiet", "-m", "initial"]);
        run(&repo, &["branch", "-M", "main"]);
        repo
    }

    #[test]
    fn captures_normal_repository_context() {
        let temp = TempDir::new("git-normal").unwrap();
        let repo = repository(&temp);
        let context = GitContextResolver.context(&repo).unwrap();
        assert_eq!(context.repository_root, Some(repo.canonicalize().unwrap()));
        assert_eq!(context.worktree, Some(repo.canonicalize().unwrap()));
        assert_eq!(context.branch.as_deref(), Some("main"));
        assert!(context
            .commit
            .as_ref()
            .is_some_and(|commit| commit.len() == 40));
        assert_eq!(
            context.repository,
            Some(repo.canonicalize().unwrap().to_string_lossy().into_owned())
        );
    }

    #[test]
    fn captures_linked_worktree_and_survives_its_deletion() {
        let temp = TempDir::new("git-linked").unwrap();
        let repo = repository(&temp);
        let linked = temp.path().join("linked worktree; [safe]");
        run(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "feature/prices",
                linked.to_str().unwrap(),
            ],
        );
        let linked_canonical = linked.canonicalize().unwrap();
        let context = GitContextResolver.context(&linked).unwrap();
        fs::remove_dir_all(&linked).unwrap();
        assert_eq!(context.repository_root, Some(repo.canonicalize().unwrap()));
        assert_eq!(context.worktree, Some(linked_canonical));
        assert_eq!(context.branch.as_deref(), Some("feature/prices"));
        assert!(context.commit.is_some());
        assert!(context.observed_at <= SystemTime::now());
    }

    #[test]
    fn detached_head_has_commit_without_branch() {
        let temp = TempDir::new("git-detached").unwrap();
        let repo = repository(&temp);
        run(&repo, &["checkout", "--quiet", "--detach", "HEAD"]);
        let context = GitContextResolver.context(&repo).unwrap();
        assert!(context.branch.is_none());
        assert!(context.commit.is_some());
    }

    #[test]
    fn non_git_and_missing_paths_are_empty_but_observed() {
        let temp = TempDir::new("git-empty").unwrap();
        let plain = temp.path().join("plain");
        fs::create_dir(&plain).unwrap();
        let missing = temp.path().join("missing");
        for path in [&plain, &missing] {
            let context = GitContextResolver.context(path).unwrap();
            assert!(context.repository_root.is_none());
            assert!(context.worktree.is_none());
            assert!(context.branch.is_none());
            assert!(context.commit.is_none());
            assert!(context.observed_at <= SystemTime::now());
        }
    }
}

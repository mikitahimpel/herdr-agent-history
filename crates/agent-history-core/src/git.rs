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

        let worktree = git_value(cwd, ["rev-parse", "--show-toplevel"]);
        let common_dir = git_value(cwd, ["rev-parse", "--git-common-dir"]);
        let git_dir = git_value(cwd, ["rev-parse", "--git-dir"]);
        let bare = git_value(cwd, ["rev-parse", "--is-bare-repository"]).as_deref() == Some("true");
        if worktree.is_none() && !bare {
            return Ok(empty());
        }
        let worktree = worktree
            .map(|path| canonical_output_path(cwd, &path).unwrap_or_else(|| PathBuf::from(path)));
        let linked = match (git_dir.as_deref(), common_dir.as_deref()) {
            (Some(git_dir), Some(common_dir)) => {
                canonical_output_path(cwd, git_dir) != canonical_output_path(cwd, common_dir)
            }
            _ => false,
        };
        let repository_root = linked
            .then(|| git_worktree_root(cwd))
            .flatten()
            .or_else(|| worktree.clone())
            .or_else(|| {
                common_dir
                    .as_deref()
                    .and_then(|path| canonical_output_path(cwd, path))
            })
            .or_else(|| common_dir.map(PathBuf::from));
        let repository = repository_root
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let branch = git_value(cwd, ["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let commit = git_value(cwd, ["rev-parse", "--verify", "HEAD"]);

        Ok(GitContext {
            repository,
            repository_root,
            worktree,
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
    let output = String::from_utf8_lossy(&output.stdout);
    let output = output.strip_suffix('\n').unwrap_or(&output);
    let value = output.strip_suffix('\r').unwrap_or(output).to_owned();
    (!value.is_empty()).then_some(value)
}

fn git_worktree_root(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let first = output
        .stdout
        .split(|byte| *byte == 0)
        .find_map(|record| record.strip_prefix(b"worktree "))?;
    let path = String::from_utf8_lossy(first);
    canonical_output_path(cwd, &path).or_else(|| Some(PathBuf::from(path.into_owned())))
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
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
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

    #[test]
    fn bare_repository_root_is_the_bare_repository() {
        let temp = TempDir::new("git-bare").unwrap();
        let bare = temp.path().join("bare repository");
        fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "--quiet", "--bare"]);
        let context = GitContextResolver.context(&bare).unwrap();
        assert_eq!(context.repository_root, Some(bare.canonicalize().unwrap()));
        assert_eq!(context.worktree, None);
    }

    #[test]
    fn linked_worktree_from_bare_repository_keeps_bare_root() {
        let temp = TempDir::new("git-bare-linked").unwrap();
        let source = repository(&temp);
        let bare = temp.path().join("bare repository");
        run(
            &source,
            &[
                "clone",
                "--quiet",
                "--bare",
                source.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let linked = temp.path().join("linked from bare");
        run(
            &bare,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "feature/bare",
                linked.to_str().unwrap(),
            ],
        );
        let context = GitContextResolver.context(&linked).unwrap();
        assert_eq!(context.repository_root, Some(bare.canonicalize().unwrap()));
        assert_eq!(context.worktree, Some(linked.canonicalize().unwrap()));
        assert_eq!(context.branch.as_deref(), Some("feature/bare"));
        assert!(context.commit.is_some());
    }

    #[test]
    fn separate_git_dir_uses_checkout_as_repository_root() {
        let temp = TempDir::new("git-separate").unwrap();
        let checkout = temp.path().join("checkout with trailing-space ");
        let git_dir = temp.path().join("metadata");
        fs::create_dir_all(&checkout).unwrap();
        run(
            &checkout,
            &[
                "init",
                "--quiet",
                "--separate-git-dir",
                git_dir.to_str().unwrap(),
            ],
        );
        run(&checkout, &["config", "user.email", "test@example.invalid"]);
        run(&checkout, &["config", "user.name", "Agent History Test"]);
        fs::write(checkout.join("file"), "content").unwrap();
        run(&checkout, &["add", "file"]);
        run(&checkout, &["commit", "--quiet", "-m", "initial"]);
        let context = GitContextResolver.context(&checkout).unwrap();
        assert_eq!(
            context.repository_root,
            Some(checkout.canonicalize().unwrap())
        );
        assert_eq!(context.worktree, Some(checkout.canonicalize().unwrap()));
        assert!(context.commit.is_some());
    }
}

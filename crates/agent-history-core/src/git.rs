//! Read-only Git context discovery for session indexing.

use crate::{GitContext, GitContextProvider, GitOrigin, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::SystemTime;

/// Discovers repository metadata without changing the repository or its worktrees.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitContextResolver;

impl GitContextProvider for GitContextResolver {
    fn context(&self, cwd: &Path) -> Result<GitContext> {
        let observed_at = SystemTime::now();
        let empty = || GitContext {
            origin: GitOrigin::Observed,
            repository: None,
            repository_root: None,
            repository_url: None,
            worktree: None,
            branch: None,
            commit: None,
            observed_at,
        };

        // Seven `git` runs resolved a cwd one at a time, and a scan resolves one cwd
        // per session directory. Almost none of that was Git working: an exec costs
        // tens of milliseconds of latency here against about one percent of a core,
        // so the cost was the starting, not the answering. Four of the values come
        // from a single `rev-parse`, and the other three do not depend on any of
        // them, so all four invocations are started together and the latency is paid
        // once for the cwd instead of once for each value.
        let locations = start(
            cwd,
            &[
                "rev-parse",
                "--git-dir",
                "--git-common-dir",
                "--is-bare-repository",
                "--show-toplevel",
            ],
        );
        let head_branch = start(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let head_commit = start(cwd, &["rev-parse", "--verify", "HEAD"]);
        let origin_url = start(cwd, &["config", "--get", "remote.origin.url"]);
        // `rev-parse` prints one line per option in the order it was given them and
        // stops at the first it cannot answer: a bare repository has no work tree, so
        // `--show-toplevel` fails there after the three before it have printed. The
        // lines it did print are therefore read by position and anything past the
        // failure stays unknown -- which is what the separate invocations left behind
        // too, since the one that failed was the only one that returned nothing.
        let locations = locations.lines();
        let line = |i: usize| locations.get(i).filter(|v| !v.is_empty()).cloned();
        let git_dir = line(0);
        let common_dir = line(1);
        let bare = line(2).as_deref() == Some("true");
        let worktree = line(3);
        // Collected whether or not they are wanted below: a child that is never waited
        // on stays a zombie for the life of the process.
        let branch = head_branch.value();
        let commit = head_commit.value();
        let repository_url = origin_url.value();
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
        // A label, not a location: `repository_root` still carries the path that
        // restoration needs. Recorded provenance already labels sessions `owner/name`,
        // so observed sessions derive the same shape instead of a full path.
        let repository = repository_url
            .as_deref()
            .and_then(crate::adapters::repository_label)
            .or_else(|| {
                repository_root
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            });

        Ok(GitContext {
            origin: GitOrigin::Observed,
            repository,
            repository_root,
            repository_url,
            worktree,
            branch,
            commit,
            observed_at,
        })
    }
}

/// Variables git exports to its own subprocesses, notably to hooks. Inheriting them
/// would resolve the surrounding repository instead of the session cwd, and would let
/// a `git` run in one repository read or write another.
///
/// Every `git` invocation in this workspace must clear these, including the ones that
/// mutate, such as `git worktree add` during confirmed recovery. Build commands with
/// [`git_command`] rather than repeating the list.
pub const INHERITED_GIT_ENVIRONMENT: [&str; 10] = [
    "GIT_DIR",
    "GIT_COMMON_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CEILING_DIRECTORIES",
    "GIT_QUARANTINE_PATH",
];

/// A `git` invocation scoped to `cwd` alone, with [`INHERITED_GIT_ENVIRONMENT`] cleared.
///
/// Use this for every `git` call, not only read-only ones: a command that inherits
/// `GIT_DIR` stages and commits in the surrounding repository instead of `cwd`.
pub fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd);
    for name in INHERITED_GIT_ENVIRONMENT {
        command.env_remove(name);
    }
    command
}

/// A `git` invocation that has been started but not yet collected.
struct Pending(Option<Child>);

/// Starts `git` under [`git_command`] without waiting for it, so that invocations
/// whose answers do not depend on each other overlap instead of queueing.
///
/// Standard error is discarded rather than inherited: a caller that asks a
/// question Git cannot answer, such as the work tree of a bare repository, reads
/// that from the exit status, and must not spill `fatal:` onto the terminal.
fn start(cwd: &Path, args: &[&str]) -> Pending {
    Pending(
        git_command(cwd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok(),
    )
}

impl Pending {
    /// Every line Git printed, whether or not it exited successfully, so that a
    /// multi-value invocation can be read up to the point where it gave up.
    fn lines(self) -> Vec<String> {
        let Some(child) = self.0 else {
            return Vec::new();
        };
        let Ok(output) = child.wait_with_output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
            .collect()
    }

    /// The one value Git printed, or none if it failed or printed nothing.
    fn value(self) -> Option<String> {
        let output = self.0?.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        let output = String::from_utf8_lossy(&output.stdout);
        let output = output.strip_suffix('\n').unwrap_or(&output);
        let value = output.strip_suffix('\r').unwrap_or(output).to_owned();
        (!value.is_empty()).then_some(value)
    }
}

fn git_worktree_root(cwd: &Path) -> Option<PathBuf> {
    let output = git_command(cwd)
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

    fn run(dir: &Path, args: &[&str]) {
        let status = crate::test_support::git_command(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    /// Git exports `GIT_DIR` and `GIT_INDEX_FILE` to hooks, and the pre-push hook runs
    /// this test suite. An invocation that inherited them would resolve, stage, and
    /// commit in the surrounding repository instead of the path it was given.
    #[test]
    fn git_invocations_do_not_inherit_a_surrounding_repository() {
        for command in [
            git_command(Path::new("/tmp")),
            crate::test_support::git_command(Path::new("/tmp")),
        ] {
            let cleared: Vec<&str> = command
                .get_envs()
                .filter(|(_, value)| value.is_none())
                .filter_map(|(name, _)| name.to_str())
                .collect();
            for name in INHERITED_GIT_ENVIRONMENT {
                assert!(cleared.contains(&name), "{name} is still inherited");
            }
        }
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
        // No remote configured, so the label falls back to the directory name.
        assert_eq!(
            context.repository.as_deref(),
            repo.canonicalize()
                .unwrap()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .as_deref()
        );
        assert_eq!(context.origin, crate::GitOrigin::Observed);
    }

    #[test]
    fn observed_repository_is_labelled_like_recorded_provenance() {
        let temp = TempDir::new("git-observed-label").unwrap();
        let repo = repository(&temp);
        run(
            &repo,
            &["remote", "add", "origin", "git@github.com:owner/name.git"],
        );
        let context = GitContextResolver.context(&repo).unwrap();
        // Rows would otherwise mix `owner/name` with a full filesystem path
        // depending on which provenance won.
        assert_eq!(context.repository.as_deref(), Some("owner/name"));
        assert_eq!(
            context.repository_url.as_deref(),
            Some("git@github.com:owner/name.git"),
            "the location restoration needs is kept verbatim"
        );
        assert_eq!(context.repository_root, Some(repo.canonicalize().unwrap()));
    }

    #[test]
    fn observed_context_reports_a_configured_remote() {
        let temp = TempDir::new("git-remote").unwrap();
        let repo = repository(&temp);
        assert_eq!(
            GitContextResolver.context(&repo).unwrap().repository_url,
            None
        );
        run(
            &repo,
            &["remote", "add", "origin", "git@github.com:owner/name.git"],
        );
        let context = GitContextResolver.context(&repo).unwrap();
        assert_eq!(
            context.repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );
        assert_eq!(context.origin, crate::GitOrigin::Observed);
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

    const LOCATIONS: [&str; 5] = [
        "rev-parse",
        "--git-dir",
        "--git-common-dir",
        "--is-bare-repository",
        "--show-toplevel",
    ];

    /// The four locations come from one `rev-parse`, read by position. `rev-parse`
    /// answers the options it can and stops at the first it cannot -- a bare
    /// repository has no work tree -- so the prefix it printed must still be read
    /// rather than thrown away with the exit status, and an option must never be
    /// answered with the line belonging to another.
    #[test]
    fn a_partly_answered_rev_parse_still_yields_the_options_it_printed() {
        let temp = TempDir::new("git-locations").unwrap();
        let repo = repository(&temp);
        let lines = start(&repo, &LOCATIONS).lines();
        assert_eq!(
            lines.len(),
            4,
            "a work tree answers every option: {lines:?}"
        );
        assert_eq!(lines[2], "false");
        assert_eq!(
            canonical_output_path(&repo, &lines[3]),
            Some(repo.canonicalize().unwrap()),
            "the work tree belongs in the fourth line: {lines:?}"
        );

        let bare = temp.path().join("bare repository");
        fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "--quiet", "--bare"]);
        let lines = start(&bare, &LOCATIONS).lines();
        assert_eq!(
            lines.len(),
            3,
            "a bare repository must answer the three before the work tree: {lines:?}"
        );
        assert_eq!(lines[2], "true");

        let plain = temp.path().join("not a repository");
        fs::create_dir_all(&plain).unwrap();
        assert!(
            start(&plain, &LOCATIONS).lines().is_empty(),
            "outside a repository the first option already fails"
        );
    }

    /// A value whose own invocation failed is absent, not empty or borrowed from
    /// elsewhere: `symbolic-ref` prints nothing at a detached head.
    #[test]
    fn a_failed_invocation_yields_no_value() {
        let temp = TempDir::new("git-no-value").unwrap();
        let repo = repository(&temp);
        assert_eq!(
            start(&repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).value(),
            Some("main".to_owned())
        );
        run(&repo, &["checkout", "--quiet", "--detach"]);
        assert_eq!(
            start(&repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).value(),
            None
        );
        assert_eq!(
            start(&repo, &["config", "--get", "remote.origin.url"]).value(),
            None,
            "an unset configuration key is not an empty string"
        );
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

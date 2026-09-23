//! Whether a session's code still exists on this machine, and which
//! recovery choices that leaves. Filesystem existence checks only: no Git
//! subprocess, no host knowledge, and no mutation. Recorded paths come from
//! transcripts and are untrusted, so only clean absolute paths are examined.
use crate::{CoreError, Result, Session};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryChoice {
    RecreateWorktree,
    ExistingRepository,
    ViewConversation,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryOptions {
    pub choices: Vec<RecoveryChoice>,
}

/// Choices for a session whose working directory is gone.
pub fn recovery_options(
    session: &Session,
    repository_exists: bool,
    worktree_exists: bool,
) -> RecoveryOptions {
    let mut choices = Vec::new();
    if repository_exists && !worktree_exists && recreation_target(session).is_ok() {
        choices.push(RecoveryChoice::RecreateWorktree)
    }
    if repository_exists {
        choices.push(RecoveryChoice::ExistingRepository)
    }
    choices.extend([RecoveryChoice::ViewConversation, RecoveryChoice::Cancel]);
    RecoveryOptions { choices }
}

/// An absolute path made only of root and normal components, outside any
/// `.git` directory. Anything else is refused rather than examined. The raw
/// bytes are checked because `Path::components` silently drops `.` segments
/// and repeated separators.
pub fn clean_absolute(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Some(rest) = path.as_os_str().as_bytes().strip_prefix(b"/") else {
        return false;
    };
    rest.is_empty()
        || rest
            .split(|b| *b == b'/')
            .all(|segment| !matches!(segment, b"" | b"." | b".." | b".git"))
            && path
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

/// The recorded facts a detached worktree recreation would use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecreationTarget {
    pub repository_root: PathBuf,
    pub worktree: PathBuf,
    pub commit: String,
}

fn fail(message: &str) -> CoreError {
    CoreError::Unsupported(message.into())
}

/// Validates, without touching the filesystem, that the session records
/// enough to recreate its worktree safely.
pub fn recreation_target(session: &Session) -> Result<RecreationTarget> {
    let root = session
        .repository_root
        .clone()
        .ok_or_else(|| fail("repository root is unavailable"))?;
    let target = session
        .worktree
        .clone()
        .ok_or_else(|| fail("recorded worktree path is unavailable"))?;
    let commit = session
        .commit
        .as_deref()
        .filter(|v| matches!(v.len(), 40 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            fail("a recorded full commit hash is required for safe detached recreation")
        })?;
    if !clean_absolute(&root) || !clean_absolute(&target) || root == target {
        return Err(fail("invalid recorded repository or worktree path"));
    }
    if session
        .cwd
        .as_deref()
        .is_none_or(|cwd| !clean_absolute(cwd) || !cwd.starts_with(&target))
    {
        return Err(fail("session cwd is outside the recorded worktree"));
    }
    Ok(RecreationTarget {
        repository_root: root,
        worktree: target,
        commit: commit.into(),
    })
}

/// How much of a session survives on this machine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Availability {
    /// The recorded working directory exists.
    OnDisk,
    /// The working directory is gone but its repository is on disk, so the
    /// worktree can be recreated or the session opened in the repository.
    Recoverable,
    /// Nothing is on disk, but the session records which repository it was.
    RepositoryKnown,
    /// No working directory and no Git context: only the transcript remains.
    TranscriptOnly,
}

/// What a probe found at a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathState {
    Missing,
    File,
    Directory,
}

/// Classifies `session` with real `stat` calls.
pub fn availability(session: &Session) -> Availability {
    availability_with(session, |path| match std::fs::metadata(path) {
        Ok(md) if md.is_dir() => PathState::Directory,
        Ok(_) => PathState::File,
        Err(_) => PathState::Missing,
    })
}

/// Classifies `session`, calling `probe` only for clean absolute paths.
pub fn availability_with(
    session: &Session,
    mut probe: impl FnMut(&Path) -> PathState,
) -> Availability {
    let mut state = |path: Option<&Path>| match path {
        Some(p) if clean_absolute(p) => probe(p),
        _ => PathState::Missing,
    };
    if state(session.cwd.as_deref()) == PathState::Directory {
        return Availability::OnDisk;
    }
    let repository_exists = state(session.repository_root.as_deref()) == PathState::Directory;
    let worktree_exists = state(session.worktree.as_deref()) != PathState::Missing;
    let options = recovery_options(session, repository_exists, worktree_exists);
    if options.choices.iter().any(|c| {
        matches!(
            c,
            RecoveryChoice::RecreateWorktree | RecoveryChoice::ExistingRepository
        )
    }) {
        return Availability::Recoverable;
    }
    let provenance = session.repository.is_some()
        || session.repository_root.is_some()
        || session.commit.is_some()
        || session.branch.is_some();
    if provenance {
        Availability::RepositoryKnown
    } else {
        Availability::TranscriptOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Agent, SessionId, SourceRef};
    use std::{cell::RefCell, collections::HashMap};

    fn session() -> Session {
        Session {
            id: SessionId {
                agent: Agent::Claude,
                native_id: "00000000-0000-4000-8000-000000000001".into(),
            },
            source: SourceRef::new("/history/s.jsonl", 1, 1, 0..1).unwrap(),
            cwd: None,
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            started_at: None,
            ended_at: None,
            git_observed_at: None,
        }
    }

    fn classify(session: &Session, disk: &[(&str, PathState)]) -> (Availability, Vec<PathBuf>) {
        let disk: HashMap<PathBuf, PathState> =
            disk.iter().map(|(p, s)| (PathBuf::from(p), *s)).collect();
        let probed = RefCell::new(Vec::new());
        let a = availability_with(session, |p| {
            probed.borrow_mut().push(p.to_path_buf());
            disk.get(p).copied().unwrap_or(PathState::Missing)
        });
        (a, probed.into_inner())
    }

    fn worktree_session() -> Session {
        Session {
            cwd: Some("/work/wt/sub".into()),
            repository: Some("repo".into()),
            repository_root: Some("/work/repo".into()),
            worktree: Some("/work/wt".into()),
            branch: Some("feature".into()),
            commit: Some("a".repeat(40)),
            ..session()
        }
    }

    #[test]
    fn existing_working_directory_is_on_disk() {
        let (a, probed) = classify(
            &worktree_session(),
            &[("/work/wt/sub", PathState::Directory)],
        );
        assert_eq!(a, Availability::OnDisk);
        assert_eq!(probed, [PathBuf::from("/work/wt/sub")]);
    }

    #[test]
    fn missing_worktree_with_repository_is_recoverable() {
        let s = worktree_session();
        let (a, _) = classify(&s, &[("/work/repo", PathState::Directory)]);
        assert_eq!(a, Availability::Recoverable);
        let options = recovery_options(&s, true, false);
        assert!(options.choices.contains(&RecoveryChoice::RecreateWorktree));
        // Without a full commit it cannot be recreated, but the repository
        // can still host the session.
        let s = Session {
            commit: Some("abc".into()),
            ..worktree_session()
        };
        assert_eq!(
            classify(&s, &[("/work/repo", PathState::Directory)]).0,
            Availability::Recoverable
        );
        assert_eq!(
            recovery_options(&s, true, false).choices,
            [
                RecoveryChoice::ExistingRepository,
                RecoveryChoice::ViewConversation,
                RecoveryChoice::Cancel
            ]
        );
    }

    #[test]
    fn recorded_provenance_without_a_repository_is_known_not_openable() {
        let s = Session {
            cwd: Some("/gone".into()),
            repository: Some("github.com/example/repo".into()),
            commit: Some("b".repeat(40)),
            ..session()
        };
        assert_eq!(classify(&s, &[]).0, Availability::RepositoryKnown);
        let bare = Session {
            cwd: Some("/gone".into()),
            ..session()
        };
        assert_eq!(classify(&bare, &[]).0, Availability::TranscriptOnly);
        assert_eq!(classify(&session(), &[]).0, Availability::TranscriptOnly);
    }

    #[test]
    fn unclean_recorded_paths_are_refused_without_being_examined() {
        for bad in [
            "relative/dir",
            "/work/../etc",
            "/work/./wt",
            "/work//wt",
            "/work/wt/",
            "/work/repo/.git",
            "/work/repo/.git/worktrees",
            "",
        ] {
            let s = Session {
                cwd: Some(bad.into()),
                repository_root: Some(bad.into()),
                worktree: Some(bad.into()),
                ..session()
            };
            let (a, probed) = classify(&s, &[(bad, PathState::Directory)]);
            assert!(probed.is_empty(), "{bad:?} was probed: {probed:?}");
            assert_ne!(a, Availability::OnDisk);
            assert_ne!(a, Availability::Recoverable);
        }
    }

    #[test]
    fn clean_absolute_accepts_plain_paths() {
        assert!(clean_absolute(Path::new("/")));
        assert!(clean_absolute(Path::new("/Users/me/work tree/.github")));
        assert!(!clean_absolute(Path::new("/Users/me/.git")));
    }

    #[test]
    fn real_stat_uses_the_filesystem() {
        let temp = crate::test_support::TempDir::new("availability").unwrap();
        let s = Session {
            cwd: Some(temp.path().to_path_buf()),
            ..session()
        };
        assert_eq!(availability(&s), Availability::OnDisk);
        let gone = Session {
            cwd: Some(temp.path().join("missing")),
            ..session()
        };
        assert_eq!(availability(&gone), Availability::TranscriptOnly);
    }
}

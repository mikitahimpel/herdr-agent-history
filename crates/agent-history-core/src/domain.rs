use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct InvalidSourceRange;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Agent {
    Claude,
    Codex,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId {
    pub agent: Agent,
    pub native_id: String,
}
impl SessionId {
    pub fn new(agent: Agent, native_id: impl Into<String>) -> Self {
        Self {
            agent,
            native_id: native_id.into(),
        }
    }
}

/// `byte_range` is a half-open byte range and is valid only for `file_id` and `generation`.
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SourceRef {
    pub path: PathBuf,
    pub file_id: u64,
    pub generation: u64,
    pub byte_range: Range<u64>,
}
impl SourceRef {
    pub fn new(
        path: impl Into<PathBuf>,
        file_id: u64,
        generation: u64,
        byte_range: Range<u64>,
    ) -> Result<Self, InvalidSourceRange> {
        if byte_range.start > byte_range.end {
            return Err(InvalidSourceRange);
        }
        Ok(Self {
            path: path.into(),
            file_id,
            generation,
            byte_range,
        })
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    User,
    Assistant,
    ToolResult,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct NormalizedEvent {
    pub session_id: SessionId,
    pub kind: EventKind,
    pub timestamp: Option<SystemTime>,
    pub source: SourceRef,
    pub text: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SessionFile {
    pub path: PathBuf,
    pub file_id: u64,
    pub generation: u64,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionMetadataPatch {
    pub native_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub started_at: Option<SystemTime>,
    /// Git facts the record carries about itself, if any.
    pub git: Option<RecordedGit>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct ParsedRecord {
    pub metadata: SessionMetadataPatch,
    pub events: Vec<NormalizedEvent>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub id: SessionId,
    pub source: SourceRef,
    pub cwd: Option<PathBuf>,
    pub repository: Option<String>,
    pub repository_root: Option<PathBuf>,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub started_at: Option<SystemTime>,
    pub ended_at: Option<SystemTime>,
    pub git_observed_at: Option<SystemTime>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct ConversationChunk {
    pub session_id: SessionId,
    pub kind: EventKind,
    pub ordinal: u64,
    pub timestamp: Option<SystemTime>,
    pub source: SourceRef,
    pub text: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub session_id: SessionId,
    pub agent: Agent,
    pub repository: Option<String>,
    /// Remote URL when the session's Git facts were recorded rather than observed.
    pub repository_url: Option<String>,
    /// Which kind of Git fact `repository`, `branch`, and `commit` are.
    pub git_origin: Option<GitOrigin>,
    pub branch: Option<String>,
    pub cwd: Option<PathBuf>,
    pub timestamp: Option<SystemTime>,
    pub kind: EventKind,
    pub source: SourceRef,
    pub snippet: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct IndexedFile {
    pub path: PathBuf,
    pub file_id: u64,
    pub generation: u64,
    pub committed_offset: u64,
    pub size: u64,
    pub modified: Option<SystemTime>,
}
/// Where a persisted Git fact came from.
///
/// The two are never interchangeable: `Observed` describes the repository as it
/// was when the indexer ran, `Recorded` repeats what the agent wrote into its own
/// transcript while the session was live.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitOrigin {
    /// Resolved by running read-only `git` against the session cwd while indexing.
    Observed,
    /// Copied from the native transcript. A historical claim, not a current fact,
    /// and it never carries a local `repository_root`.
    Recorded,
}

/// Git facts a native transcript records about its own session.
#[derive(Serialize, Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordedGit {
    /// Remote URL exactly as the agent wrote it. Never a local path.
    pub repository_url: Option<String>,
    /// Display identity such as `owner/name`, derived from what was recorded.
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}
impl RecordedGit {
    pub fn is_empty(&self) -> bool {
        self.repository_url.is_none()
            && self.repository.is_none()
            && self.branch.is_none()
            && self.commit.is_none()
    }
    /// Keeps the earliest recorded value for each field, so a branch switch late in
    /// a session cannot rewrite the context the session started in.
    pub fn fill_missing(&mut self, other: &RecordedGit) {
        for (slot, value) in [
            (&mut self.repository_url, &other.repository_url),
            (&mut self.repository, &other.repository),
            (&mut self.branch, &other.branch),
            (&mut self.commit, &other.commit),
        ] {
            if slot.is_none() {
                slot.clone_from(value);
            }
        }
    }
    /// Presents recorded provenance as a context. `repository_root` and `worktree`
    /// stay empty because a remote URL says which repository, never where it lived.
    pub fn as_context(&self, captured_at: SystemTime) -> Option<GitContext> {
        (!self.is_empty()).then(|| GitContext {
            origin: GitOrigin::Recorded,
            repository: self.repository.clone(),
            repository_root: None,
            repository_url: self.repository_url.clone(),
            worktree: None,
            branch: self.branch.clone(),
            commit: self.commit.clone(),
            observed_at: captured_at,
        })
    }
}

/// `observed_at` is when the indexer captured this context, which for
/// `GitOrigin::Recorded` is when the transcript was read and not when the facts held.
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct GitContext {
    pub origin: GitOrigin,
    pub repository: Option<String>,
    pub repository_root: Option<PathBuf>,
    /// Remote URL; only recorded provenance supplies one on most sessions.
    pub repository_url: Option<String>,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub observed_at: SystemTime,
}

/// Provenance persisted beside a session's flat Git fields.
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct GitProvenance {
    pub origin: GitOrigin,
    /// Remote URL when one was recorded. Never a local path, and never a
    /// substitute for `Session::repository_root`.
    pub repository_url: Option<String>,
}

/// A session together with the provenance of its Git fields.
///
/// `Session` keeps the shape older callers depend on; ask for the record when the
/// difference between an observed and a recorded fact matters, such as before
/// offering to recreate a deleted worktree.
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub session: Session,
    pub git: Option<GitProvenance>,
}
impl SessionRecord {
    pub fn origin(&self) -> Option<GitOrigin> {
        self.git.as_ref().map(|g| g.origin)
    }
    /// True when the repository and commit are known but the local path is not.
    pub fn is_recorded_only(&self) -> bool {
        self.origin() == Some(GitOrigin::Recorded)
    }
}
impl From<Session> for SessionRecord {
    fn from(session: Session) -> Self {
        Self { session, git: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;
    #[test]
    fn agent_qualifies_identity() {
        assert_ne!(
            SessionId::new(Agent::Claude, "same"),
            SessionId::new(Agent::Codex, "same")
        );
    }
    #[test]
    fn ranges_track_generation() {
        let s = SourceRef::new("x", 7, 2, 10..20).unwrap();
        assert_eq!(s.byte_range, 10..20);
        assert_ne!(s, SourceRef::new("x", 7, 3, 10..20).unwrap());
    }
    #[test]
    fn context_is_optional() {
        let s = Session {
            id: SessionId::new(Agent::Codex, "id"),
            source: SourceRef::new("x", 1, 0, 0..0).unwrap(),
            cwd: None,
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            started_at: None,
            ended_at: None,
            git_observed_at: None,
        };
        assert!(s.cwd.is_none());
    }
    #[test]
    fn recorded_git_keeps_the_earliest_value_for_each_field() {
        let mut first = RecordedGit {
            branch: Some("feature/prices".into()),
            ..Default::default()
        };
        first.fill_missing(&RecordedGit {
            repository_url: Some("git@github.com:owner/name.git".into()),
            repository: Some("owner/name".into()),
            branch: Some("main".into()),
            commit: Some("a".repeat(40)),
        });
        assert_eq!(first.branch.as_deref(), Some("feature/prices"));
        assert_eq!(first.repository.as_deref(), Some("owner/name"));
        assert_eq!(first.commit, Some("a".repeat(40)));
    }
    #[test]
    fn recorded_context_never_fabricates_a_local_path() {
        assert_eq!(RecordedGit::default().as_context(UNIX_EPOCH), None);
        let context = RecordedGit {
            repository_url: Some("git@github.com:owner/name.git".into()),
            repository: Some("owner/name".into()),
            branch: None,
            commit: Some("b".repeat(40)),
        }
        .as_context(UNIX_EPOCH)
        .unwrap();
        assert_eq!(context.origin, GitOrigin::Recorded);
        assert!(context.repository_root.is_none());
        assert!(context.worktree.is_none());
        assert_eq!(
            context.repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );
    }
    #[test]
    fn a_session_record_defaults_to_unlabelled_provenance() {
        let record = SessionRecord::from(Session {
            id: SessionId::new(Agent::Claude, "id"),
            source: SourceRef::new("x", 1, 0, 0..0).unwrap(),
            cwd: None,
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            started_at: None,
            ended_at: None,
            git_observed_at: None,
        });
        assert_eq!(record.origin(), None);
        assert!(!record.is_recorded_only());
    }
    #[test]
    fn invalid_ranges_are_rejected() {
        let start = 4;
        let end = 3;
        assert!(SourceRef::new("x", 1, 0, start..end).is_err());
    }
}

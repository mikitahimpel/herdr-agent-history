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
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct GitContext {
    pub repository: Option<String>,
    pub repository_root: Option<PathBuf>,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub observed_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn invalid_ranges_are_rejected() {
        let start = 4;
        let end = 3;
        assert!(SourceRef::new("x", 1, 0, start..end).is_err());
    }
}

use crate::domain::*;
use std::error::Error;
use std::fmt;
use std::path::Path;
pub type Result<T> = std::result::Result<T, CoreError>;
#[derive(Debug)]
pub enum CoreError {
    InvalidRecord(String),
    Io(std::io::Error),
    Unsupported(String),
    Storage(String),
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecord(s) => write!(f, "invalid record: {s}"),
            Self::Io(e) => e.fmt(f),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
        }
    }
}
impl Error for CoreError {}
impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for CoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value.to_string())
    }
}
pub trait AgentAdapter {
    fn agent(&self) -> Agent;
    fn discover(&self) -> Result<Vec<SessionFile>>;
    fn parse_record(
        &self,
        session: &Session,
        record: &[u8],
        source: SourceRef,
    ) -> Result<ParsedRecord>;
}
impl<T: AgentAdapter + ?Sized> AgentAdapter for &T {
    fn agent(&self) -> Agent {
        (*self).agent()
    }
    fn discover(&self) -> Result<Vec<SessionFile>> {
        (*self).discover()
    }
    fn parse_record(&self, s: &Session, r: &[u8], src: SourceRef) -> Result<ParsedRecord> {
        (*self).parse_record(s, r, src)
    }
}
pub trait GitContextProvider {
    fn context(&self, cwd: &Path) -> Result<GitContext>;
}
pub trait SessionResumer {
    fn resume(&self, session: &Session, workspace: &Path) -> Result<()>;
}
/// A transaction commits derived chunks and the last complete-record offset atomically.
#[derive(Clone, Debug, Default)]
pub struct IndexBatch {
    pub file: Option<IndexedFile>,
    /// Compare-and-swap file progress; Some(None) requires a new path.
    pub expected_file: Option<Option<IndexedFile>>,
    pub sessions: Vec<Session>,
    pub chunks: Vec<ConversationChunk>,
    pub replaced_chunks: Vec<(SessionId, u64)>,
    /// Source identities whose derived chunks must be removed before inserts.
    pub removed_sources: Vec<(u64, u64)>,
    pub open_turn_state: Option<Vec<u8>>,
}
pub trait Store {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    /// Commits file progress, metadata, removals, inserts, and open-turn state atomically.
    fn commit_batch(&mut self, batch: IndexBatch) -> Result<()>;
}
impl<T: Store + ?Sized> Store for &mut T {
    fn search(&self, q: &str, l: usize) -> Result<Vec<SearchResult>> {
        (**self).search(q, l)
    }
    fn commit_batch(&mut self, b: IndexBatch) -> Result<()> {
        (**self).commit_batch(b)
    }
}
pub trait IndexStore: Store {
    fn next_file_id(&self) -> Result<u64>;
    fn indexed_file_state(&self, path: &Path) -> Result<Option<(IndexedFile, Option<Vec<u8>>)>>;
    fn sessions(&self) -> Result<Vec<Session>>;
    fn session(&self, id: &SessionId) -> Result<Option<Session>>;
}
impl<T: IndexStore + ?Sized> IndexStore for &mut T {
    fn next_file_id(&self) -> Result<u64> {
        (**self).next_file_id()
    }
    fn indexed_file_state(&self, p: &Path) -> Result<Option<(IndexedFile, Option<Vec<u8>>)>> {
        (**self).indexed_file_state(p)
    }
    fn sessions(&self) -> Result<Vec<Session>> {
        (**self).sessions()
    }
    fn session(&self, id: &SessionId) -> Result<Option<Session>> {
        (**self).session(id)
    }
}

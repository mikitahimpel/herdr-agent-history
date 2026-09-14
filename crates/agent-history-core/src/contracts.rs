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
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecord(s) => write!(f, "invalid record: {s}"),
            Self::Io(e) => e.fmt(f),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}
impl Error for CoreError {}
impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
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
pub trait GitContextProvider {
    fn context(&self, cwd: &Path) -> Result<GitContext>;
}
pub trait SessionResumer {
    fn resume(&self, session: &Session, workspace: &Path) -> Result<()>;
}
/// A transaction commits derived chunks and the last complete-record offset atomically.
pub trait StoreTransaction {
    fn upsert_session(&mut self, session: &Session) -> Result<()>;
    fn append_chunk(&mut self, chunk: &ConversationChunk) -> Result<()>;
    fn set_committed_offset(&mut self, offset: u64) -> Result<()>;
    fn commit(self: Box<Self>) -> Result<()>;
}
#[derive(Clone, Debug, Default)]
pub struct IndexBatch {
    pub file: Option<IndexedFile>,
    pub sessions: Vec<Session>,
    pub chunks: Vec<ConversationChunk>,
    pub replaced_chunks: Vec<(SessionId, u64)>,
    pub open_turn_state: Option<Vec<u8>>,
}
pub trait Store {
    fn begin(&mut self, file: &IndexedFile) -> Result<Box<dyn StoreTransaction + '_>>;
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    fn commit_batch(&mut self, batch: IndexBatch) -> Result<()>;
}

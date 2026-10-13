use crate::domain::*;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
pub type Result<T> = std::result::Result<T, CoreError>;
#[derive(Debug)]
pub enum CoreError {
    InvalidRecord(String),
    Io(std::io::Error),
    Unsupported(String),
    Storage(String),
    /// The source was written to while it was being read. Nothing from the attempt
    /// was committed, and the next scan reads the file again from its last
    /// committed offset.
    SourceChanged,
    /// The source no longer matches what the index recorded for it, usually
    /// because it grew after the last scan. Indexing it again resolves this;
    /// nothing is wrong with the source.
    StaleSource,
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecord(s) => write!(f, "invalid record: {s}"),
            Self::Io(e) => e.fmt(f),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
            Self::SourceChanged => write!(f, "source changed during indexing; retry"),
            Self::StaleSource => write!(f, "source generation is stale; index again"),
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
    /// Like `discover`, but may give up once `stop` is set. The list is then
    /// incomplete and must not be treated as the full set of sessions.
    fn discover_until(&self, stop: &AtomicBool) -> Result<Vec<SessionFile>> {
        let _ = stop;
        self.discover()
    }
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
    fn discover_until(&self, stop: &AtomicBool) -> Result<Vec<SessionFile>> {
        (*self).discover_until(stop)
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
/// Supplies knowledge core does not have about which sessions matter most
/// right now, as [`Priorities`] for [`crate::SqliteStore::search_prioritized`].
///
/// Asked when the caller's view of the sessions changes, never per search:
/// an implementation may query another process here, and the search path only
/// reads the set it returned. That set can go stale before it is used, so
/// nothing may act on a session merely because it was prioritized.
pub trait ResultPrioritizer {
    fn prioritize(&mut self, sessions: &[Session]) -> Priorities;
}
/// Caller-assigned groups that lift sessions above relevance order. Core never
/// interprets a group: lower numbers come first, and that is all.
///
/// For each prioritized session with a matching chunk, its best match is placed
/// ahead of the ordinary results, ordered by group and then by relevance. The
/// ordinary results are what the same search returns without priorities. An
/// empty set leaves search exactly as it is without one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Priorities {
    groups: HashMap<SessionId, u32>,
}
impl Priorities {
    pub fn group(&self, id: &SessionId) -> Option<u32> {
        self.groups.get(id).copied()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    pub fn len(&self) -> usize {
        self.groups.len()
    }
    /// Sessions by group, then by ID: the order a bounded query keeps them in.
    pub fn ordered(&self) -> Vec<(&SessionId, u32)> {
        let mut entries: Vec<_> = self.groups.iter().map(|(id, g)| (id, *g)).collect();
        entries.sort_unstable_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        entries
    }
}
impl FromIterator<(SessionId, u32)> for Priorities {
    /// A session listed twice keeps its lowest group.
    fn from_iter<I: IntoIterator<Item = (SessionId, u32)>>(iter: I) -> Self {
        let mut groups = HashMap::new();
        for (id, group) in iter {
            groups
                .entry(id)
                .and_modify(|g: &mut u32| *g = (*g).min(group))
                .or_insert(group);
        }
        Self { groups }
    }
}
/// A transaction commits derived chunks and the last complete-record offset atomically.
#[derive(Clone, Debug, Default)]
pub struct IndexBatch {
    pub file: Option<IndexedFile>,
    /// The `stat` of `file` when its committed bytes were read, so that a later scan
    /// can recognise an untouched file without opening it. Absent means a later scan
    /// must compare the file the long way.
    pub fingerprint: Option<FileFingerprint>,
    /// Compare-and-swap file progress; Some(None) requires a new path.
    pub expected_file: Option<Option<IndexedFile>>,
    /// Sessions with the provenance of their Git fields; `Session` converts with `into()`.
    pub sessions: Vec<SessionRecord>,
    pub chunks: Vec<ConversationChunk>,
    pub replaced_chunks: Vec<(SessionId, u64)>,
    /// Source identities whose derived chunks must be removed before inserts.
    pub removed_sources: Vec<(u64, u64)>,
    pub open_turn_state: Option<Vec<u8>>,
}
pub trait Store {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    fn search_with_role(
        &self,
        query: &str,
        limit: usize,
        kind: Option<EventKind>,
    ) -> Result<Vec<SearchResult>> {
        if kind.is_some() {
            return Err(CoreError::Unsupported(
                "this store does not support role filters".into(),
            ));
        }
        self.search(query, limit)
    }
    /// Commits file progress, metadata, removals, inserts, and open-turn state atomically.
    fn commit_batch(&mut self, batch: IndexBatch) -> Result<()>;
}
impl<T: Store + ?Sized> Store for &mut T {
    fn search(&self, q: &str, l: usize) -> Result<Vec<SearchResult>> {
        (**self).search(q, l)
    }
    fn search_with_role(
        &self,
        q: &str,
        l: usize,
        k: Option<EventKind>,
    ) -> Result<Vec<SearchResult>> {
        (**self).search_with_role(q, l, k)
    }
    fn commit_batch(&mut self, b: IndexBatch) -> Result<()> {
        (**self).commit_batch(b)
    }
}
pub trait IndexStore: Store {
    fn next_file_id(&self) -> Result<u64>;
    fn indexed_file_state(&self, path: &Path) -> Result<Option<(IndexedFile, Option<Vec<u8>>)>>;
    /// Every committed file that carries a fingerprint, by path, in one query.
    ///
    /// This exists so that a scan can decide what to skip from `stat` alone, without a
    /// query, a checkpoint blob, or an open file per candidate. Omitting a path is
    /// always safe: it only costs that file the full comparison it would have had
    /// anyway, which is why the default implementation returns nothing.
    fn committed_files(&self) -> Result<HashMap<PathBuf, CommittedFile>> {
        Ok(HashMap::new())
    }
    /// Records `fingerprint` for a file whose bytes are already committed, without
    /// touching anything derived from them.
    ///
    /// This is how an index written before fingerprints existed earns one: the first
    /// scan after the upgrade still compares such a file the long way, finds it
    /// unchanged, and leaves this behind so no later scan has to. Implementations must
    /// apply it only while `file` is still exactly the committed row, so that a scan
    /// racing a writer cannot certify bytes that have since moved.
    fn record_fingerprint(
        &mut self,
        file: &IndexedFile,
        fingerprint: FileFingerprint,
    ) -> Result<()> {
        let _ = (file, fingerprint);
        Ok(())
    }
    fn sessions(&self) -> Result<Vec<Session>>;
    fn session(&self, id: &SessionId) -> Result<Option<Session>>;
    /// The session with the origin of its Git fields. Stores that cannot distinguish
    /// observed from recorded provenance report none.
    fn session_record(&self, id: &SessionId) -> Result<Option<SessionRecord>> {
        Ok(self.session(id)?.map(SessionRecord::from))
    }
    /// Releases free pages left by rebuilds; returns how many were released.
    fn reclaim_free_pages(&mut self) -> Result<u64> {
        Ok(0)
    }
}
impl<T: IndexStore + ?Sized> IndexStore for &mut T {
    fn next_file_id(&self) -> Result<u64> {
        (**self).next_file_id()
    }
    fn indexed_file_state(&self, p: &Path) -> Result<Option<(IndexedFile, Option<Vec<u8>>)>> {
        (**self).indexed_file_state(p)
    }
    fn committed_files(&self) -> Result<HashMap<PathBuf, CommittedFile>> {
        (**self).committed_files()
    }
    fn record_fingerprint(&mut self, f: &IndexedFile, p: FileFingerprint) -> Result<()> {
        (**self).record_fingerprint(f, p)
    }
    fn sessions(&self) -> Result<Vec<Session>> {
        (**self).sessions()
    }
    fn session(&self, id: &SessionId) -> Result<Option<Session>> {
        (**self).session(id)
    }
    fn session_record(&self, id: &SessionId) -> Result<Option<SessionRecord>> {
        (**self).session_record(id)
    }
    fn reclaim_free_pages(&mut self) -> Result<u64> {
        (**self).reclaim_free_pages()
    }
}

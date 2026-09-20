//! Transactional indexing. Native files are opened read-only.
use crate::{
    chunks::ChunkBuilder, AgentAdapter, CoreError, GitContextProvider, GitOrigin, GitProvenance,
    IndexBatch, IndexStore, IndexedFile, RecordedGit, Result, Session, SessionFile, SessionId,
    SessionRecord, SourceRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Path,
    time::SystemTime,
};
const CHECKPOINT_FORMAT_VERSION: u32 = 2;
pub const DEFAULT_MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
const PROGRESS_RECORD_INTERVAL: u64 = 128;
const PROGRESS_BYTE_INTERVAL: u64 = 1024 * 1024;
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexReport {
    pub bytes_read: u64,
    pub records: u64,
    pub malformed_records: u64,
    pub chunks: u64,
    pub skipped: bool,
    pub files: u64,
    pub failed_files: u64,
    pub errors: Vec<String>,
}
/// A snapshot of indexing work, safe to display without exposing source paths or transcript text.
///
/// Counts are cumulative for the complete call: `total_files` is the number of discovered files,
/// while the `agent_*` fields apply to the agent named by `agent`. Completed files include failed
/// attempts; `failed_files` counts failed discovery or file attempts. `records`, `bytes_read`, and
/// `chunks` count work observed so far, including work in the current file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexProgress {
    pub agent: crate::Agent,
    pub completed_files: u64,
    pub total_files: u64,
    pub agent_completed_files: u64,
    pub agent_total_files: u64,
    pub bytes_read: u64,
    pub records: u64,
    pub chunks: u64,
    pub failed_files: u64,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    #[serde(default)]
    pub format_version: u32,
    pub session: Session,
    /// Provenance of `session`'s Git fields. Absent in checkpoints written before
    /// provenance was labelled, where only live observation could fill them.
    #[serde(default)]
    pub git: Option<GitProvenance>,
    /// Git facts recovered from the transcript so far, preserved across appends.
    #[serde(default)]
    pub recorded_git: RecordedGit,
    pub builder: Vec<u8>,
    pub dev: u64,
    pub ino: u64,
    pub ctime: i64,
    pub ctime_nsec: i64,
    pub head: Vec<u8>,
    pub tail: Vec<u8>,
    pub sampled_size: u64,
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Checkpoint> {
    serde_json::from_slice(bytes)
        .map_err(|_| CoreError::Storage("invalid indexing checkpoint; rebuild index".into()))
}
pub(crate) fn same(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn sample(file: &mut File, size: u64) -> Result<(Vec<u8>, Vec<u8>)> {
    let n = size.min(4096) as usize;
    let mut head = vec![0; n];
    let mut tail = vec![0; n];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut head)?;
    file.seek(SeekFrom::Start(size - n as u64))?;
    file.read_exact(&mut tail)?;
    Ok((
        Sha256::digest(&head).to_vec(),
        Sha256::digest(&tail).to_vec(),
    ))
}
pub(crate) fn verify(file: &mut File, c: &Checkpoint) -> Result<bool> {
    let m = file.metadata()?;
    if m.dev() != c.dev || m.ino() != c.ino || m.len() < c.sampled_size {
        return Ok(false);
    }
    let (h, t) = sample(file, c.sampled_size)?;
    Ok(h == c.head && t == c.tail)
}
/// Reads one newline record with bounded allocation, draining oversized records.
/// The returned length includes newline; an incomplete tail is never committed.
pub(crate) fn record<R: BufRead>(reader: &mut R, max: usize) -> Result<(Vec<u8>, u64, bool)> {
    let mut out = Vec::new();
    let mut length = 0;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok((out, length, false));
        }
        let newline = buf.iter().position(|b| *b == b'\n');
        let take = newline.map_or(buf.len(), |p| p + 1);
        let keep = take.min(max.saturating_add(1).saturating_sub(out.len()));
        out.extend_from_slice(&buf[..keep]);
        length += take as u64;
        reader.consume(take);
        if newline.is_some() {
            return Ok((out, length, true));
        }
    }
}
pub struct Indexer<A, S> {
    pub adapter: A,
    pub store: S,
    pub max_record_bytes: usize,
}
impl<A, S> Indexer<A, S> {
    pub fn new(adapter: A, store: S) -> Self {
        Self {
            adapter,
            store,
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
        }
    }
}
impl<A: AgentAdapter, S: IndexStore> Indexer<A, S> {
    pub fn index_file(&mut self, discovered: &SessionFile) -> Result<IndexReport> {
        self.index_with_cache(discovered, &mut HashMap::new())
    }
    fn index_with_cache(
        &mut self,
        discovered: &SessionFile,
        cache: &mut HashMap<std::path::PathBuf, crate::GitContext>,
    ) -> Result<IndexReport> {
        self.index_with_cache_progress(discovered, cache, &mut |_, _, _| {})
    }
    fn index_with_cache_progress(
        &mut self,
        discovered: &SessionFile,
        cache: &mut HashMap<std::path::PathBuf, crate::GitContext>,
        progress: &mut dyn FnMut(u64, u64, u64),
    ) -> Result<IndexReport> {
        let path = &discovered.path;
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(CoreError::Unsupported(
                "source is not a regular file".into(),
            ));
        }
        let prior = self.store.indexed_file_state(path)?;
        let expected = prior.as_ref().map(|(f, _)| f.clone());
        let checkpoint = prior
            .as_ref()
            .and_then(|(_, s)| s.as_deref())
            .map(decode)
            .transpose()?;
        let unchanged = checkpoint
            .as_ref()
            .is_some_and(|c| c.ctime == meta.ctime() && c.ctime_nsec == meta.ctime_nsec())
            && prior
                .as_ref()
                .is_some_and(|(f, _)| f.size == meta.len() && f.modified == meta.modified().ok());
        let valid = match &checkpoint {
            Some(c) => verify(&mut file, c)?,
            None => false,
        };
        let append = prior.as_ref().is_some_and(|(f, _)| meta.len() > f.size) && valid;
        let same_content = valid && (unchanged || append);
        let current_format = checkpoint
            .as_ref()
            .is_some_and(|c| c.format_version == CHECKPOINT_FORMAT_VERSION);
        let rebuild = !(same_content && current_format);
        if current_format
            && valid
            && unchanged
            && prior
                .as_ref()
                .is_some_and(|(f, _)| f.committed_offset == meta.len())
        {
            if !same(&meta, &fs::metadata(path)?) {
                return Err(CoreError::Unsupported("source changed; retry".into()));
            }
            return Ok(IndexReport {
                skipped: true,
                files: 1,
                ..Default::default()
            });
        }
        let fid = match &expected {
            Some(f) => f.file_id,
            None => self.store.next_file_id()?,
        };
        let generation = expected
            .as_ref()
            .map_or(0, |f| f.generation + u64::from(rebuild));
        let start = if rebuild {
            0
        } else {
            expected.as_ref().unwrap().committed_offset
        };
        let mut builder = if rebuild {
            ChunkBuilder::default()
        } else {
            ChunkBuilder::from_state(
                crate::chunks::DEFAULT_MAX_CHUNK_BYTES,
                &checkpoint.as_ref().unwrap().builder,
            )?
        };
        let fresh = rebuild && !same_content;
        let mut recorded = if fresh {
            RecordedGit::default()
        } else {
            checkpoint.as_ref().unwrap().recorded_git.clone()
        };
        let previous_provenance = if fresh {
            None
        } else {
            checkpoint.as_ref().unwrap().provenance()
        };
        let mut session = if fresh {
            Session::placeholder(self.adapter.agent(), path, fid, generation)
        } else {
            // A format-only rebuild must not erase captured Git context for a deleted cwd.
            checkpoint.unwrap().session
        };
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(file.take(meta.len() - start));
        let mut cursor = start;
        let mut chunks = Vec::new();
        let mut pending_bytes = 0;
        let mut pending_records = 0;
        let mut reported_chunks = 0;
        let mut report = IndexReport {
            files: 1,
            ..Default::default()
        };
        loop {
            let (line, n, complete) = record(&mut reader, self.max_record_bytes)?;
            report.bytes_read += n;
            pending_bytes += n;
            if !complete {
                break;
            }
            if n > self.max_record_bytes as u64 + 1 {
                report.malformed_records += 1;
                cursor += n;
                if pending_bytes >= PROGRESS_BYTE_INTERVAL {
                    let new_chunks = chunks.len() as u64 - reported_chunks;
                    progress(pending_bytes, pending_records, new_chunks);
                    reported_chunks = chunks.len() as u64;
                    pending_bytes = 0;
                    pending_records = 0;
                }
                continue;
            }
            let source = SourceRef::new(path, fid, generation, cursor..cursor + n - 1).unwrap();
            match self
                .adapter
                .parse_record(&session, &line[..line.len() - 1], source)
            {
                Ok(parsed) => {
                    report.records += 1;
                    pending_records += 1;
                    if let Some(id) = parsed.metadata.native_id {
                        session.id = SessionId::new(self.adapter.agent(), id)
                    }
                    if let Some(cwd) = parsed.metadata.cwd {
                        session.cwd = Some(cwd)
                    }
                    if let Some(git) = &parsed.metadata.git {
                        recorded.fill_missing(git)
                    }
                    if session.started_at.is_none() {
                        session.started_at = parsed.metadata.started_at
                    }
                    for mut event in parsed.events {
                        event.session_id = session.id.clone();
                        chunks.extend(builder.push(event));
                    }
                }
                Err(CoreError::InvalidRecord(_)) => report.malformed_records += 1,
                Err(e) => return Err(e),
            }
            cursor += n;
            if pending_records >= PROGRESS_RECORD_INTERVAL
                || pending_bytes >= PROGRESS_BYTE_INTERVAL
            {
                let new_chunks = chunks.len() as u64 - reported_chunks;
                progress(pending_bytes, pending_records, new_chunks);
                reported_chunks = chunks.len() as u64;
                pending_bytes = 0;
                pending_records = 0;
            }
        }
        let new_chunks = chunks.len() as u64 - reported_chunks;
        if pending_bytes != 0 || pending_records != 0 || new_chunks != 0 {
            progress(pending_bytes, pending_records, new_chunks);
        }
        let mut file = reader.into_inner().into_inner();
        if !same(&meta, &file.metadata()?) || !same(&meta, &fs::metadata(path)?) {
            return Err(CoreError::Unsupported(
                "source changed during indexing; retry".into(),
            ));
        }
        let observed = match &session.cwd {
            Some(cwd) => match cache.get(cwd) {
                Some(context) => Some(context.clone()),
                None => {
                    let context = crate::git::GitContextResolver.context(cwd)?;
                    cache.insert(cwd.clone(), context.clone());
                    Some(context)
                }
            },
            None => None,
        };
        // A live observation is the strongest fact and replaces anything held before.
        // Otherwise a previous observation is kept, because it knows where the worktree
        // lived; only when neither exists does the transcript's own record apply.
        let applied = observed
            .filter(|context| context.repository_root.is_some())
            .or_else(|| {
                (previous_provenance.as_ref().map(|p| p.origin) != Some(GitOrigin::Observed))
                    .then(|| recorded.as_context(SystemTime::now()))
                    .flatten()
            });
        let mut provenance = previous_provenance;
        if let Some(context) = applied {
            session.repository = context.repository;
            session.repository_root = context.repository_root;
            session.worktree = context.worktree;
            session.branch = context.branch;
            session.commit = context.commit;
            session.git_observed_at = Some(context.observed_at);
            provenance = Some(GitProvenance {
                origin: context.origin,
                repository_url: context.repository_url,
            });
        }
        session.source = SourceRef::new(path, fid, generation, 0..cursor).unwrap();
        let (head, tail) = sample(&mut file, meta.len())?;
        if !same(&meta, &file.metadata()?) || !same(&meta, &fs::metadata(path)?) {
            return Err(CoreError::Unsupported(
                "source changed during indexing; retry".into(),
            ));
        }
        let state = serde_json::to_vec(&Checkpoint {
            format_version: CHECKPOINT_FORMAT_VERSION,
            session: session.clone(),
            git: provenance.clone(),
            recorded_git: recorded,
            builder: builder.state()?,
            dev: meta.dev(),
            ino: meta.ino(),
            ctime: meta.ctime(),
            ctime_nsec: meta.ctime_nsec(),
            head,
            tail,
            sampled_size: meta.len(),
        })
        .map_err(|e| CoreError::Storage(e.to_string()))?;
        if let Some(open) = builder.snapshot() {
            chunks.push(open)
        }
        report.chunks = chunks.len() as u64;
        self.store.commit_batch(IndexBatch {
            file: Some(IndexedFile {
                path: path.clone(),
                file_id: fid,
                generation,
                committed_offset: cursor,
                size: meta.len(),
                modified: meta.modified().ok(),
            }),
            expected_file: Some(expected.clone()),
            sessions: vec![SessionRecord {
                session,
                git: provenance,
            }],
            chunks,
            removed_sources: if rebuild {
                expected
                    .map(|f| vec![(f.file_id, f.generation)])
                    .unwrap_or_default()
            } else {
                vec![]
            },
            open_turn_state: Some(state),
            ..Default::default()
        })?;
        Ok(report)
    }
}
pub fn index_all<S: IndexStore>(
    store: &mut S,
    adapters: &[Box<dyn AgentAdapter>],
) -> Result<IndexReport> {
    index_all_with_progress(store, adapters, |_| {})
}

/// Index all discovered files and report bounded, cumulative progress snapshots.
pub fn index_all_with_progress<S: IndexStore, C: FnMut(IndexProgress)>(
    store: &mut S,
    adapters: &[Box<dyn AgentAdapter>],
    mut callback: C,
) -> Result<IndexReport> {
    let mut total = IndexReport::default();
    let mut cache = HashMap::new();
    let mut discovered = Vec::with_capacity(adapters.len());
    let mut total_files = 0u64;
    let mut completed_files = 0u64;
    for adapter in adapters {
        match adapter.discover() {
            Ok(files) => {
                total_files += files.len() as u64;
                discovered.push((adapter, files));
            }
            Err(e) => {
                total.failed_files += 1;
                if total.errors.len() < 32 {
                    total.errors.push(e.to_string())
                }
                discovered.push((adapter, Vec::new()));
            }
        }
    }
    for (adapter, files) in discovered {
        let mut agent_completed_files = 0u64;
        let agent_total_files = files.len() as u64;
        callback(IndexProgress {
            agent: adapter.agent(),
            completed_files,
            total_files,
            agent_completed_files,
            agent_total_files,
            bytes_read: total.bytes_read,
            records: total.records,
            chunks: total.chunks,
            failed_files: total.failed_files,
        });
        for file in files {
            let mut file_bytes = 0;
            let mut file_records = 0;
            let mut file_chunks = 0;
            let mut emit = |bytes, records, chunks| {
                file_bytes += bytes;
                file_records += records;
                file_chunks += chunks;
                callback(IndexProgress {
                    agent: adapter.agent(),
                    completed_files,
                    total_files,
                    agent_completed_files,
                    agent_total_files,
                    bytes_read: total.bytes_read + file_bytes,
                    records: total.records + file_records,
                    chunks: total.chunks + file_chunks,
                    failed_files: total.failed_files,
                });
            };
            match Indexer::new(adapter.as_ref(), &mut *store)
                .index_with_cache_progress(&file, &mut cache, &mut emit)
            {
                Ok(r) => {
                    total.bytes_read += r.bytes_read;
                    total.records += r.records;
                    total.malformed_records += r.malformed_records;
                    total.chunks += r.chunks;
                    total.files += 1;
                    completed_files += 1;
                    agent_completed_files += 1;
                    callback(IndexProgress {
                        agent: adapter.agent(),
                        completed_files,
                        total_files,
                        agent_completed_files,
                        agent_total_files,
                        bytes_read: total.bytes_read,
                        records: total.records,
                        chunks: total.chunks,
                        failed_files: total.failed_files,
                    });
                }
                Err(e) => {
                    total.failed_files += 1;
                    if total.errors.len() < 32 {
                        total.errors.push(format!("{}: {e}", file.path.display()))
                    }
                    completed_files += 1;
                    agent_completed_files += 1;
                    callback(IndexProgress {
                        agent: adapter.agent(),
                        completed_files,
                        total_files,
                        agent_completed_files,
                        agent_total_files,
                        bytes_read: total.bytes_read,
                        records: total.records,
                        chunks: total.chunks,
                        failed_files: total.failed_files,
                    });
                }
            }
        }
    }
    // Rebuilt sources leave their replaced pages free; return them to the filesystem.
    if let Err(e) = store.reclaim_free_pages() {
        if total.errors.len() < 32 {
            total.errors.push(format!("index maintenance: {e}"))
        }
    }
    Ok(total)
}
impl Checkpoint {
    /// Older checkpoints carry no origin, but only a live `git` invocation could ever
    /// have filled these fields, so they are observations.
    pub(crate) fn provenance(&self) -> Option<GitProvenance> {
        self.git.clone().or_else(|| {
            self.session
                .repository_root
                .is_some()
                .then_some(GitProvenance {
                    origin: GitOrigin::Observed,
                    repository_url: None,
                })
        })
    }
}
impl Session {
    fn placeholder(agent: crate::Agent, path: &Path, file_id: u64, generation: u64) -> Self {
        Self {
            id: SessionId::new(agent, format!("unidentified-source-{file_id}")),
            source: SourceRef::new(path, file_id, generation, 0..0).unwrap(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapters::{ClaudeAdapter, CodexAdapter},
        preview::preview_source,
        storage::SqliteStore,
        test_support::TempDir,
    };
    use std::io::Write;
    fn line(role: &str, text: &str) -> String {
        format!(
            "{{\"type\":\"{role}\",\"sessionId\":\"native\",\"message\":{{\"content\":{}}}}}\n",
            serde_json::to_string(text).unwrap()
        )
    }
    fn discovered(path: &Path) -> SessionFile {
        SessionFile {
            path: path.into(),
            file_id: 0,
            generation: 0,
        }
    }
    fn append(path: &Path, text: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text)
            .unwrap()
    }
    fn db(d: &TempDir) -> SqliteStore {
        SqliteStore::open(d.path().join("private/index.sqlite")).unwrap()
    }
    #[test]
    fn both_adapters_discover_index_query_and_normalized_preview() {
        let d = TempDir::new("integration").unwrap();
        let claude = d.path().join("claude");
        let codex = d.path().join("codex");
        fs::create_dir(&claude).unwrap();
        fs::create_dir(&codex).unwrap();
        fs::write(claude.join("one.jsonl"), line("user", "claudeword ☃")).unwrap();
        fs::write(codex.join("two.jsonl"),"{\"type\":\"session_meta\",\"payload\":{\"id\":\"real-codex\",\"cwd\":\"/nonexistent/synthetic\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"codexword\"}}\n").unwrap();
        let mut db = db(&d);
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![
            Box::new(ClaudeAdapter::with_root(claude)),
            Box::new(CodexAdapter::with_root(codex)),
        ];
        let r = index_all(&mut db, &adapters).unwrap();
        assert_eq!(r.failed_files, 0);
        assert_eq!(r.files, 2);
        for word in ["claudeword", "codexword"] {
            let hits = db.search(word, 10).unwrap();
            assert_eq!(hits.len(), 1);
            let p = preview_source(&db, &hits[0].source, 1000).unwrap();
            assert!(p.text.contains(word));
            assert!(!p.text.contains("payload"));
            assert!(!p.text.contains("sessionId"));
        }
        assert_ne!(
            db.search("claudeword", 1).unwrap()[0].source.file_id,
            db.search("codexword", 1).unwrap()[0].source.file_id
        );
    }
    #[test]
    fn append_reopen_matches_whole_file_and_preserves_metadata() {
        let d = TempDir::new("append").unwrap();
        let p = d.path().join("random-name.jsonl");
        let meta="{\"type\":\"session_meta\",\"payload\":{\"id\":\"real-id\",\"cwd\":\"/nonexistent/synthetic\"}}\n";
        fs::write(&p, meta).unwrap();
        let mut store = db(&d);
        let adapter = CodexAdapter::new([]);
        Indexer::new(&adapter, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        drop(store);
        let additions = [
            "{\"type\":\"message\",\"role\":\"user\",\"content\":\"question\"}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"answer\"}\n",
            "{\"type\":\"message\",\"role\":\"user\",\"content\":\"nextquestion\"}\n",
        ];
        for a in additions {
            append(&p, a.as_bytes());
            let mut store = db(&d);
            let r = Indexer::new(&adapter, &mut store)
                .index_file(&discovered(&p))
                .unwrap();
            assert_eq!(r.bytes_read, a.len() as u64);
        }
        let mut store = db(&d);
        assert!(
            Indexer::new(&adapter, &mut store)
                .index_file(&discovered(&p))
                .unwrap()
                .skipped
        );
        let session = store
            .session(&SessionId::new(crate::Agent::Codex, "real-id"))
            .unwrap()
            .unwrap();
        assert_eq!(session.cwd, Some("/nonexistent/synthetic".into()));
        assert_eq!(session.source.byte_range.end, p.metadata().unwrap().len());
        assert_eq!(store.status().unwrap().chunks, 3);
        let mut whole = SqliteStore::open(d.path().join("other/index.sqlite")).unwrap();
        Indexer::new(&adapter, &mut whole)
            .index_file(&discovered(&p))
            .unwrap();
        let texts = |s: &SqliteStore| {
            s.connection().prepare("SELECT ordinal,text,source_start,source_end FROM search_chunks ORDER BY ordinal").unwrap().query_map([],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?,r.get::<_,u64>(3)?))).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap()
        };
        assert_eq!(texts(&store), texts(&whole));
    }
    #[test]
    fn incomplete_utf8_and_oversize_records_are_retried_or_skipped() {
        let d = TempDir::new("partial").unwrap();
        let p = d.path().join("f.jsonl");
        let text = line("user", "snow☃");
        let cut = text.find('☃').unwrap() + 1;
        fs::write(&p, &text.as_bytes()[..cut]).unwrap();
        let mut store = db(&d);
        let a = ClaudeAdapter::new([]);
        Indexer::new(&a, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        assert_eq!(store.indexed_file(&p).unwrap().unwrap().committed_offset, 0);
        assert_eq!(store.status().unwrap().chunks, 0);
        append(&p, &text.as_bytes()[cut..]);
        Indexer::new(&a, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        assert_eq!(store.search("snow", 10).unwrap().len(), 1);
        append(
            &p,
            format!(
                "{}\ninvalid json\n{}",
                "x".repeat(10000),
                line("user", "laterword")
            )
            .as_bytes(),
        );
        let mut i = Indexer::new(&a, &mut store);
        i.max_record_bytes = 1000;
        let r = i.index_file(&discovered(&p)).unwrap();
        assert_eq!(r.malformed_records, 2);
        assert_eq!(store.search("laterword", 10).unwrap().len(), 1);
        let mut input = std::io::Cursor::new(vec![b'x'; 1_000_000]);
        let (bytes, n, complete) = record(&mut input, 100).unwrap();
        assert_eq!(bytes.len(), 101);
        assert_eq!(n, 1_000_000);
        assert!(!complete);
    }
    #[test]
    fn replacements_only_remove_affected_source_and_invalidate_previews() {
        let d = TempDir::new("mutations").unwrap();
        let p = d.path().join("a.jsonl");
        let q = d.path().join("b.jsonl");
        fs::write(&p, line("user", "oldword")).unwrap();
        fs::write(&q, line("user", "keepword")).unwrap();
        let mut store = db(&d);
        let a = ClaudeAdapter::new([]);
        for path in [&p, &q] {
            Indexer::new(&a, &mut store)
                .index_file(&discovered(path))
                .unwrap();
        }
        assert_eq!(store.status().unwrap().chunks, 2);
        let stale = store.search("oldword", 1).unwrap()[0].source.clone();
        // Equal length rewrite.
        fs::write(&p, line("user", "newword")).unwrap();
        assert!(preview_source(&store, &stale, 100).is_err());
        Indexer::new(&a, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        assert!(store.search("oldword", 10).unwrap().is_empty());
        assert_eq!(store.search("keepword", 10).unwrap().len(), 1);
        assert!(preview_source(&store, &stale, 100).is_err());
        let replacement = d.path().join("replacement");
        fs::write(&replacement, line("user", "replaced")).unwrap();
        fs::rename(&replacement, &p).unwrap();
        Indexer::new(&a, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        assert!(store.search("newword", 10).unwrap().is_empty());
        fs::write(&p, b"").unwrap();
        Indexer::new(&a, &mut store)
            .index_file(&discovered(&p))
            .unwrap();
        assert!(store.search("replaced", 10).unwrap().is_empty());
        assert_eq!(store.search("keepword", 10).unwrap().len(), 1);
        fs::remove_file(&q).unwrap();
        assert!(
            preview_source(&store, &store.search("keepword", 1).unwrap()[0].source, 20).is_err()
        );
    }
    struct Mutating {
        path: std::path::PathBuf,
    }
    impl AgentAdapter for Mutating {
        fn agent(&self) -> crate::Agent {
            crate::Agent::Claude
        }
        fn discover(&self) -> Result<Vec<SessionFile>> {
            Ok(vec![])
        }
        fn parse_record(
            &self,
            s: &Session,
            r: &[u8],
            src: SourceRef,
        ) -> Result<crate::ParsedRecord> {
            append(&self.path, b"\n");
            ClaudeAdapter::new([]).parse_record(s, r, src)
        }
    }
    #[test]
    fn concurrent_source_mutation_aborts_without_advancing() {
        let d = TempDir::new("race").unwrap();
        let p = d.path().join("f.jsonl");
        fs::write(&p, line("user", "raceword")).unwrap();
        let mut store = db(&d);
        assert!(Indexer::new(Mutating { path: p.clone() }, &mut store)
            .index_file(&discovered(&p))
            .is_err());
        assert_eq!(store.status().unwrap().chunks, 0);
        assert!(store.indexed_file(&p).unwrap().is_none());
    }
    struct Mixed {
        paths: Vec<SessionFile>,
    }

    struct MixedAgent {
        agent: crate::Agent,
        paths: Vec<SessionFile>,
    }
    impl AgentAdapter for MixedAgent {
        fn agent(&self) -> crate::Agent {
            self.agent
        }
        fn discover(&self) -> Result<Vec<SessionFile>> {
            Ok(self.paths.clone())
        }
        fn parse_record(
            &self,
            s: &Session,
            r: &[u8],
            src: SourceRef,
        ) -> Result<crate::ParsedRecord> {
            ClaudeAdapter::new([]).parse_record(s, r, src)
        }
    }
    impl AgentAdapter for Mixed {
        fn agent(&self) -> crate::Agent {
            crate::Agent::Claude
        }
        fn discover(&self) -> Result<Vec<SessionFile>> {
            Ok(self.paths.clone())
        }
        fn parse_record(
            &self,
            s: &Session,
            r: &[u8],
            src: SourceRef,
        ) -> Result<crate::ParsedRecord> {
            ClaudeAdapter::new([]).parse_record(s, r, src)
        }
    }
    #[test]
    fn file_errors_do_not_block_unrelated_files() {
        let d = TempDir::new("errors").unwrap();
        let p = d.path().join("good.jsonl");
        fs::write(&p, line("user", "goodword")).unwrap();
        let mut store = db(&d);
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![Box::new(Mixed {
            paths: vec![discovered(&d.path().join("missing")), discovered(&p)],
        })];
        let r = index_all(&mut store, &adapters).unwrap();
        assert_eq!(r.failed_files, 1);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(store.search("goodword", 1).unwrap().len(), 1);
    }

    #[test]
    fn progress_reports_start_bounded_work_and_completion() {
        let d = TempDir::new("progress").unwrap();
        let p = d.path().join("many.jsonl");
        let mut contents = String::new();
        for n in 0..300 {
            contents.push_str(&line("user", &format!("word{n}")));
        }
        fs::write(&p, contents).unwrap();
        let mut store = db(&d);
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![Box::new(Mixed {
            paths: vec![discovered(&p)],
        })];
        let mut snapshots = Vec::new();
        let report =
            index_all_with_progress(&mut store, &adapters, |progress| snapshots.push(progress))
                .unwrap();
        assert_eq!(report.files, 1);
        assert!(snapshots.len() >= 4, "start, bounded updates, completion");
        assert_eq!(snapshots[0].total_files, 1);
        assert_eq!(snapshots[0].agent_total_files, 1);
        assert_eq!(snapshots[0].completed_files, 0);
        let final_progress = snapshots.last().unwrap();
        assert_eq!(final_progress.completed_files, 1);
        assert_eq!(final_progress.total_files, 1);
        assert_eq!(final_progress.agent_completed_files, 1);
        assert_eq!(final_progress.records, report.records);
        assert_eq!(final_progress.bytes_read, report.bytes_read);
        assert_eq!(final_progress.chunks, report.chunks);
        assert!(snapshots.iter().all(|p| p.failed_files == 0));
    }

    #[test]
    fn progress_tracks_agent_counts_and_failed_attempts() {
        let d = TempDir::new("progress-failures").unwrap();
        let good = d.path().join("good.jsonl");
        fs::write(&good, line("user", "goodword")).unwrap();
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![
            Box::new(MixedAgent {
                agent: crate::Agent::Claude,
                paths: vec![discovered(&d.path().join("missing")), discovered(&good)],
            }),
            Box::new(MixedAgent {
                agent: crate::Agent::Codex,
                paths: vec![],
            }),
        ];
        let mut store = db(&d);
        let mut snapshots = Vec::new();
        let report = index_all_with_progress(&mut store, &adapters, |p| snapshots.push(p)).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.failed_files, 1);
        let claude_done = snapshots
            .iter()
            .rev()
            .find(|p| p.agent == crate::Agent::Claude && p.agent_completed_files == 2)
            .unwrap();
        assert_eq!(claude_done.agent_total_files, 2);
        assert_eq!(claude_done.completed_files, 2);
        assert_eq!(claude_done.total_files, 2);
        assert_eq!(claude_done.failed_files, 1);
        let codex_start = snapshots
            .iter()
            .find(|p| p.agent == crate::Agent::Codex)
            .unwrap();
        assert_eq!(codex_start.agent_total_files, 0);
        assert_eq!(codex_start.total_files, 2);
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    use crate::{
        adapters::{ClaudeAdapter, CodexAdapter},
        preview::preview_source,
        storage::SqliteStore,
        test_support::TempDir,
    };
    use std::io::Write;
    fn fixture(p: &Path) -> SessionFile {
        SessionFile {
            path: p.into(),
            file_id: 0,
            generation: 0,
        }
    }
    fn line(text: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"sessionId\":\"s\",\"message\":{{\"content\":{}}}}}\n",
            serde_json::to_string(text).unwrap()
        )
    }
    #[test]
    fn rename_and_truncate_regrow_preserve_other_sources() {
        let d = TempDir::new("rename").unwrap();
        let p = d.path().join("one.jsonl");
        let q = d.path().join("two.jsonl");
        fs::write(&p, line("oldword")).unwrap();
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        let a = ClaudeAdapter::new([]);
        Indexer::new(&a, &mut db).index_file(&fixture(&p)).unwrap();
        let old = db.search("oldword", 1).unwrap()[0].source.clone();
        fs::rename(&p, &q).unwrap();
        Indexer::new(&a, &mut db).index_file(&fixture(&q)).unwrap();
        assert!(preview_source(&db, &old, 100).is_err());
        assert_eq!(db.search("oldword", 10).unwrap().len(), 2);
        fs::write(&q, line("muchlongerreplacementword")).unwrap();
        Indexer::new(&a, &mut db).index_file(&fixture(&q)).unwrap();
        assert_eq!(db.search("oldword", 10).unwrap().len(), 1);
        assert_eq!(db.search("muchlongerreplacementword", 10).unwrap().len(), 1);
    }
    #[test]
    fn preview_middle_unicode_incomplete_tail_and_controls() {
        let d = TempDir::new("preview-middle").unwrap();
        let p = d.path().join("f.jsonl");
        let first = line("first");
        let middle = line("needle☃\u{1b}[31m\u{009b}bad");
        let final_line = line("last");
        fs::write(&p, format!("{first}{middle}{final_line}{{\"partial\":")).unwrap();
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        Indexer::new(ClaudeAdapter::new([]), &mut db)
            .index_file(&fixture(&p))
            .unwrap();
        let source = db.search("needle", 1).unwrap()[0].source.clone();
        let preview = preview_source(&db, &source, 0).unwrap();
        assert!(preview.text.starts_with("User: needle☃"));
        assert!(!preview.text.contains('\u{1b}'));
        assert!(!preview.text.contains('\u{009b}'));
        assert!(!preview.text.contains("partial"));
        assert!(!preview.text.contains("first"));
        assert!(preview.truncated_before);
        assert!(preview.truncated_after);
        let all = preview_source(&db, &source, u64::MAX).unwrap();
        assert!(all.text.contains("first"));
        assert!(all.text.contains("last"));
    }
    /// A rollout whose worktree no longer exists, as most historical sessions are.
    fn codex_rollout(d: &TempDir, cwd: &str, git: &str) -> std::path::PathBuf {
        let p = d.path().join("rollout.jsonl");
        fs::write(
            &p,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"native\",\"cwd\":{},{}}}}}\n\
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"provenancemarker\"}}]}}}}\n",
                serde_json::to_string(cwd).unwrap(),
                git
            ),
        )
        .unwrap();
        p
    }
    const RECORDED_GIT: &str = "\"git\":{\"commit_hash\":\"840c046cb65c43ad093bcac2e07736c86a3a8bf8\",\"branch\":\"feature/prices\",\"repository_url\":\"git@github.com:owner/name.git\"}";

    #[test]
    fn a_deleted_worktree_keeps_the_provenance_its_transcript_recorded() {
        let d = TempDir::new("recorded-codex").unwrap();
        let p = codex_rollout(&d, "/gone/worktrees/owner/name", RECORDED_GIT);
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        Indexer::new(&CodexAdapter::new([]), &mut db)
            .index_file(&fixture(&p))
            .unwrap();

        let id = SessionId::new(crate::Agent::Codex, "native");
        let record = db.session_record(&id).unwrap().unwrap();
        assert_eq!(record.origin(), Some(GitOrigin::Recorded));
        assert!(record.is_recorded_only());
        assert_eq!(record.session.repository.as_deref(), Some("owner/name"));
        assert_eq!(record.session.branch.as_deref(), Some("feature/prices"));
        assert_eq!(
            record.session.commit.as_deref(),
            Some("840c046cb65c43ad093bcac2e07736c86a3a8bf8")
        );
        assert_eq!(
            record.session.cwd,
            Some(std::path::PathBuf::from("/gone/worktrees/owner/name"))
        );
        // A remote URL says which repository, never where it lived.
        assert_eq!(record.session.repository_root, None);
        assert_eq!(record.session.worktree, None);
        assert_eq!(
            record.git.unwrap().repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );

        let result = db.search("provenancemarker", 1).unwrap().remove(0);
        assert_eq!(result.git_origin, Some(GitOrigin::Recorded));
        assert_eq!(result.repository.as_deref(), Some("owner/name"));
        assert_eq!(
            result.repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );
        assert_eq!(
            crate::preview::session_record_for_source(&db, &result.source)
                .unwrap()
                .origin(),
            Some(GitOrigin::Recorded)
        );
    }

    #[test]
    fn a_transcript_without_recorded_git_gets_no_provenance() {
        let d = TempDir::new("recorded-absent").unwrap();
        let p = codex_rollout(
            &d,
            "/gone/worktrees/owner/name",
            "\"originator\":\"codex_exec\"",
        );
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        Indexer::new(&CodexAdapter::new([]), &mut db)
            .index_file(&fixture(&p))
            .unwrap();
        let record = db
            .session_record(&SessionId::new(crate::Agent::Codex, "native"))
            .unwrap()
            .unwrap();
        assert_eq!(record.origin(), None);
        assert_eq!(record.session.repository, None);
        assert_eq!(record.session.branch, None);
        assert_eq!(
            db.search("provenancemarker", 1).unwrap()[0].git_origin,
            None
        );
    }

    #[test]
    fn a_live_observation_outranks_what_the_transcript_recorded() {
        let d = TempDir::new("recorded-observed").unwrap();
        let repo = d.git_repository().unwrap();
        let p = codex_rollout(&d, repo.to_str().unwrap(), RECORDED_GIT);
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        Indexer::new(&CodexAdapter::new([]), &mut db)
            .index_file(&fixture(&p))
            .unwrap();
        let record = db
            .session_record(&SessionId::new(crate::Agent::Codex, "native"))
            .unwrap()
            .unwrap();
        assert_eq!(record.origin(), Some(GitOrigin::Observed));
        assert_eq!(
            record.session.repository_root,
            Some(repo.canonicalize().unwrap())
        );
        assert_ne!(record.session.branch.as_deref(), Some("feature/prices"));
    }

    #[test]
    fn claude_records_a_branch_without_claiming_a_repository_path() {
        let d = TempDir::new("recorded-claude").unwrap();
        let p = d.path().join("f.jsonl");
        fs::write(
            &p,
            "{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"/gone/worktree\",\"gitBranch\":\"feature/prices\",\"message\":{\"content\":\"provenancemarker\"}}\n",
        )
        .unwrap();
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        Indexer::new(&ClaudeAdapter::new([]), &mut db)
            .index_file(&fixture(&p))
            .unwrap();
        let record = db
            .session_record(&SessionId::new(crate::Agent::Claude, "s"))
            .unwrap()
            .unwrap();
        assert_eq!(record.origin(), Some(GitOrigin::Recorded));
        assert_eq!(record.session.branch.as_deref(), Some("feature/prices"));
        assert_eq!(record.session.repository, None);
        assert_eq!(record.session.repository_root, None);
        assert_eq!(record.git.unwrap().repository_url, None);
    }

    #[test]
    fn git_context_is_retained_after_cwd_disappears() {
        let d = TempDir::new("git-index").unwrap();
        let repo = d.git_repository().unwrap();
        let p = d.path().join("f.jsonl");
        let mut record: serde_json::Value = serde_json::from_str(line("original").trim()).unwrap();
        record["cwd"] = serde_json::json!(repo);
        record["gitBranch"] = serde_json::json!("recorded-branch");
        fs::write(&p, format!("{record}\n")).unwrap();
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        let a = ClaudeAdapter::new([]);
        Indexer::new(&a, &mut db).index_file(&fixture(&p)).unwrap();
        let id = SessionId::new(crate::Agent::Claude, "s");
        let before = db.session(&id).unwrap().unwrap();
        assert!(before.repository_root.is_some());
        assert!(before.git_observed_at.is_some());
        assert_ne!(before.branch.as_deref(), Some("recorded-branch"));
        fs::remove_dir_all(&repo).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(line("appended").as_bytes())
            .unwrap();
        Indexer::new(&a, &mut db).index_file(&fixture(&p)).unwrap();
        let after = db.session(&id).unwrap().unwrap();
        assert_eq!(before.repository_root, after.repository_root);
        assert_eq!(before.git_observed_at, after.git_observed_at);
        assert_eq!(before.branch, after.branch);
        // The observation knows where the worktree lived; a recorded branch must not
        // replace it just because the directory is gone.
        assert_eq!(
            db.session_record(&id).unwrap().unwrap().origin(),
            Some(GitOrigin::Observed)
        );
    }
    #[test]
    fn competing_writers_cannot_overwrite_progress_or_reuse_file_id() {
        let d = TempDir::new("writers").unwrap();
        let p = d.path().join("private/index.sqlite");
        let mut a = SqliteStore::open(&p).unwrap();
        let mut b = SqliteStore::open(&p).unwrap();
        let row = IndexedFile {
            path: "first".into(),
            file_id: a.next_file_id().unwrap(),
            generation: 0,
            committed_offset: 10,
            size: 10,
            modified: None,
        };
        let batch = IndexBatch {
            file: Some(row.clone()),
            expected_file: Some(None),
            ..Default::default()
        };
        a.commit_batch(batch.clone()).unwrap();
        assert!(b.commit_batch(batch).is_err());
        assert!(b
            .commit_batch(IndexBatch {
                file: Some(IndexedFile {
                    path: "other".into(),
                    ..row
                }),
                expected_file: Some(None),
                ..Default::default()
            })
            .is_err());
        assert_eq!(b.status().unwrap().files, 1);
    }
}

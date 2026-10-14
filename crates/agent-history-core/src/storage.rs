//! Disposable local SQLite index. Native transcript files are never written here.
use crate::{
    Agent, CommittedFile, CoreError, FileFingerprint, GitOrigin, GitProvenance, IndexBatch,
    IndexedFile, Priorities, Result, SearchOutcome, SearchResult, Session, SessionId,
    SessionRecord, Store, Widening,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: i32 = 6;
const MAX_LIMIT: usize = 1000;
/// Location of the shared index below `$HOME`. The directory name predates the
/// standalone/Herdr split and is kept so existing installations keep their data;
/// it does not imply a Herdr dependency.
pub const DEFAULT_INDEX_RELATIVE_PATH: &str =
    "Library/Application Support/Herdr Agent History/index.sqlite";
/// Below this many free pages a rebuild is not worth the cost of rewriting the file.
const RECLAIM_MIN_FREE_PAGES: i64 = 64;

/// The index every entry point opens when no `--db` is given.
///
/// Resolved here rather than per binary so the CLI and the terminal UI cannot
/// drift onto separate databases: indexing from one and searching from the
/// other would silently read a different corpus.
pub fn default_index_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(DEFAULT_INDEX_RELATIVE_PATH))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexStatus {
    pub files: u64,
    pub sessions: u64,
    pub chunks: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedFileState {
    pub file: IndexedFile,
    pub open_turn_state: Option<Vec<u8>>,
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Ok(md) = fs::symlink_metadata(path) {
            if md.file_type().is_symlink() {
                return Err(CoreError::Storage(
                    "refusing to open a symlink database".into(),
                ));
            }
        }
        if let Some(parent) = path.parent() {
            Self::make_private_parent(parent)?;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", path.to_string_lossy(), suffix));
            if let Ok(md) = fs::symlink_metadata(&sidecar) {
                if md.file_type().is_symlink() {
                    return Err(CoreError::Storage("refusing symlink SQLite sidecar".into()));
                }
            }
        }
        let mut options = fs::OpenOptions::new();
        // O_NOFOLLOW is 0x100 on macOS and Linux; it prevents a final symlink race.
        options
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path)?;
        drop(file);
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        Self::private_file(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        store.conn.pragma_update(None, "journal_mode", "WAL")?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", path.to_string_lossy(), suffix));
            if let Ok(md) = fs::symlink_metadata(&sidecar) {
                if md.file_type().is_symlink() {
                    return Err(CoreError::Storage("refusing symlink SQLite sidecar".into()));
                }
                fs::set_permissions(sidecar, fs::Permissions::from_mode(0o600))?;
            }
        }
        Ok(store)
    }

    fn make_private_parent(parent: &Path) -> Result<()> {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        } else {
            let md = fs::symlink_metadata(parent)?;
            if md.file_type().is_symlink() || !md.is_dir() {
                return Err(CoreError::Storage("index parent is not a directory".into()));
            }
            if md.permissions().mode() & 0o077 != 0 {
                return Err(CoreError::Storage("index parent is not private".into()));
            }
        }
        Ok(())
    }
    fn private_file(path: &Path) -> Result<()> {
        if path.exists() {
            let md = fs::symlink_metadata(path)?;
            if !md.file_type().is_file() {
                return Err(CoreError::Storage("database is not a regular file".into()));
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    fn migrate(&mut self) -> Result<()> {
        // `search_chunks` rows carry the chunk text, which averages a couple of
        // kilobytes. Ranking a one-word query matches thousands of chunks, and
        // the search reads a handful of narrow columns from each before the
        // `LIMIT` applies -- through the row, so every match dragged its text
        // through the page cache. This index carries those columns instead, so
        // that phase reads about a megabyte where it used to read tens. It is
        // worth roughly 4.5 MB on an index of 23k chunks.
        const INDEX_DDL: &str = "CREATE INDEX chunks_meta ON search_chunks(rowid, kind, agent, native_id, ordinal, timestamp, source_path, source_file_id, source_generation, source_start, source_end);";
        // A never-populated database is the only moment auto-vacuum can be enabled without
        // rewriting the file, so claim it before any table exists.
        if self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))?
            == 0
        {
            self.conn
                .pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        }
        // Serialize migration and re-read the version after acquiring the writer lock.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > VERSION {
            return Err(CoreError::Unsupported(format!(
                "database schema version {version} is newer than supported"
            )));
        }
        if version == 1 {
            return Err(CoreError::Unsupported(
                "schema 1 index must be rebuilt (disposable index; native histories are unchanged)"
                    .into(),
            ));
        }
        if version == 2 {
            // Old chunks mix speakers/tools. Keep captured source/session context but rebuild
            // searchable text using the checkpoint format marker on the next indexing pass.
            tx.execute_batch("DROP TRIGGER chunks_ad;
                DELETE FROM search_chunks;
                INSERT INTO chunks_fts(chunks_fts) VALUES('delete-all');
                CREATE TRIGGER chunks_ad AFTER DELETE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); END;
                ALTER TABLE search_chunks ADD COLUMN kind INTEGER NOT NULL DEFAULT -1 CHECK(kind IN (0,1));
                PRAGMA user_version=3;")?;
        }
        if version == 2 || version == 3 {
            // Existing rows hold live observations; recorded provenance only appears once the
            // checkpoint format marker has driven each source through the new parsers.
            tx.execute_batch("\
                ALTER TABLE sessions ADD COLUMN git_origin INTEGER CHECK(git_origin IS NULL OR git_origin IN (0,1));
                ALTER TABLE sessions ADD COLUMN repository_url TEXT;
                UPDATE sessions SET git_origin=0 WHERE repository_root IS NOT NULL;
                PRAGMA user_version=4;")?;
        }
        if (2..=4).contains(&version) {
            // Nullable, and left null for rows committed before this schema: a scan
            // cannot skip such a file from `stat` alone, so it compares the file the
            // way it always did and fills these in when it commits. No reparse.
            tx.execute_batch(
                "\
                ALTER TABLE indexed_files ADD COLUMN dev INTEGER;
                ALTER TABLE indexed_files ADD COLUMN ino INTEGER;
                ALTER TABLE indexed_files ADD COLUMN ctime INTEGER;
                ALTER TABLE indexed_files ADD COLUMN ctime_nsec INTEGER;
                ALTER TABLE indexed_files ADD COLUMN checkpoint_format INTEGER;
                PRAGMA user_version=5;",
            )?;
        }
        if (2..=5).contains(&version) {
            // Every column the search reads except the chunk text, so ranking a
            // match never touches the row it came from. See the index's comment
            // beside the schema.
            tx.execute_batch(&format!("{INDEX_DDL} PRAGMA user_version=6;"))?;
        }
        if version == 0 {
            tx.execute_batch(&format!("\
                CREATE TABLE sessions (agent INTEGER NOT NULL, native_id TEXT NOT NULL, source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL DEFAULT 0, source_end INTEGER NOT NULL DEFAULT 0, cwd TEXT, repository TEXT, repository_root TEXT, worktree TEXT, branch TEXT, commit_hash TEXT, started_at INTEGER, ended_at INTEGER, git_observed_at INTEGER, git_origin INTEGER CHECK(git_origin IS NULL OR git_origin IN (0,1)), repository_url TEXT, PRIMARY KEY(agent,native_id));
                CREATE TABLE indexed_files (path TEXT PRIMARY KEY, file_id INTEGER NOT NULL, generation INTEGER NOT NULL, committed_offset INTEGER NOT NULL, size INTEGER NOT NULL, modified INTEGER, open_turn_state BLOB, dev INTEGER, ino INTEGER, ctime INTEGER, ctime_nsec INTEGER, checkpoint_format INTEGER);
                CREATE TABLE search_chunks (rowid INTEGER PRIMARY KEY, agent INTEGER NOT NULL, native_id TEXT NOT NULL, ordinal INTEGER NOT NULL, timestamp INTEGER, kind INTEGER NOT NULL CHECK(kind IN (0,1)), source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL, source_end INTEGER NOT NULL, text TEXT NOT NULL, FOREIGN KEY(agent,native_id) REFERENCES sessions(agent,native_id) ON DELETE CASCADE, UNIQUE(agent,native_id,source_file_id,source_generation,ordinal));
                CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='search_chunks', content_rowid='rowid', tokenize='unicode61');
                CREATE TRIGGER chunks_ai AFTER INSERT ON search_chunks BEGIN INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
                CREATE TRIGGER chunks_ad AFTER DELETE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); END;
                CREATE TRIGGER chunks_au AFTER UPDATE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
                {INDEX_DDL}
                PRAGMA user_version=6;"))?;
        }
        tx.commit()?;
        if version != 0 && version != VERSION {
            self.compact()?;
        }
        Ok(())
    }

    fn page_counts(&self) -> Result<(i64, i64, i64)> {
        let value = |name| {
            self.conn
                .pragma_query_value(None, name, |r| r.get::<_, i64>(0))
        };
        Ok((
            value("page_count")?,
            value("freelist_count")?,
            value("page_size")?,
        ))
    }

    /// Rewrites the file when free pages dominate it, which is what a migration that
    /// mass-deletes rows leaves behind. `VACUUM` is chosen over incremental vacuum alone
    /// because only a full rewrite can also switch an existing database to incremental
    /// auto-vacuum, after which routine rebuild churn is reclaimable without a rewrite.
    /// It cannot run inside a transaction and needs scratch space about the size of the
    /// database, so it runs after the migration commits and only when the filesystem has
    /// room; skipping it leaves a correct index that is merely larger than necessary.
    fn compact(&mut self) -> Result<()> {
        let (pages, free, page_size) = self.page_counts()?;
        if free < RECLAIM_MIN_FREE_PAGES || free.saturating_mul(4) < pages {
            return Ok(());
        }
        let required = (pages.max(0) as u64).saturating_mul(page_size.max(0) as u64);
        let path = self
            .conn
            .path()
            .map(PathBuf::from)
            .ok_or_else(|| CoreError::Storage("database path is unavailable".into()))?;
        if free_disk_bytes(&path).is_some_and(|available| available < required) {
            return Ok(());
        }
        // VACUUM cannot change the auto-vacuum mode of a WAL database; open() restores WAL.
        self.conn
            .pragma_update(None, "journal_mode", "DELETE")
            .and_then(|()| self.conn.pragma_update(None, "auto_vacuum", "INCREMENTAL"))
            .and_then(|()| self.conn.execute_batch("VACUUM"))?;
        Ok(())
    }

    /// Returns free pages released to the filesystem. Databases created before
    /// incremental auto-vacuum was adopted keep their free pages for reuse instead.
    pub fn reclaim_free_pages(&mut self) -> Result<u64> {
        if self
            .conn
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, i64>(0))?
            != 2
        {
            return Ok(0);
        }
        let (_, before, _) = self.page_counts()?;
        if before == 0 {
            return Ok(0);
        }
        // Each step of the pragma releases one page, so the statement is drained.
        let mut statement = self.conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
        drop(rows);
        drop(statement);
        let (_, after, _) = self.page_counts()?;
        Ok(before.saturating_sub(after).max(0) as u64)
    }

    pub fn indexed_file(&self, path: impl AsRef<Path>) -> Result<Option<IndexedFile>> {
        let p = path.as_ref().to_string_lossy().into_owned();
        Ok(self.conn.query_row("SELECT path,file_id,generation,committed_offset,size,modified FROM indexed_files WHERE path=?", [p], row_file).optional()?)
    }
    /// Returns persisted progress and the resumable partial-turn bytes together.
    pub fn indexed_file_state(&self, path: impl AsRef<Path>) -> Result<Option<IndexedFileState>> {
        let p = path.as_ref().to_string_lossy().into_owned();
        Ok(self.conn.query_row("SELECT path,file_id,generation,committed_offset,size,modified,open_turn_state FROM indexed_files WHERE path=?", [p], |r| Ok(IndexedFileState { file: row_file(r)?, open_turn_state: r.get(6)? })).optional()?)
    }
    /// Every committed file whose fingerprint columns are all present, by path.
    ///
    /// One statement, and deliberately no `open_turn_state`: the checkpoint blobs are
    /// tens of megabytes of JSON across a real history, and nothing here needs them.
    pub fn committed_files(&self) -> Result<HashMap<PathBuf, CommittedFile>> {
        let mut s = self.conn.prepare("SELECT path,file_id,generation,committed_offset,size,modified,dev,ino,ctime,ctime_nsec,checkpoint_format FROM indexed_files WHERE dev IS NOT NULL AND ino IS NOT NULL AND ctime IS NOT NULL AND ctime_nsec IS NOT NULL AND checkpoint_format IS NOT NULL")?;
        let rows = s
            .query_map([], |r| {
                Ok(CommittedFile {
                    file: row_file(r)?,
                    fingerprint: FileFingerprint {
                        dev: r.get(6)?,
                        ino: r.get(7)?,
                        ctime: r.get(8)?,
                        ctime_nsec: r.get(9)?,
                        format_version: r.get(10)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|c| (c.file.path.clone(), c)).collect())
    }
    pub fn session(&self, id: &SessionId) -> Result<Option<Session>> {
        Ok(self.session_record(id)?.map(|r| r.session))
    }
    pub fn sessions(&self) -> Result<Vec<Session>> {
        Ok(self
            .session_records()?
            .into_iter()
            .map(|r| r.session)
            .collect())
    }
    /// Returns the session together with whether its Git fields were observed live
    /// or recorded by the agent, which restoration must not confuse.
    pub fn session_record(&self, id: &SessionId) -> Result<Option<SessionRecord>> {
        self.conn.query_row("SELECT agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at,git_origin,repository_url FROM sessions WHERE agent=? AND native_id=?", params![agent_i(id.agent), id.native_id], row_session_record).optional().map_err(Into::into)
    }
    pub fn session_records(&self) -> Result<Vec<SessionRecord>> {
        let mut s=self.conn.prepare("SELECT agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at,git_origin,repository_url FROM sessions ORDER BY native_id")?;
        let rows = s
            .query_map([], row_session_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn status(&self) -> Result<IndexStatus> {
        let q = |sql| self.conn.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(IndexStatus {
            files: q("SELECT count(*) FROM indexed_files")? as u64,
            sessions: q("SELECT count(*) FROM sessions")? as u64,
            chunks: q("SELECT count(*) FROM search_chunks")? as u64,
        })
    }
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        <Self as Store>::search(self, query, limit)
    }
    pub fn search_with_role(
        &self,
        query: &str,
        limit: usize,
        kind: Option<crate::EventKind>,
    ) -> Result<Vec<SearchResult>> {
        <Self as Store>::search_with_role(self, query, limit, kind)
    }
    pub fn commit_batch(&mut self, batch: IndexBatch) -> Result<()> {
        <Self as Store>::commit_batch(self, batch)
    }

    /// Searches exactly as [`Self::search_with_role`] does and, only when that
    /// finds nothing, retries with each unindexed bare word widened to indexed
    /// words a small edit distance away (see
    /// [`crate::query::lenient_match_expression`]). The outcome names every
    /// widened word, so callers can show that the results are approximate.
    pub fn search_with_fallback(
        &self,
        query: &str,
        limit: usize,
        kind: Option<crate::EventKind>,
    ) -> Result<SearchOutcome> {
        self.search_prioritized(query, limit, kind, &Priorities::default())
    }

    /// [`Self::search_with_fallback`], with each prioritized session's best
    /// match placed ahead of the ordinary results (see [`Priorities`]).
    ///
    /// The prioritized matches come from their own query, so a session is
    /// surfaced however far down relevance alone would rank it. Whether the
    /// near-match retry runs is decided by the ordinary results alone, and
    /// both parts then use the same expression: an exact match elsewhere is
    /// never displaced by a prioritized near match, and a prioritized match is
    /// never exact while the rest are approximate. `limit` bounds the whole
    /// list. Without priorities this is one query, as it always was.
    pub fn search_prioritized(
        &self,
        query: &str,
        limit: usize,
        kind: Option<crate::EventKind>,
        priorities: &Priorities,
    ) -> Result<SearchOutcome> {
        let results = self.search_with_role(query, limit, kind)?;
        if !results.is_empty() || query.trim().is_empty() || limit == 0 {
            let exact = || crate::query::fts_match_expression(query);
            return self.lift(results, exact, limit, kind, priorities, Vec::new());
        }
        let Some((expression, widened)) =
            crate::query::lenient_match_expression(query, |word| self.near_words(word))?
        else {
            return Ok(SearchOutcome::default());
        };
        let results = self.search_expression(&expression, limit, kind)?;
        if results.is_empty() {
            return Ok(SearchOutcome::default());
        }
        self.lift(results, || expression, limit, kind, priorities, widened)
    }

    /// Places the best match of each prioritized session before `results`,
    /// which drop those rows and are cut so the whole list fits `limit`.
    /// `expression` is built only when there is something to lift.
    fn lift(
        &self,
        mut results: Vec<SearchResult>,
        expression: impl FnOnce() -> String,
        limit: usize,
        kind: Option<crate::EventKind>,
        priorities: &Priorities,
        widened: Vec<Widening>,
    ) -> Result<SearchOutcome> {
        if priorities.is_empty() || results.is_empty() {
            return Ok(SearchOutcome {
                results,
                widened,
                prioritized: 0,
            });
        }
        let mut lifted = self.best_prioritized(&expression(), limit, kind, priorities)?;
        let prioritized = lifted.len();
        results.retain(|r| !lifted.iter().any(|l| l.source == r.source));
        results.truncate(limit.min(MAX_LIMIT).saturating_sub(prioritized));
        lifted.append(&mut results);
        Ok(SearchOutcome {
            results: lifted,
            widened,
            prioritized,
        })
    }

    /// The best-ranked matching chunk of each prioritized session, ordered by
    /// group, then rank. Ties within a session go to the earliest chunk, and
    /// ties between sessions to the session order of the ordinary search.
    ///
    /// The first query reads only chunk IDs and scores, so snippets are made
    /// for the chosen rows alone. Each session's best row is picked here
    /// rather than in SQL: `bm25` is unavailable inside window functions and
    /// aggregate subqueries. `CROSS JOIN` keeps the full-text scan outermost:
    /// left to itself, the planner may start from the prioritized sessions and
    /// run a full-text lookup per chunk, which measured up to a hundred times
    /// slower on a real index.
    fn best_prioritized(
        &self,
        expression: &str,
        limit: usize,
        kind: Option<crate::EventKind>,
        priorities: &Priorities,
    ) -> Result<Vec<SearchResult>> {
        let sessions: Vec<_> = priorities.ordered().into_iter().take(MAX_LIMIT).collect();
        let values = vec!["(?,?,?)"; sessions.len()].join(",");
        let sql = prioritized_sql(&values);
        let mut params: Vec<rusqlite::types::Value> = Vec::with_capacity(sessions.len() * 3 + 3);
        for (id, group) in &sessions {
            params.push(agent_i(id.agent).into());
            params.push(id.native_id.clone().into());
            params.push(i64::from(*group).into());
        }
        let role = role_param(kind);
        params.extend([expression.to_string().into(), role.clone(), role]);
        struct Candidate {
            id: i64,
            group: i64,
            score: f64,
            agent: i64,
            native_id: String,
            ordinal: i64,
        }
        let key = |c: &Candidate| (c.score, c.ordinal);
        let mut best: HashMap<(i64, String), Candidate> = HashMap::new();
        let mut st = self.conn.prepare_cached(&sql)?;
        let mut rows = st.query(rusqlite::params_from_iter(params))?;
        while let Some(r) = rows.next()? {
            let c = Candidate {
                id: r.get(0)?,
                group: r.get(1)?,
                score: r.get(2)?,
                agent: r.get(3)?,
                native_id: r.get(4)?,
                ordinal: r.get(5)?,
            };
            match best.entry((c.agent, c.native_id.clone())) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if key(&c) < key(e.get()) {
                        e.insert(c);
                    }
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(c);
                }
            }
        }
        let mut chosen: Vec<Candidate> = best.into_values().collect();
        chosen.sort_by(|a, b| {
            a.group
                .cmp(&b.group)
                .then(a.score.total_cmp(&b.score))
                .then(a.agent.cmp(&b.agent))
                .then_with(|| a.native_id.cmp(&b.native_id))
        });
        chosen.truncate(limit.min(MAX_LIMIT));
        let ids: Vec<i64> = chosen.iter().map(|c| c.id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = self.search_rows(
            &format!(
                "AND c.rowid IN ({}) ORDER BY c.rowid",
                vec!["?"; ids.len()].join(",")
            ),
            expression,
            kind,
            ids.iter().map(|&id| id.into()).collect(),
        )?;
        let order: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        rows.sort_by_key(|(id, _)| order.get(id).copied().unwrap_or(usize::MAX));
        Ok(rows.into_iter().map(|(_, r)| r).collect())
    }

    /// `None` when `word` is itself indexed; otherwise the indexed words near it.
    ///
    /// Reads FTS5's own term list through a temporary `fts5vocab` table, so the
    /// fallback adds nothing to the database file. Only words sharing the first
    /// character, or starting with the first two characters swapped, are
    /// scanned: the whole vocabulary costs far more to read than a slice of it.
    fn near_words(&self, word: &str) -> Result<Option<Vec<String>>> {
        self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.chunks_vocab USING fts5vocab(main, 'chunks_fts', 'row');",
        )?;
        let indexed: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM temp.chunks_vocab WHERE term=?)",
            [word],
            |r| r.get(0),
        )?;
        if indexed {
            return Ok(None);
        }
        let mut chars = word.chars();
        let (Some(first), second) = (chars.next(), chars.next()) else {
            return Ok(Some(Vec::new()));
        };
        let mut prefixes = vec![first.to_string()];
        if let Some(second) = second.filter(|&c| c != first) {
            prefixes.push(format!("{second}{first}"));
        }
        let mut st = self
            .conn
            .prepare_cached("SELECT term,doc FROM temp.chunks_vocab WHERE term>=? AND term<?")?;
        let mut vocabulary = Vec::new();
        for prefix in prefixes {
            let rows = st.query_map(params![prefix, prefix_end(&prefix)], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
            })?;
            for row in rows {
                vocabulary.push(row?);
            }
        }
        Ok(Some(crate::query::near_words(word, vocabulary)))
    }

    fn search_expression(
        &self,
        expression: &str,
        limit: usize,
        role: Option<crate::EventKind>,
    ) -> Result<Vec<SearchResult>> {
        let limit = limit.min(MAX_LIMIT);
        let rows = self.search_rows(
            "ORDER BY bm25(chunks_fts),c.agent,c.native_id,c.ordinal LIMIT ?",
            expression,
            role,
            vec![(limit as i64).into()],
        )?;
        Ok(rows.into_iter().map(|(_, r)| r).collect())
    }

    /// Matching chunks with their chunk IDs. `tail` follows the shared
    /// `WHERE` clause and binds `extra` after its parameters.
    fn search_rows(
        &self,
        tail: &str,
        expression: &str,
        role: Option<crate::EventKind>,
        extra: Vec<rusqlite::types::Value>,
    ) -> Result<Vec<(i64, SearchResult)>> {
        let sql = search_sql(tail);
        let mut st = self.conn.prepare_cached(&sql)?;
        let role = role_param(role);
        let mut params: Vec<rusqlite::types::Value> =
            vec![expression.to_string().into(), role.clone(), role];
        params.extend(extra);
        let rows = st
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok((
                    r.get::<_, i64>(15)?,
                    SearchResult {
                        session_id: SessionId::new(agent_from(r.get(0)?)?, r.get::<_, String>(1)?),
                        agent: agent_from(r.get(0)?)?,
                        repository: r.get(2)?,
                        repository_url: r.get(13)?,
                        git_origin: r.get::<_, Option<i64>>(14)?.map(origin_from).transpose()?,
                        branch: r.get(3)?,
                        cwd: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
                        timestamp: from_ts(r.get(5)?),
                        kind: kind_from(r.get(6)?)?,
                        source: crate::SourceRef::new(
                            PathBuf::from(r.get::<_, String>(7)?),
                            r.get(8)?,
                            r.get(9)?,
                            r.get(10)?..r.get(11)?,
                        )
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        snippet: r.get(12)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// The ranked search. `tail` follows the shared `WHERE` clause.
///
/// `INDEXED BY` is load-bearing: the planner reaches a `rowid=?` row through
/// the primary key however wide that row is, and these rows carry the chunk
/// text. Ranking reads a few narrow columns from every match before the
/// `LIMIT`, so without the hint each match drags its text through the page
/// cache -- seconds for a common word on a real index.
fn search_sql(tail: &str) -> String {
    format!("SELECT c.agent,c.native_id,s.repository,s.branch,s.cwd,c.timestamp,c.kind,c.source_path,c.source_file_id,c.source_generation,c.source_start,c.source_end,snippet(chunks_fts,0,'','', ' … ', 24),s.repository_url,s.git_origin,c.rowid FROM chunks_fts JOIN search_chunks c INDEXED BY chunks_meta ON c.rowid=chunks_fts.rowid JOIN sessions s ON s.agent=c.agent AND s.native_id=c.native_id WHERE chunks_fts MATCH ? AND (? IS NULL OR c.kind=?) {tail}")
}

/// The prioritized sessions' own query; `values` holds its `(?,?,?)` rows.
/// Hinted for the same reason as [`search_sql`].
fn prioritized_sql(values: &str) -> String {
    format!(
        "WITH p(agent,native_id,grp) AS (VALUES {values}) \
         SELECT c.rowid, p.grp, bm25(chunks_fts), c.agent, c.native_id, c.ordinal \
         FROM chunks_fts CROSS JOIN search_chunks c INDEXED BY chunks_meta ON c.rowid=chunks_fts.rowid CROSS JOIN p ON p.agent=c.agent AND p.native_id=c.native_id \
         WHERE chunks_fts MATCH ? AND (? IS NULL OR c.kind=?)"
    )
}

fn role_param(role: Option<crate::EventKind>) -> rusqlite::types::Value {
    role.map_or(rusqlite::types::Value::Null, |k| kind_i(k).into())
}

/// The smallest string greater than every string starting with `prefix`, in
/// SQLite's byte order (which is code point order for UTF-8).
fn prefix_end(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let next = (last as u32 + 1..=char::MAX as u32).find_map(char::from_u32);
        if let Some(next) = next {
            chars.push(next);
            return chars.into_iter().collect();
        }
    }
    // A prefix of only `char::MAX` has no finite bound; this one covers every
    // word at most one character longer.
    char::MAX.to_string().repeat(prefix.chars().count() + 1)
}

impl Store for SqliteStore {
    fn commit_batch(&mut self, batch: IndexBatch) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != VERSION {
            return Err(CoreError::Unsupported(format!(
                "database schema version {version} changed; reopen the index"
            )));
        }
        if let (Some(expected), Some(file)) = (&batch.expected_file, &batch.file) {
            let actual = tx.query_row("SELECT path,file_id,generation,committed_offset,size,modified FROM indexed_files WHERE path=?", [pstr(&file.path)], row_file).optional()?;
            if &actual != expected {
                return Err(CoreError::Storage(
                    "index progress changed concurrently; retry".into(),
                ));
            }
            let collision: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM indexed_files WHERE file_id=? AND path<>?)",
                params![file.file_id, pstr(&file.path)],
                |r| r.get(0),
            )?;
            if collision {
                return Err(CoreError::Storage(
                    "source identity allocated concurrently; retry".into(),
                ));
            }
        }
        for (fid, gen) in batch.removed_sources {
            tx.execute(
                "DELETE FROM search_chunks WHERE source_file_id=? AND source_generation=?",
                params![fid, gen],
            )?;
        }
        for id in batch.replaced_chunks {
            tx.execute(
                "DELETE FROM search_chunks WHERE agent=? AND native_id=? AND ordinal>=?",
                params![agent_i(id.0.agent), id.0.native_id, id.1],
            )?;
        }
        for record in batch.sessions {
            let (s, git) = (record.session, record.git);
            tx.execute("INSERT INTO sessions(agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at,git_origin,repository_url) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(agent,native_id) DO UPDATE SET source_path=excluded.source_path,source_file_id=excluded.source_file_id,source_generation=excluded.source_generation,source_start=excluded.source_start,source_end=excluded.source_end,cwd=excluded.cwd,repository=excluded.repository,repository_root=excluded.repository_root,worktree=excluded.worktree,branch=excluded.branch,commit_hash=excluded.commit_hash,started_at=excluded.started_at,ended_at=excluded.ended_at,git_observed_at=excluded.git_observed_at,git_origin=excluded.git_origin,repository_url=excluded.repository_url",params![agent_i(s.id.agent),s.id.native_id,pstr(&s.source.path),s.source.file_id,s.source.generation,s.source.byte_range.start,s.source.byte_range.end,optp(&s.cwd),s.repository,s.repository_root.as_ref().map(|p|pstr(p)),s.worktree.as_ref().map(|p|pstr(p)),s.branch,s.commit,ts(s.started_at),ts(s.ended_at),ts(s.git_observed_at),git.as_ref().map(|g|origin_i(g.origin)),git.and_then(|g|g.repository_url)])?;
        }
        for c in batch.chunks {
            tx.execute("INSERT INTO search_chunks(agent,native_id,ordinal,timestamp,kind,source_path,source_file_id,source_generation,source_start,source_end,text) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(agent,native_id,source_file_id,source_generation,ordinal) DO UPDATE SET timestamp=excluded.timestamp,kind=excluded.kind,source_path=excluded.source_path,source_file_id=excluded.source_file_id,source_generation=excluded.source_generation,source_start=excluded.source_start,source_end=excluded.source_end,text=excluded.text",params![agent_i(c.session_id.agent),c.session_id.native_id,c.ordinal,ts(c.timestamp),kind_i(c.kind),pstr(&c.source.path),c.source.file_id,c.source.generation,c.source.byte_range.start,c.source.byte_range.end,c.text])?;
        }
        if let Some(f) = batch.file {
            let p = batch.fingerprint;
            tx.execute("INSERT INTO indexed_files(path,file_id,generation,committed_offset,size,modified,open_turn_state,dev,ino,ctime,ctime_nsec,checkpoint_format) VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(path) DO UPDATE SET file_id=excluded.file_id,generation=excluded.generation,committed_offset=excluded.committed_offset,size=excluded.size,modified=excluded.modified,open_turn_state=excluded.open_turn_state,dev=excluded.dev,ino=excluded.ino,ctime=excluded.ctime,ctime_nsec=excluded.ctime_nsec,checkpoint_format=excluded.checkpoint_format",params![pstr(&f.path),f.file_id,f.generation,f.committed_offset,f.size,ts(f.modified),batch.open_turn_state,p.map(|p|p.dev),p.map(|p|p.ino),p.map(|p|p.ctime),p.map(|p|p.ctime_nsec),p.map(|p|p.format_version)])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search_with_role(query, limit, None)
    }
    fn search_with_role(
        &self,
        query: &str,
        limit: usize,
        role: Option<crate::EventKind>,
    ) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let expression = crate::query::fts_match_expression(query);
        self.search_expression(&expression, limit, role)
    }
}

impl crate::IndexStore for SqliteStore {
    fn next_file_id(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(file_id),0)+1 FROM indexed_files",
            [],
            |r| r.get(0),
        )?)
    }
    fn indexed_file_state(&self, path: &Path) -> Result<Option<(IndexedFile, Option<Vec<u8>>)>> {
        SqliteStore::indexed_file_state(self, path).map(|v| v.map(|s| (s.file, s.open_turn_state)))
    }
    fn committed_files(&self) -> Result<HashMap<PathBuf, CommittedFile>> {
        SqliteStore::committed_files(self)
    }
    /// The `WHERE` clause is the guard: every byte-offset column must still hold what
    /// the caller compared against the file, or no row is touched. `modified IS ?`
    /// rather than `=` so that a row recorded without a timestamp still matches.
    fn record_fingerprint(
        &mut self,
        file: &IndexedFile,
        fingerprint: FileFingerprint,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE indexed_files SET dev=?,ino=?,ctime=?,ctime_nsec=?,checkpoint_format=? WHERE path=? AND file_id=? AND generation=? AND committed_offset=? AND size=? AND modified IS ?",
            params![
                fingerprint.dev,
                fingerprint.ino,
                fingerprint.ctime,
                fingerprint.ctime_nsec,
                fingerprint.format_version,
                pstr(&file.path),
                file.file_id,
                file.generation,
                file.committed_offset,
                file.size,
                ts(file.modified)
            ],
        )?;
        Ok(())
    }
    fn sessions(&self) -> Result<Vec<Session>> {
        SqliteStore::sessions(self)
    }
    fn session(&self, id: &SessionId) -> Result<Option<Session>> {
        SqliteStore::session(self, id)
    }
    fn session_record(&self, id: &SessionId) -> Result<Option<SessionRecord>> {
        SqliteStore::session_record(self, id)
    }
    fn reclaim_free_pages(&mut self) -> Result<u64> {
        SqliteStore::reclaim_free_pages(self)
    }
}

fn row_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedFile> {
    Ok(IndexedFile {
        path: PathBuf::from(r.get::<_, String>(0)?),
        file_id: r.get(1)?,
        generation: r.get(2)?,
        committed_offset: r.get(3)?,
        size: r.get(4)?,
        modified: from_ts(r.get(5)?),
    })
}
fn row_session_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        session: row_session(r)?,
        git: r
            .get::<_, Option<i64>>(16)?
            .map(origin_from)
            .transpose()?
            .map(|origin| GitProvenance {
                origin,
                repository_url: r.get(17).unwrap_or(None),
            }),
    })
}
fn row_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let a = agent_from(r.get(0)?)?;
    let id = SessionId::new(a, r.get::<_, String>(1)?);
    Ok(Session {
        id,
        source: crate::SourceRef::new(
            PathBuf::from(r.get::<_, String>(2)?),
            r.get(3)?,
            r.get(4)?,
            r.get(5)?..r.get(6)?,
        )
        .map_err(|_| rusqlite::Error::InvalidQuery)?,
        cwd: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
        repository: r.get(8)?,
        repository_root: r.get::<_, Option<String>>(9)?.map(PathBuf::from),
        worktree: r.get::<_, Option<String>>(10)?.map(PathBuf::from),
        branch: r.get(11)?,
        commit: r.get(12)?,
        started_at: from_ts(r.get(13)?),
        ended_at: from_ts(r.get(14)?),
        git_observed_at: from_ts(r.get(15)?),
    })
}
fn pstr(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
fn optp(p: &Option<PathBuf>) -> Option<String> {
    p.as_ref().map(|x| pstr(x))
}
fn ts(t: Option<SystemTime>) -> Option<i64> {
    t.and_then(|x| match x.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).ok(),
        Err(e) => i64::try_from(e.duration().as_nanos())
            .ok()
            .and_then(|n| n.checked_neg()),
    })
}
fn from_ts(v: Option<i64>) -> Option<SystemTime> {
    v.and_then(|x| {
        if x >= 0 {
            Some(UNIX_EPOCH + std::time::Duration::from_nanos(x as u64))
        } else {
            UNIX_EPOCH.checked_sub(std::time::Duration::from_nanos(x.unsigned_abs()))
        }
    })
}
fn agent_i(a: Agent) -> i64 {
    match a {
        Agent::Claude => 0,
        Agent::Codex => 1,
    }
}
fn agent_from(v: i64) -> rusqlite::Result<Agent> {
    match v {
        0 => Ok(Agent::Claude),
        1 => Ok(Agent::Codex),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn origin_i(o: GitOrigin) -> i64 {
    match o {
        GitOrigin::Observed => 0,
        GitOrigin::Recorded => 1,
    }
}
fn origin_from(v: i64) -> rusqlite::Result<GitOrigin> {
    match v {
        0 => Ok(GitOrigin::Observed),
        1 => Ok(GitOrigin::Recorded),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
/// Bytes available to this user on the filesystem holding `path`.
fn free_disk_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
    // Safety: `path` is a valid NUL-terminated C string and `stat` is owned here.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    (stat.f_bavail as u64).checked_mul(stat.f_frsize)
}
fn kind_i(k: crate::EventKind) -> i64 {
    match k {
        crate::EventKind::User => 0,
        crate::EventKind::Assistant => 1,
        crate::EventKind::ToolResult => 2,
    }
}
fn kind_from(v: i64) -> rusqlite::Result<crate::EventKind> {
    match v {
        0 => Ok(crate::EventKind::User),
        1 => Ok(crate::EventKind::Assistant),
        2 => Ok(crate::EventKind::ToolResult),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConversationChunk, SourceRef};

    fn session(agent: Agent, native: &str, file: u64, generation: u64) -> Session {
        Session {
            id: SessionId::new(agent, native),
            source: SourceRef::new("/tmp/native.jsonl", file, generation, 0..10).unwrap(),
            cwd: Some(PathBuf::from("/work")),
            repository: Some("repo".into()),
            repository_root: Some(PathBuf::from("/repo")),
            worktree: None,
            branch: Some("main".into()),
            commit: Some("abc".into()),
            started_at: None,
            ended_at: None,
            git_observed_at: Some(UNIX_EPOCH),
        }
    }
    fn chunk(
        id: &SessionId,
        file: u64,
        generation: u64,
        ordinal: u64,
        text: &str,
    ) -> ConversationChunk {
        ConversationChunk {
            session_id: id.clone(),
            kind: crate::EventKind::User,
            ordinal,
            timestamp: None,
            source: SourceRef::new(
                "/tmp/native.jsonl",
                file,
                generation,
                (ordinal * 10)..(ordinal * 10 + text.len() as u64),
            )
            .unwrap(),
            text: text.into(),
        }
    }
    fn lenient_fixture(name: &str) -> (crate::test_support::TempDir, SqliteStore) {
        let dir = crate::test_support::TempDir::new(name).unwrap();
        let mut db = SqliteStore::open(dir.path().join("private/index.sqlite")).unwrap();
        let texts = [
            "the database migration failed on startup",
            "we moved the database to a new worktree",
            "portfolio visibility review",
            "the cache layer is fine",
            "typo reciept kept verbatim here",
        ];
        let s = session(Agent::Codex, "lenient", 4, 1);
        let mut chunks: Vec<ConversationChunk> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| chunk(&s.id, 4, 1, i as u64, t))
            .collect();
        chunks[1].kind = crate::EventKind::Assistant;
        db.commit_batch(IndexBatch {
            sessions: vec![s.into()],
            chunks,
            ..Default::default()
        })
        .unwrap();
        (dir, db)
    }

    fn lenient(db: &SqliteStore, query: &str) -> (Vec<String>, Option<String>) {
        let outcome = db
            .search_with_fallback(query, 50, None)
            .unwrap_or_else(|e| panic!("query {query:?} failed: {e}"));
        let mut snippets: Vec<String> = outcome.results.iter().map(|r| r.snippet.clone()).collect();
        snippets.sort();
        (snippets, outcome.approximation())
    }

    #[test]
    fn fallback_finds_misspelled_and_transposed_words_and_says_so() {
        let (_dir, db) = lenient_fixture("storage-lenient-typo");
        let (hits, note) = lenient(&db, "datbase");
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| h.contains("database")));
        assert_eq!(note.as_deref(), Some("datbase → database, datbase*"));
        let (hits, note) = lenient(&db, "receipt");
        assert_eq!(hits.len(), 1);
        assert_eq!(note.as_deref(), Some("receipt → reciept, receipt*"));
        // Adjacent swaps, including of the first two characters.
        for query in ["databsae", "adtabase", "DATABSAE"] {
            let (hits, note) = lenient(&db, query);
            assert_eq!(hits.len(), 2, "{query:?} -> {hits:?}");
            assert!(note.unwrap().contains("→ database"), "{query:?}");
        }
        // An unfinished word matches as a prefix.
        let (hits, note) = lenient(&db, "worktre");
        assert_eq!(hits.len(), 1);
        assert_eq!(note.as_deref(), Some("worktre → worktre*"));
        // The role filter still applies to near matches.
        let outcome = db
            .search_with_fallback("databsae", 50, Some(crate::EventKind::Assistant))
            .unwrap();
        assert_eq!(outcome.results.len(), 1);
        assert!(outcome.is_approximate());
    }

    #[test]
    fn fallback_does_not_fire_when_the_exact_query_matches() {
        let (_dir, db) = lenient_fixture("storage-lenient-exact");
        for query in [
            "database",
            "reciept",
            "cache",
            "portfol*",
            "\"database migration\"",
        ] {
            let outcome = db.search_with_fallback(query, 50, None).unwrap();
            assert!(!outcome.results.is_empty(), "{query:?}");
            assert!(!outcome.is_approximate(), "{query:?}");
            assert_eq!(outcome.results, db.search(query, 50).unwrap(), "{query:?}");
        }
    }

    #[test]
    fn fallback_skips_short_unmatched_and_punctuation_only_queries() {
        let (_dir, db) = lenient_fixture("storage-lenient-none");
        // `fne` is one edit from `fine` but too short to widen.
        for query in [
            "fne",
            "teh",
            "zzzzqqqq",
            "?!",
            "---",
            "()",
            "\"",
            "*",
            "   ",
            "\"databsae migration\"",
            "databsae*",
        ] {
            let outcome = db.search_with_fallback(query, 50, None).unwrap();
            assert!(outcome.results.is_empty(), "{query:?}");
            assert!(!outcome.is_approximate(), "{query:?}");
        }
    }

    #[test]
    fn fallback_keeps_operators_phrases_and_prefixes_exact() {
        let (_dir, db) = lenient_fixture("storage-lenient-ops");
        let (hits, note) = lenient(&db, "\"migration failed\" AND databsae");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("migration failed"));
        assert_eq!(note.as_deref(), Some("databsae → database, databsae*"));
        let (hits, _) = lenient(&db, "databsae NOT migration");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("worktree"));
        // One side of `OR` matched exactly, so nothing is widened.
        let (hits, note) = lenient(&db, "databsae OR portfolio");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(note.is_none());
        let (hits, note) = lenient(&db, "databsae OR portfolo");
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert_eq!(
            note.as_deref(),
            Some("databsae → database, databsae*; portfolo → portfolio, portfolo*")
        );
        let (hits, note) = lenient(&db, "migr* AND databsae");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(note.is_some());
        // An indexed word is kept exact while its misspelled neighbour widens.
        let (hits, note) = lenient(&db, "worktree databsae");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(note.as_deref(), Some("databsae → database, databsae*"));
        // A negated word is never widened, so the exclusion does not grow.
        let (hits, note) = lenient(&db, "startup NOT databsae");
        assert_eq!(hits.len(), 1);
        assert!(note.is_none());
        // Nothing near every word: no results, and nothing claimed.
        let (hits, note) = lenient(&db, "databsae AND zzzzqqqq");
        assert!(hits.is_empty() && note.is_none());
        // Every retry expression parses, wherever the widened word sits.
        for query in [
            "NOT databsae",
            "databsae OR",
            "databsae AND",
            "AND databsae portfolo",
            "(databsae)",
            "\"databsae",
            "databsae \"migration failed\" portfol*",
            "databsae NOT zzzzqqqq worktre",
            "databsae -- ?! wroktree",
            &"databsae ".repeat(500),
            &"qwertyuiop ".repeat(50),
        ] {
            lenient(&db, query);
        }
    }

    #[test]
    fn fallback_leaves_the_database_file_untouched() {
        let (dir, db) = lenient_fixture("storage-lenient-size");
        let path = dir.path().join("private/index.sqlite");
        let before = (pragma(&db, "page_count"), pragma(&db, "schema_version"));
        let size = fs::metadata(&path).unwrap().len();
        assert!(db
            .search_with_fallback("databsae", 10, None)
            .unwrap()
            .is_approximate());
        assert_eq!(
            (pragma(&db, "page_count"), pragma(&db, "schema_version")),
            before
        );
        drop(db);
        assert_eq!(fs::metadata(&path).unwrap().len(), size);
        let reopened = SqliteStore::open(&path).unwrap();
        assert!(reopened
            .search_with_fallback("databsae", 10, None)
            .unwrap()
            .is_approximate());
    }

    #[test]
    fn prefix_end_bounds_every_word_with_the_prefix() {
        assert_eq!(prefix_end("d"), "e");
        assert_eq!(prefix_end("ad"), "ae");
        assert_eq!(prefix_end("\u{d7ff}"), "\u{e000}");
        assert_eq!(prefix_end("a\u{10ffff}"), "b");
        for word in ["d", "da", "dzzzz", "d\u{10ffff}"] {
            assert!(word >= "d" && word < prefix_end("d").as_str(), "{word:?}");
        }
    }

    #[test]
    fn user_queries_with_punctuation_search_words_through_fts() {
        let dir = crate::test_support::TempDir::new("storage-query").unwrap();
        let mut db = SqliteStore::open(dir.path().join("private/index.sqlite")).unwrap();
        let texts = [
            "beacon cobalt-heron-7309 recorded",
            "we hit the rate limit again",
            "is C++ faster here",
            "config foo:bar key",
            "what? nobody knows",
            "call a(b) twice",
            "mail dev@example.com today",
            "binary in /usr/local/bin",
            "she said \"quoted\" words",
            "café crème ordered",
            "日本語 の テキスト",
            "portfolio visibility",
        ];
        let s = session(Agent::Claude, "q", 3, 1);
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone().into()],
            chunks: texts
                .iter()
                .enumerate()
                .map(|(i, t)| chunk(&s.id, 3, 1, i as u64, t))
                .collect(),
            ..Default::default()
        })
        .unwrap();
        let hits = |q: &str| -> Vec<String> {
            db.search(q, 50)
                .unwrap_or_else(|e| panic!("query {q:?} failed: {e}"))
                .into_iter()
                .map(|r| r.snippet)
                .collect()
        };
        let only = |q: &str, needle: &str| {
            let found = hits(q);
            assert_eq!(found.len(), 1, "{q:?} -> {found:?}");
            assert!(found[0].contains(needle), "{q:?} -> {found:?}");
        };
        only("cobalt-heron", "cobalt-heron-7309");
        only("cobalt-heron-7309", "cobalt-heron-7309");
        only("rate-limit", "rate limit");
        only("foo:bar", "foo:bar");
        only("what?", "what?");
        only("a(b)", "a(b)");
        only("dev@example.com", "dev@example.com");
        only("/usr/local/bin", "/usr/local/bin");
        only("\"quoted\" words", "\"quoted\" words");
        only("said \"quoted", "\"quoted\"");
        only("café-crème", "café crème");
        only("日本語", "日本語");
        assert!(hits("C++").iter().any(|s| s.contains("C++")));
        only("C++ faster", "C++");
        only("\"portfolio visibility\"", "portfolio");
        only("portfol*", "portfolio");
        only("rate-lim*", "rate limit");
        only("portfolio AND visibility", "portfolio");
        assert_eq!(hits("portfolio OR nobody").len(), 2);
        assert!(hits("portfolio NOT visibility").is_empty());
        only("portfolio AND NOT nobody", "portfolio");
        assert!(hits("\"visibility portfolio\"").is_empty());
        for q in [
            "-", "\"", "\"\"", "?!", "*", "()", "AND", "NOT", "a OR", "NEAR(",
        ] {
            hits(q);
        }
        let long = vec!["rate-limit"; 2000].join(" ");
        only(&long, "rate limit");
        assert!(hits(&"zz-yy ".repeat(2000)).is_empty());
    }
    #[test]
    fn fts_insert_delete_and_reindex_are_transactional() {
        let dir = crate::test_support::TempDir::new("storage").unwrap();
        let path = dir.path().join("private/index.sqlite");
        let mut db = SqliteStore::open(&path).unwrap();
        let s = session(Agent::Claude, "one", 7, 1);
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone().into()],
            chunks: vec![chunk(&s.id, 7, 1, 0, "portfolio visibility")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(db.search("portfolio", 10).unwrap().len(), 1);
        assert_eq!(db.search("\"portfolio visibility\"", 10).unwrap().len(), 1);
        assert_eq!(db.search("portfol*", 10).unwrap().len(), 1);
        assert_eq!(db.search("portfolio AND visibility", 10).unwrap().len(), 1);
        let assistant = ConversationChunk {
            kind: crate::EventKind::Assistant,
            ordinal: 1,
            ..chunk(&s.id, 7, 1, 1, "portfolio visibility")
        };
        db.commit_batch(IndexBatch {
            chunks: vec![assistant],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            db.search_with_role("portfolio", 10, Some(crate::EventKind::User))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.search_with_role("portfolio", 10, Some(crate::EventKind::Assistant))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(db.search_with_role("portfolio", 10, None).unwrap().len(), 2);
        assert!(db.search("portfolio", 10).unwrap()[0]
            .snippet
            .contains("portfolio"));
        db.commit_batch(IndexBatch {
            removed_sources: vec![(7, 1)],
            ..Default::default()
        })
        .unwrap();
        assert!(db.search("portfolio", 10).unwrap().is_empty());
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone().into()],
            chunks: vec![chunk(&s.id, 7, 2, 0, "unrelated replacement")],
            ..Default::default()
        })
        .unwrap();
        assert!(db.search("portfolio", 10).unwrap().is_empty());
        assert_eq!(db.search("replacement", 10).unwrap().len(), 1);
    }
    #[test]
    fn failed_batch_rolls_back_all_changes() {
        let dir = crate::test_support::TempDir::new("storage-rollback").unwrap();
        let mut db = SqliteStore::open(dir.path().join("private/index.sqlite")).unwrap();
        let s = session(Agent::Codex, "two", 8, 1);
        let bad = ConversationChunk {
            session_id: s.id.clone(),
            kind: crate::EventKind::User,
            ordinal: 0,
            timestamp: None,
            source: SourceRef::new("/tmp/native.jsonl", 8, 1, 0..0).unwrap(),
            text: "bad".into(),
        };
        // Duplicate session insertion with an invalid foreign key chunk forces rollback.
        let result = db.commit_batch(IndexBatch {
            sessions: vec![s.into()],
            chunks: vec![ConversationChunk {
                session_id: SessionId::new(Agent::Codex, "missing"),
                ..bad
            }],
            ..Default::default()
        });
        assert!(result.is_err());
        assert_eq!(db.status().unwrap(), IndexStatus::default());
    }
    #[test]
    fn search_phrase_prefix_boolean_and_rank() {
        let d = crate::test_support::TempDir::new("query").unwrap();
        let mut db = SqliteStore::open(d.path().join("p/index.sqlite")).unwrap();
        let a = session(Agent::Claude, "a", 1, 1);
        let b = session(Agent::Codex, "b", 2, 1);
        db.commit_batch(IndexBatch {
            sessions: vec![a.clone().into(), b.clone().into()],
            chunks: vec![
                chunk(&a.id, 1, 1, 0, "alpha beta"),
                chunk(&b.id, 2, 1, 0, "alpha alpha beta"),
            ],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(db.search("\"alpha beta\"", 10).unwrap().len(), 2);
        assert_eq!(db.search("alph* AND beta", 10).unwrap()[0].session_id, b.id);
        assert!(db.search("alpha", 10).unwrap()[0].snippet.contains("alpha"));
    }
    #[test]
    fn file_state_nanoseconds_and_open_turn_survive_reopen() {
        let d = crate::test_support::TempDir::new("reopen").unwrap();
        let p = d.path().join("p/index.sqlite");
        let t = UNIX_EPOCH + std::time::Duration::new(42, 123456789);
        let mut db = SqliteStore::open(&p).unwrap();
        db.commit_batch(IndexBatch {
            file: Some(IndexedFile {
                path: "source".into(),
                file_id: 4,
                generation: 2,
                committed_offset: 17,
                size: 17,
                modified: Some(t),
            }),
            open_turn_state: Some(b"partial".to_vec()),
            ..Default::default()
        })
        .unwrap();
        drop(db);
        let db = SqliteStore::open(&p).unwrap();
        let x = db.indexed_file_state("source").unwrap().unwrap();
        assert_eq!(x.file.modified, Some(t));
        assert_eq!(x.open_turn_state, Some(b"partial".to_vec()));
    }
    #[test]
    fn rollback_preserves_previous_file_offset_open_turn_and_search_rows() {
        let d = crate::test_support::TempDir::new("rollback-state").unwrap();
        let p = d.path().join("p/index.sqlite");
        let mut db = SqliteStore::open(&p).unwrap();
        let s = session(Agent::Claude, "keep", 3, 1);
        db.commit_batch(IndexBatch {
            file: Some(IndexedFile {
                path: "source".into(),
                file_id: 3,
                generation: 1,
                committed_offset: 9,
                size: 9,
                modified: None,
            }),
            open_turn_state: Some(b"old".to_vec()),
            sessions: vec![s.clone().into()],
            chunks: vec![chunk(&s.id, 3, 1, 0, "keep me")],
            ..Default::default()
        })
        .unwrap();
        let r = db.commit_batch(IndexBatch {
            file: Some(IndexedFile {
                path: "source".into(),
                file_id: 3,
                generation: 1,
                committed_offset: 99,
                size: 99,
                modified: None,
            }),
            open_turn_state: Some(b"new".to_vec()),
            chunks: vec![ConversationChunk {
                session_id: SessionId::new(Agent::Codex, "missing"),
                ..chunk(&s.id, 3, 1, 1, "bad")
            }],
            ..Default::default()
        });
        assert!(r.is_err());
        let x = db.indexed_file_state("source").unwrap().unwrap();
        assert_eq!(x.file.committed_offset, 9);
        assert_eq!(x.open_turn_state, Some(b"old".to_vec()));
        assert_eq!(db.search("keep", 10).unwrap().len(), 1);
    }
    #[test]
    fn unknown_schema_and_invalid_row_return_errors() {
        let d = crate::test_support::TempDir::new("schema").unwrap();
        let p = d.path().join("p/index.sqlite");
        let db = SqliteStore::open(&p).unwrap();
        db.connection()
            .pragma_update(None, "user_version", 999)
            .unwrap();
        drop(db);
        assert!(SqliteStore::open(&p).is_err());
        let d = crate::test_support::TempDir::new("row").unwrap();
        let p = d.path().join("p/index.sqlite");
        let db = SqliteStore::open(&p).unwrap();
        db.connection().execute("INSERT INTO sessions(agent,native_id,source_path,source_file_id,source_generation) VALUES(9,'x','p',1,1)",[]).unwrap();
        assert!(db.sessions().is_err());
    }
    /// Builds the schema 2 index exactly as the released build left it: no `kind`
    /// column, no provenance columns, and no auto-vacuum.
    fn legacy_schema_two(path: &Path, chunks: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("\
            CREATE TABLE sessions (agent INTEGER NOT NULL, native_id TEXT NOT NULL, source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL DEFAULT 0, source_end INTEGER NOT NULL DEFAULT 0, cwd TEXT, repository TEXT, repository_root TEXT, worktree TEXT, branch TEXT, commit_hash TEXT, started_at INTEGER, ended_at INTEGER, git_observed_at INTEGER, PRIMARY KEY(agent,native_id));
            CREATE TABLE indexed_files (path TEXT PRIMARY KEY, file_id INTEGER NOT NULL, generation INTEGER NOT NULL, committed_offset INTEGER NOT NULL, size INTEGER NOT NULL, modified INTEGER, open_turn_state BLOB);
            CREATE TABLE search_chunks (rowid INTEGER PRIMARY KEY, agent INTEGER NOT NULL, native_id TEXT NOT NULL, ordinal INTEGER NOT NULL, timestamp INTEGER, source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL, source_end INTEGER NOT NULL, text TEXT NOT NULL, FOREIGN KEY(agent,native_id) REFERENCES sessions(agent,native_id) ON DELETE CASCADE, UNIQUE(agent,native_id,source_file_id,source_generation,ordinal));
            CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='search_chunks', content_rowid='rowid', tokenize='unicode61');
            CREATE TRIGGER chunks_ai AFTER INSERT ON search_chunks BEGIN INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
            CREATE TRIGGER chunks_ad AFTER DELETE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); END;
            CREATE TRIGGER chunks_au AFTER UPDATE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
            INSERT INTO sessions(agent,native_id,source_path,source_file_id,source_generation,repository_root,repository,branch) VALUES(0,'legacy','/tmp/native.jsonl',1,1,'/captured/repository','/captured/repository','main');
            INSERT INTO indexed_files(path,file_id,generation,committed_offset,size) VALUES('/tmp/native.jsonl',1,1,10,10);
            PRAGMA user_version=2;")
            .unwrap();
        for ordinal in 0..chunks {
            conn.execute("INSERT INTO search_chunks(agent,native_id,ordinal,source_path,source_file_id,source_generation,source_start,source_end,text) VALUES(0,'legacy',?,'/tmp/native.jsonl',1,1,0,1,?)",
                params![ordinal as i64, format!("legacymarker {ordinal} {}", "conversation text ".repeat(32))]).unwrap();
        }
        drop(conn);
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn pragma(db: &SqliteStore, name: &str) -> i64 {
        db.connection()
            .pragma_query_value(None, name, |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn provenance_round_trips_and_is_replaced_with_the_session_row() {
        let d = crate::test_support::TempDir::new("provenance").unwrap();
        let mut db = SqliteStore::open(d.path().join("private/index.sqlite")).unwrap();
        let s = Session {
            repository_root: None,
            repository: Some("owner/name".into()),
            ..session(Agent::Codex, "recorded", 4, 1)
        };
        db.commit_batch(IndexBatch {
            sessions: vec![SessionRecord {
                session: s.clone(),
                git: Some(GitProvenance {
                    origin: GitOrigin::Recorded,
                    repository_url: Some("git@github.com:owner/name.git".into()),
                }),
            }],
            chunks: vec![chunk(&s.id, 4, 1, 0, "provenancemarker")],
            ..Default::default()
        })
        .unwrap();
        let record = db.session_record(&s.id).unwrap().unwrap();
        assert_eq!(record.origin(), Some(GitOrigin::Recorded));
        assert_eq!(
            record.git.unwrap().repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );
        assert_eq!(db.session_records().unwrap().len(), 1);
        let result = db.search("provenancemarker", 1).unwrap().remove(0);
        assert_eq!(result.git_origin, Some(GitOrigin::Recorded));
        assert_eq!(
            result.repository_url.as_deref(),
            Some("git@github.com:owner/name.git")
        );
        // Rewriting the session without provenance must not leave the old label behind.
        db.commit_batch(IndexBatch {
            sessions: vec![s.into()],
            ..Default::default()
        })
        .unwrap();
        let record = db
            .session_record(&SessionId::new(Agent::Codex, "recorded"))
            .unwrap()
            .unwrap();
        assert_eq!(record.origin(), None);
        assert_eq!(
            db.search("provenancemarker", 1).unwrap()[0].git_origin,
            None
        );
        assert_eq!(
            db.search("provenancemarker", 1).unwrap()[0].repository_url,
            None
        );
    }

    #[test]
    fn migrating_a_mass_deleting_schema_reclaims_its_free_pages() {
        let d = crate::test_support::TempDir::new("compaction").unwrap();
        let p = d.path().join("private/index.sqlite");
        legacy_schema_two(&p, 2000);
        let before = fs::metadata(&p).unwrap().len();

        let db = SqliteStore::open(&p).unwrap();
        assert_eq!(pragma(&db, "user_version"), 6);
        // The upgrade drops every mixed-speaker chunk; the file must not keep the pages.
        assert_eq!(db.status().unwrap().chunks, 0);
        let (pages, free) = (pragma(&db, "page_count"), pragma(&db, "freelist_count"));
        assert!(
            free * 4 <= pages,
            "free pages {free} dominate {pages} after migration"
        );
        // Incremental auto-vacuum is now available for ordinary rebuild churn.
        assert_eq!(pragma(&db, "auto_vacuum"), 2);
        drop(db);
        let after = fs::metadata(&p).unwrap().len();
        assert!(after * 2 < before, "{after} did not shrink from {before}");

        // Identities, checkpoints, and captured Git context survive the rewrite.
        let db = SqliteStore::open(&p).unwrap();
        let session = db
            .session(&SessionId::new(Agent::Claude, "legacy"))
            .unwrap()
            .unwrap();
        assert_eq!(
            session.repository_root,
            Some(PathBuf::from("/captured/repository"))
        );
        assert_eq!(
            db.indexed_file("/tmp/native.jsonl")
                .unwrap()
                .unwrap()
                .committed_offset,
            10
        );
        assert!(db.search("legacymarker", 10).unwrap().is_empty());
    }

    #[test]
    fn rebuild_churn_is_reclaimed_without_rewriting_the_file() {
        let d = crate::test_support::TempDir::new("reclaim").unwrap();
        let p = d.path().join("private/index.sqlite");
        let mut db = SqliteStore::open(&p).unwrap();
        let s = session(Agent::Claude, "one", 7, 1);
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone().into()],
            chunks: (0..2000)
                .map(|ordinal| {
                    chunk(
                        &s.id,
                        7,
                        1,
                        ordinal,
                        &format!("reclaimmarker {}", "conversation text ".repeat(32)),
                    )
                })
                .collect(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(db.reclaim_free_pages().unwrap(), 0);
        // Measured in pages: in WAL mode the main file only shrinks at a checkpoint.
        let full = pragma(&db, "page_count");
        db.commit_batch(IndexBatch {
            removed_sources: vec![(7, 1)],
            ..Default::default()
        })
        .unwrap();
        let reclaimed = db.reclaim_free_pages().unwrap();
        assert!(reclaimed > 0, "no pages were released");
        assert!(pragma(&db, "freelist_count") * 4 <= pragma(&db, "page_count"));
        assert!(
            pragma(&db, "page_count") * 2 < full,
            "the index kept its replaced pages"
        );
    }

    #[test]
    fn permissions_and_sidecar_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let d = crate::test_support::TempDir::new("permissions").unwrap();
        let p = d.path().join("private/index.sqlite");
        let db = SqliteStore::open(&p).unwrap();
        assert_eq!(
            fs::metadata(d.path().join("private"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(db);
        let target = d.path().join("target");
        fs::write(&target, "x").unwrap();
        std::os::unix::fs::symlink(
            &target,
            PathBuf::from(format!("{}-wal", p.to_string_lossy())),
        )
        .unwrap();
        assert!(SqliteStore::open(&p).is_err());
    }

    /// One session per entry, each with the given chunks in order.
    fn sessions_fixture(
        name: &str,
        sessions: &[(&str, &[&str])],
    ) -> (crate::test_support::TempDir, SqliteStore) {
        let dir = crate::test_support::TempDir::new(name).unwrap();
        let mut db = SqliteStore::open(dir.path().join("private/index.sqlite")).unwrap();
        for (n, (native, texts)) in sessions.iter().enumerate() {
            let file = 10 + n as u64;
            let s = session(Agent::Claude, native, file, 1);
            let chunks = texts
                .iter()
                .enumerate()
                .map(|(i, t)| chunk(&s.id, file, 1, i as u64, t))
                .collect();
            db.commit_batch(IndexBatch {
                sessions: vec![s.into()],
                chunks,
                ..Default::default()
            })
            .unwrap();
        }
        (dir, db)
    }

    fn sid(native: &str) -> SessionId {
        SessionId::new(Agent::Claude, native)
    }

    fn natives(results: &[SearchResult]) -> Vec<&str> {
        results
            .iter()
            .map(|r| r.session_id.native_id.as_str())
            .collect()
    }

    /// Five sessions that say "deploy" plainly, and one, `quiet`, that only
    /// mentions it in passing, so relevance ranks it last.
    fn deploy_fixture(name: &str) -> (crate::test_support::TempDir, SqliteStore) {
        sessions_fixture(
            name,
            &[
                ("a", &["deploy deploy now", "unrelated words"]),
                ("b", &["deploy deploy again"]),
                ("c", &["deploy deploy today"]),
                ("d", &["deploy deploy staging"]),
                ("e", &["deploy deploy production"]),
                (
                    "quiet",
                    &[
                        "a long note about the weather, lunch, trains, music and, once, deploy",
                        "the rate-limit middleware",
                    ],
                ),
            ],
        )
    }

    /// The planner prefers the primary key for a `rowid=?` lookup however wide
    /// the row is, so the hint is what keeps the ranking phase off the text.
    /// Without this test, dropping it would cost seconds on a large index and
    /// nothing a passing suite would notice.
    #[test]
    fn ranking_a_match_reads_the_index_and_not_the_chunk() {
        let (_dir, db) = deploy_fixture("storage-covering-plan");
        let plans = |sql: &str| -> String {
            let mut st = db.conn.prepare(sql).unwrap();
            // A plan does not depend on the values, only on their presence.
            let nulls = vec![rusqlite::types::Value::Null; st.parameter_count()];
            let rows = st
                .query_map(rusqlite::params_from_iter(nulls), |r| r.get::<_, String>(3))
                .unwrap()
                .map(|r| r.unwrap())
                .collect::<Vec<_>>();
            rows.join("\n")
        };
        // The queries the search actually runs, not copies of them.
        for (what, sql) in [
            (
                "ranked search",
                search_sql("ORDER BY bm25(chunks_fts),c.agent,c.native_id,c.ordinal LIMIT ?"),
            ),
            ("prioritized search", prioritized_sql("(?,?,?)")),
        ] {
            let plan = plans(&format!("EXPLAIN QUERY PLAN {sql}"));
            assert!(
                plan.contains("COVERING INDEX chunks_meta"),
                "{what} stopped using the covering index:\n{plan}"
            );
        }
    }

    #[test]
    fn an_index_without_the_covering_index_gains_it_on_open() {
        let (dir, db) = deploy_fixture("storage-covering-migration");
        let before = db.search_with_role("deploy", 10, None).unwrap();
        let path = dir.path().join("private/index.sqlite");
        db.conn
            .execute_batch("DROP INDEX chunks_meta; PRAGMA user_version=5;")
            .unwrap();
        drop(db);

        let db = SqliteStore::open(&path).unwrap();
        assert_eq!(pragma(&db, "user_version"), 6);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='chunks_meta'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap(),
            1,
        );
        // The index changes how the rows are reached, never which ones.
        assert_eq!(db.search_with_role("deploy", 10, None).unwrap(), before);
    }

    #[test]
    fn without_priorities_search_is_unchanged() {
        let (_dir, db) = deploy_fixture("storage-priority-none");
        let none = Priorities::default();
        for (query, kind) in [
            ("deploy", None),
            ("deploy", Some(crate::EventKind::User)),
            ("deploy", Some(crate::EventKind::Assistant)),
            ("rate-limit", None),
            ("deplyo", None),
            ("", None),
        ] {
            let plain = db.search_with_fallback(query, 3, kind).unwrap();
            let prioritized = db.search_prioritized(query, 3, kind, &none).unwrap();
            assert_eq!(prioritized, plain, "{query:?}");
            assert_eq!(prioritized.prioritized, 0);
            if !plain.is_approximate() {
                assert_eq!(plain.results, db.search_with_role(query, 3, kind).unwrap());
            }
        }
    }

    #[test]
    fn priorities_without_a_match_change_nothing() {
        let (_dir, db) = deploy_fixture("storage-priority-unmatched");
        // `a` is indexed but does not mention the weather; `ghost` is not indexed.
        for priorities in [
            Priorities::from_iter([(sid("ghost"), 0)]),
            Priorities::from_iter([(sid("a"), 0), (sid("ghost"), 1)]),
        ] {
            for query in ["deploy", "weather", "wether"] {
                let plain = db.search_with_fallback(query, 3, None).unwrap();
                assert!(!plain.results.is_empty(), "{query:?}");
                let outcome = db.search_prioritized(query, 3, None, &priorities).unwrap();
                if query == "deploy" && priorities.group(&sid("a")).is_some() {
                    // `a` matches "deploy", so it is lifted; nothing else moves.
                    assert_eq!(outcome.prioritized, 1);
                    continue;
                }
                assert_eq!(outcome, plain, "{query:?}");
            }
        }
    }

    #[test]
    fn a_prioritized_session_ranked_past_the_limit_comes_first() {
        let (_dir, db) = deploy_fixture("storage-priority-cut");
        let plain = db.search_with_role("deploy", 3, None).unwrap();
        assert!(!natives(&plain).contains(&"quiet"), "{:?}", natives(&plain));
        let outcome = db
            .search_prioritized(
                "deploy",
                3,
                None,
                &Priorities::from_iter([(sid("quiet"), 0)]),
            )
            .unwrap();
        assert_eq!(outcome.prioritized, 1);
        assert_eq!(outcome.results.len(), 3, "the limit bounds the whole list");
        assert_eq!(natives(&outcome.results)[0], "quiet");
        assert!(outcome.results[0].snippet.contains("deploy"));
        assert_eq!(outcome.results[1..], plain[..2]);
    }

    #[test]
    fn a_prioritized_session_contributes_its_best_match_once() {
        let (_dir, db) = sessions_fixture(
            "storage-priority-best",
            &[
                (
                    "busy",
                    &["cache once", "cache cache cache", "cache twice cache"],
                ),
                ("other", &["cache cache", "more cache"]),
            ],
        );
        let plain = db.search_with_role("cache", 50, None).unwrap();
        let outcome = db
            .search_prioritized(
                "cache",
                50,
                None,
                &Priorities::from_iter([(sid("busy"), 0)]),
            )
            .unwrap();
        assert_eq!(outcome.prioritized, 1);
        assert_eq!(outcome.results[0].snippet, "cache cache cache");
        // The rest keep relevance order, without the lifted row repeated, and
        // still include the prioritized session's other matches.
        let rest: Vec<_> = plain
            .iter()
            .filter(|r| r.source != outcome.results[0].source)
            .cloned()
            .collect();
        assert_eq!(outcome.results[1..], rest[..]);
        assert_eq!(outcome.results.len(), plain.len());
    }

    #[test]
    fn prioritized_sessions_order_by_group_then_relevance_then_id() {
        let (_dir, db) = sessions_fixture(
            "storage-priority-ties",
            &[
                ("strong", &["merge merge merge conflict"]),
                (
                    "weak",
                    &["merge, after a long discussion of other matters entirely"],
                ),
                ("twin-b", &["merge conflict"]),
                ("twin-a", &["merge conflict"]),
                ("plain", &["merge merge merge merge"]),
            ],
        );
        let same = Priorities::from_iter([
            (sid("weak"), 0),
            (sid("strong"), 0),
            (sid("twin-b"), 0),
            (sid("twin-a"), 0),
        ]);
        let outcome = db.search_prioritized("merge", 50, None, &same).unwrap();
        assert_eq!(outcome.prioritized, 4);
        // Equal scores fall back to the session ID, as the ordinary search does.
        assert_eq!(
            natives(&outcome.results),
            ["strong", "twin-a", "twin-b", "weak", "plain"]
        );
        // A lower group comes first however weak its match.
        let groups = Priorities::from_iter([(sid("weak"), 0), (sid("strong"), 1)]);
        let outcome = db.search_prioritized("merge", 50, None, &groups).unwrap();
        assert_eq!(outcome.prioritized, 2);
        assert_eq!(natives(&outcome.results)[..2], ["weak", "strong"]);
        // A session given twice keeps its lowest group.
        let twice =
            Priorities::from_iter([(sid("strong"), 3), (sid("weak"), 1), (sid("strong"), 0)]);
        assert_eq!(twice.group(&sid("strong")), Some(0));
        let outcome = db.search_prioritized("merge", 50, None, &twice).unwrap();
        assert_eq!(natives(&outcome.results)[..2], ["strong", "weak"]);
    }

    #[test]
    fn near_match_fallback_is_decided_by_the_ordinary_results() {
        let (_dir, db) = sessions_fixture(
            "storage-priority-fallback",
            &[
                ("exact", &["the database migration"]),
                ("near", &["a databse typo"]),
                ("live", &["another databse typo", "the rate-limit header"]),
            ],
        );
        let live = Priorities::from_iter([(sid("live"), 0)]);
        // An exact match elsewhere means no retry: the prioritized session's near
        // match does not displace it, and nothing is lifted.
        let outcome = db.search_prioritized("database", 50, None, &live).unwrap();
        assert!(!outcome.is_approximate());
        assert_eq!(outcome.prioritized, 0);
        assert_eq!(natives(&outcome.results), ["exact"]);
        // Only near matches: both parts use the widened expression, the
        // prioritized one comes first, and the outcome says it is approximate.
        let outcome = db.search_prioritized("databsee", 50, None, &live).unwrap();
        assert_eq!(
            outcome.approximation().as_deref(),
            Some("databsee → databse, database, databsee*")
        );
        assert_eq!(outcome.prioritized, 1);
        assert_eq!(natives(&outcome.results)[0], "live");
        let mut rest = natives(&outcome.results[1..]);
        rest.sort_unstable();
        assert_eq!(rest, ["exact", "near"]);
        // Punctuation stays literal: `rate-limit` is the phrase `rate limit`.
        let outcome = db
            .search_prioritized("rate-limit", 50, None, &live)
            .unwrap();
        assert_eq!(outcome.prioritized, 1);
        assert_eq!(outcome.results.len(), 1);
        assert!(db
            .search_prioritized("limit-rate", 50, None, &live)
            .unwrap()
            .results
            .is_empty());
    }
}

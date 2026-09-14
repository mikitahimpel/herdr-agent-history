//! Disposable local SQLite index. Native transcript files are never written here.
use crate::{
    Agent, CoreError, IndexBatch, IndexedFile, Result, SearchResult, Session, SessionId, Store,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: i32 = 1;
const MAX_LIMIT: usize = 1000;

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
        let conn = Connection::open(path)?;
        Self::private_file(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        store.conn.pragma_update(None, "journal_mode", "WAL")?;
        Ok(store)
    }

    fn make_private_parent(parent: &Path) -> Result<()> {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
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
        let version: i32 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > VERSION {
            return Err(CoreError::Unsupported(format!(
                "database schema version {version} is newer than supported"
            )));
        }
        if version == 0 {
            let tx = self.conn.transaction()?;
            tx.execute_batch("\
                CREATE TABLE sessions (agent INTEGER NOT NULL, native_id TEXT NOT NULL, source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL DEFAULT 0, source_end INTEGER NOT NULL DEFAULT 0, cwd TEXT, repository TEXT, repository_root TEXT, worktree TEXT, branch TEXT, commit_hash TEXT, started_at INTEGER, ended_at INTEGER, git_observed_at INTEGER, PRIMARY KEY(agent,native_id));
                CREATE TABLE indexed_files (path TEXT PRIMARY KEY, file_id INTEGER NOT NULL, generation INTEGER NOT NULL, committed_offset INTEGER NOT NULL, size INTEGER NOT NULL, modified INTEGER, open_turn_state BLOB);
                CREATE TABLE search_chunks (rowid INTEGER PRIMARY KEY, agent INTEGER NOT NULL, native_id TEXT NOT NULL, ordinal INTEGER NOT NULL, timestamp INTEGER, source_path TEXT NOT NULL, source_file_id INTEGER NOT NULL, source_generation INTEGER NOT NULL, source_start INTEGER NOT NULL, source_end INTEGER NOT NULL, text TEXT NOT NULL, FOREIGN KEY(agent,native_id) REFERENCES sessions(agent,native_id) ON DELETE CASCADE, UNIQUE(agent,native_id,ordinal));
                CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='search_chunks', content_rowid='rowid', tokenize='unicode61');
                CREATE TRIGGER chunks_ai AFTER INSERT ON search_chunks BEGIN INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
                CREATE TRIGGER chunks_ad AFTER DELETE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); END;
                CREATE TRIGGER chunks_au AFTER UPDATE ON search_chunks BEGIN INSERT INTO chunks_fts(chunks_fts,rowid,text) VALUES ('delete',old.rowid,old.text); INSERT INTO chunks_fts(rowid,text) VALUES (new.rowid,new.text); END;
                PRAGMA user_version=1;")?;
            tx.commit()?;
        }
        Ok(())
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
    pub fn session(&self, id: &SessionId) -> Result<Option<Session>> {
        self.conn.query_row("SELECT agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at FROM sessions WHERE agent=? AND native_id=?", params![agent_i(id.agent), id.native_id], row_session).optional().map_err(Into::into)
    }
    pub fn sessions(&self) -> Result<Vec<Session>> {
        let mut s=self.conn.prepare("SELECT agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at FROM sessions ORDER BY native_id")?;
        let rows = s
            .query_map([], row_session)?
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
    pub fn commit_batch(&mut self, batch: IndexBatch) -> Result<()> {
        <Self as Store>::commit_batch(self, batch)
    }
}

impl Store for SqliteStore {
    fn commit_batch(&mut self, batch: IndexBatch) -> Result<()> {
        let tx = self.conn.transaction()?;
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
        for s in batch.sessions {
            tx.execute("INSERT INTO sessions(agent,native_id,source_path,source_file_id,source_generation,source_start,source_end,cwd,repository,repository_root,worktree,branch,commit_hash,started_at,ended_at,git_observed_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(agent,native_id) DO UPDATE SET source_path=excluded.source_path,source_file_id=excluded.source_file_id,source_generation=excluded.source_generation,source_start=excluded.source_start,source_end=excluded.source_end,cwd=excluded.cwd,repository=excluded.repository,repository_root=excluded.repository_root,worktree=excluded.worktree,branch=excluded.branch,commit_hash=excluded.commit_hash,started_at=excluded.started_at,ended_at=excluded.ended_at,git_observed_at=excluded.git_observed_at",params![agent_i(s.id.agent),s.id.native_id,pstr(&s.source.path),s.source.file_id,s.source.generation,s.source.byte_range.start,s.source.byte_range.end,optp(&s.cwd),s.repository,s.repository_root.as_ref().map(|p|pstr(p)),s.worktree.as_ref().map(|p|pstr(p)),s.branch,s.commit,ts(s.started_at),ts(s.ended_at),ts(s.git_observed_at)])?;
        }
        for c in batch.chunks {
            tx.execute("INSERT INTO search_chunks(agent,native_id,ordinal,timestamp,source_path,source_file_id,source_generation,source_start,source_end,text) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(agent,native_id,ordinal) DO UPDATE SET timestamp=excluded.timestamp,source_path=excluded.source_path,source_file_id=excluded.source_file_id,source_generation=excluded.source_generation,source_start=excluded.source_start,source_end=excluded.source_end,text=excluded.text",params![agent_i(c.session_id.agent),c.session_id.native_id,c.ordinal,ts(c.timestamp),pstr(&c.source.path),c.source.file_id,c.source.generation,c.source.byte_range.start,c.source.byte_range.end,c.text])?;
        }
        if let Some(f) = batch.file {
            tx.execute("INSERT INTO indexed_files(path,file_id,generation,committed_offset,size,modified,open_turn_state) VALUES(?,?,?,?,?,?,?) ON CONFLICT(path) DO UPDATE SET file_id=excluded.file_id,generation=excluded.generation,committed_offset=excluded.committed_offset,size=excluded.size,modified=excluded.modified,open_turn_state=excluded.open_turn_state",params![pstr(&f.path),f.file_id,f.generation,f.committed_offset,f.size,ts(f.modified),batch.open_turn_state])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_LIMIT);
        let mut st=self.conn.prepare("SELECT c.agent,c.native_id,s.repository,s.branch,s.cwd,c.timestamp,c.source_path,c.source_file_id,c.source_generation,c.source_start,c.source_end,snippet(chunks_fts,0,'','', ' … ', 24) FROM chunks_fts JOIN search_chunks c ON c.rowid=chunks_fts.rowid JOIN sessions s ON s.agent=c.agent AND s.native_id=c.native_id WHERE chunks_fts MATCH ? ORDER BY bm25(chunks_fts),c.agent,c.native_id,c.ordinal LIMIT ?")?;
        let rows = st
            .query_map(params![query, limit as i64], |r| {
                Ok(SearchResult {
                    session_id: SessionId::new(agent_from(r.get(0)?)?, r.get::<_, String>(1)?),
                    agent: agent_from(r.get(0)?)?,
                    repository: r.get(2)?,
                    branch: r.get(3)?,
                    cwd: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
                    timestamp: from_ts(r.get(5)?),
                    source: crate::SourceRef::new(
                        PathBuf::from(r.get::<_, String>(6)?),
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?..r.get(10)?,
                    )
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    snippet: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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
    #[test]
    fn fts_insert_delete_and_reindex_are_transactional() {
        let dir = crate::test_support::TempDir::new("storage").unwrap();
        let path = dir.path().join("index.sqlite");
        let mut db = SqliteStore::open(&path).unwrap();
        let s = session(Agent::Claude, "one", 7, 1);
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone()],
            chunks: vec![chunk(&s.id, 7, 1, 0, "portfolio visibility")],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(db.search("portfolio", 10).unwrap().len(), 1);
        db.commit_batch(IndexBatch {
            removed_sources: vec![(7, 1)],
            ..Default::default()
        })
        .unwrap();
        assert!(db.search("portfolio", 10).unwrap().is_empty());
        db.commit_batch(IndexBatch {
            sessions: vec![s.clone()],
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
        let mut db = SqliteStore::open(dir.path().join("index.sqlite")).unwrap();
        let s = session(Agent::Codex, "two", 8, 1);
        let bad = ConversationChunk {
            session_id: s.id.clone(),
            ordinal: 0,
            timestamp: None,
            source: SourceRef::new("/tmp/native.jsonl", 8, 1, 0..0).unwrap(),
            text: "bad".into(),
        };
        // Duplicate session insertion with an invalid foreign key chunk forces rollback.
        let result = db.commit_batch(IndexBatch {
            sessions: vec![s],
            chunks: vec![ConversationChunk {
                session_id: SessionId::new(Agent::Codex, "missing"),
                ..bad
            }],
            ..Default::default()
        });
        assert!(result.is_err());
        assert_eq!(db.status().unwrap(), IndexStatus::default());
    }
}

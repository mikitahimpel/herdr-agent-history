//! Indexing on a thread of the running process, so a browser can search the index as it
//! is while discovery runs.
//!
//! The thread opens its own connection. SQLite in WAL mode lets that writer commit while
//! other connections read: each reading statement sees one committed snapshot, never a
//! half-applied file. Every guarantee of [`index_all_until`] holds unchanged, because the
//! thread runs exactly that function. Nothing outlives the owner: dropping the handle
//! stops the scan and joins the thread.
use crate::{
    index::{index_all_until, IndexProgress, IndexReport},
    AgentAdapter, SqliteStore,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
};

/// What the scan has done so far.
#[derive(Clone, Debug, Default)]
pub struct ScanSnapshot {
    /// The latest progress report; `None` until discovery has finished.
    pub progress: Option<IndexProgress>,
    /// Chunks written by files whose commit has finished. Readers that see this grow
    /// can find new content by searching again.
    pub committed_chunks: u64,
    /// Set once the scan has ended, successfully or not.
    pub outcome: Option<Result<IndexReport, String>>,
}

impl ScanSnapshot {
    pub fn finished(&self) -> bool {
        self.outcome.is_some()
    }
}

pub struct BackgroundIndex {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<ScanSnapshot>>,
    thread: Option<JoinHandle<()>>,
}

impl BackgroundIndex {
    /// Starts indexing `db` on a new thread. `adapters` runs on that thread, so the
    /// adapters themselves never cross threads. The caller should open `db` itself
    /// first: opening applies any schema migration, which is then done before the
    /// scan's connection exists.
    pub fn spawn(
        db: PathBuf,
        adapters: impl FnOnce() -> Vec<Box<dyn AgentAdapter>> + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(ScanSnapshot::default()));
        let thread = {
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("agent-history-index".into())
                .spawn(move || {
                    let mut completed = 0;
                    let outcome = SqliteStore::open(&db).and_then(|mut store| {
                        index_all_until(&mut store, &adapters(), &stop, |p| {
                            let mut snapshot = lock(&shared);
                            // Reports that close a file carry only committed totals.
                            if p.completed_files > completed {
                                completed = p.completed_files;
                                snapshot.committed_chunks = p.chunks;
                            }
                            snapshot.progress = Some(p);
                        })
                    });
                    lock(&shared).outcome = Some(outcome.map_err(|e| e.to_string()));
                })?
        };
        Ok(Self {
            stop,
            shared,
            thread: Some(thread),
        })
    }

    pub fn snapshot(&self) -> ScanSnapshot {
        lock(&self.shared).clone()
    }

    /// Asks the scan to stop without waiting for it. A file being indexed at that
    /// moment is abandoned before its commit.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Asks the scan to stop, waits for its thread, and returns the final state.
    pub fn stop(mut self) -> ScanSnapshot {
        self.halt();
        self.snapshot()
    }

    fn halt(&mut self) {
        self.request_stop();
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                let mut snapshot = lock(&self.shared);
                if snapshot.outcome.is_none() {
                    snapshot.outcome = Some(Err("indexing stopped unexpectedly".into()));
                }
            }
        }
    }
}

impl Drop for BackgroundIndex {
    fn drop(&mut self) {
        self.halt();
    }
}

/// A panic on the scan thread must not also take down every reader of its progress.
fn lock(shared: &Mutex<ScanSnapshot>) -> MutexGuard<'_, ScanSnapshot> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapters::ClaudeAdapter, CoreError, ParsedRecord, Result, Session, SessionFile, SourceRef,
    };
    use std::{
        fs,
        io::Write,
        path::Path,
        sync::mpsc::{channel, Receiver, Sender},
        time::{Duration, Instant},
    };

    fn line(session: &str, text: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"sessionId\":\"{session}\",\"message\":{{\"content\":{}}}}}\n",
            serde_json::to_string(text).unwrap()
        )
    }

    fn append(path: &Path, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    /// Claude parsing, except that a record containing `gate` waits for the test, and
    /// one containing `poison` fails the whole file.
    struct Scripted {
        root: PathBuf,
        gate: &'static str,
        entered: Sender<()>,
        release: Receiver<()>,
    }
    impl AgentAdapter for Scripted {
        fn agent(&self) -> crate::Agent {
            crate::Agent::Claude
        }
        fn discover(&self) -> Result<Vec<SessionFile>> {
            ClaudeAdapter::with_root(self.root.clone()).discover()
        }
        fn parse_record(&self, s: &Session, r: &[u8], src: SourceRef) -> Result<ParsedRecord> {
            let text = String::from_utf8_lossy(r);
            if text.contains("poison") {
                return Err(CoreError::Io(std::io::Error::other("unreadable record")));
            }
            if text.contains(self.gate) {
                let _ = self.entered.send(());
                let _ = self.release.recv();
            }
            ClaudeAdapter::with_root(self.root.clone()).parse_record(s, r, src)
        }
    }

    struct Fixture {
        _temp: crate::test_support::TempDir,
        root: PathBuf,
        db: PathBuf,
    }
    impl Fixture {
        /// A private index that already holds `oldword` from one committed file.
        fn new(name: &str) -> Self {
            let temp = crate::test_support::TempDir::new(name).unwrap();
            let root = temp.path().join("history");
            fs::create_dir_all(root.join("project")).unwrap();
            fs::write(root.join("project/old.jsonl"), line("old", "oldword")).unwrap();
            let db = temp.path().join("private/index.sqlite");
            let mut store = SqliteStore::open(&db).unwrap();
            crate::index::index_all(
                &mut store,
                &[Box::new(ClaudeAdapter::with_root(root.clone()))],
            )
            .unwrap();
            Self {
                _temp: temp,
                root,
                db,
            }
        }
        /// Starts a scan whose record containing `gate` blocks until the returned
        /// sender sends, and waits until it is blocked there.
        fn gated_scan(&self, gate: &'static str) -> (BackgroundIndex, Sender<()>) {
            let (entered, entered_rx) = channel();
            let (release_tx, release) = channel();
            let root = self.root.clone();
            let scan = BackgroundIndex::spawn(self.db.clone(), move || {
                vec![Box::new(Scripted {
                    root,
                    gate,
                    entered,
                    release,
                }) as Box<dyn AgentAdapter>]
            })
            .unwrap();
            entered_rx
                .recv_timeout(Duration::from_secs(30))
                .expect("the scan reaches the gated record");
            (scan, release_tx)
        }
    }

    fn wait(scan: &BackgroundIndex) -> ScanSnapshot {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let snapshot = scan.snapshot();
            if snapshot.finished() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "scan did not finish");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn integrity(store: &SqliteStore) -> String {
        store
            .connection()
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn the_existing_index_answers_while_a_scan_runs_and_new_content_follows_its_commit() {
        let f = Fixture::new("bg-serve");
        fs::write(f.root.join("project/new.jsonl"), line("new", "newword")).unwrap();
        let store = SqliteStore::open(&f.db).unwrap();
        let (scan, release) = f.gated_scan("newword");

        assert!(!scan.snapshot().finished());
        assert_eq!(store.search("oldword", 10).unwrap().len(), 1);
        assert!(store.search("newword", 10).unwrap().is_empty());

        release.send(()).unwrap();
        let done = wait(&scan);
        let report = done.outcome.unwrap().unwrap();
        assert!(!report.cancelled);
        assert_eq!(report.failed_files, 0);
        assert!(done.committed_chunks > 0, "the commit is announced");
        // The reader's connection was open throughout and sees the commit on its next query.
        assert_eq!(store.search("newword", 10).unwrap().len(), 1);
        assert_eq!(store.search("oldword", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_file_that_fails_partway_leaves_the_index_usable() {
        let f = Fixture::new("bg-fail");
        let bad = f.root.join("project/bad.jsonl");
        fs::write(&bad, line("bad", "badword") + &line("bad", "poison")).unwrap();
        fs::write(f.root.join("project/good.jsonl"), line("good", "goodword")).unwrap();
        let store = SqliteStore::open(&f.db).unwrap();
        let (scan, release) = f.gated_scan("goodword");
        release.send(()).unwrap();
        let report = wait(&scan).outcome.unwrap().unwrap();

        assert_eq!(report.failed_files, 1);
        assert_eq!(report.errors.len(), 1);
        assert!(!report.cancelled);
        assert!(
            store.search("badword", 10).unwrap().is_empty(),
            "nothing of the failed file"
        );
        assert!(store.indexed_file(&bad).unwrap().is_none());
        assert_eq!(store.search("goodword", 10).unwrap().len(), 1);
        assert_eq!(store.search("oldword", 10).unwrap().len(), 1);
        assert_eq!(integrity(&store), "ok");
    }

    #[test]
    fn stopping_mid_file_commits_nothing_of_it_and_the_next_scan_resumes_at_its_offset() {
        let f = Fixture::new("bg-stop");
        let old = f.root.join("project/old.jsonl");
        let store = SqliteStore::open(&f.db).unwrap();
        let before = store.indexed_file(&old).unwrap().unwrap();
        let appended = line("old", "firstappend") + &line("old", "stopword");
        append(&old, &appended);

        let (scan, release) = f.gated_scan("stopword");
        scan.request_stop();
        release.send(()).unwrap();
        let stopped = scan.stop();
        let report = stopped.outcome.unwrap().unwrap();
        assert!(report.cancelled);
        assert_eq!(report.failed_files, 0, "a stop is not a failure");
        assert_eq!(stopped.committed_chunks, 0);

        assert_eq!(store.indexed_file(&old).unwrap().unwrap(), before);
        assert!(store.search("firstappend", 10).unwrap().is_empty());
        assert_eq!(store.search("oldword", 10).unwrap().len(), 1);
        assert_eq!(integrity(&store), "ok");

        let mut writer = SqliteStore::open(&f.db).unwrap();
        let resumed = crate::index::index_all(
            &mut writer,
            &[Box::new(ClaudeAdapter::with_root(f.root.clone()))],
        )
        .unwrap();
        assert_eq!(
            resumed.bytes_read,
            appended.len() as u64,
            "only the appended bytes"
        );
        assert_eq!(store.search("firstappend", 10).unwrap().len(), 1);
        assert_eq!(store.search("stopword", 10).unwrap().len(), 1);
        let after = store.indexed_file(&old).unwrap().unwrap();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.committed_offset, fs::metadata(&old).unwrap().len());
    }

    #[test]
    fn dropping_the_handle_stops_and_joins_the_scan() {
        let f = Fixture::new("bg-drop");
        fs::write(f.root.join("project/new.jsonl"), line("new", "dropword")).unwrap();
        let (scan, release) = f.gated_scan("dropword");
        scan.request_stop();
        release.send(()).unwrap();
        drop(scan);
        let store = SqliteStore::open(&f.db).unwrap();
        assert!(store.search("dropword", 10).unwrap().is_empty());
        assert_eq!(integrity(&store), "ok");
    }
}

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

    /// Asks the scan to stop, waits for its thread, and returns the final state. A file
    /// being indexed at that moment is abandoned before its commit.
    pub fn stop(mut self) -> ScanSnapshot {
        self.halt();
        self.snapshot()
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
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

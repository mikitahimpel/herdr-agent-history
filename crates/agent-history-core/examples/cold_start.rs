//! Time until the first search result, with activation indexing run first
//! (`sync`) or on a background thread (`background`), against an existing index.
//!
//! `cargo run --release -p agent-history-core --example cold_start -- <db> [trials] [warm]`
//!
//! `<db>` is a private copy of an index and is never opened itself: each trial
//! copies it (with any WAL sidecar) into a fresh private directory, so both
//! modes start from the same index and have the same changed files to find.
//! A copy starts outside the OS page cache, like an index that has not been
//! used since boot; `warm` reads each copy once first, like one used recently.
//! An `idle` baseline opens and searches a copy with no scan at all.
//! History is discovered as the browser does, from `CLAUDE_CONFIG_DIR` and
//! `CODEX_HOME` or their defaults under `$HOME`; it is only read.
//!
//! Once both copies of a trial have settled, the same queries are run against
//! each and their ranked results compared, and settled search latency is timed.
use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    background::BackgroundIndex,
    default_index_path,
    index::index_all,
    test_support::TempDir,
    AgentAdapter, SqliteStore,
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const QUERIES: [&str; 8] = [
    "error", "function", "test", "index", "search", "review", "config", "install",
];
const REPS: usize = 20;

fn adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(ClaudeAdapter::default()),
        Box::new(CodexAdapter::default()),
    ]
}

fn copy_index(from: &Path, dir: &Path, warm: bool) -> std::io::Result<PathBuf> {
    let private = dir.join("private");
    fs::create_dir(&private)?;
    fs::set_permissions(
        &private,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )?;
    let to = private.join("index.sqlite");
    for suffix in ["", "-wal"] {
        let source = PathBuf::from(format!("{}{suffix}", from.display()));
        if source.exists() {
            fs::copy(source, format!("{}{suffix}", to.display()))?;
        }
    }
    if warm {
        std::io::copy(&mut fs::File::open(&to)?, &mut std::io::sink())?;
    }
    Ok(to)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

type Ranked = Vec<Vec<(String, u64, u64)>>;

fn ranked(store: &SqliteStore) -> agent_history_core::Result<Ranked> {
    QUERIES
        .iter()
        .map(|q| {
            Ok(store
                .search_with_fallback(q, 50, None)?
                .results
                .into_iter()
                .map(|r| {
                    (
                        r.session_id.native_id,
                        r.source.byte_range.start,
                        r.source.byte_range.end,
                    )
                })
                .collect())
        })
        .collect()
}

fn p50_p95(store: &SqliteStore) -> agent_history_core::Result<(f64, f64)> {
    let mut t = Vec::new();
    for _ in 0..REPS {
        for q in QUERIES {
            let start = Instant::now();
            store.search_with_fallback(q, 50, None)?;
            t.push(start.elapsed());
        }
    }
    t.sort_unstable();
    Ok((ms(t[t.len() / 2]), ms(t[t.len() * 95 / 100])))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let pristine = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: cold_start <path to a private copy of an index> [trials]")?;
    if Some(&pristine) == default_index_path().as_ref() {
        return Err("refusing to read the shared installed index; pass a copy".into());
    }
    let trials: usize = match args.next() {
        Some(n) => n.to_string_lossy().parse()?,
        None => 3,
    };
    let warm = args.next().is_some_and(|a| a == "warm");
    for trial in 1..=trials {
        // Baseline: open and search with no scan at all.
        let dir_idle = TempDir::new("cold-start-idle")?;
        let db_idle = copy_index(&pristine, dir_idle.path(), warm)?;
        let start = Instant::now();
        SqliteStore::open(&db_idle)?.search_with_fallback(QUERIES[0], 50, None)?;
        let idle_first = start.elapsed();

        // Sync: what activation did before, index everything and then search.
        let dir = TempDir::new("cold-start-sync")?;
        let db = copy_index(&pristine, dir.path(), warm)?;
        let start = Instant::now();
        let mut sync_store = SqliteStore::open(&db)?;
        let report = index_all(&mut sync_store, &adapters())?;
        sync_store.search_with_fallback(QUERIES[0], 50, None)?;
        let sync_first = start.elapsed();

        // Background: open, start the scan, search at once.
        let dir_bg = TempDir::new("cold-start-bg")?;
        let db_bg = copy_index(&pristine, dir_bg.path(), warm)?;
        let start = Instant::now();
        let store = SqliteStore::open(&db_bg)?;
        let opened = start.elapsed();
        let scan = BackgroundIndex::spawn(db_bg.clone(), adapters)?;
        store.search_with_fallback(QUERIES[0], 50, None)?;
        let bg_first = start.elapsed();
        let mut during = Vec::new();
        let mut discovered = None;
        while !scan.snapshot().finished() {
            if discovered.is_none() && scan.snapshot().progress.is_some() {
                discovered = Some(start.elapsed());
            }
            for q in QUERIES {
                let s = Instant::now();
                store.search_with_fallback(q, 50, None)?;
                during.push(s.elapsed());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let settled = start.elapsed();
        let bg_report = scan.stop().outcome.ok_or("no outcome")??;
        during.sort_unstable();

        // Stopping right after launch, as Ctrl-C at the first frame would.
        let dir_stop = TempDir::new("cold-start-stop")?;
        let db_stop = copy_index(&pristine, dir_stop.path(), warm)?;
        let stop_store = SqliteStore::open(&db_stop)?;
        let early = BackgroundIndex::spawn(db_stop, adapters)?;
        thread::sleep(Duration::from_millis(30));
        let stopping = Instant::now();
        let stopped = early.stop();
        let stop_ms = ms(stopping.elapsed());
        let stopped_discovered = stopped.progress.is_some();
        drop(stop_store);

        let same = ranked(&sync_store)? == ranked(&store)?;
        let (sync_p50, sync_p95) = p50_p95(&sync_store)?;
        let (bg_p50, bg_p95) = p50_p95(&store)?;
        println!(
            "trial={trial} files={} new_records={} failed={} idle_first_result_ms={:.1} \
             sync_first_result_ms={:.1} bg_open_ms={:.1} bg_first_result_ms={:.1} \
             bg_discovery_done_ms={:.1} bg_scan_settled_ms={:.1} bg_new_records={} bg_failed={} \
             stop_30ms_after_launch_ms={stop_ms:.1} stop_after_discovery={stopped_discovered} \
             during_scan_samples={} during_scan_p50_ms={:.2} during_scan_max_ms={:.2} \
             settled_ranking_identical={same} settled_p50_ms sync={sync_p50:.2} bg={bg_p50:.2} \
             settled_p95_ms sync={sync_p95:.2} bg={bg_p95:.2}",
            report.files,
            report.records,
            report.failed_files,
            ms(idle_first),
            ms(sync_first),
            ms(opened),
            ms(bg_first),
            discovered.map_or(f64::NAN, ms),
            ms(settled),
            bg_report.records,
            bg_report.failed_files,
            during.len(),
            during.get(during.len() / 2).copied().map_or(0.0, ms),
            during.last().copied().map_or(0.0, ms),
        );
        for e in bg_report.errors.iter().chain(&report.errors) {
            eprintln!("  indexing error: {}", e.rsplit(": ").next().unwrap_or(e));
        }
        if !same {
            return Err("settled rankings differ".into());
        }
    }
    Ok(())
}

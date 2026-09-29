//! Warm, in-process search latency against an existing index.
//!
//! `cargo run --release -p agent-history-core --example search_latency -- <db>`
//!
//! Opening an index can upgrade its schema, so pass a private copy, never the
//! shared installed index. Three sets are timed: exact queries through
//! `search_with_role`, the same queries through `search_with_fallback` (which
//! must cost the same, since they match), and misspelled queries that miss
//! and take the lenient retry. Nothing is written besides what opening does.
use agent_history_core::{default_index_path, SqliteStore};
use std::{path::PathBuf, time::Instant};

const REPS: usize = 20;
const EXACT: [&str; 8] = [
    "error", "function", "test", "index", "search", "review", "config", "install",
];
const MISSPELLED: [&str; 8] = [
    "eror", "fucntion", "tset", "indx", "serach", "reveiw", "confgi", "instal",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: search_latency <path to a private copy of an index>")?;
    if Some(&db) == default_index_path().as_ref() {
        return Err("refusing to open the shared installed index; pass a copy".into());
    }
    let store = SqliteStore::open(&db)?;
    // One untimed pass so every set sees the same warm page cache.
    for q in EXACT.iter().chain(&MISSPELLED) {
        store.search_with_fallback(q, 50, None)?;
    }
    let time =
        |queries: &[&str], lenient: bool| -> agent_history_core::Result<(Vec<u128>, usize)> {
            let mut timings = Vec::new();
            let mut approximate = 0;
            for _ in 0..REPS {
                for q in queries {
                    let start = Instant::now();
                    if lenient {
                        let outcome = store.search_with_fallback(q, 50, None)?;
                        approximate += usize::from(outcome.is_approximate());
                    } else {
                        store.search_with_role(q, 50, None)?;
                    }
                    timings.push(start.elapsed().as_micros());
                }
            }
            timings.sort_unstable();
            Ok((timings, approximate))
        };
    for (label, queries, lenient) in [
        ("exact", &EXACT, false),
        ("exact_via_fallback", &EXACT, true),
        ("misspelled_via_fallback", &MISSPELLED, true),
    ] {
        let (t, approximate) = time(queries, lenient)?;
        println!(
            "{label}: samples={} approximate={approximate} p50_us={} p95_us={} max_us={}",
            t.len(),
            t[t.len() / 2],
            t[(t.len() * 95).div_ceil(100) - 1],
            t[t.len() - 1]
        );
    }
    Ok(())
}

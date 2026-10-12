//! Warm, in-process search latency against an existing index.
//!
//! `cargo run --release -p agent-history-core --example search_latency -- <db>`
//!
//! Opening an index can upgrade its schema, so pass a private copy, never the
//! shared installed index. Three sets are timed: exact queries through
//! `search_with_role`, the same queries through `search_with_fallback` (which
//! must cost the same, since they match), and misspelled queries that miss
//! and take the lenient retry. Nothing is written besides what opening does.
//!
//! The `prioritized` sets repeat the fallback sets through
//! `search_prioritized`, with the sessions that have the most chunks
//! prioritized: the worst case for the extra query, since its cost follows the
//! prioritized sessions' matches. `PRIORITIZED` (default 3) sets how many.
use agent_history_core::{default_index_path, Agent, Priorities, SessionId, SqliteStore};
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
    let count: i64 = std::env::var("PRIORITIZED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let priorities: Priorities = store
        .connection()
        .prepare("SELECT agent,native_id FROM search_chunks GROUP BY agent,native_id ORDER BY count(*) DESC LIMIT ?")?
        .query_map([count], |r| {
            let agent = if r.get::<_, i64>(0)? == 0 { Agent::Claude } else { Agent::Codex };
            Ok((SessionId::new(agent, r.get::<_, String>(1)?), 0))
        })?
        .collect::<Result<_, _>>()?;
    // One untimed pass so every set sees the same warm page cache.
    for q in EXACT.iter().chain(&MISSPELLED) {
        store.search_with_fallback(q, 50, None)?;
    }
    let none = Priorities::default();
    let time = |queries: &[&str],
                lenient: bool,
                priorities: &Priorities|
     -> agent_history_core::Result<(Vec<u128>, usize, usize)> {
        let mut timings = Vec::new();
        let mut approximate = 0;
        let mut lifted = 0;
        for _ in 0..REPS {
            for q in queries {
                let start = Instant::now();
                if lenient {
                    let outcome = if priorities.is_empty() {
                        store.search_with_fallback(q, 50, None)?
                    } else {
                        store.search_prioritized(q, 50, None, priorities)?
                    };
                    approximate += usize::from(outcome.is_approximate());
                    lifted += outcome.prioritized;
                } else {
                    store.search_with_role(q, 50, None)?;
                }
                timings.push(start.elapsed().as_micros());
            }
        }
        timings.sort_unstable();
        Ok((timings, approximate, lifted))
    };
    for (label, queries, lenient, priorities) in [
        ("exact", &EXACT, false, &none),
        ("exact_via_fallback", &EXACT, true, &none),
        ("misspelled_via_fallback", &MISSPELLED, true, &none),
        ("exact_prioritized", &EXACT, true, &priorities),
        ("misspelled_prioritized", &MISSPELLED, true, &priorities),
    ] {
        let (t, approximate, lifted) = time(queries, lenient, priorities)?;
        println!(
            "{label}: samples={} approximate={approximate} lifted={lifted} p50_us={} p95_us={} max_us={}",
            t.len(),
            t[t.len() / 2],
            t[(t.len() * 95).div_ceil(100) - 1],
            t[t.len() - 1]
        );
    }
    Ok(())
}

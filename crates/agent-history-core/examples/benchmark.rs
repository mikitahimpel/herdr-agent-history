//! Reproducible synthetic benchmark for the core index and FTS search paths.
//!
//! The benchmark creates all transcripts in a temporary directory and removes
//! them when the process exits. It intentionally does not inspect user history.
use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::{index_all, Indexer},
    test_support::TempDir,
    AgentAdapter, SessionFile, SqliteStore,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Instant,
};

const FILES_PER_AGENT: usize = 100;
const TURNS_PER_FILE: usize = 100;

fn line(agent: &str, session: &str, turn: usize, role: &str) -> String {
    let text = format!(
        "{agent} turn {turn}: We are reviewing portfolio visibility, websocket subscriptions, release indexing, and repository restoration. The implementation should preserve searchable context and transactional offsets while keeping native history canonical.{}",
        if turn == 101 { " appendmarker7f4c2a" } else { "" }
    );
    if agent == "claude" {
        serde_json::json!({
            "type": role,
            "sessionId": session,
            "cwd": "/synthetic/agent-history-benchmark",
            "timestamp": format!("2026-09-14T12:{:02}:00Z", turn % 60),
            "message": {"role": role, "content": text}
        })
        .to_string()
            + "\n"
    } else {
        serde_json::json!({
            "type": "response_item",
            "timestamp": format!("2026-09-14T12:{:02}:00Z", turn % 60),
            "payload": {"type": "message", "role": role, "content": [{"type": "output_text", "text": text}], "cwd": "/synthetic/agent-history-benchmark"},
        })
        .to_string()
            + "\n"
    }
}

fn generate(root: &Path, agent: &str) -> std::io::Result<(Vec<PathBuf>, u64)> {
    let mut paths = Vec::with_capacity(FILES_PER_AGENT);
    let mut bytes = 0;
    for file_no in 0..FILES_PER_AGENT {
        let path = root.join(format!("{agent}-{file_no:03}.jsonl"));
        let session = format!("00000000-0000-4000-8000-{file_no:012x}");
        let mut out = fs::File::create(&path)?;
        if agent == "codex" {
            let metadata = serde_json::json!({
                "type": "session_meta",
                "timestamp": "2026-09-14T12:00:00Z",
                "payload": {"id": session, "cwd": "/synthetic/agent-history-benchmark"}
            })
            .to_string()
                + "\n";
            bytes += metadata.len() as u64;
            out.write_all(metadata.as_bytes())?;
        }
        for turn in 0..TURNS_PER_FILE {
            for role in ["user", "assistant"] {
                let record = line(agent, &session, turn, role);
                bytes += record.len() as u64;
                out.write_all(record.as_bytes())?;
            }
        }
        paths.push(path);
    }
    Ok((paths, bytes))
}

fn percentile(sorted: &[u128], p: usize) -> u128 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::new("performance")?;
    let temp_path = temp.path().to_owned();
    let claude_root = temp_path.join("claude/projects");
    let codex_root = temp_path.join("codex/sessions");
    fs::create_dir_all(&claude_root)?;
    fs::create_dir_all(&codex_root)?;
    let (claude_paths, claude_bytes) = generate(&claude_root, "claude")?;
    let (_codex_paths, codex_bytes) = generate(&codex_root, "codex")?;
    let raw_bytes = claude_bytes + codex_bytes;
    let claude = ClaudeAdapter::with_root(&claude_root);
    let codex = CodexAdapter::with_root(&codex_root);
    let adapters: Vec<Box<dyn AgentAdapter>> =
        vec![Box::new(claude.clone()), Box::new(codex.clone())];
    let discovered_start = Instant::now();
    let discovered_claude = claude.discover()?;
    let discovered_codex = codex.discover()?;
    let discovery_ms = discovered_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(discovered_claude.len(), FILES_PER_AGENT);
    assert_eq!(discovered_codex.len(), FILES_PER_AGENT);

    let db_dir = temp_path.join("db");
    fs::create_dir(&db_dir)?;
    fs::set_permissions(&db_dir, fs::Permissions::from_mode(0o700))?;
    let db_path = db_dir.join("index.sqlite");
    let mut store = SqliteStore::open(&db_path)?;
    let indexing_start = Instant::now();
    let initial = index_all(&mut store, &adapters)?;
    let indexing_ms = indexing_start.elapsed().as_secs_f64() * 1000.0;
    let status = store.status()?;
    assert_eq!(initial.files, FILES_PER_AGENT as u64 * 2);
    assert_eq!(initial.records, 40_100);
    assert_eq!(initial.chunks, 20_000);
    assert_eq!(initial.failed_files, 0);
    assert_eq!(status.sessions, FILES_PER_AGENT as u64 * 2);
    assert_eq!(status.chunks, 20_000);

    let append_path = &claude_paths[0];
    let append = line(
        "claude",
        "00000000-0000-4000-8000-000000000000",
        101,
        "user",
    );
    let append_bytes = append.len() as u64;
    let mut file = OpenOptions::new().append(true).open(append_path)?;
    file.write_all(append.as_bytes())?;
    file.sync_all()?;
    let append_start = Instant::now();
    let incremental = Indexer::new(claude.clone(), &mut store).index_file(&SessionFile {
        path: append_path.clone(),
        file_id: 0,
        generation: 0,
    })?;
    let incremental_ms = append_start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(incremental.bytes_read, append_bytes);
    let sentinel_results = store.search("appendmarker7f4c2a", 10)?;
    assert!(!sentinel_results.is_empty());
    assert!(sentinel_results
        .iter()
        .any(|result| result.source.path == *append_path));
    let final_status = store.status()?;
    assert_eq!(incremental.records, 1);
    assert_eq!(final_status.sessions, 200);
    assert_eq!(final_status.chunks, 20_001);

    let queries = [
        "portfolio",
        "visibility",
        "websocket",
        "subscriptions",
        "release",
        "indexing",
        "repository",
        "restoration",
        "transactional",
        "canonical",
    ];
    let mut timings = Vec::with_capacity(200);
    for n in 0..200 {
        let start = Instant::now();
        let results = store.search(queries[n % queries.len()], 20)?;
        timings.push(start.elapsed().as_nanos() / 1_000);
        assert!(!results.is_empty());
    }
    timings.sort_unstable();
    let db_bytes: u64 = ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| {
            fs::metadata(format!("{}{}", db_path.display(), suffix))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum();
    let p50_us = percentile(&timings, 50);
    let p95_us = percentile(&timings, 95);
    assert_eq!(store.sessions()?.len(), FILES_PER_AGENT * 2);
    let final_raw_bytes = raw_bytes + append_bytes;
    drop(store);
    println!(
        "machine_os={} machine_arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("dataset_files={} files_per_agent={} turns_per_file={} raw_jsonl_bytes={} raw_jsonl_mib={:.2}", FILES_PER_AGENT * 2, FILES_PER_AGENT, TURNS_PER_FILE, final_raw_bytes, final_raw_bytes as f64 / 1_048_576.0);
    println!("discovery_ms={discovery_ms:.3} initial_index_ms={indexing_ms:.3} records={} chunks={} sessions={}", initial.records + incremental.records, final_status.chunks, final_status.sessions);
    println!("incremental_append_bytes={} incremental_bytes_read={} incremental_index_ms={incremental_ms:.3}", append_bytes, incremental.bytes_read);
    println!("search_queries=200 warm_p50_us={p50_us} warm_p95_us={p95_us}");
    println!(
        "sqlite_db_wal_shm_bytes={} sqlite_to_raw_ratio={:.3}",
        db_bytes,
        db_bytes as f64 / final_raw_bytes as f64
    );
    println!("process_exits_after_measurement=true");
    Ok(())
}

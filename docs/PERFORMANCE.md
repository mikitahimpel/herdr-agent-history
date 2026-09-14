# Core performance measurements

This report covers the core indexer and SQLite FTS search path for issue #8/#13. It uses synthetic transcripts only; no private native history, database, or user worktree is read or modified.

## Reproduce

From the repository root, run:

```sh
./scripts/setup
cargo build --release --example benchmark -p agent-history-core
/usr/bin/time -l ./target/release/examples/benchmark
```

The example creates 100 Claude and 100 Codex JSONL files, with 100 user/assistant turns per file, under a temporary directory. Each run removes that directory at process exit. It asserts 200 discovered files and sessions, nonzero chunks, complete append accounting, and successful search results before printing measurements. The 200 search calls use ten varied terms and run after indexing, so these are warm in-process FTS timings.

## Current conversation-only schema 3 measurement

The same synthetic corpus now produces 40,000 initial speaker-separated chunks (40,001 after append). A fresh Apple Silicon release run measured initial indexing at 1,487.216 ms, the 449-byte append at 55.460 ms, warm search p50/p95 at 26.191/26.828 ms, and maximum RSS at 12,140,544 bytes. SQLite plus WAL/SHM occupied 31,289,296 bytes (1.794 times raw input). This corpus contains user/assistant prose and no large tool outputs, so it measures the cost of role separation rather than storage savings from excluding tools. Native-history performance remains unmeasured for this build.

## Earlier schema 2 macOS measurement

Environment: macOS, `aarch64` (Apple Silicon). Command: `/usr/bin/time -l ./target/release/examples/benchmark`. The benchmark output was:

```text
machine_os=macos machine_arch=aarch64
dataset_files=200 files_per_agent=100 turns_per_file=100 raw_jsonl_bytes=17442249 raw_jsonl_mib=16.63
discovery_ms=0.884 initial_index_ms=839.899 records=40101 chunks=20001 sessions=200
incremental_append_bytes=449 incremental_bytes_read=449 incremental_index_ms=59.716
search_queries=200 warm_p50_us=16145 warm_p95_us=17241
sqlite_db_wal_shm_bytes=22500416 sqlite_to_raw_ratio=1.290
process_exits_after_measurement=true
```

`/usr/bin/time -l` reported `12,353,536` bytes maximum resident set size (about 11.8 MiB). Its enclosing process took 4.71 seconds wall time, including the benchmark's synthetic file creation and SQLite work. The benchmark's initial indexing and append timings are measured internally with a monotonic clock and exclude compilation.

The warm search p50 (16.145 ms) is below the RFC target of 30 ms, and p95 (17.241 ms) is below 100 ms. The 449-byte append read exactly 449 bytes and indexed in 59.716 ms, below the 100 ms normal incremental target. Discovery of 200 files took 0.884 ms in this run.

The SQLite database plus WAL and SHM sidecars occupied 22,500,416 bytes, or 1.290 times the 17,442,249-byte JSONL input. This synthetic corpus has repeated prose and a relatively high FTS5 overhead; the RFC describes storage below raw history as an aim rather than an invariant. FTS configuration, vocabulary, chunk size, and corpus composition can materially change this ratio.

The process exited after the measurements (`process_exits_after_measurement=true`). This demonstrates that this benchmark leaves no running process; it does not measure a live UI or prove idle behavior of a future daemon, which is outside the current core path.

These numbers are a reproducible baseline on one machine and one synthetic workload, not representative real-history acceptance evidence. They do not cover UI activation, cold process startup, native agent resume, Herdr integration, or real Claude/Codex transcript variation.

# Core performance measurements

This report covers the core indexer and SQLite FTS search path for issue #8/#13. The sections below through "Earlier schema 2 macOS measurement" use **synthetic transcripts only**; no private native history, database, or user worktree is read or modified there. The "Real-corpus measurement" section at the end of this file is the first evidence run against an actual private Claude/Codex history (#13, #12); it reads real history and writes only a private, disposable, temporary database, never the shared installed index.

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

## Real-corpus measurement (#13, #12)

The measurements above use a 200-file, 16.63 MiB synthetic corpus. That is three orders of magnitude smaller than an actual user's history, and it contains no large tool output, so it cannot support #13's non-functional acceptance criteria on its own. This section adds a real-corpus run: the same release binary, indexing an actual private Claude/Codex history in place.

### Reproduce

```sh
./scripts/setup
./scripts/measure-real-corpus --claude-config-dir "$HOME/.claude" --codex-home "$HOME/.codex"
```

`scripts/measure-real-corpus` builds the release CLI, then runs six phases against a private SQLite database created under `mktemp` and deleted when the script exits: initial indexing, a second-pass rescan, an isolated single-file incremental append, storage decomposition, search latency, and a CLI/process-spawn overhead baseline. It never opens the shared installed index (`~/Library/Application Support/Herdr Agent History/`) and never writes into the given Claude/Codex directories — those are only read (discovery, sizes, and copying at most two existing small files elsewhere before appending to the copies). Called with no arguments, it measures a tiny generated synthetic corpus instead, which is what makes it safe to leave out of `./scripts/check` and CI. See `scripts/measure-real-corpus --help` for all options, including a custom `--queries` file.

### Corpus and host

| | |
| --- | --- |
| Machine | Apple M1, macOS 26.6.2 (build 25G83), `arm64` |
| Toolchain | rustc/cargo 1.93.1, default release profile (no `[profile.release]` override) |
| Build | `agent-history` release binary at commit `2c4aabf` (this branch's base, unmodified `crates/**`) |
| Corpus | 2,347 real JSONL files, 10,071,982,736 bytes (9.38 GiB) raw, from `~/.claude` and `~/.codex` (`sessions` + `archived_sessions`) |
| Composition | Codex dominates: 2,326 Codex files (1.5 GiB active sessions + 7.9 GiB archived) vs. 21 Claude files (~43 MiB, growing — this session's own transcript is part of it) |

The corpus was indexed live: a Claude Code session (this one) was actively appending to its own transcript file while the benchmark ran, so later phases observe a handful of genuinely new bytes rather than a perfectly static corpus. That is reported as data, not filtered out — see Limitations.

### Results

| Measurement | Result |
| --- | --- |
| Initial indexing (2,347 files, 9.38 GiB, cold) | 84,180 ms wall; peak RSS 76,906,496 bytes (73.3 MiB) |
| Initial indexing output | 1,437,530 records read (10,072,720,205 bytes), 20,222 chunks, 273 malformed records skipped, 0 failed files |
| Second-pass rescan (same 2,347 files, discovery cost at full file count) | 1,510 ms wall; peak RSS 12.3 MiB; found 105 new records / 233,507 bytes from the concurrently-running session above, 0 elsewhere |
| Isolated incremental append (1 real Claude file + 1 real Codex file, copied; append 376 bytes) | initial pass over the 2 copies (39,860 bytes, 25 records): 160 ms; re-index after a 376-byte append (2 records): 140 ms; appended sentinel text was searchable immediately after |
| CLI/process-spawn overhead baseline (`status` against the resulting ~115 MB db, no query) | median 8.8 ms (20 samples, 8.6–9.2 ms range) |
| Search latency, cold CLI invocation (8-term generic query set, 5 reps = 40 samples) | p50 15.0 ms, p95 23.8 ms, min 9.0 ms, max 29.7 ms |
| Resulting private database (fresh `index_all`, this run) | 2,319 sessions, 20,228 chunks; 114,970,624 bytes total (109.65 MiB), 33 freelist pages of 28,069 (0.1% free); chunk text 47,865,063 bytes |
| Storage ratio (this run's fresh db vs. 10,071,982,736-byte raw corpus) | file/raw 1.14%, live-pages/raw 1.14% (free pages are negligible here), chunk-text/raw 0.48% |
| Idle process | every phase above exits after its own CLI invocation; no `agent-history` process remains between phases (checked with `pgrep`) |

The search figure is not directly comparable to the "Current conversation-only schema 3 measurement" section above: that number is a **warm in-process** FTS timing (no process spawn, no database open, from the existing `benchmark` example this task does not modify). The 15.0/23.8 ms figures here are **cold, per-invocation CLI** timings — a fresh `agent-history search` process, SQLite open, and query, repeated per sample. The phase F baseline (8.8 ms median for `status`, which does no FTS work) shows that most of that time is the CLI/database-open path itself, not the query; the marginal FTS cost on this real corpus is on the order of a few milliseconds, consistent with the synthetic benchmark's warm figures.

### The pre-fix storage defect, as a measured baseline

Before this task, the user's actual production index at `~/Library/Application Support/Herdr Agent History/index.sqlite` (read only, never modified by this work) held:

```
sessions=2291 chunks=20073 chunk_text_bytes=47650600
page_count=233422 freelist_count=205221 page_size=4096
file_bytes=956096512  ->  live_bytes=115511296 (12.1%), free_bytes=840585216 (87.9%)
```

That database was produced by the schema 2 → 3 rebuild described in `docs/INDEXING.md` ("Database file size may remain unchanged because SQLite retains free pages for reuse"). Its live content — 20,073 chunks, 47.65 MB of chunk text, ~115.5 MB of live pages — is essentially the same content this task's fresh run just measured (20,228 chunks, 47.86 MB of chunk text, ~114.8 MB of live pages; the small difference is corpus growth between when that index was built and this run, including the live-session effect above). But the production file is **956,096,512 bytes: 8.31 times larger** than this run's freshly-built, equivalent-content database (115,003,392 bytes including WAL/SHM). The difference is almost entirely unreclaimed freelist pages (87.9% of the file), not live data.

This isolates the defect to the migration/rebuild path specifically: a plain cold-start `index_all` on unmodified current `main` does **not** reproduce meaningful free-page bloat (0.1% here, vs. 87.9% in the migrated index). That is consistent with a missing compaction step after the schema 3 chunk rebuild removes the old mixed-speaker/tool rows, rather than a general defect in ordinary incremental indexing. This is offered as the pre-fix baseline for judging `claude/git-provenance`'s compaction fix, not a fix itself — this task does not touch `crates/**`.

### #13 non-functional criteria: evidenced vs. not

Evidenced by this run:
- **Initial indexing time** on a real, private, multi-gigabyte corpus: 84.18 s for 9.38 GiB / 2,347 files.
- **Peak memory during initial indexing**: 76.9 MB max RSS, well bounded relative to the 9.38 GiB corpus (consistent with the per-record streaming design in `docs/INDEXING.md`).
- **SQLite size relative to raw history**, decomposed into live vs. freelist bytes, on both a freshly-built index (0.1% free, 1.14% of raw) and the existing pre-fix production index (87.9% free) — the storage criterion is no longer unevidenced, and the free-page finding is now quantified rather than asserted.
- **Discovery cost as file count grows**: a full rescan of 2,347 unchanged (plus a few genuinely-changed) files completed in 1.51 s, versus 84.18 s for the initial parse of all 9.38 GiB — discovery is characterized separately from full-corpus parse cost.
- **Incremental byte reads**: the isolated append test read exactly the 376 appended bytes (2 records) and made them searchable; `bytes_read` accounting matches the synthetic benchmark's methodology.
- **Idle resource use** in the sense this CLI supports: every phase's process exits after its own invocation; there is no daemon to hold memory or CPU between commands.

Not evidenced, or only partially evidenced, by this run:
- **Search p50 < 30 ms / p95 < 100 ms**: met numerically (15.0 / 23.8 ms) but only under a **cold-CLI-invocation** methodology (process spawn + DB open + query), which is stricter than the synthetic benchmark's warm in-process timing. There is no in-process, long-lived search harness for the real corpus in this task (that would require changes under `crates/**`, out of scope here), so this is the closest available evidence, not a like-for-like comparison with the RFC's number.
- **Normal small incremental update < 100 ms**: the isolated real-corpus append measured 140 ms end-to-end via a fresh CLI invocation against the resulting ~115 MB database — over the RFC's target as a raw wall-clock number. The phase F baseline (8.8 ms for a read-only `status` call against the same database) shows CLI/process overhead alone does not explain the gap; the remainder is plausibly SQLite write-transaction and FTS trigger cost against an existing ~115 MB index, which the existing synthetic benchmark's 55.460 ms in-process figure (smaller database, no process spawn) does not exercise. This is a methodology gap, not a demonstrated regression, and it is left unresolved: producing an in-process incremental-append harness against a real-scale database would need a change under `crates/**`, which is out of scope for this task.
- **Idle CPU/RAM with Agent History not running**: shown at the CLI-process level (no process remains between commands), but this task does not exercise the Herdr overlay's activation/idle lifecycle, which is a different code path (`agent-history-herdr`, out of scope here).
- **Apple Silicon release artifact on a clean macOS user environment**: unaffected by this task; still open per `docs/RELEASE_STATUS.md`.

### Limitations

- **The Claude sample is not representative.** The real corpus is almost entirely Codex (2,326 of 2,347 files, 9.34 of 9.38 GiB). Claude contributed 21 files (~43 MiB) at the time of this run — small enough, and growing quickly enough during the course of this task's own work, that any Claude-specific number here (for example, which file `scripts/measure-real-corpus` chose to copy for the isolated append test) reflects one small, idiosyncratic, moving-target history, not general Claude-side behavior. This is reported plainly rather than implied to be balanced coverage.
- **The corpus changed while it was being measured.** This benchmark was run from inside an active Claude Code session, which kept appending to its own transcript during the ~84 s initial index. The second-pass rescan therefore shows a small nonzero delta (105 records / 233,507 bytes) rather than a clean zero-byte no-op; that delta is itself evidence that discovery cost is dominated by file count, not by the presence of a single small real change, but it means "zero new bytes" was not achieved and is not claimed.
- **One machine, one real corpus, one run.** No repeated trials, no other hardware, no cold-vs-warm filesystem cache comparison beyond what the OS did on its own.
- **Search timing methodology differs between the synthetic and real-corpus sections of this document** (warm in-process vs. cold CLI invocation), as noted above; they are not directly comparable numbers.
- **This does not cover Herdr overlay activation, native resume, or worktree recovery** against the real corpus — those require a Herdr-managed session and are out of scope for this task (see `docs/RELEASE_STATUS.md`'s own noted external blocker).
- **This task does not fix the storage defect.** The 87.9%-free finding on the existing production index is reported as the pre-fix baseline for `claude/git-provenance`'s compaction work, not remediated here; `crates/**` is intentionally untouched.

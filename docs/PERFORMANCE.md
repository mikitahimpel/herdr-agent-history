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
| Build | `agent-history` release binary at commit `2c4aabf` (`main`, unmodified `crates/**`) for the "Results" and "pre-fix storage defect" measurements below; the "Post-compaction re-measure" further down was run after rebasing onto `claude/herdr-acceptance` (which includes `claude/git-provenance`'s fix), so it uses a different, newer binary — noted again at that measurement |
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

This isolates the defect to the migration/rebuild path specifically: a plain cold-start `index_all` on unmodified current `main` does **not** reproduce meaningful free-page bloat (0.1% here, vs. 87.9% in the migrated index). That is consistent with a missing compaction step after the schema 3 chunk rebuild removes the old mixed-speaker/tool rows, rather than a general defect in ordinary incremental indexing.

### Post-compaction re-measure

This branch now sits on top of `claude/git-provenance`'s free-space-reclamation fix (schema 4: a one-time `VACUUM` after a schema upgrade when free pages dominate, plus incremental auto-vacuum going forward; see `docs/INDEXING.md`'s "Free space reclamation" section). To measure its effect on the exact pre-fix database above without ever touching the shared installed index, the file (plus its `-wal`/`-shm` sidecars) was `cp`'d to a private, disposable path, and the new `agent-history status --db <copy>` was run once against the copy — opening a database is enough to trigger the schema upgrade and its one-time `VACUUM`, no reindex required. The original file was left untouched (verified after: still `956096512` bytes, still schema 3) and the copy was deleted immediately after this measurement:

```sh
copy="$(mktemp -d)/index.sqlite"; chmod 700 "$(dirname "$copy")"
cp "$HOME/Library/Application Support/Herdr Agent History/index.sqlite" "$copy"
./target/release/agent-history status --db "$copy"
sqlite3 "$copy" 'PRAGMA page_count; PRAGMA freelist_count; PRAGMA page_size; PRAGMA user_version;'
rm -rf "$(dirname "$copy")"
```

```
before: page_count=233422 freelist_count=205221 (87.9% free) file_bytes=956096512 schema=3
after:  page_count=27912  freelist_count=0      (0.0% free) file_bytes=114327552 schema=4
sessions=2291 chunks=20073 chunk_text_bytes=47650600   (unchanged before -> after: no data loss)
```

The compaction fix took this exact database from 956,096,512 to 114,327,552 bytes: **8.36x smaller, an 88.0% size reduction**, with live content (2,291 sessions, 20,073 chunks, 47,650,600 bytes of chunk text) byte-for-byte unchanged. Freelist pages went from 205,221 (87.9%) to 0. Against this task's measured raw corpus size (10,071,982,736 bytes), the compacted file is 1.135% — matching, to three significant figures, the 1.14% this task's independent fresh-build measurement found above. That agreement is itself evidence the fix works as intended: a migrated-and-compacted database and a fresh cold-start database now converge on the same storage profile, where before the migrated one was 8.31x larger than the fresh one for equivalent content.

This is the before/after pair for #13's "SQLite size relative to raw history" criterion: the pre-fix baseline (88% free, unevidenced ratio) and the post-fix result (0% free, 1.14% of raw, measured on the identical real corpus and identical live content) are now both recorded on the same real data.

### #13 non-functional criteria: evidenced vs. not

Evidenced by this run:
- **Initial indexing time** on a real, private, multi-gigabyte corpus: 84.18 s for 9.38 GiB / 2,347 files.
- **Peak memory during initial indexing**: 76.9 MB max RSS, well bounded relative to the 9.38 GiB corpus (consistent with the per-record streaming design in `docs/INDEXING.md`).
- **SQLite size relative to raw history**, decomposed into live vs. freelist bytes, with a full before/after pair on the same real data: the pre-fix production index (956,096,512 bytes, 87.9% free), that identical database after the compaction fix (114,327,552 bytes, 0.0% free, same 2,291 sessions/20,073 chunks/47,650,600 bytes of live text), and a freshly-built index for comparison (0.1% free, 1.14% of raw). The storage criterion is no longer unevidenced, and the fix's effect is quantified rather than asserted.
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
- **This task does not author the storage fix.** The compaction logic (`crates/**`) is `claude/git-provenance`'s work, merged into this branch's base; this task only measured its effect before and after, on the real corpus and the real pre-fix database, and did not modify `crates/**` itself.

## Near-match fallback (#28)

Search retries leniently only when the exact query returns nothing (shape B in #28). The retry reads FTS5's existing term dictionary through a temporary `fts5vocab` table and compares spelling by edit distance. It adds no table, column, trigger or schema version, so the exact path runs the same SQL as before. See the README's "Near matches" paragraph for the behaviour.

### Reproduce

```sh
# Synthetic, in process: exact p50/p95 as before, plus a misspelled-query set.
cargo build --release --locked --example benchmark -p agent-history-core
./target/release/examples/benchmark

# Real index, in process. Opening can upgrade a schema, so pass a private copy.
copy="$(mktemp -d)/index.sqlite"; chmod 700 "$(dirname "$copy")"
cp "$HOME/Library/Application Support/Herdr Agent History/index.sqlite" "$copy"
cargo run --release --locked -p agent-history-core --example search_latency -- "$copy"
rm -rf "$(dirname "$copy")"

# Storage against the real corpus, as in the section above.
./scripts/measure-real-corpus --claude-config-dir "$HOME/.claude" --codex-home "$HOME/.codex"
```

"Before" is `main` at `7f8a3fd` and "after" is this change. Both were built with the same toolchain on the same Apple M1 (macOS 26.6.2) and run on 2026-09-29.

### Exact-path latency: unchanged

| Measurement | Before | After |
| --- | --- | --- |
| Synthetic benchmark, warm in process, 200 queries, three alternating runs: p50 | 27.72 / 28.06 / 27.62 ms | 27.98 / 27.59 / 27.49 ms |
| Same runs: p95 | 29.77 / 33.38 / 29.72 ms | 33.29 / 30.18 / 31.23 ms |
| Real index (21,146 chunks), warm in process, 8 queries × 20, three runs: p50 via `search_with_role` | — | 10.82 / 10.86 / 10.80 ms |
| Same queries via `search_with_fallback` (they match, so no retry): p50 | — | 10.80 / 10.92 / 10.82 ms |
| Real index, cold CLI (spawn + open + query), 8 queries × 10, before and after alternated per sample: p50 / p95 | 17.8 / 31.7 ms | 17.8 / 27.1 ms |

On a hit, the only added work is a check that the result list is non-empty. The before and after figures agree within run-to-run noise, and exact ranking is untouched because the same expression and `ORDER BY bm25(...)` run.

### Fallback latency: the new cost, paid only on a miss

| Measurement | Result |
| --- | --- |
| Synthetic benchmark, warm in process, 10 misspellings × 20, three runs: p50 / p95 | 31.19 / 35.68, 30.89 / 36.04, 30.65 / 36.99 ms |
| Real index, warm in process, 8 misspellings × 20, three runs: p50 / p95 / max | 37.4 / 52.2 / 116 ms; 37.4 / 50.7 / 121 ms; 37.2 / 51.0 / 190 ms |
| Real index, cold CLI, 8 misspellings × 10: p50 / p95 | before (empty result) 6.4 / 6.9 ms; after (near matches) 44.1 / 57.5 ms |

A miss therefore costs about 26–38 ms more than it did, which stays inside the RFC's 100 ms p95 search target. The cost is dominated by the vocabulary scan: `fts5vocab` counts documents for every term it returns, which means reading doclists. Scanning the whole dictionary (50,471 terms) took about 1.2 s. That is why the retry only reads terms that share the typed word's first letter, or that start with its first two letters swapped (about 1,700–2,600 terms per letter here). Seven of the eight real-corpus misspellings (`fucntion`, `tset`, `indx`, `serach`, `reveiw`, `confgi`, `instal`) returned near matches. The eighth, `eror`, occurs verbatim once in the corpus, so the exact query matched and no retry ran.

### Index size: no growth

| Measurement | Before | After |
| --- | --- | --- |
| Synthetic benchmark database + WAL/SHM | 30,930,664 bytes | 30,930,664 bytes (identical) |
| Real corpus, fresh private index (`scripts/measure-real-corpus`) | 123,432,960 bytes for 21,231 chunks from 10,756,080,592 raw bytes (1.15%) | 123,531,264 bytes for 21,238 chunks from 10,761,604,391 raw bytes (1.15%) |

The 98,304-byte real-corpus difference is corpus growth between the two runs (5.5 MB more raw history and 7 more chunks from sessions active during the measurement), not the fallback. The fixed synthetic corpus shows the same file byte for byte. The temporary `fts5vocab` table lives in SQLite's per-connection `temp` schema and is never written to the index file; a unit test checks the page count, schema version and file size.

For comparison, shape A (a trigram FTS5 index over the same chunk text) was measured on a vacuumed private copy of the real index. It adds 139,866,112 bytes of index pages, 2.87 times the existing 48,783,360-byte `unicode61` index, and grew that database from 153,759,744 to 293,797,888 bytes (+91%). That, plus a schema migration and changed ranking for every query, is why shape B was chosen.

### Limitations

- A misspelled first letter is not corrected, except when the first two letters are swapped. The vocabulary is read one first-letter range at a time to keep the retry fast.
- Words shorter than four characters are never widened, and at most eight distinct words per query are.
- A typo that happens to occur verbatim somewhere in the history matches exactly, so no retry runs.
- The browser searches on every keystroke, so an unfinished word that matches nothing now costs the fallback's ~40 ms on that keystroke, and shows prefix near matches rather than an empty list.
- One machine and one real corpus; the fallback timings depend on how many terms share the typed word's first letter.

## Background activation scan (#34)

The browser (standalone and the Herdr overlay) used to run the whole activation scan before its first frame, so a query typed at launch waited for discovery and indexing. It now opens the existing index, draws at once, and runs the same scan (`index_all_until`, unchanged in what it commits) on a thread with its own SQLite connection. See `docs/INDEXING.md` for the concurrency and cancellation guarantees.

"Before" is `main` at `1033fcb` and "after" is this change. Both are release builds with rustc 1.98.1 on the same Apple M1 (macOS 26.6.2), run on 2026-09-30. Another workload (a `cargo` build and tests in a different checkout) shared the machine during some runs, which widens the ranges below. Treat them as ranges, not a benchmark.

### Method

Every trial starts from its own copy of one snapshot of the real installed index (150,036,480 bytes, schema 4), placed in a fresh private directory. The shared installed index is only read to take that snapshot and is never opened. History comes from the real `~/.claude` and `~/.codex`, which are only read. The snapshot was taken while sessions were writing, so each trial found about 4,600–6,500 new records (37–54 MB) across 2,518–2,519 files. That delta grew during the measurement because live sessions kept appending. This is more work per activation than the 125 records in #34's 3.3 s example, so "before" here is slower than that example.

- **Real binary.** A PTY harness (Python `pty`, 140×40) launches `agent-history browse --db <copy>` and types `error` at t=0. It records when the browser first appears (the role tabs are drawn) and when the first frame shows a nonzero result count. Then it sends Ctrl-C and records the exit time and code. After exit it runs `PRAGMA integrity_check` on the copy and a follow-up `agent-history index --db <copy>`, and checks with `pgrep` that no process remains. "Cold" copies are fresh APFS clones, whose pages are not in the OS cache, like an index not used since boot. "Warm" copies are read once first, like an index used recently.
- **In process.** The following runs the three variants on separate copies per trial: `idle` (open and search, no scan), `sync` (open, full scan, search, which is what activation did before) and `background` (open, spawn the scan, search). After both scans settle, it compares the ranked top 50 of eight queries between the `sync` and `background` copies and times settled search on each:

  ```sh
  cargo build --release --locked -p agent-history-core --example cold_start
  copy="$(mktemp -d)/index.sqlite"; chmod 700 "$(dirname "$copy")"
  cp "$HOME/Library/Application Support/Herdr Agent History/index.sqlite" "$copy"
  ./target/release/examples/cold_start "$copy" 3        # cold copies
  ./target/release/examples/cold_start "$copy" 3 warm   # warm copies
  rm -rf "$(dirname "$copy")"
  ```

### Perceived cold start: real binary, five trials each

| Page cache | Before: browser visible = first result | After: browser visible | After: first result for `error` |
| --- | --- | --- | --- |
| Cold copy | 4,828 / 4,892 / 5,022 / 6,704 / 10,669 ms (median 5,022) | 7 / 7 / 12 / 13 / 21 ms (median 12) | 22 / 25 / 38 / 44 / 61 ms (median 38) |
| Warm copy | 3,776 / 5,363 / 7,967 / 9,202 / 13,001 ms (median 7,967) | 7 / 7 / 10 / 11 / 25 ms (median 10) | 21 / 23 / 27 / 44 / 88 ms (median 27) |

In every "after" trial, the frame with the first result also showed `Indexing … results may be incomplete` in the header, because the scan was still running. Before, the browser did not exist until the scan ended, and keys typed meanwhile were replayed afterwards, so "visible" and "first result" coincide.

The in-process harness agrees. Warm: `sync` first result 2,282 / 2,732 / 5,370 ms, `background` 17 / 18 / 30 ms, `idle` baseline 23 / 25 / 29 ms. Cold: `sync` 6,954 / 8,167 / 12,522 ms, `background` 507 / 544 / 587 ms, `idle` baseline 405 / 526 / 2,305 ms. In a cold copy, the first FTS query reads its pages from disk whether or not a scan runs. That is the few hundred milliseconds in both `idle` and `background`, and it is not contention. The PTY harness's cold first results are lower than the in-process ones. Its first nonzero count can come from a prefix of `error`, and its Python copy may not leave the file fully uncached; I did not isolate which.

### Search while the scan runs, and after it settles

| Measurement | Result |
| --- | --- |
| Settled ranked top 50, eight queries, `sync` vs `background` copy | identical in all 14 trials run (the example exits with an error otherwise) |
| Settled search p50, `sync` / `background` copy, warm (20 reps × 8 queries) | 17.7 / 19.1, 27.9 / 26.5, 20.8 / 12.9 ms |
| Settled search p50, cold copies | 33.4 / 21.9, 36.5 / 29.7, 34.0 / 33.5 ms |
| Search p50 while the scan runs, warm | 16.0, 20.1, 31.1 ms (max 74, 91, 289 ms) |

The search code, SQL and schema are unchanged (`storage.rs` and `query.rs` are untouched), so settled ranking cannot differ. The identical result lists confirm that a scan settled in the background leaves the same index as one run up front. The settled latency pairs differ in both directions by run-to-run noise on the shared machine. While a scan is writing, search is only somewhat slower. A search is one FTS statement, which reads one committed WAL snapshot and does not wait for the writer. The in-scan maxima (74–289 ms) were not investigated further; in WAL mode they can only come from sharing CPU and I/O with the scan, not from locking.

### Ctrl-C during the scan

| Moment of Ctrl-C (warm copies, five trials) | Before | After |
| --- | --- | --- |
| Key, 150 ms after launch, no query typed | 1 / 1 / 5 / 22 / 32 ms, exit 130 (process aborted) | 23 / 38 / 41 / 94 / 213 ms, exit 0 (scan stopped and joined) |
| `SIGINT`, 150 ms after launch, `error` typed | 82–100 ms, exit 130 | 3 / 4 / 4 / 9 / 171 ms, exit 0 |
| Key, 150 ms after launch, `error` typed | 1–6 ms (keys were only buffered) | 177–518 ms |
| Key, right after the first result (cold / warm) | — | 117–344 / 95–194 ms |

Every run, before and after, left a database that passed `PRAGMA integrity_check`. The follow-up `index` run completed with 0 failed files, and no process remained. The stop flag is checked while directories are walked, between records, before Git is consulted and before commit. A commit already under way finishes first, which accounts for the tail. The "`error` typed" key row is not the scan: each typed character runs a search on the UI thread, a one-letter prefix misses and takes the near-match fallback (#28), and the Ctrl-C key waits behind them. `SIGINT` bypasses the key queue, and there the stop usually took 3–9 ms. Before discovery was made cancellable, an early Ctrl-C waited for the whole file walk (up to 1,936 ms measured), so this change includes that.

### Synthetic benchmark: indexing guarantees unchanged

The existing `benchmark` example was run three times per build, alternating old and new, on the same shared machine:

| | Before | After |
| --- | --- | --- |
| Initial indexing (200 files) | 3,058 / 1,759 / 2,788 ms | 2,820 / 4,055 / 1,779 ms |
| 449-byte append: bytes read | 449 / 449 / 449 | 449 / 449 / 449 |
| 449-byte append: time | 61.5 / 85.2 / 67.4 ms | 67.9 / 241.6 / 57.6 ms |
| Chunks after append | 40,001 each run | 40,001 each run |

Append accounting is identical: an append still reads exactly the appended bytes. The timings overlap and scatter in both directions. The slow "after" run (4,055 ms, 241.6 ms) overlapped a `cargo test` build of this change, so these timings show no measurable cost from the per-record stop check, not a precise comparison.

### Idle cost

No thread or process outlives the browser. Closing it stops and joins the scan thread, and `pgrep` found no leftover process after any trial. There is still no daemon, so resource use between invocations remains zero.

### Directory pruning: not built

#34 suggested skipping directories whose mtime has not advanced. It was not built. After this change the scan is off the interactive path: the browser is usable in about 10 ms, and the scan's only visible effect is the header's `Indexing N/M files` line. The directory walk it would shorten is also small: the PTY header shows the file total 39–119 ms after launch (up to 359 ms in process under load). Most scan time goes to per-file checks and to indexing the new records themselves, which pruning would not remove. It would save a fraction of a scan nobody waits for, at the risk of silently missing sessions, which is worse than a slow scan.

### Limitations

- One machine, one real corpus, five PTY trials and three in-process trials per variant, on a machine that was sometimes shared with other work.
- The live corpus grew during measurement, so "before" and "after" trials had slightly different amounts of new history (about 4,600–6,500 records). Before/after PTY runs of the same kind were run back to back to keep this small. The difference is far smaller than the effect.
- The header shows only checked/total files during the scan. The byte, record and chunk counts of the old full-screen progress box are no longer displayed, although the core still reports them.
- A file whose source changes while it is read (an active session appending) fails that attempt with `source changed during indexing; retry` and is retried next launch. This existing behaviour occurred in both modes during these runs and is reported in the status line.

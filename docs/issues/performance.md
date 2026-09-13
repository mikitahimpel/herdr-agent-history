## Outcome

Measure search, incremental latency, storage, and idle resources.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 10, 24, 27.

## Scope

- Build repeatable generated-history benchmarks and document representative local-history measurement procedure without uploading transcripts.
- Measure warm/cold search p50/p95, discovery separately from append parsing, activation latency, DB/sidecar size, and idle lifecycle.
- Record hardware, dataset bytes/session counts/query set, repetitions, and deviations; optimize measured bottlenecks only.

## Acceptance criteria

- [ ] Report assesses p50 <30 ms and p95 <100 ms search; normal small incremental updates <100 ms.
- [ ] Demonstrate incremental byte reads and characterize discovery cost as file count grows.
- [ ] Report DB/raw-history ratio and prove no Agent History process remains after exit; no unmeasured daemon/vector work is added.

## Dependencies

- cli
- overlay-actions

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:performance -->

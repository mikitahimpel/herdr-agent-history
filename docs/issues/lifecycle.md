## Outcome

Coordinate activation indexing and visible initial progress.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 15, 16.

## Scope

- On activation, serve existing search results while discovering and indexing changed files.
- Report per-agent file progress, chunk counts, cancellation, and recoverable errors.
- Coordinate multiple CLI/Herdr clients so writers are serialized without duplicate work; stop background work when its owner exits.

## Acceptance criteria

- [ ] Tests demonstrate responsive reads during indexing and safe concurrent activation.
- [ ] Interrupted initial scans resume without duplicates or skipped records.
- [ ] Document whether search is available during first scan; no persistent daemon remains after exit.

## Dependencies

- incremental
- mutations
- git-context
- search

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:lifecycle -->

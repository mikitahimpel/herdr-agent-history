## Outcome

Implement transactional byte-offset incremental indexing.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 12, 13.

## Scope

- Read appended bytes from the committed offset; parse complete newline-terminated records only.
- Commit chunks, FTS changes, open-turn state, and offsets atomically.
- Define deterministic handling for malformed complete records, bounded diagnostics, concurrent writes, interruption, and retry.

## Acceptance criteria

- [ ] Partial final JSON, newline completion, UTF-8 boundary, and crash-before/after-commit tests pass.
- [ ] Repeated indexing is idempotent; appends read new bytes plus only explicitly bounded state.
- [ ] A malformed complete record cannot permanently block all later valid records or silently hide skipped data.

## Dependencies

- chunks
- storage
- privacy

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:incremental -->

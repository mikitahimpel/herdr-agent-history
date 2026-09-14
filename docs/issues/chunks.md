## Outcome

Build bounded conversation chunks with resumable turn state.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 7, 10–13.

## Scope

- Keep user and assistant text in separate speaker-qualified chunks; exclude tool output and split oversized messages at deterministic boundaries (September 14 product refinement).
- Retain timestamps and exact source ranges; normalize whitespace without losing meaningful search terms.
- Define how the final open turn is extended after an append or process restart, without duplicating searchable text.

## Acceptance criteria

- [ ] Whole-file indexing and indexing the same bytes in arbitrary append batches produce equivalent chunks/search results.
- [ ] Restart mid-turn and assistant-only continuation tests pass.
- [ ] Chunk size limits are explicit; source ranges round-trip Unicode and multi-record messages; tool output is excluded.

## Dependencies

- claude
- codex

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:chunks -->

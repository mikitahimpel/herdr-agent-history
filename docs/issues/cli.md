## Outcome

Implement index, search, and status CLI commands.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 23.

## Scope

- Wire the standalone binary to core index/search/status services with help, configurable data paths, and clear exit codes.
- Print searchable result context and progress/errors on appropriate streams.
- Expose index counts, freshness, and actionable indexing errors without leaking transcript content in status.

## Acceptance criteria

- [ ] Process-level integration tests run against isolated synthetic history and a temporary database.
- [ ] Index → search → status works without Herdr and after process restart.
- [ ] Invalid arguments/queries and missing/unreadable files have deterministic outcomes.

## Dependencies

- search
- lifecycle
- preview

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:cli -->

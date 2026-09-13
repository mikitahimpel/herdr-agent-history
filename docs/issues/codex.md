## Outcome

Implement Codex session discovery and parsing.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 8, 10, 17.

## Scope

- Discover supported Codex native JSONL sessions independently of open workspaces.
- Extract session metadata and conversational text; deduplicate equivalent event representations and streamed updates.
- Define compatibility behavior for unknown schema versions and events.

## Acceptance criteria

- [ ] Fixture tests cover session metadata, user/assistant turns, tool results, Unicode, and missing fields.
- [ ] Only supported conversational text enters search; native IDs remain resumable.
- [ ] Discovery handles absent/unreadable roots and malformed records without aborting unrelated files.

## Dependencies

- foundation
- fixtures

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:codex -->

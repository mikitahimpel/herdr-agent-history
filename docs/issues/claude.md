## Outcome

Implement Claude session discovery and parsing.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 8, 10, 17.

## Scope

- Discover supported Claude JSONL locations without depending on open Herdr workspaces.
- Extract native session identity, cwd, timestamps, user/assistant content, and bounded useful tool results.
- Ignore non-searchable metadata and unsupported event types using documented diagnostics.

## Acceptance criteria

- [ ] Versioned fixture tests cover all supported content variants and malformed input.
- [ ] Discovery handles missing roots, unreadable files, symlinks, and duplicate paths predictably.
- [ ] No tool IDs, protocol JSON, hidden reasoning, or token accounting enters normalized text.

## Dependencies

- foundation
- fixtures

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:claude -->

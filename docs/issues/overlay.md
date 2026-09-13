## Outcome

Build the Agent History search overlay in Herdr.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 5, 15, 16, 22.

## Scope

- Add the named command and configurable shortcut using Herdr's existing overlay/UI patterns.
- Connect query editing, result metadata/snippets, selection, loading/progress, empty/error states, and async result updates.
- Resolve Space behavior explicitly: query input must accept multiword text while Space on a focused result previews.

## Acceptance criteria

- [ ] Keyboard tests cover multiword typing, arrows, Space preview intent, Enter selection, Esc dismissal, and focus restoration.
- [ ] Stale query responses cannot replace newer results; selection stays stable where possible.
- [ ] Search works with no workspaces open and remains responsive while indexing.

## Dependencies

- host-contract
- search
- lifecycle

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:overlay -->

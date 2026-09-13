## Outcome

Implement verified native Claude and Codex resume commands.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 17, 19.

## Scope

- Verify native resume CLI flags against supported installed versions or authoritative CLI help.
- Construct agent-specific argument arrays using the original native session ID and chosen cwd.
- Detect absent executables, unsupported versions, unavailable sessions, and launch errors without falling back silently to a new conversation.

## Acceptance criteria

- [ ] Adapter tests assert exact argv/cwd with spaces and shell metacharacters.
- [ ] Manual smoke tests for both supported agents demonstrate continuation of the same native session.
- [ ] No shell interpolation or transcript-provided instructions control launch arguments.

## Dependencies

- claude
- codex
- host-contract

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:resume-agents -->

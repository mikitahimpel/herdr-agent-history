## Outcome

Build sanitized Claude and Codex compatibility fixtures.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 8, 10, 13, 17.

## Scope

- Document discovery layouts and versioned event schemas from supported agent versions.
- Create synthetic or explicitly sanitized JSONL fixtures: user/assistant turns, tool outputs, metadata, Unicode, missing fields, unknown events, malformed lines, and partial tails.
- Document the policy for human-readable tool output and explicitly exclude raw credentials, private transcripts, and hidden reasoning/internal events.

## Acceptance criteria

- [ ] Fixtures cover both agents and native session IDs/cwd/timestamps.
- [ ] Parser expectations are deterministic and tests never read the developer's home history.
- [ ] Document format drift, unsupported records, and duplicate message/streaming variants.

## Dependencies

- foundation

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:fixtures -->

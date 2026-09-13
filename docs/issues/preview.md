## Outcome

Read original conversation context around search matches.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 7, 11, 21.

## Scope

- Load bounded surrounding original records from source references and render role-separated conversation.
- Support expanding context without loading an entire large transcript.
- Verify source generation before reading; sanitize terminal control sequences and distinguish missing/replaced sources.

## Acceptance criteria

- [ ] Tests cover middle-of-file matches, Unicode, large records, incomplete tails, and source replacement.
- [ ] Preview shows original conversational context with agent/repo/branch metadata.
- [ ] No raw transcript archive is introduced in SQLite.

## Dependencies

- claude
- codex
- mutations

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:preview -->

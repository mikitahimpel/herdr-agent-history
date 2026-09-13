## Outcome

Enforce local-only processing and private index permissions.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 25.

## Scope

- Choose a macOS user-scoped index location and override for tests; create directories as 0700 and DB/sidecars as 0600.
- Protect temporary files, WAL/SHM, diagnostics, and error messages; avoid transcript content in default logs.
- Keep the runtime free of network/telemetry services and treat transcript text as untrusted display data.

## Acceptance criteria

- [ ] Permission tests under permissive umask cover new/reopened DB and SQLite sidecars.
- [ ] Source transcripts are never modified; fixtures and logs contain no private history.
- [ ] Document local sensitive data handling and rebuild/removal behavior.

## Dependencies

- storage

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:privacy -->

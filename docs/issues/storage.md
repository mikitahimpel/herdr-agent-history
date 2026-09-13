## Outcome

Implement SQLite metadata schema, migrations, and FTS5 storage.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 9, 10, 24, 30.

## Scope

- Choose and verify SQLite FTS5 support; create sessions, indexed_files, search_chunks, foreign keys, and versioned migrations.
- Use an external-content FTS table or equivalent single-copy normalized-text design with transactional synchronization.
- Support concurrent readers during updates, document WAL/locking policy, and keep the index rebuildable.

## Acceptance criteria

- [ ] Fresh creation, reopen, migration rollback, foreign-key cleanup, and FTS insert/update/delete consistency tests pass.
- [ ] Schema stores file generation and committed offsets without a second complete transcript copy.
- [ ] Packaged SQLite actually supports FTS5; crash rollback leaves metadata and FTS consistent.

## Dependencies

- foundation

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:storage -->

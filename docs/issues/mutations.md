## Outcome

Handle truncated, replaced, renamed, and missing source files.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 14, 21, 30.

## Scope

- Classify new/appended/unchanged/truncated/replaced files using path, identity, size, mtime, and offset.
- Rebuild only affected source generations atomically; define rename, deletion, and same-size rewrite policies.
- Invalidate stale source references and retain truthful metadata when native history disappears.

## Acceptance criteria

- [ ] Tests cover inode replacement, truncate-and-regrow, same-size edits, rename, deletion, and replacement during a read.
- [ ] No old FTS chunks survive a successful replacement rebuild.
- [ ] Unchanged files are not reparsed; missing originals yield explicit preview/resume errors.

## Dependencies

- incremental

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:mutations -->

## Outcome

Define core domain models and adapter contracts.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 1–4, 7, 17, 22, 30.

## Scope

- Define agent-qualified session identity, normalized events, sessions, search chunks/results, source references, and typed errors.
- Separate pure core contracts from agent parsing, filesystem/process access, and Herdr host types; document crate ownership and dependency direction.
- Record supported versions/fixtures before assuming native session layouts or CLI options.

## Acceptance criteria

- [ ] Claude and Codex IDs cannot collide; timestamps and missing context have explicit semantics.
- [ ] Source ranges are documented as half-open byte offsets, with file identity/generation for stale-reference detection.
- [ ] Core builds and tests without Herdr; contract tests cover identity and optional metadata.

## Dependencies

None; ready to start.

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:foundation -->

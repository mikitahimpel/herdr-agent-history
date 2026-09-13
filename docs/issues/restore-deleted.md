## Outcome

Add confirmed deleted-worktree recovery and fallbacks.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 20.

## Scope

- Return recovery choices: recreate and resume, resume in repository, view conversation, cancel.
- Require explicit runtime confirmation immediately before Git/filesystem mutation; validate recorded repository, branch, commit, and target path.
- Handle checked-out branches, missing refs/commits, collisions, detached HEAD, and insufficient metadata without destructive checkout/reset/clean.

## Acceptance criteria

- [ ] No recreation occurs before confirmation; cancel is side-effect free.
- [ ] Temporary Git integration tests exercise available/missing branch and commit plus target collisions.
- [ ] Failure leaves existing user work intact; fallback options accurately reflect available repository/transcript state.

## Dependencies

- restore-existing

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:restore-deleted -->

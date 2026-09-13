## Outcome

Capture repository and worktree context during indexing.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 18.

## Scope

- Resolve cwd to canonical repository/common Git directory, worktree, branch, and HEAD commit.
- Handle normal repositories, linked worktrees, detached HEAD, non-Git paths, missing paths, and unusual path characters.
- Persist provenance and observation time; do not overwrite useful recorded context with missing current state or claim index-time HEAD is historical truth.

## Acceptance criteria

- [ ] Temporary-repository tests cover linked/deleted worktrees and detached HEAD.
- [ ] Metadata remains usable after worktree deletion.
- [ ] Git invocations use structured arguments and never mutate repositories during indexing.

## Dependencies

- foundation
- storage

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:git-context -->

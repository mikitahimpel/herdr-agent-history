## Outcome

Resume in existing or closed workspaces with existing worktrees.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 19, 29.

## Scope

- Implement the restore decision service: resolve canonical cwd, focus matching workspace/session, or create a workspace for an existing worktree.
- Reuse an already-running matching agent and avoid duplicate workspaces/processes.
- Handle stale host state and workspace/process creation failures with recoverable errors.

## Acceptance criteria

- [ ] Tests cover open workspace with active session, open workspace without matching agent, and closed workspace with existing cwd.
- [ ] Repeated Enter and concurrent requests do not duplicate processes/workspaces.
- [ ] Existing-worktree paths require no confirmation or extra user decisions.

## Dependencies

- git-context
- host-contract
- resume-agents

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:restore-existing -->

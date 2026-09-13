## Outcome

Connect preview and resume/recovery interactions in Herdr.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 5, 19–22, 29.

## Scope

- Connect Space to role-separated preview, Enter to the restore service, and Esc back to retained query/selection.
- Present deleted-worktree choices and confirmation using native Herdr interaction patterns.
- Show actionable launch/source errors and maintain accessible keyboard focus through each transition.

## Acceptance criteria

- [ ] UI/integration tests cover search → preview → resume and return to search without losing state.
- [ ] Normal existing-worktree flow hides CLI/workspace mechanics and requires no extra decisions.
- [ ] Missing worktree confirmation and cancellation produce the expected host effects only.

## Dependencies

- overlay
- preview
- restore-existing
- restore-deleted

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:overlay-actions -->

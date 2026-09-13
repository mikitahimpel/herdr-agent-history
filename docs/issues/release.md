## Outcome

Validate V1 end to end and document macOS installation.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 3, 25, 26, 29, 30.

## Scope

- Run the defining user journey for both agents across open, closed, and deleted worktree cases.
- Document macOS build/install, supported agent/Herdr versions, shortcut setup, privacy, rebuild, limitations, and troubleshooting.
- Exercise corruption/disposable-index rebuild and restart recovery; document packaging of core/CLI/host integration.

## Acceptance criteria

- [ ] Automated isolated integration suite and manual real-agent smoke evidence are recorded.
- [ ] Deleting/rebuilding the index preserves all native session data and restores search coverage.
- [ ] Lint, tests, release build, and performance report pass or explicitly record unresolved release blockers before declaring V1 complete.

## Dependencies

- privacy
- cli
- overlay-actions
- performance

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:release -->

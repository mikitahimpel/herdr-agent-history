## Outcome

Define and implement the Herdr host integration boundary.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 17, 19, 22.

## Scope

- Inspect the supported Herdr revision and document the actual API/extension strategy; identify any host-side changes.
- Define testable operations to list/focus/create workspaces and identify/focus/start an agent session.
- Keep shared runtime/session facts out of overlay-only state and keep core independent from Herdr types.

## Acceptance criteria

- [ ] An ADR identifies supported Herdr versions, source ownership, build/distribution route, and any prerequisite host work.
- [ ] A mock host tests workspace lookup and process launch requests.
- [ ] No unverified Herdr command or private protocol is assumed; this issue owns any necessary companion integration changes.

## Dependencies

- foundation

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:host-contract -->

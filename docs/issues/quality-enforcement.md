## Outcome

Require the quality gate on GitHub main.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections Repository setup requirement.

## Scope

- Configure a GitHub branch rule/ruleset for main requiring pull requests and the Quality gate status; review bypass permissions with the owner.
- Verify a failing or missing required check blocks merging.
- Keep ./scripts/check, CI, AGENTS.md, CLAUDE.md, and the pre-push hook aligned as new crates/tests are added.

## Acceptance criteria

- [ ] An active main rule requires the Quality gate CI result and blocks failed/pending checks.
- [ ] Record the verified rule and bypass policy; local hooks alone are not remote merge enforcement.
- [ ] Scaffold formatting, lint, tests, and release build have a green CI run.

## Dependencies

None; ready to start.

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:quality-enforcement -->

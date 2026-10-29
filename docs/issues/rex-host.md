## Outcome

Add Rex as a second host for the browser: `agent-history-rex` searches history like the other entry points and resumes sessions in Rex, with the resume pieces Herdr and Rex share moved into their own crate.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 17, 19 (resume and restoration), applied to a second host.

## Scope

- Extract host-independent resume code from `agent-history-herdr` into `agent-history-restore`: native resume argv (`plan`), the running-session process scan (`live`), confirmed worktree recreation (`restore`), resume notices. Herdr behaviour is unchanged. (Done.)
- Add `agent-history-rex`: a `RexHost` boundary driven only through the `rex-agent` CLI, focus of running agents, resume in Rex, the shared recovery screen, a `REX_SESSION` preflight, and a background prioritizer. (Done.)
- Package, install and uninstall `agent-history-rex` the way `agent-history-herdr` is handled, opt-in with `./install --with-rex`.
- Document the Rex integration in README.md and docs/MODULES.md.
- Mark prioritized results with the state Rex reports for each Claude agent, ordered by urgency.
- Resume into the Rex session already working in the session's directory, when there is exactly one.
- Remove Herdr wording and conventions from the Rex binary's UI.

## Acceptance criteria

- [ ] `agent-history-rex` is built by `scripts/package`, included in the archive, installed by `./install --with-rex` (which needs no Rex), and removed by `./uninstall`; `scripts/test-packaging` covers it.
- [ ] README.md has a Rex section: what it is, launching inside a Rex terminal (`REX_SESSION`), a Rex action and key binding that opens it in a floating layer, the `rex-agent`/`REX_AGENT_BIN` requirement, and Remote Control.
- [ ] docs/MODULES.md lists six crates and describes the Rex integration.
- [ ] Results show `blocked`, `error`, `done`, `working` or `idle` for sessions Rex reports, in that order of priority; sessions found only by the process scan show `running`. Covered by tests with fake hosts.
- [ ] Enter on a session that is not running runs `rex-agent session-for <dir>`; exit 0 opens the resume as a new tab in that session (`rex-agent open --session=… --cwd=… --label=… -- <argv>`), any other exit opens a new Rex session. The recovery paths do the same. Commands are argv only. Covered by tests.
- [ ] The Rex binary's title, key hints, notices, recovery screen, help and errors contain no Herdr wording; the title is the product name alone; Esc from the search box closes the browser. The Herdr binary's behaviour and wording are unchanged.
- [ ] Tests need no running Rex, Herdr or agents, and use synthetic fixtures only.
- [ ] Manual check in Rex: shift+cmd+H opens the browser, Enter focuses a running agent, resumes a closed session into the session working in its directory, and markers follow agent state.

## Dependencies

- resume-agents
- restore-deleted
- overlay-actions

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build) and `./scripts/test-packaging`. Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:rex-host -->

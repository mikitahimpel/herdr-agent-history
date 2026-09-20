# V1 release status — 0.1.0-rc.1 prerelease

Stable V1 is **not released** and GitHub issue #13 remains open. The 0.1.0-rc.1 prerelease is intended for testing on another Apple Silicon Mac; publication does not establish the remaining acceptance criteria. The repository now contains a working local CLI, terminal overlay, verified-source preview, host resume adapter, confirmed Git worktree recreation, installable package scripts, and synthetic measurements. Implementation and local tests do not establish native-agent resume compatibility or clean-user installation acceptance.

## Architecture

`agent-history-core` owns native adapters, bounded conversation chunks, disposable SQLite/FTS5, transactional indexing, captured Git context, and source-verified previews. `agent-history-tui` owns shared terminal browsing, rendering, and progress. `agent-history-cli` provides standalone browse/index/search/status/preview and the compatibility `agent-history-overlay` entry point. The optional `agent-history-herdr` executable adds native-session command construction, public Herdr CLI operations, and recovery confirmation through the shared UI integration boundary. Native files remain read-only; no daemon or transcript network service is introduced.

## Issue and PR map

| Issue | Local implementation and evidence | Remaining acceptance |
| --- | --- | --- |
| #7 Foundation | Shared types/contracts, isolated fixtures, CI definition, local quality gate | Remote CI execution not verified |
| #1 Ingestion | Claude/Codex adapters, synthetic fixtures, bounded resumable chunks | Real native-format compatibility evidence |
| #2 SQLite/FTS | Schema 3, role-filtered external-content FTS, search/ranking, atomic source-scoped updates | Broader real-corpus measurements |
| #3 Incremental indexing | Committed offsets, partial-record retry, replacement handling, concurrent-writer checks | Documented interior rewrite/regrow limitation |
| #4 Git context | Main/linked/bare/separate Git directory tests; preserved observations | Index-time metadata is not proof of historical branch state |
| #5 CLI | Cross-process index/search/status/preview/append tests | Real-history user validation |
| #8 Core integration | Both agents through discovery → index → search → original preview; synthetic benchmark | Representative private-history validation without committing data |
| #9 Native resume | UUID-only argv, source validation, official session provenance, no plain-session fallback; live macOS resume of both agents after process exit, with resumed turns appended to the original transcript and session ID | Codex could not produce a post-resume model turn (account usage limit); historical sessions with absent or non-UUID native IDs remain unexercised |
| #6 Herdr | Functional terminal overlay, confirmed isolated Git recovery; live active/closed workspace resume, deleted-worktree recreation, unavailable-repository handling, and the linked plugin overlay route | A Herdr-rendered native overlay is still outside plugin v1; overlay placement remains a terminal pane |
| #11 Safeguards | Private index, safe open/sidecar rejection, rollback/corruption errors, confirmed recovery | Broader security review; see documented source-mutation limits |
| #10 Packaging | Apple Silicon archive/checksum, extracted installer, durable plugin path, isolated smoke | Clean-user Mac installation and GitHub release artifacts |
| #12 Documentation | README, indexing/privacy/recovery notes, plugin guide, benchmark and troubleshooting | Real UI screenshots/native acceptance evidence |
| #13 Release gate | Local quality gate, synthetic integration evidence, and scenarios 5-8 executed live against a running Herdr host for both agents | Scenarios 1-4 and 9-10 on a real installation, clean-user macOS installation, and remote CI/branch protection; no release tag |

All GitHub issues were inspected as the source backlog. No issue is closed by this prerelease. GitHub CI passed on integration commit `3dc90cf`; branch protection has not been verified. The integration work is being promoted to `main` for prerelease distribution.

## Validation evidence

- `./scripts/setup` enabled the pre-push hook.
- `./scripts/check` passed formatting, Clippy with warnings denied, all 102 tests (69 core, 24 Herdr adapter/preflight/hook, 6 CLI, 3 shared TUI), and the release build using the lockfile.
- Core regressions cover source identity, generation, partial Unicode records, rollback and competing writers, both adapters, normalized previews, and selected-source session context.
- Host tests cover exact live-session matching, safe command construction, UI effects, explicit recovery cancellation/confirmation, locked and existing worktree targets, and preserving the original checkout.
- The current release overlay passed a synthetic PTY interaction check: a 200,000-tool-record startup displayed live elapsed/per-agent progress; F2 restricted visible results to User then Assistant; original preview excluded tools and scrolled to the end of a long wrapped reply; Esc preserved the query/filter and Ctrl-C exited cleanly. No native agent was launched.
- Independent conversation-flow tests verify both agents’ role filters, excluded tool/reasoning traffic, source immutability, append/restart, and schema 2 upgrade with captured context retained.
- Packaging smoke installs/uninstalls into an isolated prefix and checks that unrelated output files and synthetic native history remain intact.

## Standalone and optional Herdr modules

Standalone CLI and TUI have no dependency on the Herdr crate. Both `agent-history browse` and `agent-history-overlay` passed synthetic PTY search, Enter-preview, role-filter, and exit checks with no Herdr environment or executables on PATH. Their native fixture remained unchanged. The Herdr entry point refuses a missing managed-pane context before opening the database. Packaging tests cover standalone defaults, explicit `--with-herdr`, missing optional artifacts, and retained unrelated/native files. See MODULES.md for the dependency graph and installation choices.

## Conversation-only update (September 14)

The current build excludes tool traffic and supports All/User/Assistant search filters (F2 in the overlay; `--role` in the CLI), wrapped results and previews, and live startup progress. Schema 2 is automatically rebuilt into speaker-separated schema 3 while retaining captured source/session context. Close older clients before launching the updated build. The current schema 3 synthetic run measured search p50/p95 at 26.191/26.828 ms and initial indexing at 1,487.216 ms; the earlier measurements below remain historical. See PERFORMANCE.md for the current storage and append measurements.

## Measurements

The reproducible synthetic corpus contains 200 files, 40,100 initial records, 20,000 chunks, and about 16.63 MiB of JSONL. The recorded Apple Silicon release run measured initial indexing at 839.899 ms; a uniquely identifiable 449-byte append at 59.716 ms; warm search p50/p95 at 16.145/17.241 ms; and maximum RSS at 12,353,536 bytes. SQLite plus WAL/SHM occupied 22,500,416 bytes, 1.290 times raw input. This is synthetic evidence, not representative private-history acceptance. See [PERFORMANCE.md](PERFORMANCE.md) for reproduction and scope.

## Live Herdr acceptance run (September 20)

Scenarios 5-8 of issue #13 were executed for both agents against the running
Herdr host, from a Herdr-managed pane (`HERDR_ENV=1`, workspace `wBS`, pane
`wBS:p2`). Versions were re-confirmed on the machine: Herdr `0.7.1`, Claude
Code `2.1.278`, Codex CLI `0.153.4`. No behavior below is mocked.

### Test material

A disposable repository at `/Users/mikitahimpel/Developer/hah-acceptance-sandbox`
(main checkout `repo` plus linked worktrees `wt-claude` and `wt-codex`, commit
`2d2a67ae8ecea75010aac2f08061f88a3dca34ed`) carried the run. Four real native
sessions were produced by launching agents in Herdr panes and then letting each
process exit: Claude `98954618-5750-4e90-99c9-9de33981e3f6` in `wt-claude`,
Codex `01a0bf48-6de4-70d2-a3db-1496ef1c4747` in `wt-codex`, and Claude
`c02a0be9-6c2a-493f-b815-ab63701179de` and Codex
`01a0bf57-d486-78f1-a8f3-61fc5f9d89e4` in throwaway repositories that were then
deleted. Testing used a private index at `/tmp/hah-acceptance/index.sqlite` over
the real native roots; the shared index was not modified. Only sandbox
worktrees were deleted or recreated, and only workspaces created by the run
were closed.

### Results

- **Scenario 5, active Herdr workspace.** For both agents Enter issued
  `workspace focus`, `agent list`, `agent focus <pane>` and no `agent start`.
  Process identity was unchanged across the second Enter (Claude PID `97418`,
  Codex PID `21663`; totals 8 and 2 before and after). Verified by tracing the
  adapter's host commands through `HERDR_BIN_PATH`, not by reading UI text.
- **Scenario 6, closed workspace with an existing worktree.** After closing the
  workspace and confirming the agent process had exited, Enter created a
  workspace for the recorded cwd and started `claude --resume <id>` /
  `codex resume <id>`. The resumed Claude answered a question about the earlier
  turn without being told the answer, and re-indexing showed the new turns
  appended to the *original* transcript file under the *original* session ID —
  the resume continued that conversation rather than forking a new one. Codex
  replayed the recorded conversation and warned that the session had been
  recorded under a different model, confirming it loaded the recorded session.
- **Scenario 7, deleted worktree.** Deleting a sandbox worktree produced the
  recovery menu with no host command and no filesystem change. Declining the
  confirmation left the checkout absent, the registration merely prunable, and
  the main checkout at its original commit and branch. Confirming recreated the
  checkout in detached HEAD at the captured commit and then resumed the native
  session. Exercised for both agents.
- **Scenario 8, repository unavailable.** With the whole repository deleted,
  both transcripts stayed searchable and previewable from native history, and
  Enter reported that the recorded workspace is unavailable, offering only
  viewing and cancelling. No host command was issued and no unrelated agent was
  started.
- **Plugin route.** `herdr plugin link` and
  `herdr plugin pane open --plugin agent-history --entrypoint search` opened a
  working overlay pane running `agent-history-herdr` against the shared index,
  and `herdr plugin pane close <pane_id>` closed it. The documented
  `plugin pane close` invocation takes a pane ID, not the plugin/entrypoint
  pair.

### Defect found and fixed: a resumed Codex pane was not recognized

Herdr's Codex integration reports a native session ID only when Codex *creates*
a session; a pane running `codex resume <id>` reports none. The adapter
therefore failed to recognize its own live Codex resume, issued `agent start`,
and Herdr refused it with `agent_name_taken`. No duplicate process was created,
but the overlay reported only a bare Herdr exit status instead of focusing
the live session. The adapter now names a resumed pane
`agent-history-<native session id>` and, when the host reports no session ID for
a pane, matches that name; a pane reporting a *different* session ID is still
never treated as a match. Host failures now report the subcommand and the
host's own message. Both changes are covered by new adapter regressions and
were re-validated live: Codex scenario 5 then issued `agent focus` with no
`agent start`, and Claude continued to match on its reported session ID.

### Defect found and fixed: the quality gate mutated the repository it guarded

Git exports its directory variables to hooks, and pushing **from a linked
worktree** exports `GIT_DIR` — which is how every agent worktree in this
repository is arranged (verified against a disposable repository: a plain
checkout exports nothing, a linked worktree exports the worktree's Git
directory). Those variables override `git -C`, so the pre-push hook ran the
whole suite against the real repository: it failed core's `git::tests` *and*
wrote to the repository it was guarding, producing a fixture commit titled
`initial` on the branch being pushed and a fixture identity in the shared
`.git/config`. The gate passed and left the repository untouched when run
directly, which is why this stayed invisible until a push. Nothing reached the
remote; the repository owner has since restored the local state.

The canonical fix is `agent-history-core`'s published `git_command` helper and
its `INHERITED_GIT_ENVIRONMENT` list of ten variables, delivered separately by
the `claude/git-provenance` work that owns that crate. This branch is stacked on
it and consumes it:

* Every `git` invocation in `agent-history-herdr` now goes through
  `agent_history_core::git_command`, including the `git worktree add` that
  performs confirmed recovery, so a recorded repository is the only repository
  recreation can reach. The crate no longer contains an unisolated `git` call,
  and it does not keep a second copy of the variable list.
* `.githooks/pre-push`, which no crate owns, now resolves the repository root
  while the variables still describe the pushing worktree, then clears the same
  ten names before running the gate. Nothing is skipped — `scripts/check` still
  runs in full; it simply stops leaking Git state into the suite.
* Two tests keep the hook and the constant from drifting apart: one asserts the
  hook unsets every name in `INHERITED_GIT_ENVIRONMENT`, the other asserts the
  hook resolves the root before clearing, runs the gate afterwards, and never
  weakens or skips it.

## Exact external blocker and next step

Codex could not produce a *new* model turn after resume: this machine's Codex
account is rate limited until September 22, and every Codex turn returned
`You've hit your usage limit`. Codex resume was therefore verified by recorded
conversation replay and by Codex's own recorded-model warning, not by asking the
resumed agent to recall an earlier fact. Repeat the Claude recall check for
Codex once credits are available.

The remaining V1 acceptance work is unchanged by this run: issue #13 scenarios
1-4 and 9-10 against a real installation, clean-user macOS installation of the
Apple Silicon artifact, and verified remote CI and branch protection. Record
those results before tagging stable V1; mocked tests are not substitutes.

## Material limitations

- Only sampled boundaries verify prior content during append; arbitrary interior rewrite followed by regrowth can evade detection. Same-size changes and ordinary replacements/truncations are covered.
- Renamed sources retain unavailable old-path search rows alongside the new path.
- Initial activation indexing is synchronous; elapsed time and per-agent/file progress are visible, but search waits for the scan and there is no cancellation API. Derived chunks are retained per file until commit, so memory grows with that file's extracted text.
- The overlay is a terminal plugin, not an in-process native Herdr widget. Herdr 0.7.1 is the target; later CLI changes require compatibility work.
- Safe worktree recreation uses the captured commit in detached HEAD state; it does not recreate uncommitted changes or reconstruct unavailable commits.
- The package is a testing prerelease; clean-user installation acceptance remains pending.

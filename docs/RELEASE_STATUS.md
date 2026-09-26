# V1 release status — 0.1.0-rc.2 prerelease

Stable V1 is **not released** and GitHub issue #13 remains open. The 0.1.0-rc.2 prerelease is intended for testing on another Apple Silicon Mac; publication does not establish the remaining acceptance criteria. The repository now contains a working local CLI, terminal overlay, verified-source preview, host resume adapter, confirmed Git worktree recreation, installable package scripts, and synthetic measurements. Implementation and local tests do not establish native-agent resume compatibility or clean-user installation acceptance.

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
| #9 Native resume | UUID-only argv, source validation, official session provenance, no plain-session fallback; live macOS resume of both agents after process exit, each answering from the earlier turn with the new turns appended to the original transcript under the original session ID (Codex on 2026-09-25) | Historical sessions with absent or non-UUID native IDs remain unexercised; the product reports them as unavailable to resume rather than resuming them |
| #6 Herdr | Functional terminal overlay, confirmed isolated Git recovery; live active/closed workspace resume, deleted-worktree recreation, unavailable-repository handling, the linked plugin overlay route, and Claude/Codex parity through a post-resume model turn | A Herdr-rendered native overlay is still outside plugin v1; overlay placement remains a terminal pane |
| #11 Safeguards | Private index, safe open/sidecar rejection, rollback/corruption errors, confirmed recovery | Broader security review; see documented source-mutation limits |
| #10 Packaging | Apple Silicon archive/checksum published as GitHub prereleases rc.1 and rc.2; extracted installer, durable plugin path, isolated smoke; the published rc.2 artifact installed, upgraded and uninstalled with no toolchain or checkout (see the clean-install simulation below) | Installation on a second Mac or fresh user account; unnotarized binaries are blocked by Gatekeeper when browser-downloaded and Finder-extracted, currently handled by a documented manual step |
| #12 Documentation | README, indexing/privacy/recovery notes, plugin guide, benchmark and troubleshooting; install and first-run path walked literally from the release page in a stripped environment, with gaps fixed | A new user following the documentation alone on a clean machine, through resume |
| #13 Release gate | Local quality gate, synthetic integration evidence, and scenarios 5-8 executed live against a running Herdr host for both agents | Scenarios 1-4 and 9-10 on a real installation, clean-user macOS installation, and remote CI/branch protection; no release tag |

All GitHub issues were inspected as the source backlog. No issue is closed by this prerelease. GitHub CI passed on integration commit `3dc90cf`; branch protection has not been verified. The integration work is being promoted to `main` for prerelease distribution.

## Validation evidence

- `./scripts/setup` enabled the pre-push hook.
- `./scripts/check` passed formatting, Clippy with warnings denied, all 103 tests (69 core, 25 Herdr adapter/preflight/hook, 6 CLI, 3 shared TUI), and the release build using the lockfile.
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
* Three tests keep the arrangement from drifting apart: one asserts the hook
  unsets every name in `INHERITED_GIT_ENVIRONMENT`; one asserts the hook
  resolves the root before clearing, runs the gate afterwards, and never weakens
  or skips it; and one scans the crate's own sources so a future raw
  `Command::new("git")` fails the gate instead of silently reintroducing the
  defect.

## Clean-install simulation (September 25)

Issues #10 and #12 require installation on a clean Mac and a newcomer following
the documentation alone. Neither was possible here: there was no second Mac and
no fresh macOS user account. This run instead removed every advantage the
development machine has and followed the README literally. **It does not close
either criterion.**

### Conditions

- The published v0.1.0-rc.2 assets were downloaded with `curl`, not built; the
  archive's SHA-256 (`3050d9cb…3f04c`) matched the published `.sha256`. The
  published rc.1 assets were downloaded the same way for the upgrade check.
- Every command ran under `env -i` with a scratch `HOME` holding only synthetic
  Claude and Codex fixtures at the native default locations, and
  `PATH=/usr/bin:/bin:/usr/sbin:/sbin`: no Rust toolchain and no repository
  checkout reachable. Installs went to the scratch `HOME`'s `~/.local/bin` and to
  isolated `--prefix` directories. The real `~/.claude`, `~/.codex`, shared index
  and `~/.local/bin` were not read or written.
- macOS 26.6.2 (25G83), Apple Silicon, Gatekeeper assessments enabled.

### Verified

- **Install → index → search → preview → browse, from the release alone.**
  Default-root and explicit `--claude-root`/`--codex-root`/`--db` indexing found
  both fixtures; term, phrase, prefix and `--role` searches returned the expected
  rows; `preview` showed both conversations; `browse` in a 140×40 PTY searched,
  previewed with Space and exited with status 0. Fixture hashes were unchanged.
- **Upgrade over an existing install, published artifacts only.** rc.1 was
  installed with `--with-herdr` and run, then rc.2 installed over it, then rc.2
  over itself, with nothing removed in between. Every install produced new
  inodes, every binary launched (none SIGKILLed), `codesign -v` passed, and an
  index built by rc.1 was read by rc.2 with identical counts. rc.1 refuses an
  index written by rc.2 with an explicit schema-version error.
- **Uninstall.** `./uninstall` removed the three binaries and the plugin manifest
  from both the default and an isolated prefix, kept the index and the native
  fixtures, and was idempotent.

### Found and fixed in the documentation

- **Release blocker — Gatekeeper.** With `com.apple.quarantine` set on the archive
  as a browser sets it, command-line `tar` did not propagate the attribute, but
  extracting with Finder's Archive Utility put it on every file, and `./install`
  copied it into the prefix. Launching either copy printed `Killed: 9` (exit 137)
  and macOS showed “agent-history” Not Opened / “Apple could not verify
  “agent-history” is free of malware that may harm your Mac or compromise your
  privacy.”, whose highlighted button moves the binary to the Trash. The system
  log recorded `Terminating process due to Gatekeeper rejection`. Clearing the
  attribute on the extracted folder, on the archive before extraction, or on the
  installed binaries each fixed it, as did downloading with `curl`. The README
  now leads with the `curl` path and documents the remedy; TROUBLESHOOTING covers
  the symptom. Notarizing the binaries would remove the manual step and remains
  open.
- The download link pointed at `releases/latest`, which only redirects to the
  release list because GitHub never treats a prerelease as latest; the matching
  `releases/latest/download/…` asset URL returns 404. It now links the tag.
- The README never said to download the `.sha256` file and placed verification
  after extraction.
- `export PATH=…` was presented without saying it lasts only for one window.
- `preview` needs a session ID, but nothing said where it comes from; search
  output columns were undocumented. Search with no matches prints nothing, and
  `indexed 0 files` does not say where it looked; the default roots were never
  stated.
- Building from source (Rust toolchain, `cargo build`) was interleaved with the
  release instructions; `./uninstall` needs the extracted folder and the same
  prefix, with no fallback documented.

### Not fixed here (code, outside this documentation change)

- `agent-history --version` prints `0.1.0`, not the prerelease tag.
- `agent-history-overlay` and `agent-history-herdr` print errors as a Rust debug
  structure (`Error: Custom { kind: Other, error: "…" }`).
- Uninstall leaves the empty `share/agent-history/plugin` directory behind.

### Still requires a second Mac or a fresh user account

- A first launch on a machine whose Gatekeeper has never seen these binaries,
  including whether **Privacy & Security → Open Anyway** is offered for them.
- Safari's default *open safe files after downloading* behavior, which may
  decompress the archive itself; only Archive Utility extraction was reproduced.
- A newcomer following the README without the author's knowledge. This run
  followed it literally, but by someone who knows the code.
- A real history of useful size: fixtures here are two synthetic sessions.
- Resume from Herdr, tracked by #9 and #13.

## Exact external blockers and next steps

The September 20 blocker is resolved: Codex produced a post-resume model turn on
September 25, recorded below. Three blockers remain, and two of them need a
machine other than this one.

1. **Gatekeeper refuses browser-downloaded binaries.** They are ad-hoc signed and
   not notarized, so an archive carrying `com.apple.quarantine` yields
   `Killed: 9` and a *Move to Trash* dialog. A `curl` download or one `xattr -dr`
   command avoids it, and both are documented, but a browser user still has a
   manual step. Notarization would remove it and needs an Apple Developer
   account.
2. **No clean Mac or fresh user account** has installed from the release page.
   The simulation above removed this machine's advantages but cannot establish
   a first launch where Gatekeeper has never seen these binaries.
3. **The RFC's native Herdr-rendered overlay** is not implementable against
   plugin v1, which has no non-terminal UI extension point. The surface is a
   terminal pane instead; accepting that deviation, or funding a companion Herdr
   change, is a product decision (#6).

## Live Codex resume check (September 25)

The September 20 run could not get a new Codex model turn after resuming,
because every turn returned `You've hit your usage limit`. The account limit has
now expired, and the recall check that Claude passed was repeated for Codex.
The run was made from a Herdr-managed pane (`HERDR_ENV=1`, workspace `wCB`,
pane `wCB:p2`) with Herdr `0.7.1`. Codex CLI was `0.156.1` at the start of the
run and `0.157.0` at the end (see the self-update below). No step was mocked.

### Setup

* Disposable repository `/Users/mikitahimpel/Developer/hah-codex-sandbox/repo`
  with commit `76f0d67` (`sandbox: initial commit`). A Herdr workspace `wCD`
  was created for it with `herdr workspace create --cwd <repo> --label
  hah-codex-sandbox`, without `--focus`.
* Private index `hah-codex-sandbox/state/db/index.sqlite` in a mode-700
  directory. Every indexing and overlay run passed all three options
  explicitly: `--db <private db> --claude-root <empty directory>
  --codex-root ~/.codex/sessions/2026/09/25`. The shared index was not
  touched and default discovery was never used.
* Host commands were recorded by setting `HERDR_BIN_PATH` to a wrapper that
  logs each call and then runs the real `herdr` binary.

### Steps and evidence

1. **Beacon turn.** `herdr agent start hah-codex-beacon --workspace wCD --cwd
   <repo> --no-focus -- codex "This is a memory test. The beacon phrase is:
   cobalt-heron-7309. Do not run any commands or edit any files. Reply with
   exactly one line: BEACON ACKNOWLEDGED."` Codex (PID `21064`, model
   `gpt-5.6-sol`) replied `BEACON ACKNOWLEDGED.` Herdr reported
   `agent_session` `01a0d97c-3a5c-72c2-a47e-5ce9549b7401`, and the transcript
   was `rollout-2026-09-25T18-53-12-01a0d97c-3a5c-72c2-a47e-5ce9549b7401.jsonl`:
   15 records, 76,533 bytes, SHA-256 `799b087a…abe259dd0d`. Codex was then
   stopped, and PID `21064` was confirmed gone.
2. **Index and find.** `agent-history index` reported `indexed 1 files
   (0 failed), 15 records (76533 bytes, 3 chunks; 0 malformed)`. Searching for
   `cobalt` in `agent-history-herdr` showed the session as `resumable`.
3. **Resume with Enter.** The adapter issued `workspace focus wCD`,
   `agent list`, and `agent start
   agent-history-01a0d97c-3a5c-72c2-a47e-5ce9549b7401 --workspace wCD --cwd
   <repo> --focus -- codex resume 01a0d97c-3a5c-72c2-a47e-5ce9549b7401`.
   The workspace was open but had no live agent. The first attempt did not
   reach a turn (see the self-update below). On the second Enter the resumed
   Codex (PID `23179`) replayed the recorded conversation.
4. **The check.** The resumed Codex was asked: "What was the beacon phrase I
   gave you earlier in this conversation? Answer with the phrase only. Do not
   run any commands or read any files." It answered `cobalt-heron-7309`. The
   appended records contain no tool or shell calls, and the phrase exists only
   in the earlier turn: it appears nowhere in the repository or the new prompt.
5. **Continuation, not a fork.** After PID `23179` exited, the same file had
   grown to 27 records and 90,663 bytes. The first 76,533 bytes still hash to
   `799b087a…abe259dd0d`, so the change was a pure append. There was still one
   file under `~/.codex/sessions/2026/09/25`, and `session_index.jsonl` gained
   no new entry. Re-indexing read only the appended data: `12 records (14130
   bytes, 3 chunks; 0 malformed)`. Status still showed `sessions: 1`, and an
   assistant-only search returned `cobalt-heron-7309` under the original ID
   `01a0d97c-3a5c-72c2-a47e-5ce9549b7401`, at byte range `88217-88707` of the
   original file.
6. **No duplicate on 0.157.0.** With a newly resumed pane (`wCD:p8`, PID
   `24136`) live and not yet reporting `agent_session`, Enter from a freshly
   started overlay issued `workspace focus wCD`, `agent list`, and
   `agent focus wCD:p8`, with no `agent start`. It remained one process, and
   the PID was unchanged. That match relied on the `agent-history-<id>` pane
   name.

Afterwards workspace `wCD`, which this run had created, was closed.

### Observations from the run

* **The trust prompt blocks a first launch.** In a new directory, Codex
  0.156.1 will not start until the folder is trusted, and trusting it saves a
  `[projects."<path>"]` entry to `~/.codex/config.toml`. Neither
  `-c projects."<path>".trust_level="trusted"` nor `-s read-only -a
  on-request` skipped the prompt. With the owner's approval the prompt was
  accepted once. Codex's own changes to `config.toml` were that trust entry and
  one announcement counter. This affects only the first launch in a new
  directory; `codex resume` did not prompt again.
* **Codex updated itself in the middle of the run.** During the beacon turn,
  Codex recorded `latest_version 0.157.0` in `~/.codex/version.json`. When the
  first `codex resume` then launched in the resumed pane, it ran
  `brew upgrade --cask codex`, and the process exited after upgrading
  (`0.156.1 -> 0.157.0`). No keys were sent to that pane, and why the upgrade
  ran without a prompt was not established. The transcript was byte-identical
  afterwards, and the resume contract on 0.157.0 is unchanged. The recall
  check above ran on 0.157.0 against a session recorded by 0.156.1.
* **Session reporting on resume changed.** On 0.157.0, a resumed pane *did*
  report `agent_session` with the original ID after its first model turn. It
  did not report it before that turn (step 6). So the name-based match is
  still required. Whether 0.156.1 differed could not be observed, because that
  binary upgraded itself before the resumed pane ran.
* **A stale overlay result is refused safely.** An overlay started before the
  transcript grew rejected Enter with `unsupported: source generation is stale;
  index again` and issued no host command. Restarting the overlay re-indexed
  and cleared the error.
* **A hyphenated query fails in core.** An unrelated defect outside this
  change: `agent-history search cobalt-heron` fails with `storage error: no such
  column: heron`. The raw query is passed to FTS5 `MATCH`, where `-` is query
  syntax. A quoted phrase or a plain word works.

## Next step

Codex no longer blocks issue #9. The remaining V1 acceptance work is unchanged by this run: issue #13 scenarios
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
- The binaries are not notarized. A browser download extracted in Finder is blocked by Gatekeeper until the quarantine attribute is cleared; see TROUBLESHOOTING.md.

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
| #9 Native resume | UUID-only argv, source validation, official session provenance, no plain-session fallback; live macOS resume of both agents after process exit, each answering from the earlier turn with the new turns appended to the original transcript under the original session ID (Codex on 2026-09-25); both re-run through Herdr 0.9.3 on 2026-09-30 | Historical sessions with absent or non-UUID native IDs remain unexercised; the product reports them as unavailable to resume rather than resuming them |
| #6 Herdr | Functional terminal overlay, confirmed isolated Git recovery; live active/closed workspace resume, deleted-worktree recreation, unavailable-repository handling, the linked plugin overlay route, and Claude/Codex parity through a post-resume model turn; all four workspace states re-run for both agents against Herdr 0.9.3 on 2026-09-30 (see below) | A Herdr-rendered native overlay is still outside plugin v1; overlay placement remains a terminal pane. On 0.9.3 the focus commands' visual effect and a full plugin-pane session were not exercised |
| #11 Safeguards | Private index, safe open/sidecar rejection, rollback/corruption errors, confirmed recovery | Broader security review; see documented source-mutation limits |
| #10 Packaging | Apple Silicon archive/checksum published as GitHub prereleases rc.1 and rc.2; extracted installer, durable plugin path, isolated smoke; the published rc.2 artifact installed, upgraded and uninstalled with no toolchain or checkout (see the clean-install simulation below); `scripts/package` signs with Developer ID and a hardened runtime, and submits for notarization, when given an identity and a notary profile (see signing and notarization below) | Installation on a second Mac or fresh user account; the notarization submit has never run because no notary profile exists yet, and published rc.1/rc.2 binaries are still blocked by Gatekeeper when browser-downloaded and Finder-extracted |
| #12 Documentation | README, indexing/privacy/recovery notes, plugin guide, benchmark and troubleshooting; install and first-run path walked literally from the release page in a stripped environment, then the documented Herdr path through a live Claude resume from the rc.2 binaries (see the documented resume walkthrough below), with gaps fixed | A new user following the documentation alone on a clean machine, through resume |
| #13 Release gate | Local quality gate, synthetic integration evidence, and scenarios 5-8 executed live against a running Herdr host for both agents | Scenarios 1-4 and 9-10 on a real installation, clean-user macOS installation, and remote CI/branch protection; no release tag |

All GitHub issues were inspected as the source backlog. No issue is closed by this prerelease. GitHub CI passed on integration commit `3dc90cf`; branch protection has not been verified. The integration work is being promoted to `main` for prerelease distribution.

## Validation evidence

- `./scripts/setup` enabled the pre-push hook.
- `./scripts/check` passed formatting, Clippy with warnings denied, all 103 tests (69 core, 25 Herdr adapter/preflight/hook, 6 CLI, 3 shared TUI), and the release build using the lockfile.
- Core regressions cover source identity, generation, partial Unicode records, rollback and competing writers, both adapters, normalized previews, and selected-source session context.
- Host tests cover exact live-session matching, safe command construction, UI effects, explicit recovery cancellation/confirmation, locked and existing worktree targets, and preserving the original checkout.
- The current release overlay passed a synthetic PTY interaction check: a 200,000-tool-record startup displayed live elapsed/per-agent progress; F2 restricted visible results to User then Assistant; original preview excluded tools and scrolled to the end of a long wrapped reply; Esc preserved the query/filter and Ctrl-C exited cleanly. No native agent was launched.
- Independent conversation-flow tests verify both agents’ role filters, excluded tool/reasoning traffic, source immutability, append/restart, and schema 2 upgrade with captured context retained.
- Packaging smoke installs/uninstalls into an isolated prefix and checks that unrelated output files and synthetic native history remain intact. It also runs `scripts/package` against a synthetic source tree with stubbed build and Apple tools, covering the ad-hoc fallback, signing only, signing plus notarization, and each refusal, and checks that the git tag (or, with no tag, the commit) is stamped into `--version`. Uninstall is also checked to remove the empty `share/agent-history` directories it created while keeping anything else in them, `share/` itself, the index and the native fixtures.

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

### Fixed afterwards in code (#29, not in the rc.2 binaries)

- `agent-history --version` printed `0.1.0`, not the prerelease tag. Packaged
  builds now print the tag they were built from, e.g. `0.1.0 (v0.1.0-rc.3)`.
- `agent-history-overlay` and `agent-history-herdr` printed errors as a Rust
  debug structure (`Error: Custom { kind: Other, error: "…" }`). They now print
  one sanitized `name: message` line, as the CLI does.
- Uninstall left the empty `share/agent-history/plugin` directory behind. It now
  removes that directory and `share/agent-history` when they are empty.

### Still requires a second Mac or a fresh user account

- A first launch on a machine whose Gatekeeper has never seen these binaries,
  including whether **Privacy & Security → Open Anyway** is offered for them.
- Safari's default *open safe files after downloading* behavior, which may
  decompress the archive itself; only Archive Utility extraction was reproduced.
- A newcomer following the README without the author's knowledge. This run
  followed it literally, but by someone who knows the code.
- A real history of useful size: fixtures here are two synthetic sessions.
- Resume from Herdr, tracked by #9 and #13. The documented resume path was walked on September 29 (below), still on this machine.

## Documented resume walkthrough (September 29)

The clean-install simulation stopped before resume. This run followed the
README and the plugin guide from the published rc.2 through **search → preview →
resume**, using the documentation rather than the code to get through each
step. It ran from a Herdr-managed pane (`HERDR_ENV=1`) with Herdr 0.7.1 and
Claude Code 2.1.285. **It does not close #12:** it is still this machine and
someone who knows the code.

### Conditions

- The rc.2 assets were downloaded with `curl` exactly as the README prints
  them and the checksum printed `OK`. Install, index, search, preview, browse
  and uninstall ran under `env -i` with a scratch `HOME` that held only two
  synthetic sessions, and `PATH=/usr/bin:/bin:/usr/sbin:/sbin` until the
  README's `~/.zshrc` line added `~/.local/bin`. The fixture hashes were
  unchanged at the end.
- The plugin steps ran in a second, isolated Herdr server started with that
  scratch `HOME`, so it had its own `config.toml`, no linked plugins and no
  integrations. The user's Herdr configuration and plugin registration were not
  touched.
- Resume needs a real, signed-in agent, and Claude Code's credentials are not
  reachable under a scratch `HOME`. The resume step therefore ran in the user's
  Herdr against a new session recorded in a disposable repository
  (`hah-docs-sandbox/repo`, commit `44f7c94`), with the rc.2
  `agent-history-herdr` given a private `--db`, `--claude-root` limited to that
  repository's Claude project folder and an empty `--codex-root`. The shared
  index was not opened. Host commands were logged through `HERDR_BIN_PATH`.

### Verified

- **Plugin, as documented.** `herdr plugin link "$HOME/.local/share/agent-history/plugin"`
  and `herdr plugin pane open --plugin agent-history --entrypoint search`
  opened the overlay once the server's `PATH` contained the install directory
  (see the first finding below). The `prefix+f` binding now in the plugin guide,
  applied with `herdr server reload-config`, opened it from the keyboard
  (Ctrl-b, f). Esc from the search box closed it.
  `herdr plugin pane close <pane_id>` closed it by pane ID, and the
  plugin/entrypoint form printed `usage: herdr plugin pane close <pane_id>`.
  `herdr plugin unlink agent-history` removed the registration.
- **Search and preview in the overlay.** Typing, Tab and Space showed both
  speakers of the synthetic session. Enter on a session whose directory does not
  exist showed `The recorded workspace is unavailable.` with only view and
  cancel.
- **Resume.** A Claude session was started in the sandbox with a beacon phrase
  and allowed to exit. Its transcript was 208,946 bytes, SHA-256 `bd277112…`.
  In the overlay it was listed as `resumable`. Enter issued
  `workspace focus wCM`, `agent list`, and
  `agent start agent-history-a797900d-7409-45fe-bf5c-3585dec963c2 --workspace wCM --cwd <repo> --focus -- claude --resume a797900d-7409-45fe-bf5c-3585dec963c2`.
  The overlay then reported `✓ Resumed Claude session in repo`. Asked for the
  phrase without being told it, the resumed Claude answered `amber-lynx-4417`.
  After it exited, the same file was 233,100 bytes and its first 208,946 bytes
  still hashed to `bd277112…`, so the change was a pure append. It was still the
  only transcript in that project folder. Re-indexing read 12 new records, and
  an assistant-only search found the answer under the original session ID.
- **Gatekeeper remedies, as printed.** The archive was given a browser's
  `com.apple.quarantine` attribute and extracted with Archive Utility. Every
  extracted file carried the attribute, and so did every installed copy.
  Running one exited with status 137. `xattr -d com.apple.quarantine ~/.local/bin/agent-history*`
  fixed all three binaries in place. `cd ~/Downloads/agent-history`,
  `xattr -dr com.apple.quarantine .` and `./install` fixed the two standalone
  binaries, but not `agent-history-herdr` (second finding below).
- **Every command in the docs.** The README, TROUBLESHOOTING, HERDR_PLUGIN,
  MODULES, PERFORMANCE (synthetic parts), GITHUB_SETUP (read-only parts) and
  HOST_COMPATIBILITY commands were run against rc.2 and against a release
  build of `main` at `4a25bcc`. The synthetic `benchmark` example,
  `scripts/measure-real-corpus` without arguments and `scripts/test-packaging`
  completed. Not rerun: the real-corpus measurements and the `search_latency`
  run against a copy of the shared index (both read private history), the
  signing and notarization commands, and the one-time publication and
  branch-protection writes in GITHUB_SETUP.

### Found and fixed in the documentation

- **The plugin runs with the Herdr server's `PATH`.** With the server started
  before `~/.local/bin` was on `PATH`, the documented `plugin pane open` failed
  with `No viable candidates found in PATH "…"` even though the calling shell
  could run `agent-history-herdr`. A server started from a shell with the
  README's `PATH` line opened it. Documented in the README, the plugin guide and
  TROUBLESHOOTING.
- **The Gatekeeper folder remedy said `./install`.** After a quarantined
  `./install --with-herdr`, that left `agent-history-herdr` quarantined, and it
  still exited with status 137. The remedy now says to repeat the original
  install command. The in-place `xattr -d` also prints `No such xattr` for
  copies that are already clear; that is now called harmless.
- **The search syntax section described `main`, not rc.2.** On rc.2,
  `rate-limit`, `foo:bar`, `what?`, `C++` and an email address fail with
  `storage error: no such column: …` or `fts5: syntax error`, an unclosed
  quote fails with `unterminated string`, misspellings find nothing, and
  `search -- -v` fails with `unknown option '--'`. On `main` all of these
  behave as documented. The README now marks what came after rc.2 and gives the
  rc.2 workaround, a double-quoted phrase, which was checked for every example.
- **No keybinding was documented.** The plugin guide and README now give the
  `[[keys.command]]` block for `prefix+f`, explain why it must be `type =
  "shell"`, and say that `plugin pane close` takes a pane ID.
- **Nothing said how to check Herdr's agent integrations**, although the README
  required them. The README now starts the resume section with `herdr
  integration status` and `herdr integration install`.
- **Nothing said what Enter does on screen.** It moves focus to the session's
  workspace and starts the agent in a new pane there. The README now says so,
  and TROUBLESHOOTING lists the Enter failures seen in this and earlier runs.
- `--db /tmp/index.sqlite` fails with `index parent is not a directory`,
  because `/tmp` is a symbolic link on macOS. This is now in TROUBLESHOOTING.
- GITHUB_SETUP still said publication had been blocked, and the README said CI
  and branch protection were unverified. Protection is live, but it is weaker
  than `.github/branch-protection.json`. It requires `Quality gate` on an
  up-to-date branch and blocks force pushes and deletion. It does not require a
  pull request or resolved conversations, and it does not apply to
  administrators. Both documents now say this; the policy was not applied.

### Defects reported, not fixed here

- `index parent is not a directory` is misleading for a parent that is a
  symbolic link to a directory, such as `/tmp`.
- `./install`'s closing hint prints the plugin path as
  `~/.local/bin/../share/agent-history/plugin` rather than the resolved
  `~/.local/share/agent-history/plugin` that the README uses.
- Enter always passes `--focus`, so resuming always moves the user's focus, as
  documented. There is no way to resume in the background.

### Still requires a second Mac or a fresh user account

- A newcomer who does not know the code, following the README alone from the
  release page through resume. Every run so far, this one included, was done by
  someone with the source at hand.
- Resume under a user account whose Herdr, Claude Code and Codex were installed
  fresh. Here the resume step used this machine's signed-in Claude and its
  already-installed Herdr integrations. The isolated Herdr showed the plugin
  steps without integrations, but no agent could sign in there.
- A Codex resume through the documented path. Only Claude was resumed in this
  run; Codex was resumed live on September 25 (above).
- The Gatekeeper items listed under the clean-install simulation: a first
  launch on a Mac that has never seen these binaries, **Open Anyway**, and
  Safari's automatic extraction.
- A real history of useful size, opened through the plugin with default roots.
  That would have indexed this machine's 9 GiB private history into a new
  index, so it was not done.

## Live Herdr 0.9.3 run (September 30)

Herdr was upgraded from 0.7.1 to 0.9.3 on this machine, and its `agent start`
no longer accepts `--workspace` (issue #35). The evidence of September 20–29
above was gathered on 0.7.1 and no longer describes this host. This run fixed
the adapter and repeated scenarios 5–8 for both agents against the running
0.9.3 server, from a Herdr-managed pane (`HERDR_ENV=1`, workspace `wBY`, pane
`wBY:p1`). The contract it relies on is in
[host compatibility](HOST_COMPATIBILITY.md#herdr-093-contract).

### Conditions

- Herdr 0.9.3 (client and server, protocol 22), Claude Code 2.1.285, and Codex
  0.157.0, which upgraded itself to 0.159.0 through `brew upgrade --cask codex`
  the first time a resumed pane launched. Herdr's Claude v7 and Codex v6
  integrations, which 0.9.3 reports as outdated, were left as installed.
- A disposable repository at `/Users/mikitahimpel/Developer/hah-093-sandbox`
  (main checkout `repo`, detached linked worktrees `wt-claude` and `wt-codex`,
  commit `aafb25d`). One Claude session (`8f0f2724-…`, beacon
  `violet-heron-9312`) and one Codex session (`01a0ef3e-2f02-…`, beacon
  `copper-otter-5174`, model `gpt-5.6-luna` because the account rejects the
  configured default) were recorded there and allowed to exit.
- The release `agent-history-herdr` ran with a private `--db` and roots that
  held copies of only those two transcripts; the shared index was not opened.
  The native files stayed the ones resumed and appended to.
- Host commands went through a `HERDR_BIN_PATH` wrapper that logged each
  argv exactly as the adapter issued it. To keep the user's focus, the wrapper
  ran `--focus` as `--no-focus`, and `workspace focus`/`agent focus` as
  `workspace get`/`agent get` on the same target. Every other command ran
  unchanged. Only workspaces created by the run were closed.
- Claude's folder-trust prompt for the sandbox was accepted once (recorded in
  `~/.claude.json`). Nothing under `~/.claude` or `~/.codex` was edited or
  deleted by hand, and Codex's folder trust was not needed.

### Results

- **Scenario 5, active workspace.** Claude: Enter issued `workspace focus wB0`,
  `agent list`, `agent focus wB0:p2` and no `agent start`; PID 10343 was the only
  `claude --resume 8f0f2724-…` process before and after. Codex: a first Enter in
  the open but agent-less workspace issued `pane split wC1:p1 --direction right
  --cwd … --focus` and `agent start ah-03gyknzurn25ndhfuld9c5ejk --kind codex
  --pane wC1:p2 -- resume 01a0ef3e-…`. The second Enter issued `agent focus
  wC1:p2` and no start, and PID 72269 remained the only Codex process for that
  session. The resumed Codex answered `copper-otter-5174`.
- **Scenario 6, closed workspace, worktree present.** After closing the
  workspaces and confirming both processes had exited, Enter issued
  `workspace create --cwd <worktree> --focus`, `agent list`, and `agent start …
  --pane <root pane> -- --resume <id>` (Claude) or `-- resume <id>` (Codex),
  with no split. Asked for the phrase from the first message, Claude answered
  `violet-heron-9312` and Codex `copper-otter-5174`. The Claude transcript went
  from 232,504 to 242,256 bytes and the Codex rollout from 131,733 to 148,693
  bytes. In both, the original bytes hashed the same afterwards, and each was
  still the only file for its session.
- **Scenario 7, deleted worktree.** With both worktrees deleted, Enter showed
  the recovery menu and issued no host command. Declining (`w`, then `n`)
  issued none either: the checkout stayed absent and `repo` stayed on `main` at
  `aafb25d` with a clean status. Confirming (`w`, then `y`) recreated each
  worktree detached at `aafb25d` and then created a workspace and resumed. The
  resumed Claude answered `violet-heron-9312`. The Codex launch stopped at
  Codex's own "Cannot use the background server" prompt and `agent start` timed
  out. The adapter then issued `agent rename wC6:p1 ah-03gyk…` and reported
  `Codex was not ready in pane wC6:p1 within Herdr's startup timeout`. The next
  Enter issued `agent focus wC6:p1` and no start, while Herdr reported no
  session ID for that pane: the resume name alone prevented a duplicate.
  Choosing "Run without daemon this time" let Codex continue, and it answered
  `copper-otter-5174`.
- **Scenario 8, repository unavailable.** With the whole sandbox deleted, both
  sessions stayed searchable and previewable. Enter showed `The recorded
  workspace is unavailable.` with only view and cancel, and issued no host
  command.
- **Unsupported host.** With `HERDR_BIN_PATH` pointing at a stub that reports
  `herdr 0.7.1`, Enter showed "Herdr 0.7.1 is not supported; resume needs Herdr
  0.9.3 or newer. Run `herdr update`.", and `--version` was the only command
  the stub received.

### Defects found and fixed

- `agent start --workspace` fails on 0.9.3 (`unknown option: --workspace`). The
  adapter now obtains a pane first and uses `--kind`/`--pane`.
- The resume name `agent-history-<uuid>` is 50 characters; 0.9.3 allows 32 and
  fails with `invalid_agent_name`. It is now `ah-` plus the UUID in base 36.
- When `agent start` times out, Herdr drops the agent's name although the agent
  keeps running. The adapter restores it with `agent rename`.

### Not established by this run

- The visual effect of `workspace focus`, `agent focus`, `workspace create
  --focus` and `pane split --focus`: those were run as `get`/`--no-focus`.
  Their syntax comes from the 0.9.3 help and the targets were resolved live.
- A full overlay session opened through the plugin on 0.9.3. `plugin pane open`
  was accepted and started the pane, but it was given an index path that
  cannot exist so that it would not index real history.
- The `agent_not_ready` path through the adapter. It was seen live from Claude's
  trust prompt with a direct `agent start`, but not through Enter.
- Herdr's current Claude v10 and Codex v8 integrations, and Herdr 0.9.0–0.9.2.
- The published rc.1/rc.2 binaries contain the 0.7.1 adapter, so resume from
  them fails on 0.9.3 until a new release is cut.

## Exact external blockers and next steps

The September 20 blocker is resolved: Codex produced a post-resume model turn on
September 25, recorded below. Four blockers remain, and two of them need a
machine other than this one.

1. **Gatekeeper refuses browser-downloaded binaries.** The published rc.1 and
   rc.2 binaries are ad-hoc signed and not notarized, so an archive carrying
   `com.apple.quarantine` yields `Killed: 9` and a *Move to Trash* dialog. A
   `curl` download or one `xattr -dr` command avoids it, and both are
   documented, but a browser user still has a manual step. Developer ID signing
   alone was shown not to help. `scripts/package` can now notarize, but the
   owner must first create a notary profile and cut a new release (see
   signing and notarization above). Until then the submit path is untested.
2. **No clean Mac or fresh user account** has installed from the release page.
   The simulation above removed this machine's advantages but cannot establish
   a first launch where Gatekeeper has never seen these binaries.
3. **The RFC's native Herdr-rendered overlay** is not implementable against
   plugin v1, which has no non-terminal UI extension point. The surface is a
   terminal pane instead; accepting that deviation, or funding a companion Herdr
   change, is a product decision (#6).
4. **The published binaries predate Herdr 0.9.3.** rc.1 and rc.2 build the
   0.7.1 `agent start --workspace` call, which 0.9.3 rejects, so resume from a
   published release fails on a current Herdr until a new release is cut from
   a commit that includes the September 30 fix.

## Signing and notarization (September 26)

The clean-install simulation found that browser-downloaded binaries are killed
by Gatekeeper. `scripts/package` can now sign and notarize a release. Signing
has been done for real. Notarization has not, because no notarytool credential
profile exists on this machine yet.

### What packaging does

Signing is **opt-in**, controlled by two environment variables. Neither holds a
secret:

| Variables set | Result |
| --- | --- |
| neither | ad-hoc build, as before; ends with a `WARNING` and `signing: ad-hoc only, NOT notarized` |
| `AGENT_HISTORY_SIGN_IDENTITY` | each binary signed with that Developer ID Application identity, hardened runtime, secure timestamp and identifier `io.github.mikitahimpel.<name>`; ends with a `WARNING` and `signing: Developer ID signed, NOT notarized` |
| both, plus `AGENT_HISTORY_NOTARY_PROFILE` | the signed binaries are zipped with `ditto`, submitted with `notarytool submit --keychain-profile <name> --wait`, and the archive is only written if Apple returns `Accepted` and `spctl` then reports `source=Notarized Developer ID` for all three; ends with `signing: Developer ID signed and notarized by Apple…` |

The identity and profile are checked before the quality gate runs, so a typo
fails in seconds. A profile without an identity is refused. After signing,
each binary must pass `codesign --verify --strict`, carry
`Authority=Developer ID Application:`, and have the runtime flag. A rejected
submission prints Apple's log and leaves no archive. Auto-detecting the
identity was rejected: a machine that happened to hold the certificate would
silently produce different artifacts from CI.

### Verified on this machine

- `AGENT_HISTORY_SIGN_IDENTITY=<SHA-1 of Developer ID Application: Mikita Himpel (2666YPBJTB)> ./scripts/package`
  under Rust 1.98.1 passed the full quality gate and signed all three binaries
  with no keychain prompt. From the extracted archive, every binary passed
  `codesign --verify --strict --verbose=2`, and `codesign -dvv` showed
  `flags=0x10000(runtime)`, the Developer ID → Developer ID Certification
  Authority → Apple Root CA chain, a secure timestamp and `TeamIdentifier=2666YPBJTB`.
  `spctl --assess` reported `rejected source=Unnotarized Developer ID`, as
  expected before notarization.
- The hardened runtime does not break the programs: the installed signed
  `agent-history` indexed, reported status and searched against a synthetic
  index, and `agent-history-overlay --help` exited 0.
- With a real but nonexistent profile name, packaging stopped before building
  with notarytool's `No Keychain password item found for profile` and wrote
  nothing.

### Signing alone does not get past Gatekeeper

This was an open question, and the answer is that notarization is required.
The same release binary was copied twice. One copy kept the linker's ad-hoc
signature. The other was re-signed with the Developer ID identity, hardened
runtime and timestamp. Both ran normally. Both got
`com.apple.quarantine` set to `0083;<time>;Safari;`, as a browser sets it.
Then:

| Binary, quarantined | `--version` | syspolicyd |
| --- | --- | --- |
| ad-hoc | `Killed: 9`, exit 137 | `GK evaluateScanResult: 1 … (team: (null))`, `Prompt shown` |
| Developer ID signed, not notarized | `Killed: 9`, exit 137 | `GK evaluateScanResult: 1 … (team: 2666YPBJTB), (id: agent-history)`, `Prompt shown` |

The same result held for the signed archive installed with its own
`./install` from quarantined files: the installed copy was killed with exit 137,
while an unquarantined install of the same archive ran. The dialog's wording
for the signed case was not captured. Shipping a signed but unnotarized build
therefore changes nothing for a browser user.

### Not exercised: submit, and why there is no staple

- `notarytool submit`, the `Accepted` check, the Apple log on rejection and the
  post-notarization `spctl` check have run only against stubs in
  `scripts/test-packaging`, never against Apple. The first real release run is
  their first real test.
- **Nothing is stapled.** Apple does not support stapling tickets to bare Mach-O
  executables, and a `.tar.gz` cannot hold a ticket either. The ticket
  therefore lives only on Apple's servers. On the first launch of a quarantined
  copy, Gatekeeper looks it up online. That is the same lookup the packaging
  script's `spctl` check performs. A quarantined first launch with no network
  is expected to be refused. That is unverified. Covering the offline case would
  mean shipping a signed, notarized and stapled `.dmg` (or a `.pkg`, which
  needs a *Developer ID Installer* certificate this machine lacks) instead of
  the tar.gz, and changing the install instructions to match.

### What the owner must run

Once per machine, create the credential profile. This is the only step that
touches a credential. Replace the Apple ID placeholder with your own, and when
prompted, enter an app-specific password generated at
[account.apple.com](https://account.apple.com) → Sign-In and Security →
App-Specific Passwords. Omitting `--password` makes notarytool prompt for it
instead of leaving it in shell history:

```sh
xcrun notarytool store-credentials agent-history-notary --apple-id <your-apple-id> --team-id 2666YPBJTB
```

Then tag the release commit and cut a signed, notarized release from a clean checkout of that tag. The binaries report the tag in `--version`, and `package` prints the version it built (`package: agent-history 0.1.0 (v0.1.0-rc.3)`); a `-dirty` or `-N-g<hash>` suffix there means the checkout is not the tagged commit:

```sh
AGENT_HISTORY_SIGN_IDENTITY="Developer ID Application: Mikita Himpel (2666YPBJTB)" \
AGENT_HISTORY_NOTARY_PROFILE=agent-history-notary \
./scripts/package
```

The last line must read `signing: Developer ID signed and notarized by Apple; …`.
Submission usually takes a few minutes. Before publishing, check the result as
a browser user would get it:

```sh
tar -xzf dist/agent-history-macos-arm64.tar.gz -C /tmp
xattr -w -r com.apple.quarantine "0083;$(printf %x "$(date +%s)");Safari;" /tmp/agent-history
/tmp/agent-history/agent-history --version    # must print the version, not Killed: 9
```

After that release is published, the quarantine sections in the README and
TROUBLESHOOTING can be limited to rc.1 and rc.2.

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
  syntax. A quoted phrase or a plain word works. Fixed after rc.2 by #26.

## Next step

Codex no longer blocks issue #9. The remaining V1 acceptance work is unchanged by this run: issue #13 scenarios
1-4 and 9-10 against a real installation, clean-user macOS installation of the
Apple Silicon artifact, and verified remote CI and branch protection. Record
those results before tagging stable V1; mocked tests are not substitutes.

## Material limitations

- Only sampled boundaries verify prior content during append; arbitrary interior rewrite followed by regrowth can evade detection. Same-size changes and ordinary replacements/truncations are covered.
- Renamed sources retain unavailable old-path search rows alongside the new path.
- Initial activation indexing is synchronous; elapsed time and per-agent/file progress are visible, but search waits for the scan and there is no cancellation API. Derived chunks are retained per file until commit, so memory grows with that file's extracted text.
- The overlay is a terminal plugin, not an in-process native Herdr widget. Herdr 0.9.3 or newer is required and older hosts are refused (see the September 30 run); later CLI changes require compatibility work.
- Safe worktree recreation uses the captured commit in detached HEAD state; it does not recreate uncommitted changes or reconstruct unavailable commits.
- The package is a testing prerelease; clean-user installation acceptance remains pending.
- The published binaries are not notarized. A browser download extracted in Finder is blocked by Gatekeeper until the quarantine attribute is cleared; see TROUBLESHOOTING.md. Packaging can notarize, but that path has not yet run against Apple, and notarized bare executables cannot be stapled, so a quarantined first launch needs network access.

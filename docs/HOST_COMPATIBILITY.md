# Herdr host compatibility evidence

Research date: 2026-09-14; live host validation added 2026-09-20; live Codex
resume check added 2026-09-25; Herdr 0.9.3 contract and live re-validation added
2026-09-30. This records
the installed tools and the local Herdr source that were inspected for issues #6
(overlay and resume UX) and #9 (native resume compatibility), and the behavior
observed when the adapter was run against the running host.

## Observed versions

The installed commands report:

| Component | Evidence |
| --- | --- |
| Herdr CLI and server | `herdr --version` → `herdr 0.9.3`; `herdr status` → client and server 0.9.3, protocol 22, on 2026-09-30 (`herdr 0.7.1` through 2026-09-29) |
| Claude Code | `claude --version` → `2.1.285 (Claude Code)` on 2026-09-29 and 2026-09-30 (`2.1.278` on 2026-09-20, `2.1.241` when first researched) |
| Codex CLI | `codex --version` → `codex-cli 0.159.0` at the end of the 2026-09-30 run (`0.157.0` at its start, before Codex updated itself; `0.156.1`/`0.157.0` on 2026-09-25; `0.153.4` when first researched) |
| Herdr Claude integration | `herdr integration status` → outdated, v7 < v10 on 2026-09-30 (current v7 under Herdr 0.7.1) |
| Herdr Codex integration | `herdr integration status` → outdated, v6 < v8 on 2026-09-30 (current v6 under Herdr 0.7.1) |

The 2026-09-30 run used the outdated v7/v6 integrations as installed:
`herdr integration install` writes under `~/.claude` and `~/.codex`, which the
run was not permitted to modify. Everything recorded for 0.9.3 below therefore
holds for those integrations; the current v10/v8 integrations are untested.

The available sibling source checkout is `/Users/mikitahimpel/Developer/herdr`,
owned by `ogulcancelik/herdr`. Its `Cargo.toml` is version `0.7.5`; the
inspected checkout is commit `471041690af928d9a4d4cda8e4963cc7dcb3d6a8`
(`preview-2026-07-21-0f10e1453a7f-56-g4710416`). It has local, unrelated
uncommitted changes, so this research treats the source as read-only.

The installed integration files are:

* Claude: `~/.claude/hooks/herdr-agent-state.sh`
* Codex: `~/.codex/herdr-agent-state.sh` and `~/.codex/hooks.json`

Both hooks report a native `agent_session_id` through the local Herdr socket
using `pane.report_agent_session`. Claude also reports a transcript path, but
Herdr's resume planner uses the ID for Claude and Codex.

Under Herdr 0.7.1 the two integrations did not report equally. Codex's hook is registered for
`SessionStart` only (`~/.codex/hooks.json`). With Codex 0.153.4, a live pane
running `codex resume <id>` never carried an `agent_session` field in
`herdr agent list`. With Codex 0.157.0 (2026-09-25), such a pane reports the
original session ID only after its first post-resume model turn, and has none
before that. Claude's pane reports its session ID on resume as well. The adapter therefore
cannot identify a resumed Codex pane by native session ID. It names every pane
it starts `agent-history-<native session id>` and falls back to that name when
the host reports no session ID, while still refusing to match a pane that
reports a different one. Herdr 0.7.1 accepted and echoed that 50-character name,
and it additionally rejected a second `agent start` under a name already in use
(`agent_name_taken`), which is what surfaced the gap. Herdr 0.9.3 rejects that
name (`invalid_agent_name`); see the 0.9.3 contract below for the current name
and why it is still needed.

## Verified native resume contract

The installed CLI help is direct evidence for these invocations:

* Claude: `claude --resume <session-id>` (`-r, --resume [value]` accepts a
  session ID).
* Codex: `codex resume <session-id>` (the positional session ID is a UUID or
  session name; `-C, --cd <DIR>` selects the working directory). Re-confirmed
  unchanged from `codex resume --help` on 0.156.1 and 0.157.0 on 2026-09-25:
  `Usage: codex resume [OPTIONS] [SESSION_ID] [PROMPT]`, where "UUIDs take
  precedence if it parses".

Herdr's source implementation independently constructs exactly those argv
vectors in `src/agent_resume.rs`:

* `("herdr:claude", "claude")` → `claude --resume <id>`
* `("herdr:codex", "codex")` → `codex resume <id>`

The same source validates that only official `herdr:claude`/`claude` and
`herdr:codex`/`codex` reports can become persisted resume references. Invalid,
missing, stale, or unsupported references fall back to an ordinary shell on
Herdr restore; the product adapter must surface that as an unavailable resume
capability rather than silently claiming success.

Both invocations were executed live on 2026-09-20 against sessions whose
original processes had exited. `claude --resume <id>` restored the conversation
and answered a question that only the earlier turn supported; the new turns were
appended to the original transcript file under the original session ID, so the
resume continued that conversation rather than starting an unrelated one.
`codex resume <id>` restored the recorded conversation and warned that the
session had been recorded under a different model. The account's usage limit
blocked a post-resume Codex turn on that date. On 2026-09-25 the same check
passed for Codex: the resumed agent recalled a beacon phrase that only the
earlier turn contained. Its new turns were appended to the original rollout
file under the original session ID, and the original bytes were unchanged. Deleted-worktree recovery was
exercised for both agents against a disposable repository: the recovery menu
appears with no host command and no filesystem change, declining leaves the
checkout absent and the main checkout untouched, and confirming recreates the
checkout in detached HEAD at the captured commit. See
[release status](RELEASE_STATUS.md) for the full run and its limits.

On 2026-09-30 both invocations were run again, this time launched by Herdr
0.9.3's `agent start` (Claude Code 2.1.285, Codex 0.159.0; `codex resume --help`
unchanged). Each resumed agent answered a beacon phrase that only its first
turn contained, and each transcript grew by a pure append under the original
session ID. See [release status](RELEASE_STATUS.md#live-herdr-093-run-september-30).

## Herdr 0.9.3 contract

Established on 2026-09-30 from a Herdr-managed pane (`HERDR_ENV=1`, workspace
`wBY`, pane `wBY:p1`) against the installed and running 0.9.3. The authorities
were the binary's own help (`herdr <group> help`, `herdr agent start --help`),
`herdr api schema --json` (protocol 22, schema version 1), and live runs of each
command in workspaces created for the run. No Herdr source was consulted.

**Minimum supported version: Herdr 0.9.3.** Earlier versions are refused before
any other host command. The adapter reads `herdr --version` (present in 0.7.1
and 0.9.3), and parses `herdr X.Y.Z`. On an older host Enter shows:

> unsupported: Herdr 0.7.1 is not supported; resume needs Herdr 0.9.3 or newer. Run `herdr update`.

This was seen live through the release binary with a stub `herdr` reporting
0.7.1: `--version` was the only host command issued. Output it cannot parse
fails the same way with `could not read the Herdr version from herdr --version`.
The plugin manifest's `min_herdr_version` is 0.9.3 as well. 0.9.0–0.9.2 were never installed here, so
they are refused rather than assumed compatible.

Support for 0.7.1 was dropped rather than kept beside 0.9.3. The two
`agent start` contracts differ in who creates the pane, how the executable is
named, and which agent names are legal, so keeping both would mean two launch
paths, and only one of them can be exercised against a real host on this
machine. A path that cannot be verified is the silent-fallback risk issue #35
warns against. `herdr update` is a single command.

Commands the adapter issues, each checked against 0.9.3:

| Purpose | Command | 0.9.3 evidence |
| --- | --- | --- |
| Version gate | `herdr --version` | prints `herdr 0.9.3` |
| Workspaces | `herdr workspace list` | `result.workspaces[].workspace_id` |
| Workspace cwd | `herdr pane list --workspace ID` | `result.panes[]` with `pane_id`, `cwd`, `agent` (null for a shell) |
| Live agents | `herdr agent list` | `result.agents[]` with `workspace_id`, `pane_id`, `agent`, `name`, and `agent_session {source, agent, kind, value}`; `source` is `herdr:claude`/`herdr:codex`, `kind` is `id` |
| Focus | `herdr workspace focus ID`, `herdr agent focus PANE` | usage lines in `herdr workspace help` / `herdr agent help` |
| Open a workspace | `herdr workspace create --cwd PATH --focus` | `result.root_pane` with `pane_id`, `cwd`, `agent`; the root pane is a shell at `PATH` |
| Pane for an existing workspace | `herdr pane split PANE --direction right --cwd PATH --focus` | returns `pane_info` with the new `pane_id` at `PATH` |
| Launch | `herdr agent start NAME --kind claude\|codex --pane PANE -- ARGS` | returns `agent_started` with the launched `argv` |
| After a timeout | `herdr agent rename PANE NAME` | restores the name on the running agent |
| After a refused launch | `herdr pane close PANE` | closes only a split this resume opened |

Behaviour observed live, which the adapter depends on:

* **`--kind` names the executable; arguments after `--` are appended.**
  `--kind claude -- --resume ID` reported `argv: ["claude","--resume",ID]`, and
  `--kind codex -- resume ID` reported `["codex","resume",ID]`. The adapter
  checks the reported argv and treats any difference as an error.
* **`agent start` no longer creates layout.** The pane must exist and be at a
  shell prompt. A workspace just created by `workspace create` supplies one; an
  existing workspace gets a split beside its root pane, so no pane in use is
  written to. `--workspace`, `--cwd` and `--focus` are gone from `agent start`.
* **Agent names are 1–32 characters of `[a-z0-9_-]`, starting with a letter.**
  `agent-history-<uuid>` failed with `invalid_agent_name`. The adapter now uses
  `ah-` plus the 128-bit session ID in 25 fixed-width base-36 digits (28
  characters), so different sessions cannot share a name. A name already in use
  still fails with `agent_name_taken` before anything is launched.
* **Success means ready for input.** `agent start` waits (default 30 s) until
  Herdr detects the agent ready. Two other outcomes leave the agent running:
  - `agent_not_ready` ("blocked during startup"): Claude's folder-trust prompt
    produced it. The name was kept and the pane reported `launch_pending`.
  - `timeout` ("timed out waiting for agent startup"): Codex's update prompt
    and its "Cannot use the background server" prompt produced it. Herdr then
    **dropped the agent's name**, although the process kept running.
  The adapter reports both as waiting in that pane. After a timeout it renames
  the pane back; live, a second Enter then issued `agent focus` on that pane
  and no `agent start`, while Herdr reported no session ID for it.
* **Session reporting.** With the outdated v7 Claude integration, a pane
  started with `claude --resume ID` reported no `agent_session` in the start
  response and the next poll, and reported the original ID within about 4 seconds;
  in one of three launches it was already present in the start response. With Codex 0.159.0 and the v6 integration, `codex resume ID`
  reported the original ID in the start response. When Codex had fallen back
  to running without its background server, it reported none until its first
  post-resume turn. A new session reports nothing until its first turn for
  either agent.

The resume-name fallback therefore stays: it is what prevents a duplicate in
the gaps above. A pane whose reported session ID contradicts its name is still
never matched.

## Supported Herdr control surface

The public CLI and socket API provide the operations needed by the host adapter:

1. `workspace list` / `workspace.get` discover current workspaces; a workspace
   record includes `workspace_id`, focus state, and optional worktree
   provenance (`repo_root`, `checkout_path`, and linked-worktree state).
2. `workspace focus <workspace-id>` focuses an existing workspace.
3. `workspace create --cwd <path> [--label <text>] [--focus]` creates a
   workspace for an existing checkout. Its response includes the workspace,
   tab, and root pane IDs.
4. `agent list`, `agent get`, and `agent focus` identify/focus a live agent.
5. On Herdr 0.9.3, `pane split` provides a pane and `agent start <name> --kind
   <kind> --pane <id> -- <args>` starts a supported agent in it, as recorded
   above. Herdr 0.7.1's `agent start <name> --workspace <id> -- <argv...>`,
   which created the split itself, no longer exists; 0.9.3 answers it with
   `unknown option: --workspace`.
6. `session.snapshot` plus event subscriptions provide a bootstrap cache and
   live updates for a companion client. `herdr api schema` is available on
   0.9.3 and was used above.

Under 0.7.1, the local `v0.7.1` tag at
`fe30dd9a0fcf55cf07fe8dfedc99abdfc801e42d` was the implementation authority:
`src/app/agents.rs::start_agent` routed a supplied workspace to
`spawn_agent_split`. That is historical; 0.9.3's behaviour above replaces it.
Exact live native sessions are focused instead of started, preventing an
unnecessary duplicate process. The adapter derives workspace cwd from the
first listed pane; complex multi-cwd workspaces still require live
compatibility testing.

## Overlay and extension route

Herdr plugins are executable workflow packages, not an in-process UI SDK. The
plugin documentation explicitly says that runtime action registration and
native non-terminal plugin UI are not part of plugin v1. It does, however,
support declared actions with keybindings and terminal panes whose placement
can be `overlay`, `popup`, `split`, `tab`, or `zoomed`. A plugin command can call
back into Herdr through `HERDR_BIN_PATH`/the CLI or the socket API.

Therefore the lowest-risk compatible route for issue #6 is a companion plugin:
declare a keybound action that opens an overlay terminal pane, run the Agent
History UI there, and use the public workspace/agent APIs above for focus,
creation, and launch. This route was compatible with Herdr 0.7.x's published
plugin contract, but its UI is a terminal pane rather than a native Herdr
popup. Herdr 0.9.3 still lists the linked plugin (`herdr plugin list`) and
accepts `herdr plugin pane open --plugin agent-history --entrypoint search`
with `--placement split --target-pane PANE --no-focus`; `split` and `zoomed`
placements now require a target pane. On 2026-09-30 that opened a pane running
the installed plugin command, which exited at once as intended because it was
given an index path that cannot exist, so no history was indexed. A full
overlay session through the plugin on 0.9.3 was not run.

The installed 0.7.1 binary confirms this route with `herdr plugin --help`, which
exposes `plugin link`, `plugin list`, declared `plugin action`, and
`plugin pane open|focus|close`. The route was run end to end on 2026-09-20:
`plugin link` on the installed durable path re-registered the plugin,
`plugin pane open --plugin agent-history --entrypoint search` opened a working
overlay pane running `agent-history-herdr`, and the pane closed with
`plugin pane close <pane_id>` — that subcommand takes a pane ID rather than the
plugin and entrypoint pair. It does not expose `herdr api schema`; schema
export is present in the inspected 0.7.5 source/docs and must therefore be
treated as a newer-host convenience rather than a 0.7.1 prerequisite. The
plugin CLI itself is sufficient for linking and operating a declared pane.

If the requested overlay must be a native Herdr-rendered search surface with
custom key handling, a companion Herdr change is required. The likely source
areas to review are the existing overlay renderers under `src/ui/` (for example
`release_notes.rs` and `widgets.rs`) and the input/action dispatch under
`src/app/input/` and `src/app/actions.rs`; the public plugin/socket API cannot
register such a view. That host change must be versioned and released with the
Agent History integration, and the supported minimum Herdr revision should be
recorded in an ADR before implementation.

## Current decision and blockers

* Use the public CLI/socket API and official Claude/Codex integrations as the
  host boundary. Do not depend on Herdr's internal Rust types.
* Require Herdr 0.9.3 or newer, detected from `herdr --version` before any
  other host command; 0.7.1 is no longer supported (see the 0.9.3 contract).
* Integration versions: Herdr 0.9.3 reports the installed Claude v7 and Codex
  v6 integrations as outdated (current v10 and v8). Resume, live matching and
  session reporting were verified with the outdated ones; the current ones are
  untested here.
* Native resume command construction and actual historical resume behavior are
  verified on macOS for both agents, including a post-resume model turn that
  answers from the earlier conversation and continues the original transcript.
* Operational notes for Codex: the first launch in a new directory stops at a
  folder-trust prompt that saves to `~/.codex/config.toml`. Also, Codex 0.156.1
  ran `brew upgrade --cask codex` on its own when a resumed pane launched,
  after an earlier run had recorded a newer version. That process then exited
  without reaching the conversation, and a second Enter resumed normally.
* A terminal-pane plugin is implementable against the current host contract.
  A native search overlay is blocked on a Herdr companion change because
  plugin v1 has no native non-terminal UI extension point.
* Live socket/API probing has now been performed from a Herdr-managed pane
  against the user's running server; no server was started or stopped, and only
  workspaces created by that run were closed. Adapter host commands were
  captured by pointing `HERDR_BIN_PATH` at a logging wrapper around the real
  `herdr` binary.
* No source/API was unavailable: a local Herdr source checkout and published
  source documentation were available for inspection.

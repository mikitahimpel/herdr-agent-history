# Herdr host compatibility evidence

Research date: 2026-09-14; live host validation added 2026-09-20. This records
the installed tools and the local Herdr source that were inspected for issues #6
(overlay and resume UX) and #9 (native resume compatibility), and the behavior
observed when the adapter was run against the running host.

## Observed versions

The installed commands report:

| Component | Evidence |
| --- | --- |
| Herdr CLI | `herdr --version` → `herdr 0.7.1` |
| Claude Code | `claude --version` → `2.1.278 (Claude Code)` on 2026-09-20 (`2.1.241` when first researched) |
| Codex CLI | `codex --version` → `codex-cli 0.153.4` |
| Herdr Claude integration | `herdr integration status` → current, v7 |
| Herdr Codex integration | `herdr integration status` → current, v6 |

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

The two integrations do not report equally. Codex's hook is registered for
`SessionStart` only (`~/.codex/hooks.json`), and a live pane running
`codex resume <id>` carries no `agent_session` field in `herdr agent list`;
Claude's pane reports its session ID on resume as well. The adapter therefore
cannot identify a resumed Codex pane by native session ID. It names every pane
it starts `agent-history-<native session id>` and falls back to that name when
the host reports no session ID, while still refusing to match a pane that
reports a different one. Herdr 0.7.1 accepts and echoes that 50-character name,
and it additionally rejects a second `agent start` under a name already in use
(`agent_name_taken`), which is what surfaced the gap.

## Verified native resume contract

The installed CLI help is direct evidence for these invocations:

* Claude: `claude --resume <session-id>` (`-r, --resume [value]` accepts a
  session ID).
* Codex: `codex resume <session-id>` (the positional session ID is a UUID or
  session name; `--cd <DIR>` selects the working directory).

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
session had been recorded under a different model. Deleted-worktree recovery was
exercised for both agents against a disposable repository: the recovery menu
appears with no host command and no filesystem change, declining leaves the
checkout absent and the main checkout untouched, and confirming recreates the
checkout in detached HEAD at the captured commit. See
[release status](RELEASE_STATUS.md) for the full run and its limits.

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
5. On the installed Herdr 0.7.1, `agent start <name> --workspace <id> --
   <argv...>` starts a supported agent in that workspace. Passing the resume
   argv after `--` is the supported launch path. The installed command does
   not accept the newer source checkout's `--kind` or `--pane` options.
6. `session.snapshot` plus event subscriptions provide a bootstrap cache and
   live updates for a companion client. The raw method names are
   `workspace.list`, `workspace.focus`, `workspace.create`, `agent.list`,
   `agent.focus`, and `agent.start`.

The installed command help and the local `v0.7.1` tag at
`fe30dd9a0fcf55cf07fe8dfedc99abdfc801e42d` are the implementation authority.
In that tag, `src/app/agents.rs::start_agent` routes a supplied workspace to
`spawn_agent_split`; the adapter passes `--cwd` and `--focus` to select the
original directory and focus the new agent. An occupied pane is not overwritten.
Exact live native sessions are focused instead, preventing an unnecessary
duplicate process. The adapter derives workspace cwd from pane records;
complex multi-cwd workspaces still require live compatibility testing.

The earlier inspection of the 0.7.5 checkout was useful research, but its
changed agent-start syntax must not be treated as proof of 0.7.1 compatibility.

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
creation, and launch. This route is compatible with Herdr 0.7.x's published
plugin contract, but its UI is a terminal pane rather than a native Herdr
popup.

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
* Require current integration versions at install/runtime. The observed
  installed versions are current (Claude v7, Codex v6); Herdr 0.7.5's session
  state documentation lists native restore minimums of Claude v6 and Codex v5.
* Native resume command construction and actual historical resume behavior are
  both verified on macOS for Claude. Codex resume is verified up to conversation
  restoration; a post-resume model turn is still unverified because the account
  is rate limited.
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

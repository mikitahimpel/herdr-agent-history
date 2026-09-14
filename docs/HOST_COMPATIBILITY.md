# Herdr host compatibility evidence

Research date: 2026-09-14. This records the installed tools and the local
Herdr source that were inspected for issues #6 (overlay and resume UX) and #9
(native resume compatibility). No agent process was launched and no native
transcript was read.

## Observed versions

The installed commands report:

| Component | Evidence |
| --- | --- |
| Herdr CLI | `herdr --version` → `herdr 0.7.1` |
| Claude Code | `claude --version` → `2.1.241 (Claude Code)` |
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

This proves command construction and the CLI's accepted shape. It does not
prove that a historical session can be resumed on this machine. Manual
validation remains required for a sanitized test session in each agent, with
the original process stopped, in the original directory and in a different
existing checkout. Deleted-worktree recovery also remains unvalidated and must
continue to require explicit confirmation before Git/filesystem mutation.

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

`agent start` targets a workspace (or tab/cwd) on 0.7.1; the caller must first
ensure that the target workspace has an available shell. A host adapter should
match a historical result to a workspace by persisted cwd/worktree metadata,
focus that workspace, then resolve the target pane/agent before deciding
whether to start a resume command. The public surface has no direct
“workspace by cwd” lookup. The sibling 0.7.5 source has a different lower-level
`AgentStartParams` shape, so it is not evidence for the installed 0.7.1 CLI.

These operations are documented in the sibling source's
`docs/versions/0.7.5/website/src/content/docs/agent-automation.mdx` and
`socket-api.mdx`, with request types in `src/api/schema/workspaces.rs`,
`src/api/schema/agents.rs`, and `src/api/schema/plugins.rs`.

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
`plugin pane open|focus|close`. It does not expose `herdr api schema`; schema
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
* Treat native resume command construction as verified, while marking actual
  historical resume behavior as pending macOS manual validation.
* A terminal-pane plugin is implementable against the current host contract.
  A native search overlay is blocked on a Herdr companion change because
  plugin v1 has no native non-terminal UI extension point.
* Live socket/API probing was not performed: this checkout is outside a
  Herdr-managed pane (`HERDR_ENV` is not set), and the Herdr control skill
  requires that environment for inspecting a running session. No live server
  was started.
* No source/API was unavailable: a local Herdr source checkout and published
  source documentation were available for inspection.

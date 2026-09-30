# Agent History Herdr plugin contract (#6)

The companion plugin in `plugin/agent-history/herdr-plugin.toml` opens the `agent-history-herdr` terminal pane through Herdr's public plugin surface. It requires Herdr 0.9.3 or newer; see [host compatibility](HOST_COMPATIBILITY.md). This is a terminal overlay, not an in-process native widget. The optional executable must be installed on the plugin process's PATH (the README's install command, or `./install --with-herdr`). It requires a Herdr-managed pane (`HERDR_ENV=1`) before opening the index. `agent-history browse` and `agent-history-overlay` are standalone search/preview commands and never dispatch Herdr actions.

## Opening and closing the overlay

Register the plugin once, from a pane inside Herdr, then open:

```sh
herdr plugin install mikitahimpel/herdr-agent-history/plugin/agent-history --ref v0.1.0-rc.3
herdr plugin pane open --plugin agent-history --entrypoint search
```

`herdr plugin install OWNER/REPO/SUBDIR` (checked against Herdr 0.9.3) clones the repository at `--ref` into `~/.config/herdr/plugins/github/` and reads `herdr-plugin.toml` from `SUBDIR`, so the path must end in `plugin/agent-history`; `…/plugin` fails with `No such file or directory`. It prints the actions and pane command the manifest declares and asks before registering, unless given `--yes`. It needs no running server. Pin `--ref` to the release tag of the installed programs, so the manifest matches them. `herdr plugin uninstall agent-history` removes it.

A Herdr without `plugin install` can link the copy that `./install --with-herdr` places in `~/.local/share/agent-history/plugin` instead: `herdr plugin link "$HOME/.local/share/agent-history/plugin"`, removed with `herdr plugin unlink agent-history`. Linking does not lower the requirement: resume refuses any host below 0.9.3, so on an older Herdr only standalone search and preview work.

The manifest also declares a global action, `open`, which runs that same command; `herdr plugin action list` shows it. Herdr's default configuration binds no key to it. To open the overlay with **prefix+f** (Ctrl-b, then f, with Herdr's default prefix), add a command binding to `~/.config/herdr/config.toml` and apply it with `herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+f"
type = "shell"
command = "herdr plugin pane open --plugin agent-history --entrypoint search"
```

`type = "shell"` runs the command in the background. The command opens the overlay pane itself, so `type = "pane"` would wrap it in a second, temporary pane. Choose another key if `prefix+f` is already bound in your configuration.

Herdr starts `agent-history-herdr` with the Herdr server's environment. The server's `PATH`, not the calling shell's, must contain the install directory; otherwise `open` fails with `plugin_pane_open_failed` and `No viable candidates found in PATH "…"`. Start the Herdr server from a shell where `command -v agent-history-herdr` succeeds. Restarting it with `herdr server stop` closes every pane and agent, so save work first.

**Esc** from the search box closes the overlay pane. From a script, close it by pane ID: `herdr plugin pane close <pane_id>`, where the ID is the pane labelled `Agent History` in `herdr pane list`. `close` accepts only the pane ID; `herdr plugin pane close --plugin agent-history --entrypoint search` prints a usage error. Neither `./uninstall` nor deleting the programs removes the registration.

## Behavior

The integration uses the shared `agent-history-tui` module for rendering, progress, filters, and preview. Host commands and recovery remain in `agent-history-herdr`.

The overlay opens `~/Library/Application Support/Herdr Agent History/index.sqlite` by default. `--db PATH` overrides `AGENT_HISTORY_DB`, which overrides that default. It accepts queries against the existing index immediately and runs the incremental activation scan on a background thread. While the scan runs, the Search box's bottom border shows checked/total files and says results may be incomplete, and an empty result says it may still change. Results refresh as files commit, and again when the scan finishes. The border then shows processed/failure/skipped-record counts, how many files changed while they were read and were left for the next scan (#41), and bounded errors. Records count all parsed JSONL records, while chunks contain only conversation text. Closing the overlay stops the scan before its next record. There is no daemon.

For isolated fixtures, `--claude-root PATH` and `--codex-root PATH` disable both agents' default root discovery and use only the explicitly supplied roots. `--help` opens neither histories nor the database.

Type a multiword query normally, including spaces. F2 cycles All, User, and Assistant search filters. Tool calls/results, loaded files, reasoning, and system/developer messages are excluded. Results identify the matching speaker and wrap snippets; preview retains both speakers for context. Down/Up or Tab focuses results; Space on focused results loads normalized, role-separated original transcript context through the core's verified preview API. Preview supports Up/Down scrolling and Esc returns to the retained query and selection. Enter validates the original source and loads the selected source's persisted session metadata before dispatching resume. Rows show agent, repository, branch, date, and snippet. Control characters are sanitized for rendering. Ctrl-C exits; a terminal guard restores raw/alternate-screen state on normal return, errors, and Rust unwinding.

Resume commands use separate argv entries and honor `HERDR_BIN_PATH` when supplied. Before its first host command the adapter reads `herdr --version` and refuses anything older than 0.9.3, or output it cannot read, with a message naming the version it found. The host coordinator focuses an exact `(agent, native session ID)` match; otherwise it creates or reuses the matching workspace. Herdr 0.9 starts an agent only in an existing pane at a shell prompt, so the adapter uses the root pane of a workspace it has just created, or opens a focused split beside the root pane of an existing one (`herdr pane split PANE --direction right --cwd PATH --focus`). It then runs `herdr agent start NAME --kind claude|codex --pane PANE -- ARGS`, where Herdr supplies the executable and `ARGS` is `--resume ID` or `resume ID`, and checks that the argv Herdr reports is exactly `claude --resume ID` or `codex resume ID`. No commands are sent to a pane already in use. Invalid native IDs and stale/missing sources fail before host mutation.

A missing cwd opens explicit choices: resume in the available recorded repository, view original conversation, or cancel. Worktree recreation is offered only with an existing repository, an absolute recorded worktree path, an in-worktree cwd, and a full recorded commit hash. Choosing recreation displays the target and commit; only an explicit `y` response performs Git mutation. `n` or Esc cancels without effects.

Recreation validates commit availability, rejects existing targets and symlinks, rejects symlink traversal, and recreates a detached checkout at the captured commit. A missing but still registered target uses a single `--force` solely to replace that exact unlocked registration; locked registrations are refused. It never forces a branch, resets or prunes a repository, deletes paths, or overwrites an existing checkout. The repository's existing branch and checkout remain intact. The target parent must already exist. Missing commits, unavailable parents, locked registrations, or other Git failures remain visible errors with repository/view/cancel fallbacks. If the original cwd was a deleted untracked subdirectory not present in the saved commit, creation can succeed while resume still requires fallback; no directory content is invented.

Isolated tests exercise real database indexing/search/original preview, query focus and space behavior, stale-source refusal, mock exact-session dispatch, cancellation, existing-repository fallback, and a manually deleted registered worktree recreated only after confirmation. Temporary Git tests also check locked registrations, unavailable commits, symlink/target collisions, and preservation of the other checkout. Tests never read private histories or launch real agents/Herdr. Plugin keyboard presentation and native launches have since been exercised live (see [release status](RELEASE_STATUS.md)); clean-machine operation still requires acceptance evidence.

A release-binary PTY smoke check using an isolated synthetic history verified activation counts, multiword query typing, result focus, normalized original preview, Esc returning with selection retained, and Ctrl-C restoring the alternate screen/cursor. It intentionally did not press Enter or launch any native agent.

The launch sequence was established against the installed Herdr 0.9.3 on September 30, 2026: its command help, `herdr api schema`, and live runs recorded in [host compatibility](HOST_COMPATIBILITY.md#herdr-093-contract). If `agent start` reports the agent blocked at a prompt of its own (`agent_not_ready`), the overlay says it is waiting in that pane. If it times out, Herdr has dropped the agent's name, so the adapter restores it with `herdr agent rename`; a second Enter then focuses that pane instead of starting another. A split that Herdr refuses before launching anything is closed again. Exact live-session matches still use focus without creating another agent. Workspace cwd lookup currently uses the first listed pane; matching arbitrary directories in other panes/tabs remains a limitation.

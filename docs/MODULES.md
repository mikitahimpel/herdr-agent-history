# Standalone app and optional Herdr and Rex integrations

The repository contains six Rust crates. Herdr and Rex are optional application integrations, not requirements for browsing history.

| Module | Owns | Dependencies within this repository |
| --- | --- | --- |
| `agent-history-core` | Native transcript adapters, indexing, SQLite search, role filters, Git observations, source-verified preview | None |
| `agent-history-tui` | Terminal rendering, keyboard navigation, indexing progress, search and preview state | Core |
| `agent-history-cli` | Standalone browser and index/search/status/preview commands | Core, TUI |
| `agent-history-restore` | Host-independent resume pieces: the native resume argv (`plan`), the process scan for sessions already running (`live`), confirmed worktree recreation (`restore`), and the resume notices | Core |
| `agent-history-herdr` | Herdr workspace discovery/focus, resume in Herdr panes, recovery choices, Herdr theme | Core, TUI, Restore |
| `agent-history-rex` | Rex agent discovery/focus through `rex-agent`, resume in Rex sessions, agent-state priorities, recovery choices | Core, TUI, Restore |

The shared TUI accepts an integration that supplies its title, Enter action, optional action screen, and color palette. The standalone integration uses the ordinary preview action, performs no host operations, and keeps the default palette: ANSI-16 plus the terminal's own foreground and background. The Herdr integration intercepts Enter to resume, owns recovery confirmation state, and resolves Herdr's configured theme (`agent-history-herdr/src/theme.rs`) into the palette it passes to the TUI. The Rex integration intercepts Enter the same way, with the terminal palette. Shared UI code contains no host commands, process launcher, worktree mutation, or host config reading; anything host-specific it draws (title, Enter wording, markers, the word for closing, key spellings) comes from `Integration` methods.

### Title and input ownership

Whoever draws the pane's chrome owns its title. Everything above the results lives on the three-row Search box (#42): its top border carries the title and the role tabs with `F2`, the input row carries the query and result count, and the bottom border carries the index status, in a warning color while a scan runs or after one did not finish. Standalone titles the box with its own title. A Herdr plugin pane is already framed with the manifest's title, and Herdr marks such processes with `HERDR_PLUGIN_ID`. There, `Integration::host_draws_title` is true and the box is titled `Search`. A manual run of `agent-history-herdr` in an ordinary pane keeps its title. When the border is too narrow for all three tabs, only the active one is shown beside `F2`, and clicking it advances the filter; the scan status shortens but always says results may be partial.

Mouse input is additive: every mouse action also has a key. Clicking anywhere in a pane focuses it, clicking a result selects it, clicking a role tab switches the filter, and the wheel scrolls the pane under the pointer. The recovery screen ignores the mouse. Capturing the mouse takes drag-to-select away from the terminal, so F3 releases and restores capture, and every exit path (normal return, panic, Ctrl-C, SIGINT/SIGTERM) releases it along with the rest of the terminal state. Herdr 0.7.1 forwards mouse events to a plugin pane's process when it enables reporting, translated to pane coordinates. With capture off, Herdr's own selection works again.

### Session availability

Each result row shows whether its session still exists on this machine: on disk, recoverable (the repository is present, so the worktree can be recreated or the session opened in the repository), repository known but absent, or transcript only. `agent-history-core::availability` classifies a session with filesystem existence checks alone, and only examines clean absolute recorded paths. It also owns the recovery mapping (`recovery_options`, `recreation_target`, `clean_absolute`) that the Herdr integration's confirmed recreation uses, so there is one copy. The Git command and the mutation stay in `agent-history-herdr`. The TUI runs the checks on a worker thread, once per session, and draws from a cache. A fifth state, prioritized, comes from the result-priority extension below; the Herdr integration uses it for sessions already running in Herdr. Marker wording comes from `Integration::availability_label`; the standalone wording describes the disk and never promises resuming. A conversation that grew after the last scan (a running agent's always does) is shown as loading rather than as an error: the browser starts an incremental scan when none is running, at most every 3 seconds, and reloads it. Herdr's Enter on such a session asks for the same scan and acts on the next press.

### Result priority (#44)

Core has one extension point for ordering results by knowledge it does not have, beside the other contracts in `contracts.rs`. A `ResultPrioritizer` returns `Priorities`: sessions mapped to opaque groups, lower first. Core never interprets a group. `SqliteStore::search_prioritized` places each prioritized session's best-matching chunk ahead of the ordinary results, ordered by group, then BM25, then session ID; the ordinary results follow without those rows, and `limit` bounds the whole list. `SearchOutcome::prioritized` says how many leading results form the group.

The group comes from its own query, restricted to the prioritized sessions, so a session is surfaced however far past the `LIMIT` relevance alone would put it. Re-sorting the returned page could not do that. With no priorities, search is the same single query as before, and `search_with_fallback` is that case. The near-match fallback (#28) is decided by the ordinary results alone, and both parts then use the same expression. An exact match elsewhere is never displaced by a prioritized near match, and approximate results are labelled as before. Punctuation (#26) goes through the same expression builder.

The prioritizer is asked when the caller's view of the sessions changes, never per keystroke, so the search path only reads a precomputed set. In the TUI, `Integration::prioritizer` registers one (standalone registers none) and the browser asks it when it opens and after a scan changes the sessions. Prioritized sessions get `SessionState::Prioritized(group)`, worded by the integration. The group is drawn with a stripe down its left edge and an `◉ N <label>` count on the Results border, so it adds no row.

`agent-history-herdr` implements the trait from Herdr's `workspace list` and `agent list`, with the same matching rule resume uses, and puts running sessions in group `RUNNING`, labelled `running`. The set can go stale when an agent exits before Enter. That only affects order and the marker, because Enter asks Herdr again and resumes instead of focusing.

### Theme resolution

Herdr does not pass its theme to the panes it hosts, so the Herdr integration reads `config.toml` itself, from `HERDR_CONFIG_PATH`, `$XDG_CONFIG_HOME/herdr/`, or `~/.config/herdr/`, the same order Herdr uses. `[theme] name` selects a copy of one of Herdr's built-in palettes; `[theme.custom]` tokens and a legacy `[ui] accent` are applied on top, read live from the file. The built-in tables are copied from Herdr v0.7.5 `src/app/state.rs` (identical in v0.7.1) and can drift when Herdr changes them. A theme name missing from the copy, the `terminal` theme, and a missing or malformed config all fall back to the terminal palette; an unparseable custom color keeps the base theme's token. With `auto_switch`, the dark theme is used, because this process does not query the terminal's background the way Herdr does.

### Rex integration

`agent-history-rex` uses the same restore crate and the same recovery screen as Herdr; only the host boundary differs. Every Rex operation is a `rex-agent` command (agent-box's CLI over `rex`), run as an argv vector with stdin closed, through the `RexHost` trait so tests use a fake host. Values are passed as `--opt=value`, and a directory after `--`, so nothing a session recorded can be read as an option; nothing goes through a shell. `REX_AGENT_BIN` overrides the executable found on `PATH`. A command that exits 0 but prints to stderr (such as the app not switching because Remote Control is off) is kept as a warning that keeps the browser open with a notice.

The binary refuses to run without `REX_SESSION`, before opening the index; `--help` works anywhere. Its title is the product name alone (`integration::TITLE`), since the user is already in Rex.

**Enter.** For Claude, `rex-agent list --json` names the pane serving the session ID, which is focused; two panes for one ID are an error, not a guess. If the process scan (`restore::live`) says a process serves the session under an ID Rex does not report, the one Rex agent in its directory is focused, or nothing happens when there is not exactly one. Rex reports only Claude Code, so a running Codex session is never started twice but cannot be focused. Otherwise `rex-agent session-for <dir>` picks the Rex session whose terminals work in the directory: exit 0 opens the resume as a tab in it (`open --session=…`, tab named for the agent), any other exit (no session, or several) opens a new Rex session named after the directory. The recovery paths (recreated worktree, resume in repository) go through the same function. The browser closes after a successful focus or open, since it floats over the window it was opened from.

**Priorities.** The prioritizer runs on a worker thread with its own `rex-agent` host, alongside the process scan. A session exactly one Rex agent reports is grouped by that agent's state, most urgent first: `blocked` (waiting on a permission or answer), `error`, `done` (finished turn not yet seen), `working`, `idle`. A session found only by the process scan, or a Rex state the crate does not know, is group `RUNNING`, labelled `running`. When several groups are present, the Results border counts each in its own wording (`◉ 1 blocked 2 idle`). A Rex that cannot answer prioritizes nothing.

**Wording.** No Herdr terms appear in the Rex UI: the recovery screen speaks of the session's directory rather than a workspace, the key bar says `close` rather than `quit` and spells Control as `⌃C`. Esc from the search box closes the browser, as it does everywhere.

## Running and building

```sh
# Build only the standalone app and its dependencies.
cargo build --release -p agent-history-cli
./target/release/agent-history browse
# Equivalent standalone entry point:
./target/release/agent-history-overlay

# Optional Herdr integration, from a terminal pane managed by Herdr:
cargo build --release -p agent-history-herdr
./target/release/agent-history-herdr

# Optional Rex integration, from a Rex terminal (needs rex-agent):
cargo build --release -p agent-history-rex
./target/release/agent-history-rex
```

Standalone Enter opens preview. In the explicitly titled Herdr integration, Enter resumes. F2 role filtering and Space preview are shared. The Herdr executable checks its runtime context before opening the index; its help remains available anywhere.

The package contains both applications, but the installer selects standalone by default. `./install --with-herdr` additionally installs the Herdr executable and plugin manifest, and `./install --with-rex` the Rex executable. Native Claude Code/Codex programs are needed for resume, not for reading existing history files.

## Existing data and compatibility

Both applications use the same core schema and existing index location, `~/Library/Application Support/Herdr Agent History/index.sqlite`. The directory keeps its historical name so this module separation does not relocate data or trigger a rebuild. The database remains disposable; native histories remain canonical and read-only.

The previous `agent-history-overlay` executable is now explicitly standalone. Existing plugin installations should be updated with `--with-herdr` so the manifest launches `agent-history-herdr`. Standalone installation does not silently change a registered Herdr plugin.

This separation does not establish native resume compatibility: live Herdr/Claude/Codex acceptance is still tracked separately from standalone tests.

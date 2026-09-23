# Standalone app and optional Herdr integration

The repository contains four Rust crates. Herdr is an optional application integration, not a requirement for browsing history.

| Module | Owns | Dependencies within this repository |
| --- | --- | --- |
| `agent-history-core` | Native transcript adapters, indexing, SQLite search, role filters, Git observations, source-verified preview | None |
| `agent-history-tui` | Terminal rendering, keyboard navigation, indexing progress, search and preview state | Core |
| `agent-history-cli` | Standalone browser and index/search/status/preview commands | Core, TUI |
| `agent-history-herdr` | Herdr workspace discovery/focus, native resume, recovery choices and confirmed Git mutation | Core, TUI |

The shared TUI accepts an integration that supplies its title, Enter action, optional action screen, and color palette. The standalone integration uses the ordinary preview action, performs no host operations, and keeps the default palette: ANSI-16 plus the terminal's own foreground and background. The Herdr integration intercepts Enter to resume, owns recovery confirmation state, and resolves Herdr's configured theme (`agent-history-herdr/src/theme.rs`) into the palette it passes to the TUI. Shared UI code contains no Herdr commands, process launcher, worktree mutation, or Herdr config reading.

### Title and input ownership

Whoever draws the pane's chrome owns its title. Standalone draws its own title row, with the index status on the right. A Herdr plugin pane is already framed with the manifest's title, and Herdr marks such processes with `HERDR_PLUGIN_ID`. There, `Integration::host_draws_title` is true and the row becomes an index status strip. A manual run of `agent-history-herdr` in an ordinary pane keeps its title.

Mouse input is additive: every mouse action also has a key. Clicking anywhere in a pane focuses it, clicking a result selects it, clicking a role tab switches the filter, and the wheel scrolls the pane under the pointer. The recovery screen ignores the mouse. Capturing the mouse takes drag-to-select away from the terminal, so F3 releases and restores capture, and every exit path (normal return, panic, Ctrl-C, SIGINT/SIGTERM) releases it along with the rest of the terminal state. Herdr 0.7.1 forwards mouse events to a plugin pane's process when it enables reporting, translated to pane coordinates. With capture off, Herdr's own selection works again.

### Session availability

Each result row shows whether its session still exists on this machine: on disk, recoverable (the repository is present, so the worktree can be recreated or the session opened in the repository), repository known but absent, or transcript only. `agent-history-core::availability` classifies a session with filesystem existence checks alone, and only examines clean absolute recorded paths. It also owns the recovery mapping (`recovery_options`, `recreation_target`, `clean_absolute`) that the Herdr integration's confirmed recreation uses, so there is one copy. The Git command and the mutation stay in `agent-history-herdr`. The TUI runs the checks on a worker thread, once per session, and draws from a cache. A fifth state, already running in the host, is Herdr knowledge: it arrives through `Integration::live_sessions`, which the Herdr integration answers with the same matching rule that resume uses. Marker wording comes from `Integration::availability_label`; the standalone wording describes the disk and never promises resuming.

### Theme resolution

Herdr does not pass its theme to the panes it hosts, so the Herdr integration reads `config.toml` itself, from `HERDR_CONFIG_PATH`, `$XDG_CONFIG_HOME/herdr/`, or `~/.config/herdr/`, the same order Herdr uses. `[theme] name` selects a copy of one of Herdr's built-in palettes; `[theme.custom]` tokens and a legacy `[ui] accent` are applied on top, read live from the file. The built-in tables are copied from Herdr v0.7.5 `src/app/state.rs` (identical in v0.7.1) and can drift when Herdr changes them. A theme name missing from the copy, the `terminal` theme, and a missing or malformed config all fall back to the terminal palette; an unparseable custom color keeps the base theme's token. With `auto_switch`, the dark theme is used, because this process does not query the terminal's background the way Herdr does.

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
```

Standalone Enter opens preview. In the explicitly titled Herdr integration, Enter resumes. F2 role filtering and Space preview are shared. The Herdr executable checks its runtime context before opening the index; its help remains available anywhere.

The package contains both applications, but the installer selects standalone by default. `./install --with-herdr` additionally installs the Herdr executable and plugin manifest. Native Claude Code/Codex programs are needed for resume, not for reading existing history files.

## Existing data and compatibility

Both applications use the same core schema and existing index location, `~/Library/Application Support/Herdr Agent History/index.sqlite`. The directory keeps its historical name so this module separation does not relocate data or trigger a rebuild. The database remains disposable; native histories remain canonical and read-only.

The previous `agent-history-overlay` executable is now explicitly standalone. Existing plugin installations should be updated with `--with-herdr` so the manifest launches `agent-history-herdr`. Standalone installation does not silently change a registered Herdr plugin.

This separation does not establish native resume compatibility: live Herdr/Claude/Codex acceptance is still tracked separately from standalone tests.

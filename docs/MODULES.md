# Standalone app and optional Herdr integration

The repository contains four Rust crates. Herdr is an optional application integration, not a requirement for browsing history.

| Module | Owns | Dependencies within this repository |
| --- | --- | --- |
| `agent-history-core` | Native transcript adapters, indexing, SQLite search, role filters, Git observations, source-verified preview | None |
| `agent-history-tui` | Terminal rendering, keyboard navigation, indexing progress, search and preview state | Core |
| `agent-history-cli` | Standalone browser and index/search/status/preview commands | Core, TUI |
| `agent-history-herdr` | Herdr workspace discovery/focus, native resume, recovery choices and confirmed Git mutation | Core, TUI |

The shared TUI accepts an integration that supplies its title, Enter action, and optional action screen. The standalone integration uses the ordinary preview action and performs no host operations. The Herdr integration intercepts Enter to resume and owns recovery confirmation state. Shared UI code contains no Herdr commands, process launcher, or worktree mutation.

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

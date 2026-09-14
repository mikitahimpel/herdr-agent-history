# Herdr Agent History

Local Claude Code and Codex history search for macOS: **search → preview the original conversation → resume in Herdr**.

## Status

A local 0.1.0 candidate implements the CLI, terminal overlay, SQLite indexing, and confirmed worktree recovery. **V1 is not released:** real native-agent resume in Herdr and clean-user macOS installation acceptance remain pending. See [release status](docs/RELEASE_STATUS.md), [indexing limitations](docs/INDEXING.md), and [synthetic performance measurements](docs/PERFORMANCE.md).

## Build and install

Apple Silicon macOS is the packaged target. Herdr, Claude Code, and Codex are separate runtime requirements; they are not bundled. The adapter targets the installed Herdr 0.7.1 CLI, with official native-session integrations enabled. Native compatibility evidence and remaining checks are in [host compatibility](docs/HOST_COMPATIBILITY.md).

```sh
./scripts/setup
./scripts/package
```

The archive and SHA-256 checksum are written to `dist/`. Extract the archive, then run its installer:

```sh
tar -xzf agent-history-macos-arm64.tar.gz
cd agent-history
./install
```

The default binaries go in `~/.local/bin`; add that directory to `PATH`. The plugin is copied to `~/.local/share/agent-history/plugin`. From a Herdr-managed pane:

```sh
herdr plugin link "$HOME/.local/share/agent-history/plugin"
herdr plugin pane open --plugin agent-history --entrypoint search
```

See [overlay controls and recovery](docs/HERDR_PLUGIN.md) for keyboard behavior and the declared plugin action that can be bound in Herdr. Indexing runs on activation; there is no permanent daemon.

## Standalone CLI

```sh
agent-history index
agent-history search "portfolio visibility"
agent-history status
agent-history preview claude <native-session-id>
agent-history preview codex <native-session-id>
```

Search uses SQLite FTS5: ordinary terms, quoted phrases, prefixes such as `portfolio*`, and boolean operators. Shell quoting must preserve FTS phrase quotes, for example `agent-history search '"portfolio visibility"'`.

The default index is `~/Library/Application Support/Herdr Agent History/index.sqlite`. Use a dedicated private directory for `--db`; existing shared directories are refused. Custom histories are supported without changing native files:

```sh
agent-history index --db /tmp/agent-history-private/index.sqlite \
  --claude-root /path/to/claude/projects \
  --codex-root /path/to/codex/sessions
```

Default discovery honors `CLAUDE_CONFIG_DIR` and `CODEX_HOME`; Codex discovery includes archived sessions. Incomplete final records are retried. Failed sources are reported while unrelated files continue indexing. Source previews require an unchanged indexed generation; rerun `index` after new writes.

## Privacy and removal

Native JSONL is canonical and read-only. SQLite contains normalized searchable text, metadata, source references, and incremental checkpoints. It is sensitive local data and is rebuildable. No runtime transcript upload, telemetry, embeddings, or LLM processing is used.

From Herdr, unlink the plugin with `herdr plugin unlink agent-history`. Run the extracted package's `./uninstall` to remove the installed binaries and manifest. Uninstall preserves the index and native histories. For corruption or schema recovery, close all clients before removing only the disposable database and its SQLite sidecars; see [troubleshooting](docs/TROUBLESHOOTING.md).

## Development

| Crate | Responsibility |
| --- | --- |
| `agent-history-core` | Native adapters, chunks, transactional indexing, SQLite/FTS5, Git context, source preview |
| `agent-history-cli` | Index/search/status/preview debugging interface |
| `agent-history-herdr` | Terminal overlay, host commands, exact native resume, confirmed recovery |

Run `./scripts/setup` once per checkout and `./scripts/check` after changes. The gate runs formatting, warnings-denied Clippy, all workspace tests, and the release build with the committed lockfile. `./scripts/test-packaging` checks isolated installation/removal behavior. Tests use synthetic histories and temporary repositories; they do not launch native agents.

[AGENTS.md](AGENTS.md), the [RFC](docs/RFC.md), and GitHub issues [#1–#13](https://github.com/mikitahimpel/herdr-agent-history/issues) define the scope. The [backlog map](docs/BACKLOG.md) connects the detailed work items. Remote CI and branch protection have not been verified in this implementation run.

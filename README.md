# Agent History

Local Claude Code and Codex conversation search for macOS. Search and preview work independently; optional Herdr integration adds session resume and worktree recovery.

![Agent History searching past Claude and Codex sessions](docs/media/agent-history.png)

*Running inside Herdr, so colors follow the configured Herdr theme and Enter resumes the selected session. The conversations shown are fictional fixtures, not captured history.*

## Two ways to run it

Agent History runs **with or without Herdr**. Searching, previewing and indexing never need Herdr, a running coding agent, or a network connection — they read the native transcript files already on disk.

| | Standalone | Inside Herdr |
| --- | --- | --- |
| Command | `agent-history browse` (or `agent-history-overlay`) | `agent-history-herdr`, usually via the plugin pane |
| Search, role filters, preview | yes | yes |
| <kbd>Enter</kbd> | opens the conversation preview | resumes that Claude or Codex session |
| Worktree recovery | not offered | offered, and never mutates Git without confirmation |
| Colors | your terminal's own palette | the theme from Herdr's `config.toml` |
| Requires | nothing but the binary | Herdr, plus the matching agent executable to resume |

The standalone build has no dependency on the Herdr crate and does not read Herdr's configuration; the two entry points simply share the same index. Install the integration only if you want it:

```sh
./install                # standalone only
./install --with-herdr   # also installs the integration and plugin
```

## Status

The 0.1.0-rc.2 prerelease implements the CLI, terminal overlay, SQLite indexing, and confirmed worktree recovery. **V1 is not released:** real native-agent resume in Herdr and clean-user macOS installation acceptance remain pending. See [release status](docs/RELEASE_STATUS.md), [indexing limitations](docs/INDEXING.md), and [synthetic performance measurements](docs/PERFORMANCE.md).

## Install from a release

Apple Silicon macOS only — there are no Intel, Windows or Linux binaries. You need no Rust toolchain, no repository checkout, and no Herdr for search and preview.

The current build is the prerelease **[v0.1.0-rc.2](https://github.com/mikitahimpel/herdr-agent-history/releases/tag/v0.1.0-rc.2)**. GitHub never marks a prerelease as "latest", so use that tag link rather than the repository's *Latest release* shortcut. It has two assets: `agent-history-macos-arm64.tar.gz` and its checksum, `agent-history-macos-arm64.tar.gz.sha256`.

### Download with Terminal (recommended)

Run these in Terminal from any empty directory. They download both files, verify the checksum *before* anything is extracted, then install:

```sh
base=https://github.com/mikitahimpel/herdr-agent-history/releases/download/v0.1.0-rc.2
curl -fLO "$base/agent-history-macos-arm64.tar.gz"
curl -fLO "$base/agent-history-macos-arm64.tar.gz.sha256"
shasum -a 256 -c agent-history-macos-arm64.tar.gz.sha256   # must print: OK
tar -xzf agent-history-macos-arm64.tar.gz
cd agent-history
./install                # standalone
./install --with-herdr   # instead, to also install the Herdr integration and plugin
```

### Downloaded with a browser? Clear the quarantine first

The binaries are ad-hoc signed and **not notarized by Apple**. A browser marks what it downloads as quarantined, and double-clicking the archive in Finder passes that mark on to every extracted file — including through `./install` into `~/.local/bin`. macOS then refuses to run the program: Terminal prints only `Killed: 9`, and a dialog says:

> **“agent-history” Not Opened**
> Apple could not verify “agent-history” is free of malware that may harm your Mac or compromise your privacy.

Click **Done**. The highlighted button is **Move to Trash** (**Move to Bin** in some regions), which deletes the program. Then clear the mark from the extracted folder and install again:

```sh
cd ~/Downloads/agent-history            # wherever the archive was extracted
xattr -dr com.apple.quarantine .
./install
```

If you already installed quarantined copies, clear them in place instead: `xattr -d com.apple.quarantine ~/.local/bin/agent-history*`. The Terminal commands above avoid this entirely: `curl` does not quarantine, and neither does extracting with `tar`. Only clear the quarantine on an archive whose checksum you verified.

### Put it on your PATH

The installer places `agent-history` and `agent-history-overlay` in `~/.local/bin` (pass an absolute directory, for example `./install /opt/agent-history/bin`, to choose another). If `command -v agent-history` prints nothing, that directory is not on your `PATH`. macOS's default shell is zsh; add it permanently and open a new Terminal window:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
```

The extracted folder is only needed again to uninstall. You can delete the downloaded archive.

## First run: index, search, preview

```sh
agent-history index                          # read your Claude Code and Codex history
agent-history search "portfolio visibility"  # find a conversation
agent-history preview claude <session-id>    # read it; use codex for Codex results
agent-history browse                         # or do all of this interactively
```

`index` reads Claude Code history from `~/.claude/projects` (or `$CLAUDE_CONFIG_DIR/projects`) and Codex history from `~/.codex/sessions` and `~/.codex/archived_sessions` (or the same folders under `$CODEX_HOME`). It prints how many files it read; `indexed 0 files` means nothing was found in those places — see [troubleshooting](docs/TROUBLESHOOTING.md#search-returns-no-sessions). Rerun `index` whenever you want newer conversations included; `browse` also indexes when it starts.

`search` prints up to 50 matches, one per line, as tab-separated columns: rank, agent (`Claude` or `Codex`), role (`User` or `Assistant`), repository / branch (`- / -` when unknown), **session ID**, timestamp, source file and byte range, and the matching text. Give the agent and session ID from a result to `preview`:

```text
1	Claude	User	- / -	11111111-2222-4333-8444-555555555555	2026-09-20T10:00:00+00:00	…	Why is the portfolio visibility toggle hidden…
```

```sh
agent-history preview claude 11111111-2222-4333-8444-555555555555
```

A search with no matches prints nothing. In `browse`, type to search, press **Down** or **Tab** to reach the results, and **Space** or **Enter** to preview; **Esc** goes back and **Ctrl-C** quits. Resuming a session from the results needs the optional Herdr integration below.

### Upgrading and removing

To upgrade, download and verify the new release the same way and run its `./install` over the existing one; there is no need to uninstall first. Close any running `agent-history browse` first. The index is kept and migrated automatically when needed. An older build refuses an index created by a newer one (`database schema version … is newer than supported`) rather than altering it.

To remove the programs, run `./uninstall` from the extracted folder, passing the same directory if you installed somewhere other than `~/.local/bin`. It removes the binaries and the Herdr plugin manifest, and deliberately keeps both your native Claude/Codex history and the search index. If you no longer have the folder, delete the files yourself: `rm -f ~/.local/bin/agent-history ~/.local/bin/agent-history-overlay ~/.local/bin/agent-history-herdr ~/.local/share/agent-history/plugin/herdr-plugin.toml`. See [privacy and removal](#privacy-and-removal) for deleting the index.

## Optional: resume from Herdr

Searching existing native history files requires no Herdr installation or running coding agent. Resuming through the optional integration requires Herdr and the corresponding Claude Code or Codex executable; these are not bundled. The adapter targets the installed Herdr 0.7.1 CLI, with official native-session integrations enabled. Native compatibility evidence and remaining checks are in [host compatibility](docs/HOST_COMPATIBILITY.md).

To also install the optional Herdr executable and plugin, run `./install --with-herdr`. The plugin is copied to `~/.local/share/agent-history/plugin`. From a Herdr-managed pane:

```sh
herdr plugin link "$HOME/.local/share/agent-history/plugin"
herdr plugin pane open --plugin agent-history --entrypoint search
```

See [overlay controls and recovery](docs/HERDR_PLUGIN.md) for keyboard behavior and the declared plugin action that can be bound in Herdr. Indexing runs on activation; there is no permanent daemon.

## Standalone app and CLI reference

`agent-history browse` and the equivalent `agent-history-overlay` are standalone. Use `agent-history-herdr` inside a Herdr-managed pane when you want Enter to resume. Its title and controls identify that integration explicitly.

Both share one keyboard model: type to search, **Down/Tab** focuses results, **Space** previews, **F2** cycles the role filter, **F3** toggles mouse capture, and **Esc** goes back. The mouse is additive — click a pane to focus it, click a result to select it, and scroll the pane under the pointer. Because capturing the mouse takes drag-to-select away from your terminal, **F3** hands it back; most terminals also keep selection available while holding **Shift** (iTerm2, Terminal.app, kitty, WezTerm) or **Option** (Alacritty).

```sh
agent-history index
agent-history search "portfolio visibility"
agent-history search "portfolio visibility" --role user
agent-history search "portfolio visibility" --role assistant
agent-history status
agent-history preview claude <session-id>
agent-history preview codex <session-id>
```

Search includes user messages and assistant replies, excluding tool calls/results, loaded files, reasoning, and system/developer messages. Code deliberately included in a message remains searchable. Use `--role user`, `--role assistant`, or `--role all` (the default); in the overlay, **F2** cycles the same filters.

Search uses SQLite FTS5. Type ordinary text: punctuation is never query syntax, so `rate-limit`, `foo:bar`, `what?`, `C++` or an email address search for the words they contain (`rate-limit` finds "rate limit"). Words are split on punctuation, so `C++` matches the word `C`. Three things keep a special meaning:

- a double-quoted phrase, `"portfolio visibility"` (an unclosed quote runs to the end of the query);
- a trailing `*` for a prefix, `portfol*`;
- the uppercase operators `AND`, `OR` and `NOT` between two terms, as in `portfolio NOT draft`. Lowercase `and`/`or`/`not`, or an operator with nothing on one side, is searched as a word.

`-` does not exclude a word; use `NOT`. Parentheses and FTS column filters are not supported and are searched as text. Shell quoting must preserve phrase quotes, for example `agent-history search '"portfolio visibility"'`; put `--` before a query that starts with `-`, for example `agent-history search -- -v`.

Both apps share the index at `~/Library/Application Support/Herdr Agent History/index.sqlite`; `agent-history status` prints its location and counts. The historical directory name is retained to reuse existing data; it does not imply a Herdr dependency. Use a dedicated private directory for `--db`; existing shared directories are refused. Custom histories are supported without changing native files:

```sh
agent-history index --db /tmp/agent-history-private/index.sqlite \
  --claude-root /path/to/claude/projects \
  --codex-root /path/to/codex/sessions
```

Pass the same `--db` to `search`, `status` and `preview` afterwards. Incomplete final records are retried. Failed sources are reported while unrelated files continue indexing. Source previews require an unchanged indexed generation; rerun `index` after new writes.

## Privacy and removal

Native JSONL is canonical and read-only. SQLite contains normalized searchable text, metadata, source references, and incremental checkpoints. It is sensitive local data and is rebuildable. No runtime transcript upload, telemetry, embeddings, or LLM processing is used.

Removal has three separate parts, and nothing removes your native Claude Code or Codex history:

| What | Where | Removed by |
| --- | --- | --- |
| Programs and Herdr plugin manifest | `~/.local/bin`, `~/.local/share/agent-history/plugin` | `./uninstall` from the extracted folder |
| Herdr plugin registration | Herdr | `herdr plugin unlink agent-history`, from Herdr |
| Search index (disposable, sensitive) | `~/Library/Application Support/Herdr Agent History/` | you, after closing all clients: `rm -r ~/Library/Application\ Support/Herdr\ Agent\ History` |
| Native history (canonical) | `~/.claude/projects`, `~/.codex/sessions` | never touched by Agent History |

For corruption or schema recovery, close all clients before removing only the disposable database and its SQLite sidecars; see [troubleshooting](docs/TROUBLESHOOTING.md).

## Build from source

Building needs a checkout of this repository and the Rust toolchain pinned in `rust-toolchain.toml`; installing from a release needs neither.

```sh
./scripts/setup
./scripts/package
```

The archive and SHA-256 checksum are written to `dist/`; install from it exactly as from a release. A locally built archive is not quarantined. To build and run just the standalone app without compiling the Herdr integration:

```sh
cargo build --release -p agent-history-cli
./target/release/agent-history browse
```

## Development

| Crate | Responsibility |
| --- | --- |
| `agent-history-core` | Native adapters, chunks, transactional indexing, SQLite/FTS5, Git context, source preview |
| `agent-history-tui` | Shared terminal rendering, progress, role filters, search and preview; no Herdr dependency |
| `agent-history-cli` | Standalone app (`browse` / `agent-history-overlay`) and index/search/status/preview commands |
| `agent-history-herdr` | Optional integration executable, host commands, native resume and confirmed recovery |

See [module boundaries and entry points](docs/MODULES.md) for the standalone/integration split.

Run `./scripts/setup` once per checkout and `./scripts/check` after changes. The gate runs formatting, warnings-denied Clippy, all workspace tests, and the release build with the committed lockfile. `./scripts/test-packaging` checks isolated installation/removal behavior. Tests use synthetic histories and temporary repositories; they do not launch native agents.

[AGENTS.md](AGENTS.md), the [RFC](docs/RFC.md), and GitHub issues [#1–#13](https://github.com/mikitahimpel/herdr-agent-history/issues) define the scope. The [backlog map](docs/BACKLOG.md) connects the detailed work items. Remote CI and branch protection have not been verified in this implementation run.

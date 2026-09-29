# Troubleshooting

## “agent-history” Not Opened, or `Killed: 9`

The v0.1.0-rc.1 and rc.2 release binaries are ad-hoc signed and not notarized by Apple. If the archive was downloaded with a browser and extracted by double-clicking it in Finder, every extracted file carries the `com.apple.quarantine` attribute, and `./install` copies it into the install directory. Running any of the programs then prints only `Killed: 9` (exit status 137) in Terminal, and macOS shows:

> **“agent-history” Not Opened**
> Apple could not verify “agent-history” is free of malware that may harm your Mac or compromise your privacy.

with the buttons **Done** and a highlighted **Move to Trash** (**Move to Bin** in some regions). Choose **Done**; the other button deletes the program. After verifying the archive's checksum, clear the attribute and reinstall:

```sh
cd ~/Downloads/agent-history            # the extracted folder
xattr -dr com.apple.quarantine .
./install                               # or ./install --with-herdr, as you ran it first
```

Reinstall with the same options as before: a plain `./install` does not replace `agent-history-herdr`, so a quarantined copy of it keeps being killed. Or clear the already installed copies in place (`No such xattr` for a copy that is already clear is harmless):

```sh
xattr -d com.apple.quarantine ~/.local/bin/agent-history*
```

`xattr -l ~/.local/bin/agent-history` shows whether the attribute is present. Downloading with `curl` and extracting with `tar`, as the README shows, never sets it. The `./install` script itself is not blocked, which is why installation appears to succeed. Approving the program through **System Settings → Privacy & Security → Open Anyway** has not been tested with these command-line binaries; clearing the attribute has.

Signing is not enough on its own. A binary signed with a Developer ID certificate but not notarized is killed the same way when quarantined; only notarization avoids it. To see what a binary carries, run `codesign -dvv <binary>` (a notarizable build lists `Authority=Developer ID Application: …` and `flags=0x10000(runtime)`) and `spctl --assess --type execute --verbose=4 <binary>`, which prints `source=Notarized Developer ID` only for a notarized one. It prints `rejected` for everything else, including a local build that runs normally, because it does not consider whether the file is quarantined.

If `Killed: 9` appears on a binary with no quarantine attribute, it was most likely overwritten in place by something other than `./install`. Rerun the release's `./install`, which replaces files by rename.

## `zsh: command not found: agent-history`

The installer puts the programs in `~/.local/bin`, which is not on the default macOS `PATH`. Check with `ls ~/.local/bin/agent-history`, then add the directory to `PATH` for new Terminal windows:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
```

Open a new window afterwards. A plain `export PATH=...` lasts only for the current window.

## The index cannot be opened

The SQLite index and its parent directory are private by design. Use a path you own with `--db`, and ensure its parent directory is not group or world readable; otherwise commands stop with `storage error: index parent is not private`. A parent that is a symbolic link is refused as `index parent is not a directory`; on macOS `/tmp` is one, so `--db /tmp/index.sqlite` fails that way. A directory that does not exist yet, such as `/tmp/agent-history-private`, is created private. The default path on macOS is `~/Library/Application Support/Herdr Agent History/index.sqlite`.

If `HOME` is unavailable, commands that need the default database fail with an actionable message. Pass `--db /path/to/index.sqlite` instead.

## Search returns no sessions

`search` prints nothing when nothing matches, and it only sees what the last `index` run stored. Run `agent-history index` and read its first line. `indexed 0 files` means no history was found where Agent History looks by default:

- Claude Code: `~/.claude/projects`, or `$CLAUDE_CONFIG_DIR/projects` when that variable is set;
- Codex: `~/.codex/sessions` and `~/.codex/archived_sessions`, or the same folders under `$CODEX_HOME`.

`agent-history status` shows how many sessions each agent contributed. If your history lives elsewhere, point `index` at it:

```sh
agent-history index \
  --claude-root /path/to/claude/projects \
  --codex-root /path/to/codex/sessions
```

The database is disposable. Removing it never removes or edits native Claude or Codex history; rerun `index` to rebuild it. Do not place native transcript files at the database path.

## `preview` says session not found

`preview` takes the agent and the session ID exactly as `search` printed them: the second column (`Claude` or `Codex`) and the fifth, a long identifier such as `11111111-2222-4333-8444-555555555555`. If you indexed with `--db`, pass the same `--db` to `preview`.

## `database schema version … is newer than supported`

An older build found an index written by a newer one and refused to open it rather than altering it. Install the newer release again, or remove the disposable index (below) and rerun `index` with the older build.

## Preview says the source is missing or stale

Preview reads the canonical transcript at the stored source range. A deleted or replaced source cannot be reconstructed from SQLite. Restore the native file, then rerun `index`.

## Packaging or installation stops before copying binaries

The installer runs only on macOS Apple Silicon (`install: only macOS Apple Silicon (Darwin arm64) is supported` elsewhere) and needs an absolute directory when one is given. The release includes the standalone `agent-history` and `agent-history-overlay` binaries plus the optional `agent-history-herdr` binary. Herdr itself is not needed to build or install the standalone app. Use `--with-herdr` when installing the optional integration. The scripts do not claim clean-user installation or real native-session acceptance. See RELEASE_STATUS.md for the remaining release gate.

`scripts/package` signs and notarizes only when asked to (see RELEASE_STATUS.md for the exact commands):

- `package: AGENT_HISTORY_NOTARY_PROFILE needs AGENT_HISTORY_SIGN_IDENTITY`: Apple notarizes only Developer ID signed code; set both.
- `package: no valid Developer ID Application identity matches …`: `security find-identity -v -p codesigning` lists what the keychain holds. *Apple Development* and *Apple Distribution* certificates cannot be used outside the App Store.
- `package: notarytool cannot use keychain profile "…"`: the profile has not been created on this machine (`xcrun notarytool store-credentials`), or its credentials were revoked.
- `package: notarization failed (status: Invalid …)`: Apple's log for the submission follows the message and names each rejected file.
- `package: cannot identify this build`: the source is not a git checkout, so there is no tag or commit to put in `--version`. Package from a checkout, or set `AGENT_HISTORY_BUILD_ID` to the release tag.
- `package: build identifier "…" may contain only letters, digits and . _ + -`: `AGENT_HISTORY_BUILD_ID` holds something other than a tag-like name.

Without either variable the archive is still built, ad-hoc signed, and the output ends with a `WARNING` and `signing: ad-hoc only, NOT notarized`. Such an archive is fine for local use and must not be published.

## Uninstalling without the extracted folder

`./uninstall` lives in the extracted release folder. Without it, remove the same files by hand; adjust the directory if you installed with a custom prefix:

```sh
rm -f ~/.local/bin/agent-history ~/.local/bin/agent-history-overlay ~/.local/bin/agent-history-herdr
rm -f ~/.local/share/agent-history/plugin/herdr-plugin.toml
```

Neither route touches the index or native history. To remove the index as well, close every Agent History window and run `rm -r ~/Library/Application\ Support/Herdr\ Agent\ History`.

## `plugin pane open` fails with `No viable candidates found in PATH`

Herdr starts the overlay with its server's environment. The `PATH` in the message is the one the Herdr server started with, and it does not contain the directory `./install` used. Adding `~/.local/bin` to `~/.zshrc` does not reach a server that is already running. Save your work, run `herdr server stop` (it closes every pane and agent in Herdr), open a new Terminal window, check `command -v agent-history-herdr`, and start `herdr` again. The plugin link is kept.

## Enter does not resume

- `The recorded workspace is unavailable`: the directory the session ran in is gone and no repository was recorded to recover from. Choose **v** to read the conversation; resuming needs that directory back.
- A recovery menu offering to recreate a worktree: the repository still exists but the worktree was deleted. Nothing changes until you confirm with `y`; `n` or **Esc** cancels. See [the plugin guide](HERDR_PLUGIN.md).
- `unsupported: source generation is stale; index again`: the transcript grew after the overlay opened, often because the session is still running. Close the overlay with **Esc** and open it again; opening re-indexes.
- A second copy of an agent that was already running: Herdr's integration for that agent is missing or outdated. Check `herdr integration status` and run `herdr integration install claude` or `herdr integration install codex`.
- The new pane exits at once or shows the agent's own error: the agent itself failed to start. Check that `claude` or `codex` runs in a plain Herdr pane. Codex can stop for a folder-trust prompt or update itself on launch; press Enter in the overlay again afterwards.

## The Herdr executable says it needs a managed pane

For standalone search and preview, run `agent-history browse` or `agent-history-overlay` in any terminal. To resume through Herdr, open a terminal pane inside Herdr and run `agent-history-herdr`, or use the installed plugin. The two entry points have different Enter actions and share the same index.

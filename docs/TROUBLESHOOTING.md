# Troubleshooting

## The index cannot be opened

The SQLite index and its parent directory are private by design. Use a path you own with `--db`, and ensure its parent directory is not group or world readable. The default path on macOS is `~/Library/Application Support/Herdr Agent History/index.sqlite`.

If `HOME` is unavailable, commands that need the default database fail with an actionable message. Pass `--db /path/to/index.sqlite` instead.

## Search returns no sessions

Run an explicit indexing pass with roots containing native JSONL files:

```sh
agent-history index --db /tmp/agent-history-private/index.sqlite \
  --claude-root "$HOME/.claude/projects" \
  --codex-root "$HOME/.codex/sessions"
```

The database is disposable. Removing it never removes or edits native Claude or Codex history; rerun `index` to rebuild it. Do not place native transcript files at the database path.

## Preview says the source is missing or stale

Preview reads the canonical transcript at the stored source range. A deleted or replaced source cannot be reconstructed from SQLite. Restore the native file, then rerun `index`.

## Packaging or installation stops before copying binaries

Packaging targets macOS Apple Silicon and includes the standalone `agent-history` and `agent-history-overlay` binaries plus the optional `agent-history-herdr` binary. Herdr itself is not needed to build or install the standalone app. Use `--with-herdr` when installing the optional integration. The scripts do not claim clean-user installation or real native-session acceptance. See RELEASE_STATUS.md for the remaining release gate.

## The Herdr executable says it needs a managed pane

For standalone search and preview, run `agent-history browse` or `agent-history-overlay` in any terminal. To resume through Herdr, open a terminal pane inside Herdr and run `agent-history-herdr`, or use the installed plugin. The two entry points have different Enter actions and share the same index.

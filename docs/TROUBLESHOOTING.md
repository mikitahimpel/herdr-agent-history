# Troubleshooting

## The index cannot be opened

The SQLite index and its parent directory are private by design. Use a path you own with `--db`, and ensure its parent directory is not group or world readable. The default path on macOS is `~/Library/Application Support/Herdr Agent History/index.sqlite`.

If `HOME` is unavailable, commands that need the default database fail with an actionable message. Pass `--db /path/to/index.sqlite` instead.

## Search returns no sessions

Run an explicit indexing pass with roots containing native JSONL files:

```sh
agent-history index --db /tmp/agent-history.sqlite \
  --claude-root "$HOME/.claude/projects" \
  --codex-root "$HOME/.codex/sessions"
```

The database is disposable. Removing it never removes or edits native Claude or Codex history; rerun `index` to rebuild it. Do not place native transcript files at the database path.

## Preview says the source is missing or stale

Preview reads the canonical transcript at the stored source range. A deleted or replaced source cannot be reconstructed from SQLite. Restore the native file, then rerun `index`.

## Packaging or installation stops before copying binaries

Packaging currently targets macOS Apple Silicon and requires both `agent-history` and the host-provided `agent-history-overlay` release binaries. The overlay integration is still a release blocker. The scripts do not claim a clean-user installation until that artifact and release validation are available.

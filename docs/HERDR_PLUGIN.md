# Agent History Herdr plugin contract

The current Herdr 0.7.1 CLI exposes plugin linking and terminal panes. Agent
History can ship a companion plugin whose action opens a terminal overlay; the
plugin process then runs the Agent History UI and calls Herdr's public CLI.

Minimal manifest shape:

```toml
id = "agent-history"
name = "Agent History"
version = "0.1.0"
min_herdr_version = "0.7.1"
description = "Search and resume local Claude Code and Codex sessions"
platforms = ["macos"]

[[actions]]
id = "open"
title = "Open Agent History"
contexts = ["global"]
command = ["agent-history", "overlay"]

[[panes]]
id = "search"
title = "Agent History"
placement = "overlay"
command = ["agent-history", "overlay"]
```

The exact action keybinding is configured by the user in Herdr's keybinding
configuration or by the plugin's supported action declaration. The overlay
process should use `HERDR_BIN_PATH` when invoking Herdr and preserve each
argument as a separate argv item. The resume sequence is:

1. list workspaces and match the session's persisted cwd;
2. focus the matching workspace, or create one with `workspace create --cwd`;
3. inspect live agents and focus only an exact `(agent, native session ID)`;
4. when no exact live match exists, start the agent with the verified native
   argv (`claude --resume ID` or `codex resume ID`) in the returned shell pane;
5. report an unavailable resume capability when the ID or workspace cannot be
   resolved. Never start a plain Claude or Codex process as a fallback.

Herdr plugin v1 does not provide in-process native widgets. A native Herdr
popup would require a companion Herdr release and source changes, while this
terminal-pane route is available in the installed 0.7.1 plugin surface.

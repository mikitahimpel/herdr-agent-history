# Herdr Agent History

Local search and restoration for Claude Code and Codex development sessions on macOS.

**Search → Find → Preview → Resume in Herdr.** Native agent session files remain canonical; the SQLite + FTS5 index is disposable.

## Status

The standalone CLI supports indexing, full-text search, status, and bounded source preview. Herdr overlay integration and session restoration remain in progress.

- [RFC](docs/RFC.md): product requirements and architectural constraints.
- [Implementation backlog](docs/BACKLOG.md): sequenced work, dependencies, and acceptance criteria.
- [Issue definitions](docs/issues/): publication-ready GitHub issue bodies.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `agent-history-core` | Agent formats, normalization, indexing, SQLite, search, source preview, Git context |
| `agent-history-cli` | Debugging interface: `index`, `search`, `status`, and `preview` commands |
| `agent-history-herdr` | Host boundary, resume orchestration, workspace restoration |

Herdr's overlay will require integration in Herdr itself. This repository owns the core and host adapter; the integration issue must identify the supported host revision and extension mechanism.

## Development

```sh
./scripts/setup  # once per checkout: enables the pre-push hook
./scripts/check  # formatting, Clippy, tests, release build
```

Agents must follow [AGENTS.md](AGENTS.md); Claude Code also loads [CLAUDE.md](CLAUDE.md). CI and the pre-push hook run the same required gate. [GitHub publication and required-check setup](docs/GITHUB_SETUP.md) remain pending because remote writes were blocked.

The CLI uses SQLite with FTS5. See [Troubleshooting](docs/TROUBLESHOOTING.md) for private-index and rebuild guidance.

## V1 constraints

Local files and processes only. No cloud, transcript telemetry, embeddings, LLM processing, or permanent daemon. Index user/assistant text and selected useful tool output, never a second raw transcript archive. Protect the database and its sidecars with user-only permissions.

Worktree recreation requires explicit confirmation at runtime. Existing worktrees should resume without extra decisions.

# Herdr Agent History

Local search and restoration for Claude Code and Codex development sessions on macOS.

**Search → Find → Preview → Resume in Herdr.** Native agent session files remain canonical; the SQLite + FTS5 index is disposable.

## Status

Planning and repository scaffold. Indexing, search, preview, and restoration are not implemented yet. The CLI currently reports this explicitly.

- [RFC](docs/RFC.md): product requirements and architectural constraints.
- [Implementation backlog](docs/BACKLOG.md): sequenced work, dependencies, and acceptance criteria.
- [Issue definitions](docs/issues/): publication-ready GitHub issue bodies.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `agent-history-core` | Agent formats, normalization, indexing, SQLite, search, source preview, Git context |
| `agent-history-cli` | Debugging interface: planned `index`, `search`, and `status` commands |
| `agent-history-herdr` | Host boundary, resume orchestration, workspace restoration |

Herdr's overlay will require integration in Herdr itself. This repository owns the core and host adapter; the integration issue must identify the supported host revision and extension mechanism.

## Development

```sh
./scripts/setup  # once per checkout: enables the pre-push hook
./scripts/check  # formatting, Clippy, tests, release build
```

Agents must follow [AGENTS.md](AGENTS.md); Claude Code also loads [CLAUDE.md](CLAUDE.md). CI and the pre-push hook run the same required gate. [GitHub publication and required-check setup](docs/GITHUB_SETUP.md) remain pending because remote writes were blocked.

The initial scaffold has no third-party dependencies. SQLite with FTS5 will be selected and verified in the storage issue.

## V1 constraints

Local files and processes only. No cloud, transcript telemetry, embeddings, LLM processing, or permanent daemon. Index user/assistant text and selected useful tool output, never a second raw transcript archive. Protect the database and its sidecars with user-only permissions.

Worktree recreation requires explicit confirmation at runtime. Existing worktrees should resume without extra decisions.

# V1 implementation backlog

GitHub issues [#1–#13](https://github.com/mikitahimpel/herdr-agent-history/issues) are the authoritative V1 backlog. The smaller work items below elaborate their scope; they are not separately published issues.

Repository: https://github.com/mikitahimpel/herdr-agent-history

The bootstrap creates three Rust crates, CI, formatting/lint/test/build gate, a local pre-push hook, and agent instructions. Shared domain contracts are implemented; product integration and the release gate remain pending. Server-side required checks have not been verified.

## GitHub issue mapping

| GitHub issue | Detailed work items below |
| --- | --- |
| #7 Foundation | foundation |
| #1 Ingestion | fixtures, claude, codex, chunks |
| #2 Storage/search | storage, search |
| #3 Incremental indexing | incremental, mutations |
| #4 Git context | git-context |
| #5 CLI | cli, preview |
| #8 Core integration | lifecycle, performance (core measurements) |
| #9 Native resume compatibility | resume-agents |
| #6 Herdr integration | host-contract, restore-existing, restore-deleted, overlay, overlay-actions |
| #11 Safeguards | privacy, restoration and corruption safeguards |
| #10 Packaging | release (installation/artifacts) |
| #12 Documentation | release (usage/troubleshooting) |
| #13 Release gate | performance, release (end-to-end evidence) |

Implementation does not imply acceptance: real native-agent/Herdr validation and clean macOS installation evidence are required before closing dependent release issues.

## Work items

| ID | Work | Depends on |
| --- | --- | --- |
| quality-enforcement | [Require the quality gate on GitHub main](issues/quality-enforcement.md) | — |
| foundation | [Define core domain models and adapter contracts](issues/foundation.md) | — |
| fixtures | [Build sanitized Claude and Codex compatibility fixtures](issues/fixtures.md) | foundation |
| claude | [Implement Claude session discovery and parsing](issues/claude.md) | foundation, fixtures |
| codex | [Implement Codex session discovery and parsing](issues/codex.md) | foundation, fixtures |
| chunks | [Build bounded conversation chunks with resumable turn state](issues/chunks.md) | claude, codex |
| storage | [Implement SQLite metadata schema, migrations, and FTS5 storage](issues/storage.md) | foundation |
| privacy | [Enforce local-only processing and private index permissions](issues/privacy.md) | storage |
| incremental | [Implement transactional byte-offset incremental indexing](issues/incremental.md) | chunks, storage, privacy |
| mutations | [Handle truncated, replaced, renamed, and missing source files](issues/mutations.md) | incremental |
| git-context | [Capture repository and worktree context during indexing](issues/git-context.md) | foundation, storage |
| search | [Implement ranked FTS5 search with snippets](issues/search.md) | storage, chunks |
| lifecycle | [Coordinate activation indexing and visible initial progress](issues/lifecycle.md) | incremental, mutations, git-context, search |
| preview | [Read original conversation context around search matches](issues/preview.md) | claude, codex, mutations |
| cli | [Implement index, search, and status CLI commands](issues/cli.md) | search, lifecycle, preview |
| host-contract | [Define and implement the Herdr host integration boundary](issues/host-contract.md) | foundation |
| resume-agents | [Implement verified native Claude and Codex resume commands](issues/resume-agents.md) | claude, codex, host-contract |
| restore-existing | [Resume in existing or closed workspaces with existing worktrees](issues/restore-existing.md) | git-context, host-contract, resume-agents |
| restore-deleted | [Add confirmed deleted-worktree recovery and fallbacks](issues/restore-deleted.md) | restore-existing |
| overlay | [Build the Agent History search overlay in Herdr](issues/overlay.md) | host-contract, search, lifecycle |
| overlay-actions | [Connect preview and resume/recovery interactions in Herdr](issues/overlay-actions.md) | overlay, preview, restore-existing, restore-deleted |
| performance | [Measure search, incremental latency, storage, and idle resources](issues/performance.md) | cli, overlay-actions |
| release | [Validate V1 end to end and document macOS installation](issues/release.md) | privacy, cli, overlay-actions, performance |

## Delivery sequence

1. Models, fixtures, storage, and the Herdr integration contract.
2. Agent parsers, chunking, privacy, and transactional indexing.
3. File mutations, Git context, search, source preview, and CLI.
4. Native resume, workspace restoration, and Herdr overlay.
5. End-to-end validation, benchmarks, and macOS installation documentation.

The quality-enforcement task can be completed as soon as the scaffold is published and CI runs. Core/parser work and the Herdr contract can proceed independently.

## Architectural decisions to resolve within issues

- Open-turn chunk continuation must match whole-file indexing across restarts.
- Source generations must prevent stale ranges from previewing replaced files.
- Index-time Git observations are not automatically historical branch/commit facts.
- Space must still work in multiword query input; preview uses result focus.
- The Herdr contract issue must establish an actual supported host integration route.
- Unknown complete records need a visible, deterministic skip/error policy; incomplete tails must be retried.

## Publishing

See [GitHub setup](GITHUB_SETUP.md). The publisher creates one tracking issue plus 23 implementation issues and fills dependency links. Stable markers make retries reuse existing issues. Deferred semantic search/daemon features are deliberately excluded from the V1 backlog.

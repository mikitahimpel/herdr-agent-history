# RFC: Herdr Agent History

Status: Draft · Platform: macOS · Host: Herdr · Agents: Claude Code, Codex · Implementation: Rust · Storage: SQLite + FTS5

Editorially condensed from the supplied RFC, preserving its numbered requirements. The source's final sentence was truncated; no missing wording is attributed to it.

## 1. Summary

Global search across local Claude Code and Codex sessions, with historical sessions resumed directly in Herdr. Open search → type remembered terms → find matching session → preview → Enter → resume. Users need not remember the agent, repository, worktree, date, or whether the workspace remains open. This is an index and restoration layer over native storage.

## 2. Product principle

A result represents a resumable development session, not a document. Search → Find → Resume. Original agent session files remain authoritative.

## 3. Goals

Global search across both agents; speed independent of open workspace count; closed/historical sessions; conversation preview; repository/worktree context; direct resumption; restoration of closed workspaces when worktrees exist; low disk usage; negligible idle resources; incremental indexing.

## 4. Non-goals

No embeddings, semantic/vector search, LLM summaries, cloud synchronization, remote storage, classification, knowledge extraction, Git knowledge graphs, code-symbol indexing, cross-device history, or permanent daemon unless measurements justify it.

## 5. UX

A configurable shortcut opens **Agent History**. Rows show agent, repository, branch/worktree, date, and snippet. Example search: `portfolio visibility`.

```text
Agent History
> portfolio visibility
Claude · crypto / portfolio-refactor · Sep 11
  We don't need PortfolioPriceSubscription because...
Codex · crypto / main · Sep 09
  Another option is to make subscriptions driven by...
Claude · trading / prices · Aug 28
  The websocket subscription should be stopped when...
↑↓ Navigate    Space Preview    Enter Resume
```

Type searches, Space previews, Enter resumes. Normal flows hide workspace creation and agent CLI implementation details.

## 6. Search

Local SQLite FTS5 over normalized conversation text: inverted full-text indexing, phrases, prefixes, BM25 ranking, snippets, and retrieval without per-query JSONL scanning. No embeddings.

## 7. Search result model

A conversation match belongs to a session:

```rust
struct SearchResult {
    session_id: SessionId,
    agent: Agent,
    repo: Option<String>,
    branch: Option<String>,
    cwd: Option<PathBuf>,
    timestamp: Option<DateTime<Utc>>,
    source_start: u64,
    source_end: u64,
    snippet: String,
}
```

It identifies the discussion, locates original preview text, and supports context restoration.

## 8. Original sessions remain canonical

Claude: `~/.claude/.../*.jsonl`. Codex: `~/.codex/sessions/.../*.jsonl`. Original JSONL is the canonical transcript; normalized searchable text feeds SQLite FTS. Do not maintain a second complete raw archive.

## 9. Storage model

- `sessions`: session ID, agent, source file, cwd, repository, worktree, branch, commit, timestamps.
- `indexed_files`: path, inode, size, last processed offset, mtime.
- `search_chunks`: session ID, timestamp, source byte range, normalized text.
- FTS index of normalized text.

Exclude tool protocol metadata, JSON structure, UUIDs, and irrelevant fields from search.

## 10. Storage optimization

Product refinement (September 14, 2026): index user messages and assistant replies to the user only. Exclude tool calls and results (including loaded files and command output), reasoning, system/developer messages, token accounting, and internal events. Preserve code deliberately written in a user message or assistant reply. Search supports all conversation text, user messages only, or assistant replies only. The source's example of 1 GB raw history containing 400 MB useful text is illustrative, not a measured guarantee. Avoid unnecessary transcript duplication; store no vectors.

## 11. Conversation chunks

A chunk contains text from one speaker so role filters apply to the actual matching text. Split long messages into bounded chunks and preserve the speaker across incremental restarts. Original conversation preview can include both speakers around a match.

```rust
struct SearchChunk {
    session_id: SessionId,
    timestamp: Option<DateTime<Utc>>,
    source_start: u64,
    source_end: u64,
    text: String,
}
```

Source byte ranges enable larger previews from originals.

## 12. Incremental indexing

Persist the last successful offset and read only appended bytes. Processing should follow new data volume rather than total historical data.

## 13. Transactional offsets

Advance only after complete JSONL records are processed and committed. An incomplete final record stays before the committed offset and is retried when complete. Never permanently skip it.

## 14. File mutation handling

Track path/inode/size/mtime/offset. New → start at zero; appended → continue from offset; truncated/replaced → rebuild affected session index; unchanged → no work. Full-history rebuilds should be exceptional.

## 15. Index lifecycle

No permanent daemon. On activation discover changes and incrementally update; the existing index remains immediately usable. Consider a watcher later only if activation measurements justify it.

## 16. Initial indexing

One-time discovery and indexing of both agents. Show per-agent file progress and total indexed conversation count. Search during initial indexing is desirable if implementation complexity is reasonable.

## 17. Agent adapters

```rust
trait AgentAdapter {
    fn discover(&self) -> Result<Vec<SessionFile>>;
    fn parse_record(&self, record: &[u8]) -> Result<Option<NormalizedEvent>>;
    fn resume(&self, session: &Session, workspace: &Workspace) -> Result<()>;
}
```

ClaudeAdapter and CodexAdapter isolate format and CLI differences. This is an architectural sketch; core must remain independent of Herdr.

## 18. Git context

Resolve session cwd to repository, worktree, branch, and HEAD commit when available during indexing. Persist metadata even after worktree deletion. Example: Claude / capital-web / feature-prices / feature/prices / a3f28cd. Index-time observations must not be misrepresented as proven historical state.

## 19. Resume

Enter means resume this development session. Existing workspace → focus workspace and focus/resume agent. Closed workspace with existing cwd → create workspace and resume native agent session. No additional decisions in normal cases.

## 20. Deleted worktree

Offer “Recreate worktree and resume”, “Resume in repository”, “View conversation”, and “Cancel”. Recreation needs explicit runtime confirmation because it changes Git/filesystem state. Use repository/branch/commit metadata where sufficient. If reconstruction is impossible, retain search and preview where the original transcript is available.

## 21. Preview

Space opens original surrounding user/assistant conversation with repository/branch context. Enter resumes, Esc returns. Read additional original JSONL context as needed; SQLite need not contain the full transcript.

## 22. Herdr integration

Independent core (indexing, SQLite, search, agents) → Herdr adapter → overlay. Host integration opens UI, discovers/focuses active workspaces, creates workspaces for existing worktrees, and starts/focuses agent processes. Core is testable through a CLI.

## 23. CLI

Planned debugging interface:

```sh
agent-history search "portfolio visibility"
agent-history index
agent-history status
```

Search prints agent, repository/branch, date, and snippet. Herdr remains the primary UX.

## 24. Performance targets

After initial indexing: typical search p50 < 30 ms, p95 < 100 ms; normal small incremental updates < 100 ms. No CPU/RAM use when Agent History is not running. Aim for database size below raw history, without treating it as an invariant; FTS overhead depends on content. Measure on representative local histories.

## 25. Privacy and security

Local files → local Rust process → local SQLite → local Herdr UI. No network requests, transcript telemetry, cloud index, embeddings API, or LLM processing. Sensitive index data requires user-only filesystem permissions.

## 26. V1 implementation

Claude parser; Codex parser; normalized chunks; SQLite metadata; FTS5; byte-offset incremental indexing; Git context; Herdr overlay; source preview; session resume; workspace restoration.

## 27. Deferred optimizations

Do not initially build a daemon, vector database, embeddings, summaries, classification, knowledge graph, code index, custom search algorithm, or distributed storage. First measure FTS5 against real local history.

## 28. Future hybrid semantic search

Optional FTS + vectors + reranking could address remembered concepts without matching wording. Added storage/computation keeps this outside V1; FTS remains useful.

## 29. Success criterion

A developer finds a discussion from weeks ago without remembering agent or worktree, previews it, and continues the original native agent session in Herdr in seconds. An existing worktree requires no extra decisions.

## 30. Core architectural principle

Separate SEARCH (disposable SQLite + FTS5), SESSION (native history), and RESTORE (Herdr + Git). Agents own persistence; Herdr owns workspaces; Agent History connects them. The database must be deletable and rebuildable without losing native session data.

The supplied final sentence ends after “without losing a single coding”.

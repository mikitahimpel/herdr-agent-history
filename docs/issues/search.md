## Outcome

Implement ranked FTS5 search with snippets.

## RFC coverage

[Herdr Agent History RFC](https://github.com/mikitahimpel/herdr-agent-history/blob/main/docs/RFC.md), sections 6, 7, 24.

## Scope

- Implement normal term queries, explicit phrase/prefix syntax, BM25 ordering, snippet generation, limits, and stable tie-breaking.
- Join results to session/agent/Git context and exact source references; define repeated matches within one session.
- Bound input and handle invalid FTS syntax as a user-facing result rather than a crash.
- Filter matches by user messages, assistant replies, or both; the matched text must belong to the selected speaker, not an adjacent turn.

## Acceptance criteria

- [ ] Tests cover punctuation, quotes, prefixes, Unicode, empty queries, ranking, and deterministic limits.
- [ ] Search performs no JSONL scans and works with no Herdr workspaces open.
- [ ] Queries are parameterized; snippets exclude protocol metadata and unsafe terminal controls.

## Dependencies

- storage
- chunks

## Required validation

Run `./scripts/check` (formatting, Clippy, workspace tests, release build). Add isolated behavior tests appropriate to this change and record acceptance evidence. Never commit native transcripts or private index data.

<!-- agent-history-issue:search -->

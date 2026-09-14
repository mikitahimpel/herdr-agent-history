# Supported ingestion fixture layouts

The repository uses synthetic JSONL fixtures in parser unit tests. They contain no
captured home-directory history, credentials, or hidden reasoning.

Claude discovery accepts `.jsonl` files below the configured Claude history root
(the native default is `~/.claude`). Records with `type: user` or `type: assistant`
read text from `message.content`; text blocks are searchable and tool-result text
is bounded. Metadata and tool-use arguments are discarded.

Codex discovery accepts `.jsonl` files below the configured sessions root (the
native default is `~/.codex/sessions`). `response_item` message records are read
from their payload. `event_msg` mirrors are ignored to prevent streamed or
transport duplicates. Function output may contribute bounded human-readable
output; function arguments and protocol fields never enter searchable text.

Unknown event types, missing optional fields, and incomplete tails are skipped by
the adapter caller. A malformed complete JSON record returns a deterministic
`InvalidRecord` error for that record while unrelated files remain discoverable.

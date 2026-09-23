//! Native JSONL readers. Discovery is root-configurable so tests never inspect home history.
pub mod claude;
pub mod codex;
mod common;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub(crate) use common::repository_label;

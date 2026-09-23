//! Search and indexing core, independent of Herdr.
pub mod adapters;
pub mod chunks;
pub mod contracts;
pub mod domain;
pub mod git;
pub mod index;
pub mod preview;
pub mod storage;
pub mod test_support;
pub use contracts::{
    AgentAdapter, CoreError, GitContextProvider, IndexBatch, IndexStore, Result, SessionResumer,
    Store,
};
pub use domain::*;
pub use git::{git_command, GitContextResolver, INHERITED_GIT_ENVIRONMENT};
pub use storage::{
    default_index_path, IndexStatus, IndexedFileState, SqliteStore, DEFAULT_INDEX_RELATIVE_PATH,
};

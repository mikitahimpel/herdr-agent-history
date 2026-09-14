//! Search and indexing core, independent of Herdr.
pub mod contracts;
pub mod domain;
pub mod test_support;
pub use contracts::{
    AgentAdapter, CoreError, GitContextProvider, IndexBatch, Result, SessionResumer, Store,
};
pub use domain::*;

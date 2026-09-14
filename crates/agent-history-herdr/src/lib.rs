//! Small host boundary; Herdr integration owns workspace and process behavior.
use agent_history_core::{CoreError, Result, Session};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    pub cwd: PathBuf,
}

/// Host operations are intentionally narrow so core remains independent of Herdr.
pub trait HostAdapter {
    fn workspaces(&self) -> Result<Vec<Workspace>>;
    fn focus_workspace(&self, workspace: &Workspace) -> Result<()>;
    fn open_workspace(&self, cwd: &Path) -> Result<Workspace>;
    fn start_session(&self, session: &Session, workspace: &Workspace) -> Result<()>;
}

pub fn workspace_required(cwd: Option<&Path>) -> Result<&Path> {
    cwd.ok_or_else(|| CoreError::Unsupported("session has no workspace path".into()))
}

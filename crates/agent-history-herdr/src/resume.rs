use agent_history_core::{Agent, SessionId};
pub use agent_history_restore::plan::NativeResumePlan;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub cwd: std::path::PathBuf,
    #[serde(default)]
    pub root_pane_id: Option<String>,
    #[serde(default)]
    pub root_pane_occupied: bool,
    /// Created by this resume, so its root pane is a new shell at the
    /// session's directory that no one else is using.
    #[serde(default)]
    pub created: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveAgent {
    pub id: String,
    pub workspace_id: String,
    pub agent: Agent,
    pub session_id: Option<SessionId>,
    /// Host-reported agent name, used only as provenance for a pane this
    /// integration started itself.
    pub name: Option<String>,
    /// The directory the agent was started in, which identifies a pane when
    /// the host's session ID has gone stale.
    pub cwd: Option<std::path::PathBuf>,
}

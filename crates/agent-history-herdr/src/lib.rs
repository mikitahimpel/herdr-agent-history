//! Small host boundary; Herdr integration owns workspace and process behavior.
use agent_history_core::{CoreError, Result, Session};
use std::path::Path;

pub mod restore;
pub mod resume;
pub mod socket;

/// A narrow, mockable host surface used by the restoration coordinator. The
/// implementation may be backed by Herdr's CLI or socket API.
pub trait HostRuntime {
    fn workspaces(&mut self) -> Result<Vec<resume::WorkspaceRecord>>;
    fn agents(&mut self) -> Result<Vec<resume::LiveAgent>>;
    fn focus_workspace(&mut self, workspace_id: &str) -> Result<()>;
    fn focus_agent(&mut self, agent_id: &str) -> Result<()>;
    fn open_workspace(&mut self, cwd: &Path) -> Result<resume::WorkspaceRecord>;
    fn start_agent(
        &mut self,
        workspace: &resume::WorkspaceRecord,
        session: &Session,
        plan: &resume::NativeResumePlan,
    ) -> Result<()>;
}

/// Finds the exact native session before starting anything. In particular, a
/// live agent with another session ID is never treated as a match.
pub fn resume_in_host<H: HostRuntime>(host: &mut H, session: &Session) -> Result<()> {
    let plan = resume::NativeResumePlan::for_session(session)?;
    let cwd = workspace_required(session.cwd.as_deref())?;
    let workspaces = host.workspaces()?;
    let workspace = if let Some(workspace) = workspaces.iter().find(|w| w.cwd == cwd) {
        host.focus_workspace(&workspace.id)?;
        workspace.clone()
    } else {
        host.open_workspace(cwd)?
    };

    let agents = host.agents()?;
    let matches = agents.iter().filter(|agent| {
        agent.workspace_id == workspace.id
            && agent.agent == session.id.agent
            && agent.session_id.as_ref() == Some(&session.id)
    });
    let mut matching = matches.collect::<Vec<_>>();
    if matching.len() > 1 {
        return Err(CoreError::Unsupported(
            "multiple live agents match the native session".into(),
        ));
    }
    if let Some(agent) = matching.pop() {
        host.focus_agent(&agent.id)?;
        return Ok(());
    }
    host.start_agent(&workspace, session, &plan)
}

pub fn workspace_required(cwd: Option<&Path>) -> Result<&Path> {
    cwd.ok_or_else(|| CoreError::Unsupported("session has no workspace path".into()))
}

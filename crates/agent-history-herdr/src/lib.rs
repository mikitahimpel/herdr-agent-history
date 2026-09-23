//! Small host boundary; Herdr integration owns workspace and process behavior.
use agent_history_core::{CoreError, Result, Session};
use std::path::Path;

pub mod integration;
pub mod restore;
pub mod resume;
pub mod socket;
pub mod theme;

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
    resume_in_host_with_checker(host, session, |path| path.exists())
}

pub fn resume_in_host_with_checker<H, F>(host: &mut H, session: &Session, exists: F) -> Result<()>
where
    H: HostRuntime,
    F: Fn(&Path) -> bool,
{
    let plan = resume::NativeResumePlan::for_session(session)?;
    let cwd = workspace_required(session.cwd.as_deref())?;
    if !exists(cwd) {
        return Err(CoreError::Unsupported(
            "session workspace path does not exist; refusing to create a workspace".into(),
        ));
    }
    let workspaces = host.workspaces()?;
    let workspace = if let Some(workspace) = workspaces.iter().find(|w| w.cwd == cwd) {
        host.focus_workspace(&workspace.id)?;
        workspace.clone()
    } else {
        host.open_workspace(cwd)?
    };

    let agents = host.agents()?;
    let mut matching = matching_agents(session, &plan, &workspace.id, &agents);
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

/// Live agents in `workspace_id` running exactly this native session.
pub fn matching_agents<'a>(
    session: &Session,
    plan: &resume::NativeResumePlan,
    workspace_id: &str,
    agents: &'a [resume::LiveAgent],
) -> Vec<&'a resume::LiveAgent> {
    agents
        .iter()
        .filter(|agent| {
            agent.workspace_id == workspace_id
                && agent.agent == session.id.agent
                && match &agent.session_id {
                    Some(reported) => reported == &session.id,
                    // Herdr's Codex integration reports a native session ID only
                    // when Codex creates a session, not when it resumes one. A pane
                    // this integration started carries the session ID in its agent
                    // name, so that name identifies the exact session instead.
                    None => agent.name.as_deref() == Some(plan.agent_name.as_str()),
                }
        })
        .collect()
}

/// Sessions for which Enter would focus a running agent instead of starting
/// one: exactly one matching agent in the workspace open at the session's
/// recorded working directory, the same rule `resume_in_host` applies.
pub fn live_sessions(
    sessions: &[Session],
    workspaces: &[resume::WorkspaceRecord],
    agents: &[resume::LiveAgent],
) -> Vec<agent_history_core::SessionId> {
    sessions
        .iter()
        .filter(|session| {
            let (Ok(plan), Some(cwd)) = (
                resume::NativeResumePlan::for_session(session),
                session.cwd.as_deref(),
            ) else {
                return false;
            };
            workspaces
                .iter()
                .find(|w| w.cwd == cwd)
                .is_some_and(|w| matching_agents(session, &plan, &w.id, agents).len() == 1)
        })
        .map(|session| session.id.clone())
        .collect()
}

pub fn workspace_required(cwd: Option<&Path>) -> Result<&Path> {
    cwd.ok_or_else(|| CoreError::Unsupported("session has no workspace path".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::{Agent, SessionId, SourceRef};
    use resume::{LiveAgent, WorkspaceRecord};

    fn session(n: u8, cwd: &str) -> Session {
        Session {
            id: SessionId {
                agent: Agent::Codex,
                native_id: format!("00000000-0000-4000-8000-0000000000{n:02}"),
            },
            source: SourceRef::new("/h/s.jsonl", 1, 1, 0..1).unwrap(),
            cwd: Some(cwd.into()),
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            started_at: None,
            ended_at: None,
            git_observed_at: None,
        }
    }

    fn agent(
        id: &str,
        workspace: &str,
        session: Option<&Session>,
        name: Option<String>,
    ) -> LiveAgent {
        LiveAgent {
            id: id.into(),
            workspace_id: workspace.into(),
            agent: Agent::Codex,
            session_id: session.map(|s| s.id.clone()),
            name,
        }
    }

    #[test]
    fn live_sessions_follow_the_resume_matching_rule() {
        let reported = session(1, "/w/a");
        let named = session(2, "/w/b");
        let elsewhere = session(3, "/w/c");
        let ambiguous = session(4, "/w/a");
        let closed = session(5, "/w/a");
        let workspaces = [
            WorkspaceRecord {
                id: "wa".into(),
                cwd: "/w/a".into(),
                root_pane_id: None,
                root_pane_occupied: false,
            },
            WorkspaceRecord {
                id: "wb".into(),
                cwd: "/w/b".into(),
                root_pane_id: None,
                root_pane_occupied: false,
            },
        ];
        let plan = |s: &Session| resume::NativeResumePlan::for_session(s).unwrap().agent_name;
        let agents = [
            agent("1", "wa", Some(&reported), None),
            // Resumed Codex panes report no session ID, only our agent name.
            agent("2", "wb", None, Some(plan(&named))),
            // Running, but not in the workspace for the session's cwd.
            agent("3", "wb", Some(&elsewhere), None),
            agent("4a", "wa", Some(&ambiguous), None),
            agent("4b", "wa", Some(&ambiguous), None),
        ];
        let live = live_sessions(
            &[
                reported.clone(),
                named.clone(),
                elsewhere,
                ambiguous,
                closed,
            ],
            &workspaces,
            &agents,
        );
        assert_eq!(live, [reported.id, named.id]);
    }
}

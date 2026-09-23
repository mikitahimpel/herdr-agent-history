use agent_history_core::{Agent, CoreError, Result, Session, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResumePlan {
    pub agent: Agent,
    /// Host agent name this integration assigns to the resumed pane. It carries
    /// the validated native session ID, so a later overlay run can recognize
    /// its own resume even when the host reports no session ID for that pane.
    pub agent_name: String,
    pub argv: Vec<String>,
}

impl NativeResumePlan {
    pub fn for_session(session: &Session) -> Result<Self> {
        let id = &session.id.native_id;
        if !is_uuid(id) {
            return Err(CoreError::Unsupported(
                "native session ID is missing or invalid; refusing to start a new session".into(),
            ));
        }
        let argv = match session.id.agent {
            Agent::Claude => vec!["claude".into(), "--resume".into(), id.clone()],
            Agent::Codex => vec!["codex".into(), "resume".into(), id.clone()],
        };
        Ok(Self {
            agent: session.id.agent,
            agent_name: format!("agent-history-{id}"),
            argv,
        })
    }
}

fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].into_iter().all(|i| bytes[i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| [8, 13, 18, 23].contains(&i) || b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub cwd: std::path::PathBuf,
    #[serde(default)]
    pub root_pane_id: Option<String>,
    #[serde(default)]
    pub root_pane_occupied: bool,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::SourceRef;
    use std::path::PathBuf;

    fn session(agent: Agent, id: &str) -> Session {
        Session {
            id: SessionId::new(agent, id),
            source: SourceRef::new("history.jsonl", 1, 0, 0..0).unwrap(),
            cwd: Some(PathBuf::from("/tmp/project")),
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

    #[test]
    fn plans_only_verified_native_commands() {
        assert_eq!(
            NativeResumePlan::for_session(&session(
                Agent::Claude,
                "00000000-0000-4000-8000-000000000001"
            ))
            .unwrap()
            .argv,
            ["claude", "--resume", "00000000-0000-4000-8000-000000000001"]
        );
        assert_eq!(
            NativeResumePlan::for_session(&session(
                Agent::Codex,
                "00000000-0000-4000-8000-000000000002"
            ))
            .unwrap()
            .argv,
            ["codex", "resume", "00000000-0000-4000-8000-000000000002"]
        );
    }

    #[test]
    fn rejects_control_characters_without_fallback() {
        assert!(NativeResumePlan::for_session(&session(Agent::Codex, "bad\n-id")).is_err());
    }
}

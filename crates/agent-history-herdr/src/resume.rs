use agent_history_core::{Agent, CoreError, Result, Session, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResumePlan {
    pub agent: Agent,
    pub argv: Vec<String>,
}

impl NativeResumePlan {
    pub fn for_session(session: &Session) -> Result<Self> {
        let id = &session.id.native_id;
        if id.is_empty() || id.chars().any(char::is_control) {
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
            argv,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub cwd: std::path::PathBuf,
    #[serde(default)]
    pub root_pane_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveAgent {
    pub id: String,
    pub workspace_id: String,
    pub agent: Agent,
    pub session_id: Option<SessionId>,
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
            NativeResumePlan::for_session(&session(Agent::Claude, "c1"))
                .unwrap()
                .argv,
            ["claude", "--resume", "c1"]
        );
        assert_eq!(
            NativeResumePlan::for_session(&session(Agent::Codex, "x1"))
                .unwrap()
                .argv,
            ["codex", "resume", "x1"]
        );
    }

    #[test]
    fn rejects_control_characters_without_fallback() {
        assert!(NativeResumePlan::for_session(&session(Agent::Codex, "bad\n-id")).is_err());
    }
}

use agent_history_core::{Agent, CoreError, Result, Session, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResumePlan {
    pub agent: Agent,
    /// Host agent name this integration assigns to the resumed pane. It
    /// encodes the whole validated native session ID, so a later overlay run
    /// can recognize its own resume while the host reports no session ID for
    /// that pane, as it does for the first seconds after `claude --resume`.
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
            agent_name: resume_name(id),
            argv,
        })
    }
}

/// Herdr 0.9 accepts agent names of at most 32 characters from `[a-z0-9_-]`,
/// starting with a letter, which a hyphenated UUID does not fit. The 128-bit
/// value in fixed-width base 36 (25 digits) does, so distinct sessions never
/// share a name.
fn resume_name(uuid: &str) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    let mut value = u128::from_str_radix(&hex, 16).expect("validated UUID is 32 hex digits");
    let mut encoded = [b'0'; 25];
    for digit in encoded.iter_mut().rev() {
        *digit = DIGITS[(value % 36) as usize];
        value /= 36;
    }
    format!(
        "ah-{}",
        std::str::from_utf8(&encoded).expect("base-36 digits are ASCII")
    )
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
    fn resume_names_fit_herdr_and_identify_exactly_one_session() {
        let name = |id: &str| {
            NativeResumePlan::for_session(&session(Agent::Codex, id))
                .unwrap()
                .agent_name
        };
        let ids = [
            "00000000-0000-0000-0000-000000000000",
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
            "10000000-0000-4000-8000-000000000001",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            "8f0f2724-2477-43d9-a05e-c8534d8d2beb",
        ];
        let names: Vec<String> = ids.iter().map(|id| name(id)).collect();
        for n in &names {
            // Herdr 0.9.3 rejects anything else with `invalid_agent_name`.
            assert!(n.len() <= 32, "{n}");
            assert!(n.starts_with(|c: char| c.is_ascii_lowercase()), "{n}");
            assert!(n
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'));
        }
        let distinct: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(distinct.len(), ids.len());
        assert_eq!(names[0], "ah-0000000000000000000000000");
        assert_eq!(names[4], "ah-f5lxx1zz5pnorynqglhzmsp33");
        assert_eq!(name("8F0F2724-2477-43D9-A05E-C8534D8D2BEB"), names[5]);
    }

    #[test]
    fn rejects_control_characters_without_fallback() {
        assert!(NativeResumePlan::for_session(&session(Agent::Codex, "bad\n-id")).is_err());
    }
}

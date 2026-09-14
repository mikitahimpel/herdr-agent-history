//! Public Herdr CLI command boundary. Commands are argv vectors so IDs and
//! paths are never interpolated into shell source.

use crate::resume::{NativeResumePlan, WorkspaceRecord};
use agent_history_core::Session;
use agent_history_core::{CoreError, Result};
use std::path::Path;

pub trait CommandRunner {
    fn run(&mut self, argv: &[String]) -> Result<String>;
}

pub struct HerdrCli<R> {
    runner: R,
}
impl<R> HerdrCli<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
    pub fn into_inner(self) -> R {
        self.runner
    }
}

impl<R: CommandRunner> HerdrCli<R> {
    pub fn list_workspaces(&mut self) -> Result<String> {
        self.runner.run(&words(["herdr", "workspace", "list"]))
    }
    pub fn focus_workspace(&mut self, id: &str) -> Result<String> {
        self.runner.run(&words(["herdr", "workspace", "focus", id]))
    }
    pub fn create_workspace(&mut self, cwd: &Path) -> Result<String> {
        self.runner.run(&[
            "herdr".into(),
            "workspace".into(),
            "create".into(),
            "--cwd".into(),
            cwd.display().to_string(),
            "--focus".into(),
        ])
    }
    pub fn focus_agent(&mut self, id: &str) -> Result<String> {
        self.runner.run(&words(["herdr", "agent", "focus", id]))
    }
    pub fn start_resume(
        &mut self,
        workspace: &WorkspaceRecord,
        session: &Session,
    ) -> Result<String> {
        let plan = NativeResumePlan::for_session(session)?;
        let pane_id = workspace.root_pane_id.as_ref().ok_or_else(|| {
            CoreError::Unsupported("Herdr workspace response has no root pane".into())
        })?;
        let mut argv = vec![
            "herdr".into(),
            "agent".into(),
            "start".into(),
            "agent-history-resume".into(),
            "--kind".into(),
            match plan.agent {
                agent_history_core::Agent::Claude => "claude",
                agent_history_core::Agent::Codex => "codex",
            }
            .into(),
            "--workspace".into(),
            workspace.id.clone(),
            "--pane".into(),
            pane_id.clone(),
            "--".into(),
        ];
        argv.extend(plan.argv);
        self.runner.run(&argv)
    }
}

fn words<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.into_iter().map(str::to_owned).collect()
}

pub fn parse_workspace_list(json: &str) -> Result<Vec<WorkspaceRecord>> {
    serde_json::from_str(json)
        .map_err(|e| CoreError::Unsupported(format!("invalid Herdr workspace response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::{Agent, SessionId, SourceRef};

    struct Recorder(Vec<Vec<String>>);
    impl CommandRunner for Recorder {
        fn run(&mut self, argv: &[String]) -> Result<String> {
            self.0.push(argv.to_vec());
            Ok("{}".into())
        }
    }

    fn session(agent: Agent, id: &str) -> Session {
        Session {
            id: SessionId::new(agent, id),
            source: SourceRef::new("history.jsonl", 1, 0, 0..0).unwrap(),
            cwd: Some("/tmp/project".into()),
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
    fn resume_uses_pane_and_native_argv_as_separate_arguments() {
        let mut cli = HerdrCli::new(Recorder(Vec::new()));
        cli.start_resume(
            &WorkspaceRecord {
                id: "w1".into(),
                cwd: "/tmp/project".into(),
                root_pane_id: Some("w1:p1".into()),
            },
            &session(Agent::Codex, "id with spaces"),
        )
        .unwrap();
        let commands = cli.into_inner().0;
        assert_eq!(
            commands[0],
            vec![
                "herdr",
                "agent",
                "start",
                "agent-history-resume",
                "--kind",
                "codex",
                "--workspace",
                "w1",
                "--pane",
                "w1:p1",
                "--",
                "codex",
                "resume",
                "id with spaces"
            ]
        );
    }
}

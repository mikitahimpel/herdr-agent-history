//! Public Herdr CLI command boundary. Commands are argv vectors so IDs and
//! paths are never interpolated into shell source.

use crate::resume::{NativeResumePlan, WorkspaceRecord};
use agent_history_core::{Agent, CoreError, Result, Session, SessionId};
use serde::Deserialize;
use std::path::Path;

pub trait CommandRunner {
    fn run(&mut self, argv: &[String]) -> Result<String>;
}

#[derive(Default)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, argv: &[String]) -> Result<String> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| CoreError::Unsupported("empty Herdr command".into()))?;
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(CoreError::Io)?;
        if !output.status.success() {
            return Err(CoreError::Unsupported(format!(
                "Herdr command `{program}` failed with {}",
                output.status
            )));
        }
        String::from_utf8(output.stdout)
            .map_err(|e| CoreError::Unsupported(format!("Herdr returned non-UTF-8 output: {e}")))
    }
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

    pub fn workspace_records(&mut self) -> Result<Vec<WorkspaceRecord>> {
        let mut records = parse_workspace_list(&self.list_workspaces()?)?;
        for record in &mut records {
            let panes = parse_pane_list(&self.list_panes(&record.id)?)?;
            if let Some(pane) = panes.first() {
                record.cwd = pane.cwd.clone().unwrap_or_default().into();
                record.root_pane_id = Some(pane.pane_id.clone());
                record.root_pane_occupied = pane.agent.is_some();
            }
        }
        Ok(records)
    }

    pub fn list_panes(&mut self, workspace_id: &str) -> Result<String> {
        self.runner.run(&[
            "herdr".into(),
            "pane".into(),
            "list".into(),
            "--workspace".into(),
            workspace_id.into(),
        ])
    }

    pub fn live_agents(&mut self) -> Result<Vec<crate::resume::LiveAgent>> {
        parse_agent_list(&self.list_agents()?)
    }

    fn list_agents(&mut self) -> Result<String> {
        self.runner.run(&words(["herdr", "agent", "list"]))
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
        if workspace.root_pane_occupied {
            return Err(CoreError::Unsupported(
                "Herdr workspace pane is occupied; refusing to replace a live agent".into(),
            ));
        }
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
    let response: Envelope<WorkspaceListResult> = parse_envelope(json)?;
    Ok(response
        .result
        .workspaces
        .into_iter()
        .map(|w| WorkspaceRecord {
            id: w.workspace_id,
            cwd: std::path::PathBuf::new(),
            root_pane_id: None,
            root_pane_occupied: false,
        })
        .collect())
}

#[derive(Deserialize)]
struct Envelope<T> {
    result: T,
}

#[derive(Deserialize)]
struct WorkspaceListResult {
    workspaces: Vec<WorkspaceWire>,
}

#[derive(Deserialize)]
struct WorkspaceWire {
    workspace_id: String,
}

#[derive(Deserialize)]
struct PaneListResult {
    panes: Vec<PaneWire>,
}
#[derive(Deserialize)]
pub struct PaneWire {
    pane_id: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub fn parse_pane_list(json: &str) -> Result<Vec<PaneWire>> {
    Ok(parse_envelope::<PaneListResult>(json)?.result.panes)
}

#[derive(Deserialize)]
struct AgentListResult {
    agents: Vec<AgentWire>,
}
#[derive(Deserialize)]
struct AgentWire {
    workspace_id: String,
    pane_id: String,
    agent: Option<String>,
    #[serde(default)]
    agent_session: Option<AgentSessionWire>,
}
#[derive(Deserialize)]
struct AgentSessionWire {
    value: String,
}

pub fn parse_agent_list(json: &str) -> Result<Vec<crate::resume::LiveAgent>> {
    parse_envelope::<AgentListResult>(json)?
        .result
        .agents
        .into_iter()
        .map(|a| {
            let agent = match a.agent.as_deref() {
                Some("claude") => Agent::Claude,
                Some("codex") => Agent::Codex,
                _ => return Err(CoreError::Unsupported("unknown live Herdr agent".into())),
            };
            let session_id = a.agent_session.map(|s| SessionId::new(agent, s.value));
            Ok(crate::resume::LiveAgent {
                id: a.pane_id,
                workspace_id: a.workspace_id,
                agent,
                session_id,
            })
        })
        .collect()
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(json: &str) -> Result<Envelope<T>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| CoreError::Unsupported(format!("invalid Herdr response: {e}")))?;
    if let Some(error) = value.get("error") {
        return Err(CoreError::Unsupported(format!("Herdr API error: {error}")));
    }
    serde_json::from_value(value)
        .map_err(|e| CoreError::Unsupported(format!("invalid Herdr response envelope: {e}")))
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
                root_pane_occupied: false,
            },
            &session(Agent::Codex, "00000000-0000-4000-8000-000000000003"),
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
                "00000000-0000-4000-8000-000000000003"
            ]
        );
    }
}

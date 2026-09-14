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
        let executable = if program == "herdr" {
            std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| program.into())
        } else {
            program.into()
        };
        let output = std::process::Command::new(executable)
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

    pub fn list_agents(&mut self) -> Result<String> {
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
        // Herdr v0.7.1 starts a new split for --workspace; it never replaces
        // an occupied pane. Explicit --focus makes Enter activate that split.
        let name = format!("agent-history-{}", &session.id.native_id[..8]);
        let mut argv = vec![
            "herdr".into(),
            "agent".into(),
            "start".into(),
            name,
            "--workspace".into(),
            workspace.id.clone(),
            "--cwd".into(),
            workspace.cwd.display().to_string(),
            "--focus".into(),
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
    source: String,
    agent: String,
    kind: String,
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
                _ => return Ok(None),
            };
            let session_id = a.agent_session.and_then(|s| {
                (s.source == format!("herdr:{}", a.agent.as_deref().unwrap_or_default())
                    && s.agent == a.agent.as_deref().unwrap_or_default()
                    && s.kind == "id")
                    .then(|| SessionId::new(agent, s.value))
            });
            Ok(Some(crate::resume::LiveAgent {
                id: a.pane_id,
                workspace_id: a.workspace_id,
                agent,
                session_id,
            }))
        })
        .filter_map(|item| match item {
            Ok(Some(agent)) => Some(Ok(agent)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn parse_envelope<T: for<'de> Deserialize<'de>>(json: &str) -> Result<Envelope<T>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| CoreError::Unsupported(format!("invalid Herdr response: {e}")))?;
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(CoreError::Unsupported(format!("Herdr API error: {error}")));
    }
    serde_json::from_value(value)
        .map_err(|e| CoreError::Unsupported(format!("invalid Herdr response envelope: {e}")))
}

impl<R: CommandRunner> crate::HostRuntime for HerdrCli<R> {
    fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>> {
        self.workspace_records()
    }

    fn agents(&mut self) -> Result<Vec<crate::resume::LiveAgent>> {
        self.live_agents()
    }

    fn focus_workspace(&mut self, workspace_id: &str) -> Result<()> {
        self.focus_workspace(workspace_id).map(|_| ())
    }

    fn focus_agent(&mut self, agent_id: &str) -> Result<()> {
        self.focus_agent(agent_id).map(|_| ())
    }

    fn open_workspace(&mut self, cwd: &std::path::Path) -> Result<WorkspaceRecord> {
        let response: Envelope<WorkspaceCreatedResult> =
            parse_envelope(&self.create_workspace(cwd)?)?;
        let root = response.result.root_pane;
        Ok(WorkspaceRecord {
            id: response.result.workspace.workspace_id,
            cwd: root
                .cwd
                .map(Into::into)
                .unwrap_or_else(|| cwd.to_path_buf()),
            root_pane_id: Some(root.pane_id),
            root_pane_occupied: root.agent.is_some(),
        })
    }

    fn start_agent(
        &mut self,
        workspace: &WorkspaceRecord,
        session: &agent_history_core::Session,
        _plan: &crate::resume::NativeResumePlan,
    ) -> Result<()> {
        self.start_resume(workspace, session).map(|_| ())
    }
}

#[derive(Deserialize)]
struct WorkspaceCreatedResult {
    workspace: WorkspaceCreatedWorkspace,
    root_pane: PaneWire,
}
#[derive(Deserialize)]
struct WorkspaceCreatedWorkspace {
    workspace_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::{Agent, SessionId, SourceRef};
    use std::collections::VecDeque;

    struct Recorder(Vec<Vec<String>>);
    impl CommandRunner for Recorder {
        fn run(&mut self, argv: &[String]) -> Result<String> {
            self.0.push(argv.to_vec());
            Ok("{}".into())
        }
    }

    struct Queued {
        commands: Vec<Vec<String>>,
        responses: VecDeque<String>,
    }
    impl CommandRunner for Queued {
        fn run(&mut self, argv: &[String]) -> Result<String> {
            self.commands.push(argv.to_vec());
            self.responses
                .pop_front()
                .ok_or_else(|| CoreError::Unsupported("mock response queue exhausted".into()))
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
                "agent-history-00000000",
                "--workspace",
                "w1",
                "--cwd",
                "/tmp/project",
                "--focus",
                "--",
                "codex",
                "resume",
                "00000000-0000-4000-8000-000000000003"
            ]
        );
    }

    fn queued(responses: &[&str]) -> HerdrCli<Queued> {
        HerdrCli::new(Queued {
            commands: Vec::new(),
            responses: responses.iter().map(|s| (*s).into()).collect(),
        })
    }

    #[test]
    fn coordinator_closed_workspace_creates_then_starts_exact_resume() {
        let mut cli = queued(&[
            r#"{"result":{"workspaces":[]}}"#,
            r#"{"result":{"workspace":{"workspace_id":"w2"},"root_pane":{"pane_id":"w2:p1","cwd":"/tmp/project"}}}"#,
            r#"{"result":{"agents":[]}}"#,
            r#"{"result":{"agent":{}}}"#,
        ]);
        crate::resume_in_host_with_checker(
            &mut cli,
            &session(Agent::Codex, "00000000-0000-4000-8000-000000000004"),
            |_| true,
        )
        .unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 4);
        assert_eq!(
            commands[1][..5],
            ["herdr", "workspace", "create", "--cwd", "/tmp/project"]
        );
        assert_eq!(
            commands[3].last().unwrap(),
            "00000000-0000-4000-8000-000000000004"
        );
    }

    #[test]
    fn coordinator_matching_live_session_focuses_without_starting() {
        let mut cli = queued(&[
            r#"{"result":{"workspaces":[{"workspace_id":"w1"}]}}"#,
            r#"{"result":{"panes":[{"pane_id":"w1:p1","cwd":"/tmp/project","agent":"codex"}]}}"#,
            r#"{"result":{}}"#,
            r#"{"result":{"agents":[{"workspace_id":"w1","pane_id":"w1:p1","agent":"codex","agent_session":{"source":"herdr:codex","agent":"codex","kind":"id","value":"00000000-0000-4000-8000-000000000005"}}]}}"#,
            r#"{"result":{}}"#,
        ]);
        crate::resume_in_host_with_checker(
            &mut cli,
            &session(Agent::Codex, "00000000-0000-4000-8000-000000000005"),
            |_| true,
        )
        .unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands[2][..4], ["herdr", "workspace", "focus", "w1"]);
        assert_eq!(commands[4][..4], ["herdr", "agent", "focus", "w1:p1"]);
        assert!(!commands.iter().any(|c| c.contains(&"start".into())));
    }

    #[test]
    fn occupied_workspace_starts_focused_split_without_writing_existing_pane() {
        let mut cli = queued(&[
            r#"{"result":{"workspaces":[{"workspace_id":"w1"}]}}"#,
            r#"{"result":{"panes":[{"pane_id":"w1:p1","cwd":"/tmp/project","agent":"claude"}]}}"#,
            r#"{"result":{}}"#,
            r#"{"result":{"agents":[{"workspace_id":"w1","pane_id":"w1:p1","agent":"claude","agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"00000000-0000-4000-8000-000000000006"}}]}}"#,
            r#"{"result":{"agent":{}}}"#,
        ]);
        crate::resume_in_host_with_checker(
            &mut cli,
            &session(Agent::Codex, "00000000-0000-4000-8000-000000000007"),
            |_| true,
        )
        .unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 5);
        assert_eq!(
            commands[4],
            [
                "herdr",
                "agent",
                "start",
                "agent-history-00000000",
                "--workspace",
                "w1",
                "--cwd",
                "/tmp/project",
                "--focus",
                "--",
                "codex",
                "resume",
                "00000000-0000-4000-8000-000000000007"
            ]
        );
        assert!(!commands.iter().any(|c| c
            .iter()
            .any(|arg| matches!(arg.as_str(), "run" | "send" | "--pane"))));
    }

    #[test]
    fn malformed_workspace_envelope_maps_to_explicit_error() {
        let mut cli = queued(&[r#"{"result":{"unexpected":[]}}"#]);
        let error = crate::HostRuntime::workspaces(&mut cli).unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid Herdr response envelope"));
    }

    #[test]
    fn agent_parser_skips_unrelated_agents_and_rejects_mismatched_provenance() {
        let json = r#"{"error":null,"result":{"agents":[
            {"workspace_id":"w1","pane_id":"w1:p1","agent":"pi"},
            {"workspace_id":"w1","pane_id":"w1:p2","agent":"claude","agent_session":{"source":"other","agent":"claude","kind":"id","value":"00000000-0000-4000-8000-000000000008"}},
            {"workspace_id":"w1","pane_id":"w1:p3","agent":"claude","agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"00000000-0000-4000-8000-000000000009"}}
        ]}}"#;
        let agents = parse_agent_list(json).unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents[0].session_id.is_none());
        assert_eq!(
            agents[1].session_id.as_ref().unwrap().native_id,
            "00000000-0000-4000-8000-000000000009"
        );
    }

    #[test]
    fn missing_cwd_refuses_before_workspace_listing() {
        let mut cli = queued(&[]);
        let error = crate::resume_in_host_with_checker(
            &mut cli,
            &session(Agent::Claude, "00000000-0000-4000-8000-000000000010"),
            |_| false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not exist"));
        assert!(cli.into_inner().commands.is_empty());
    }
}

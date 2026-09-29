//! Public Herdr CLI command boundary. Commands are argv vectors so IDs and
//! paths are never interpolated into shell source.

use crate::resume::{NativeResumePlan, WorkspaceRecord};
use agent_history_core::{Agent, CoreError, Result, Session, SessionId};
use serde::Deserialize;
use std::path::Path;

/// Oldest Herdr whose `agent start` launches into an existing pane with
/// `--kind` and `--pane`. Herdr 0.7 took `--workspace` instead and created the
/// split itself; that form is gone, and this adapter only builds the new one.
pub const MINIMUM_HERDR: (u64, u64, u64) = (0, 9, 3);

pub trait CommandRunner {
    fn run(&mut self, argv: &[String]) -> std::result::Result<String, RunError>;
}

#[derive(Debug)]
pub enum RunError {
    /// Herdr ran the command and refused it.
    Host(HostFailure),
    Other(CoreError),
}

/// A command Herdr rejected. Only the command name and the host's own message
/// are kept; arguments may contain session paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFailure {
    pub subcommand: String,
    pub status: String,
    /// Herdr's machine-readable error code, such as `agent_not_ready`.
    pub code: Option<String>,
    pub message: String,
}

impl From<HostFailure> for CoreError {
    fn from(failure: HostFailure) -> Self {
        CoreError::Unsupported(format!(
            "Herdr `herdr {}` failed with {}: {}",
            failure.subcommand, failure.status, failure.message
        ))
    }
}

impl From<RunError> for CoreError {
    fn from(error: RunError) -> Self {
        match error {
            RunError::Host(failure) => failure.into(),
            RunError::Other(error) => error,
        }
    }
}

#[derive(Default)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, argv: &[String]) -> std::result::Result<String, RunError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| RunError::Other(CoreError::Unsupported("empty Herdr command".into())))?;
        let executable = if program == "herdr" {
            std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| program.into())
        } else {
            program.into()
        };
        let output = std::process::Command::new(executable)
            .args(args)
            .output()
            .map_err(|e| RunError::Other(CoreError::Io(e)))?;
        if !output.status.success() {
            return Err(RunError::Host(failure_detail(
                args,
                &output.status.to_string(),
                &output.stdout,
                &output.stderr,
            )));
        }
        String::from_utf8(output.stdout).map_err(|e| {
            RunError::Other(CoreError::Unsupported(format!(
                "Herdr returned non-UTF-8 output: {e}"
            )))
        })
    }
}

pub struct HerdrCli<R> {
    runner: R,
    version_checked: bool,
}
impl<R> HerdrCli<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            version_checked: false,
        }
    }
    pub fn into_inner(self) -> R {
        self.runner
    }
}

impl<R: CommandRunner> HerdrCli<R> {
    /// Every host command goes through here, so an unsupported Herdr is
    /// reported before any command whose syntax depends on its version.
    fn run(&mut self, argv: &[String]) -> std::result::Result<String, RunError> {
        if !self.version_checked {
            self.ensure_supported_version().map_err(RunError::Other)?;
            self.version_checked = true;
        }
        self.runner.run(argv)
    }

    fn ensure_supported_version(&mut self) -> Result<()> {
        let output = self.runner.run(&words(["herdr", "--version"]))?;
        check_version(&output)
    }

    pub fn list_workspaces(&mut self) -> Result<String> {
        Ok(self.run(&words(["herdr", "workspace", "list"]))?)
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
        Ok(self.run(&words([
            "herdr",
            "pane",
            "list",
            "--workspace",
            workspace_id,
        ]))?)
    }

    pub fn live_agents(&mut self) -> Result<Vec<crate::resume::LiveAgent>> {
        parse_agent_list(&self.list_agents()?)
    }

    pub fn list_agents(&mut self) -> Result<String> {
        Ok(self.run(&words(["herdr", "agent", "list"]))?)
    }
    pub fn focus_workspace(&mut self, id: &str) -> Result<String> {
        Ok(self.run(&words(["herdr", "workspace", "focus", id]))?)
    }
    pub fn create_workspace(&mut self, cwd: &Path) -> Result<String> {
        Ok(self.run(&[
            "herdr".into(),
            "workspace".into(),
            "create".into(),
            "--cwd".into(),
            cwd.display().to_string(),
            "--focus".into(),
        ])?)
    }
    pub fn focus_agent(&mut self, id: &str) -> Result<String> {
        Ok(self.run(&words(["herdr", "agent", "focus", id]))?)
    }

    /// Opens a focused shell pane beside `pane_id`, never replacing it.
    fn split_pane(&mut self, pane_id: &str, cwd: &Path) -> Result<String> {
        let output = self.run(&[
            "herdr".into(),
            "pane".into(),
            "split".into(),
            pane_id.into(),
            "--direction".into(),
            "right".into(),
            "--cwd".into(),
            cwd.display().to_string(),
            "--focus".into(),
        ])?;
        Ok(parse_envelope::<PaneInfoResult>(&output)?
            .result
            .pane
            .pane_id)
    }

    /// Herdr 0.9 starts an agent only in an existing pane at a shell prompt.
    /// A workspace this resume just created supplies its new root pane; any
    /// other workspace gets a new split, so no pane in use is ever written to.
    pub fn start_resume(&mut self, workspace: &WorkspaceRecord, session: &Session) -> Result<()> {
        let plan = NativeResumePlan::for_session(session)?;
        let (kind, agent_args) = plan
            .argv
            .split_first()
            .ok_or_else(|| CoreError::Unsupported("empty native resume command".into()))?;
        let root = workspace.root_pane_id.as_deref().ok_or_else(|| {
            CoreError::Unsupported(format!(
                "Herdr reported no pane in workspace {}; cannot start the agent",
                workspace.id
            ))
        })?;
        let fresh_root = workspace.created && !workspace.root_pane_occupied;
        let pane = if fresh_root {
            root.to_owned()
        } else {
            self.split_pane(root, &workspace.cwd)?
        };
        let mut argv = vec![
            "herdr".into(),
            "agent".into(),
            "start".into(),
            plan.agent_name.clone(),
            "--kind".into(),
            kind.clone(),
            "--pane".into(),
            pane.clone(),
            "--".into(),
        ];
        argv.extend(agent_args.iter().cloned());
        match self.run(&argv) {
            Ok(output) => {
                let started = parse_envelope::<AgentStartedResult>(&output)?.result.argv;
                if started != plan.argv {
                    return Err(CoreError::Unsupported(format!(
                        "Herdr started `{}` instead of `{}`",
                        started.join(" "),
                        plan.argv.join(" ")
                    )));
                }
                Ok(())
            }
            Err(RunError::Host(failure)) => {
                Err(self.launch_failure(failure, &plan, &pane, !fresh_root))
            }
            Err(RunError::Other(error)) => Err(error),
        }
    }

    /// Maps what `agent start` reports into what the user should do. Herdr
    /// reports some failures after the agent is already running in the pane.
    fn launch_failure(
        &mut self,
        failure: HostFailure,
        plan: &NativeResumePlan,
        pane: &str,
        split_created: bool,
    ) -> CoreError {
        let agent = match plan.agent {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
        };
        match failure.code.as_deref() {
            // The agent was detected but stopped at a prompt of its own, such
            // as folder trust. Herdr keeps its name, so Enter focuses it.
            Some("agent_not_ready") => CoreError::Unsupported(format!(
                "{agent} started in pane {pane} but is waiting for input there ({}). Answer it in that pane.",
                failure.message
            )),
            // The process is running but Herdr dropped the name when it gave
            // up waiting. Restoring it keeps a second Enter from starting a
            // duplicate before the agent reports its session.
            Some("timeout") => {
                let renamed = self
                    .run(&words(["herdr", "agent", "rename", pane, &plan.agent_name]))
                    .is_ok();
                let hint = if renamed {
                    "Check that pane; Enter focuses it once it is running."
                } else {
                    "Check that pane before pressing Enter again."
                };
                CoreError::Unsupported(format!(
                    "{agent} was not ready in pane {pane} within Herdr's startup timeout. {hint}"
                ))
            }
            // Refused before anything ran: remove the empty split this resume
            // opened for it.
            Some("agent_name_taken" | "invalid_agent_name") if split_created => {
                let _ = self.run(&words(["herdr", "pane", "close", pane]));
                failure.into()
            }
            _ => failure.into(),
        }
    }
}

fn words<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.into_iter().map(str::to_owned).collect()
}

/// Parses `herdr --version` output such as `herdr 0.9.3`.
pub fn parse_version(output: &str) -> Option<(u64, u64, u64)> {
    let token = output
        .trim()
        .strip_prefix("herdr ")?
        .split_whitespace()
        .next()?;
    let release = token.split(['-', '+']).next()?;
    let mut parts = release.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn check_version(output: &str) -> Result<()> {
    let (major, minor, patch) = MINIMUM_HERDR;
    let minimum = format!("{major}.{minor}.{patch}");
    match parse_version(output) {
        Some(version) if version >= MINIMUM_HERDR => Ok(()),
        Some((a, b, c)) => Err(CoreError::Unsupported(format!(
            "Herdr {a}.{b}.{c} is not supported; resume needs Herdr {minimum} or newer. Run `herdr update`."
        ))),
        None => Err(CoreError::Unsupported(format!(
            "could not read the Herdr version from `herdr --version` ({:?}); resume needs Herdr {minimum} or newer",
            output.trim().chars().take(80).collect::<String>()
        ))),
    }
}

/// Host failures are otherwise indistinguishable from each other in the
/// overlay, which reports only an exit status. Only the command name and the
/// host's own message are surfaced; arguments may contain session paths.
fn failure_detail(args: &[String], status: &str, stdout: &[u8], stderr: &[u8]) -> HostFailure {
    let subcommand = args
        .iter()
        .take_while(|arg| !arg.starts_with('-') && *arg != "--")
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let (code, message) = match host_message(stdout) {
        Some(found) => found,
        None => host_message(stderr).unwrap_or((None, "no diagnostic output".into())),
    };
    HostFailure {
        subcommand,
        status: status.into(),
        code,
        message,
    }
}

fn host_message(bytes: &[u8]) -> Option<(Option<String>, String)> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let error = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let field = |name: &str| {
        error
            .as_ref()
            .and_then(|e| e.get(name)?.as_str().map(str::to_owned))
    };
    let message = field("message").unwrap_or_else(|| text.to_owned());
    Some((field("code"), message.chars().take(200).collect()))
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
            created: false,
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
    name: Option<String>,
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
                name: a.name,
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
            created: true,
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

#[derive(Deserialize)]
struct PaneInfoResult {
    pane: PaneWire,
}

#[derive(Deserialize)]
struct AgentStartedResult {
    argv: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::{Agent, SessionId, SourceRef};
    use std::collections::VecDeque;

    const VERSION: &str = "herdr 0.9.3\n";

    type Reply = std::result::Result<String, HostFailure>;

    struct Queued {
        commands: Vec<Vec<String>>,
        responses: VecDeque<Reply>,
    }
    impl CommandRunner for Queued {
        fn run(&mut self, argv: &[String]) -> std::result::Result<String, RunError> {
            self.commands.push(argv.to_vec());
            self.responses
                .pop_front()
                .ok_or_else(|| {
                    RunError::Other(CoreError::Unsupported(
                        "mock response queue exhausted".into(),
                    ))
                })?
                .map_err(RunError::Host)
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

    fn name(id: &str) -> String {
        NativeResumePlan::for_session(&session(Agent::Codex, id))
            .unwrap()
            .agent_name
    }

    fn ok(json: &str) -> Reply {
        Ok(json.into())
    }

    fn refused(code: &str, message: &str) -> Reply {
        Err(HostFailure {
            subcommand: "agent start".into(),
            status: "exit status: 1".into(),
            code: Some(code.into()),
            message: message.into(),
        })
    }

    /// Replies to `herdr --version` first, as every session with the host does.
    fn host(replies: Vec<Reply>) -> HerdrCli<Queued> {
        HerdrCli::new(Queued {
            commands: Vec::new(),
            responses: std::iter::once(ok(VERSION)).chain(replies).collect(),
        })
    }

    fn queued(responses: &[&str]) -> HerdrCli<Queued> {
        host(responses.iter().map(|s| ok(s)).collect())
    }

    fn started(argv: &[&str]) -> String {
        serde_json::json!({"result":{"type":"agent_started","argv":argv,"agent":{}}}).to_string()
    }

    fn workspace(created: bool, occupied: bool) -> WorkspaceRecord {
        WorkspaceRecord {
            id: "w1".into(),
            cwd: "/tmp/project".into(),
            root_pane_id: Some("w1:p1".into()),
            root_pane_occupied: occupied,
            created,
        }
    }

    const SPLIT: &str =
        r#"{"result":{"type":"pane_info","pane":{"pane_id":"w1:p2","cwd":"/tmp/project"}}}"#;

    #[test]
    fn resume_in_existing_workspace_splits_then_starts_kind_with_native_args() {
        let id = "00000000-0000-4000-8000-000000000003";
        let mut cli = host(vec![ok(SPLIT), ok(&started(&["codex", "resume", id]))]);
        cli.start_resume(&workspace(false, false), &session(Agent::Codex, id))
            .unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands[0], ["herdr", "--version"]);
        assert_eq!(
            commands[1],
            [
                "herdr",
                "pane",
                "split",
                "w1:p1",
                "--direction",
                "right",
                "--cwd",
                "/tmp/project",
                "--focus"
            ]
        );
        assert_eq!(
            commands[2],
            [
                "herdr",
                "agent",
                "start",
                name(id).as_str(),
                "--kind",
                "codex",
                "--pane",
                "w1:p2",
                "--",
                "resume",
                id
            ]
        );
        assert_eq!(commands.len(), 3);
    }

    #[test]
    fn resume_in_created_workspace_uses_its_new_root_pane() {
        let id = "00000000-0000-4000-8000-000000000014";
        let mut cli = host(vec![ok(&started(&["claude", "--resume", id]))]);
        cli.start_resume(&workspace(true, false), &session(Agent::Claude, id))
            .unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[1],
            [
                "herdr",
                "agent",
                "start",
                name(id).as_str(),
                "--kind",
                "claude",
                "--pane",
                "w1:p1",
                "--",
                "--resume",
                id
            ]
        );
    }

    #[test]
    fn host_reporting_a_different_launch_is_an_error() {
        let id = "00000000-0000-4000-8000-000000000015";
        let mut cli = host(vec![ok(&started(&["claude"]))]);
        let error = cli
            .start_resume(&workspace(true, false), &session(Agent::Claude, id))
            .unwrap_err();
        assert!(error.to_string().contains(&format!(
            "Herdr started `claude` instead of `claude --resume {id}`"
        )));
    }

    #[test]
    fn workspace_without_panes_refuses_before_any_launch() {
        let mut cli = queued(&[]);
        let mut record = workspace(false, false);
        record.root_pane_id = None;
        let error = cli
            .start_resume(
                &record,
                &session(Agent::Codex, "00000000-0000-4000-8000-000000000016"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("no pane in workspace w1"));
        assert_eq!(cli.into_inner().commands.len(), 0);
    }

    #[test]
    fn agent_blocked_at_startup_is_reported_as_waiting_in_its_pane() {
        let id = "00000000-0000-4000-8000-000000000017";
        let mut cli = host(vec![
            ok(SPLIT),
            refused(
                "agent_not_ready",
                "agent x is blocked during startup and is not ready for prompts",
            ),
        ]);
        let error = cli
            .start_resume(&workspace(false, true), &session(Agent::Claude, id))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Claude started in pane w1:p2 but is waiting for input there"));
        assert!(error.contains("blocked during startup"));
        // The agent is running there: its pane must not be closed.
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 3);
    }

    #[test]
    fn startup_timeout_restores_the_resume_name_herdr_dropped() {
        let id = "00000000-0000-4000-8000-000000000018";
        let mut cli = host(vec![
            ok(SPLIT),
            refused("timeout", "timed out waiting for agent startup"),
            ok(r#"{"result":{"type":"agent_info","agent":{}}}"#),
        ]);
        let error = cli
            .start_resume(&workspace(false, false), &session(Agent::Codex, id))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Codex was not ready in pane w1:p2"));
        assert!(error.contains("Enter focuses it"));
        let commands = cli.into_inner().commands;
        assert_eq!(
            commands[3],
            ["herdr", "agent", "rename", "w1:p2", name(id).as_str()]
        );
        assert_eq!(commands.len(), 4);
    }

    #[test]
    fn startup_timeout_without_rename_warns_against_a_second_enter() {
        let mut cli = host(vec![
            ok(SPLIT),
            refused("timeout", "timed out waiting for agent startup"),
            refused("agent_not_found", "no agent"),
        ]);
        let error = cli
            .start_resume(
                &workspace(false, false),
                &session(Agent::Codex, "00000000-0000-4000-8000-000000000019"),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("Check that pane before pressing Enter again"));
    }

    #[test]
    fn refused_launch_closes_only_the_split_it_opened() {
        let id = "00000000-0000-4000-8000-000000000020";
        let mut cli = host(vec![
            ok(SPLIT),
            refused("agent_name_taken", "agent name is already used"),
            ok(r#"{"result":{"type":"ok"}}"#),
        ]);
        let error = cli
            .start_resume(&workspace(false, false), &session(Agent::Codex, id))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "unsupported: Herdr `herdr agent start` failed with exit status: 1: agent name is already used"
        );
        let commands = cli.into_inner().commands;
        assert_eq!(commands[3], ["herdr", "pane", "close", "w1:p2"]);

        // A created workspace's root pane is not a split and stays open.
        let mut cli = host(vec![refused("agent_name_taken", "taken")]);
        cli.start_resume(&workspace(true, false), &session(Agent::Codex, id))
            .unwrap_err();
        assert_eq!(cli.into_inner().commands.len(), 2);
    }

    #[test]
    fn unsupported_or_unreadable_version_fails_before_any_host_command() {
        for (output, expected) in [
            (
                "herdr 0.7.1\n",
                "Herdr 0.7.1 is not supported; resume needs Herdr 0.9.3 or newer. Run `herdr update`.",
            ),
            (
                "herdr 0.9.2\n",
                "Herdr 0.9.2 is not supported; resume needs Herdr 0.9.3 or newer.",
            ),
            (
                "something else\n",
                "could not read the Herdr version from `herdr --version` (\"something else\")",
            ),
        ] {
            let mut cli = HerdrCli::new(Queued {
                commands: Vec::new(),
                responses: [ok(output)].into_iter().collect(),
            });
            let error = crate::resume_in_host_with_checker(
                &mut cli,
                &session(Agent::Claude, "00000000-0000-4000-8000-000000000021"),
                |_| true,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(expected), "{error}");
            assert_eq!(cli.into_inner().commands, [["herdr", "--version"]]);
        }
    }

    #[test]
    fn version_parser_accepts_releases_and_prereleases_only() {
        assert_eq!(parse_version("herdr 0.9.3\n"), Some((0, 9, 3)));
        assert_eq!(parse_version("herdr 0.10.0-preview.2"), Some((0, 10, 0)));
        assert_eq!(parse_version("herdr 1.0.0+abc (stable)"), Some((1, 0, 0)));
        assert_eq!(parse_version("herdr 0.9"), None);
        assert_eq!(parse_version("herdr 0.9.3.1"), None);
        assert_eq!(parse_version("herdr-cli 0.9.3"), None);
        assert_eq!(parse_version(""), None);
        assert!(check_version("herdr 0.9.3").is_ok());
        assert!(check_version("herdr 0.10.0").is_ok());
        assert!(check_version("herdr 0.9.2").is_err());
    }

    #[test]
    fn version_is_checked_once_per_host_session() {
        let mut cli = queued(&[r#"{"result":{"agents":[]}}"#, r#"{"result":{"agents":[]}}"#]);
        cli.live_agents().unwrap();
        cli.live_agents().unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0], ["herdr", "--version"]);
    }

    #[test]
    fn coordinator_closed_workspace_creates_then_starts_in_its_root_pane() {
        let id = "00000000-0000-4000-8000-000000000004";
        let mut cli = host(vec![
            ok(r#"{"result":{"workspaces":[]}}"#),
            ok(
                r#"{"result":{"workspace":{"workspace_id":"w2"},"root_pane":{"pane_id":"w2:p1","cwd":"/tmp/project"}}}"#,
            ),
            ok(r#"{"result":{"agents":[]}}"#),
            ok(&started(&["codex", "resume", id])),
        ]);
        crate::resume_in_host_with_checker(&mut cli, &session(Agent::Codex, id), |_| true).unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 5);
        assert_eq!(
            commands[2][..6],
            [
                "herdr",
                "workspace",
                "create",
                "--cwd",
                "/tmp/project",
                "--focus"
            ]
        );
        assert_eq!(
            commands[4][..8],
            [
                "herdr",
                "agent",
                "start",
                name(id).as_str(),
                "--kind",
                "codex",
                "--pane",
                "w2:p1"
            ]
        );
        assert!(!commands.iter().any(|c| c.contains(&"split".into())));
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
        assert_eq!(commands[3][..4], ["herdr", "workspace", "focus", "w1"]);
        assert_eq!(commands[5][..4], ["herdr", "agent", "focus", "w1:p1"]);
        assert!(!commands.iter().any(|c| c.contains(&"start".into())));
        assert!(!commands.iter().any(|c| c.contains(&"split".into())));
    }

    #[test]
    fn occupied_workspace_starts_in_a_new_split_without_writing_existing_pane() {
        let id = "00000000-0000-4000-8000-000000000007";
        let mut cli = queued(&[
            r#"{"result":{"workspaces":[{"workspace_id":"w1"}]}}"#,
            r#"{"result":{"panes":[{"pane_id":"w1:p1","cwd":"/tmp/project","agent":"claude"}]}}"#,
            r#"{"result":{}}"#,
            r#"{"result":{"agents":[{"workspace_id":"w1","pane_id":"w1:p1","agent":"claude","agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"00000000-0000-4000-8000-000000000006"}}]}}"#,
            SPLIT,
            &started(&["codex", "resume", id]),
        ]);
        crate::resume_in_host_with_checker(&mut cli, &session(Agent::Codex, id), |_| true).unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands.len(), 7);
        assert_eq!(commands[5][..4], ["herdr", "pane", "split", "w1:p1"]);
        assert_eq!(
            commands[6][..8],
            [
                "herdr",
                "agent",
                "start",
                name(id).as_str(),
                "--kind",
                "codex",
                "--pane",
                "w1:p2"
            ]
        );
        // The pane already in use is only ever split from, never written to.
        let touching: Vec<_> = commands
            .iter()
            .filter(|c| c.contains(&"w1:p1".into()))
            .collect();
        assert_eq!(touching, [&commands[5]]);
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
    fn parsers_accept_herdr_0_9_3_records() {
        // Trimmed from `herdr workspace list`, `pane list` and `agent list` on
        // Herdr 0.9.3; extra fields must not break parsing.
        let workspaces = r#"{"id":"cli:workspace:list","result":{"type":"workspace_list","workspaces":[{"workspace_id":"wB0","number":7,"label":"hah093-claude","focused":false,"pane_count":1,"tab_count":1,"active_tab_id":"wB0:t1","agent_status":"idle","worktree":{"checkout_path":"/tmp/project","is_linked_worktree":true,"repo_key":"/tmp/repo/.git","repo_name":"repo","repo_root":"/tmp/repo"}}]}}"#;
        assert_eq!(parse_workspace_list(workspaces).unwrap()[0].id, "wB0");
        let panes = r#"{"id":"cli:pane:list","result":{"type":"pane_list","panes":[{"agent":"claude","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"8f0f2724-2477-43d9-a05e-c8534d8d2beb"},"agent_status":"idle","cwd":"/tmp/project","focused":false,"foreground_cwd":"/tmp/project","pane_id":"wB0:p1","revision":4,"tab_id":"wB0:t1","terminal_id":"term_1","workspace_id":"wB0"}]}}"#;
        let pane = &parse_pane_list(panes).unwrap()[0];
        assert_eq!(
            (
                pane.pane_id.as_str(),
                pane.cwd.as_deref(),
                pane.agent.as_deref()
            ),
            ("wB0:p1", Some("/tmp/project"), Some("claude"))
        );
        let agents = r#"{"id":"cli:agent:list","result":{"type":"agent_list","agents":[{"agent":"codex","agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"01a0ef3e-2f02-74d3-8345-5ef40da038e0"},"agent_status":"unknown","cwd":"/tmp/project","focused":false,"interactive_ready":true,"name":"ah-0000000000000000000000000","pane_id":"wC1:p1","revision":2,"tab_id":"wC1:t1","terminal_id":"term_2","workspace_id":"wC1"}]}}"#;
        let agent = &parse_agent_list(agents).unwrap()[0];
        assert_eq!(agent.id, "wC1:p1");
        assert_eq!(agent.workspace_id, "wC1");
        assert_eq!(agent.name.as_deref(), Some("ah-0000000000000000000000000"));
        assert_eq!(
            agent.session_id.as_ref().unwrap().native_id,
            "01a0ef3e-2f02-74d3-8345-5ef40da038e0"
        );
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
    fn live_agent_without_reported_session_is_focused_by_its_resume_name() {
        // Right after `claude --resume`, and while an agent waits at a startup
        // prompt, Herdr reports no session ID for the pane. Starting again
        // would create a duplicate instead of focusing it.
        let id = "00000000-0000-4000-8000-000000000011";
        let agents = format!(
            r#"{{"result":{{"agents":[{{"workspace_id":"w1","pane_id":"w1:p2","agent":"codex","name":"{}"}}]}}}}"#,
            name(id)
        );
        let mut cli = queued(&[
            r#"{"result":{"workspaces":[{"workspace_id":"w1"}]}}"#,
            r#"{"result":{"panes":[{"pane_id":"w1:p1","cwd":"/tmp/project"}]}}"#,
            r#"{"result":{}}"#,
            &agents,
            r#"{"result":{}}"#,
        ]);
        crate::resume_in_host_with_checker(&mut cli, &session(Agent::Codex, id), |_| true).unwrap();
        let commands = cli.into_inner().commands;
        assert_eq!(commands[5][..4], ["herdr", "agent", "focus", "w1:p2"]);
        assert!(!commands.iter().any(|c| c.contains(&"start".into())));
    }

    #[test]
    fn resume_name_of_another_session_is_never_focused() {
        let id = "00000000-0000-4000-8000-000000000012";
        let other = "00000000-0000-4000-8000-000000000013";
        let start = |agents: String| {
            let mut cli = queued(&[
                r#"{"result":{"workspaces":[{"workspace_id":"w1"}]}}"#,
                r#"{"result":{"panes":[{"pane_id":"w1:p1","cwd":"/tmp/project"}]}}"#,
                r#"{"result":{}}"#,
                &format!(r#"{{"result":{{"agents":[{agents}]}}}}"#),
                SPLIT,
                &started(&["codex", "resume", id]),
            ]);
            crate::resume_in_host_with_checker(&mut cli, &session(Agent::Codex, id), |_| true)
                .unwrap();
            cli.into_inner().commands
        };
        // A pane resuming a different session, and a pane whose reported
        // session ID contradicts the name, both start a fresh resume instead.
        for agents in [
            format!(
                r#"{{"workspace_id":"w1","pane_id":"w1:p2","agent":"codex","name":"{}"}}"#,
                name(other)
            ),
            format!(
                r#"{{"workspace_id":"w1","pane_id":"w1:p2","agent":"codex","name":"{}","agent_session":{{"source":"herdr:codex","agent":"codex","kind":"id","value":"{other}"}}}}"#,
                name(id)
            ),
        ] {
            let commands = start(agents);
            assert_eq!(
                commands[6][..4],
                ["herdr", "agent", "start", name(id).as_str()]
            );
            assert!(!commands.iter().any(|c| c[1] == "agent" && c[2] == "focus"));
        }
    }

    #[test]
    fn host_failures_report_the_command_the_code_and_the_hosts_own_message() {
        let failure = failure_detail(
            &words(["agent", "start", "ah-x", "--kind", "codex"]),
            "exit status: 1",
            b"",
            br#"{"error":{"code":"unknown_option","message":"unknown option: --workspace"},"id":"cli:agent:start"}"#,
        );
        assert_eq!(failure.subcommand, "agent start");
        assert_eq!(failure.code.as_deref(), Some("unknown_option"));
        assert_eq!(failure.message, "unknown option: --workspace");
        assert_eq!(
            CoreError::from(failure).to_string(),
            "unsupported: Herdr `herdr agent start` failed with exit status: 1: unknown option: --workspace"
        );
        let plain = failure_detail(
            &words(["workspace", "focus"]),
            "",
            b"",
            b"no such workspace\n",
        );
        assert_eq!(
            (plain.code, plain.message.as_str()),
            (None, "no such workspace")
        );
        assert_eq!(
            failure_detail(&words(["agent", "list"]), "", b"", b"").message,
            "no diagnostic output"
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

//! The `rex-agent` command boundary. Commands are argv vectors, so paths and
//! IDs are never interpolated into shell source.
use crate::{AgentState, Opened, RexAgent, RexHost, RexSession};
use agent_history_core::{Agent, CoreError, Result, SessionId};
use agent_history_restore::plan::is_uuid;
use agent_history_tui::report::sanitize;
use serde::Deserialize;
use std::{ffi::OsString, path::Path, path::PathBuf};

/// Names the `rex-agent` executable to run instead of the one on `PATH`.
pub const BIN_ENV: &str = "REX_AGENT_BIN";

/// `REX_AGENT_BIN` when set, else `rex-agent` from `PATH`.
pub fn rex_agent_bin() -> OsString {
    std::env::var_os(BIN_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rex-agent".into())
}

/// What a successful command printed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Ran {
    pub stdout: String,
    /// `rex-agent` reports a partial success here with exit status 0, such as
    /// a focus the app did not follow because its Remote Control is off.
    pub stderr: String,
}

/// How a command that ran ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Exit {
    Success(Ran),
    /// A nonzero exit: `status` describes it, `message` is the tool's own
    /// sanitized stderr.
    Failure {
        status: String,
        message: String,
    },
}

pub trait CommandRunner {
    /// Runs `rex-agent` with `args`. Failing to start it is an error; its
    /// exit status is the caller's to judge.
    fn run(&mut self, args: &[String]) -> Result<Exit>;
}

pub struct ProcessRunner {
    program: OsString,
}

impl ProcessRunner {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    pub fn from_env() -> Self {
        Self::new(rex_agent_bin())
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&mut self, args: &[String]) -> Result<Exit> {
        let output = std::process::Command::new(&self.program)
            .args(args)
            // The browser owns the terminal; a command reading it would take
            // keystrokes meant for the search box.
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => CoreError::Unsupported(format!(
                    "rex-agent was not found; put it on PATH or set {BIN_ENV}"
                )),
                _ => CoreError::Io(error),
            })?;
        let stderr = message(&output.stderr);
        if !output.status.success() {
            return Ok(Exit::Failure {
                status: output.status.to_string(),
                message: stderr,
            });
        }
        let stdout = String::from_utf8(output.stdout).map_err(|e| {
            CoreError::Unsupported(format!("rex-agent returned non-UTF-8 output: {e}"))
        })?;
        Ok(Exit::Success(Ran { stdout, stderr }))
    }
}

/// `rex-agent`'s own stderr, as one sanitized line without its program prefix.
fn message(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let text = text.strip_prefix("rex-agent: ").unwrap_or(text);
    sanitize(&text.replace('\n', " "))
}

/// Rex driven through `rex-agent`.
pub struct RexAgentCli<R> {
    runner: R,
    warning: Option<String>,
}

impl<R> RexAgentCli<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            warning: None,
        }
    }

    pub fn into_inner(self) -> R {
        self.runner
    }
}

impl<R: CommandRunner> RexAgentCli<R> {
    /// Runs a command whose failure is an error.
    fn run(&mut self, args: Vec<String>) -> Result<String> {
        Ok(self.run_status(&args, false)?.unwrap_or_default())
    }

    /// Runs a command; `Ok(None)` for a nonzero exit when `may_fail`.
    fn run_status(&mut self, args: &[String], may_fail: bool) -> Result<Option<String>> {
        match self.runner.run(args)? {
            Exit::Success(ran) => {
                if !ran.stderr.is_empty() {
                    self.warning = Some(ran.stderr);
                }
                Ok(Some(ran.stdout))
            }
            Exit::Failure { .. } if may_fail => Ok(None),
            Exit::Failure { status, message } => Err(CoreError::Unsupported(format!(
                "`rex-agent {}` failed with {status}: {}",
                args.first().map(String::as_str).unwrap_or_default(),
                if message.is_empty() {
                    "no message"
                } else {
                    &message
                }
            ))),
        }
    }
}

impl<R: CommandRunner> RexHost for RexAgentCli<R> {
    fn agents(&mut self) -> Result<Vec<RexAgent>> {
        parse_agent_list(&self.run(vec!["list".into(), "--json".into()])?)
    }

    fn focus(&mut self, block_id: &str) -> Result<()> {
        // A target that looks like an option would be parsed as one.
        if block_id.is_empty()
            || block_id.starts_with('-')
            || block_id.chars().any(char::is_control)
        {
            return Err(CoreError::Unsupported(
                "Rex reported an invalid block ID".into(),
            ));
        }
        self.run(vec!["focus".into(), block_id.into()])?;
        Ok(())
    }

    /// `rex-agent session-for` exits nonzero both when no session works in
    /// the directory and when several do; either way there is none to pick.
    fn session_for(&mut self, cwd: &Path) -> Result<Option<RexSession>> {
        let cwd = utf8(cwd)?;
        // `--` keeps a directory that starts with `-` from being read as an
        // option.
        let args = ["session-for".to_string(), "--".into(), cwd.into()];
        self.run_status(&args, true)?
            .map(|stdout| parse_session_for(&stdout))
            .transpose()
    }

    fn open(
        &mut self,
        session: Option<&RexSession>,
        cwd: &Path,
        label: &str,
        argv: &[String],
    ) -> Result<Opened> {
        let cwd = utf8(cwd)?;
        // `--name=value` keeps a value that starts with `-` from being read
        // as an option.
        let mut args = vec!["open".to_string()];
        if let Some(session) = session {
            args.push(format!("--session={}", session.session_id));
        }
        args.extend([
            format!("--cwd={cwd}"),
            format!("--label={label}"),
            "--".into(),
        ]);
        args.extend(argv.iter().cloned());
        parse_opened(&self.run(args)?)
    }

    fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }
}

fn utf8(cwd: &Path) -> Result<&str> {
    cwd.to_str()
        .ok_or_else(|| CoreError::Unsupported("session directory is not UTF-8".into()))
}

#[derive(Deserialize)]
struct AgentList {
    #[serde(default)]
    agents: Vec<RawAgent>,
}

#[derive(Deserialize)]
struct RawAgent {
    session_id: String,
    block_id: String,
    #[serde(default)]
    claude_session_id: Option<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    state: Option<String>,
}

/// `rex-agent list --json`. Every listed agent is Claude Code; an ID that is
/// not a session UUID (or none yet, before Claude writes its session file)
/// identifies no session.
pub fn parse_agent_list(json: &str) -> Result<Vec<RexAgent>> {
    let list: AgentList = serde_json::from_str(json).map_err(|e| {
        CoreError::Unsupported(format!("unexpected `rex-agent list --json` output: {e}"))
    })?;
    Ok(list
        .agents
        .into_iter()
        .map(|raw| RexAgent {
            rex_session_id: raw.session_id,
            block_id: raw.block_id,
            session: raw
                .claude_session_id
                .filter(|id| is_uuid(id))
                .map(|id| SessionId::new(Agent::Claude, id)),
            cwd: raw.cwd,
            state: raw.state.as_deref().and_then(AgentState::parse),
        })
        .collect())
}

#[derive(Deserialize)]
struct RawSession {
    session_id: String,
    #[serde(default)]
    label: String,
}

/// The `{"session_id","label"}` line `rex-agent session-for` prints. A
/// session ID that could be read as an option or carries control characters
/// is refused rather than passed on to `open`.
pub fn parse_session_for(json: &str) -> Result<RexSession> {
    let raw: RawSession = serde_json::from_str(json.trim()).map_err(|e| {
        CoreError::Unsupported(format!("unexpected `rex-agent session-for` output: {e}"))
    })?;
    if raw.session_id.is_empty() || raw.session_id.chars().any(char::is_control) {
        return Err(CoreError::Unsupported(
            "Rex reported an invalid session ID".into(),
        ));
    }
    Ok(RexSession {
        session_id: raw.session_id,
        label: raw.label,
    })
}

#[derive(Deserialize)]
struct RawOpened {
    session_id: String,
    block_id: String,
}

/// The `{"session_id","block_id"}` line `rex-agent open` prints.
pub fn parse_opened(json: &str) -> Result<Opened> {
    let raw: RawOpened = serde_json::from_str(json.trim())
        .map_err(|e| CoreError::Unsupported(format!("unexpected `rex-agent open` output: {e}")))?;
    Ok(Opened {
        rex_session_id: raw.session_id,
        block_id: raw.block_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    /// Records each command and answers from a queue.
    #[derive(Default)]
    struct Queued {
        calls: Vec<Vec<String>>,
        answers: Vec<Result<Exit>>,
    }

    impl CommandRunner for Queued {
        fn run(&mut self, args: &[String]) -> Result<Exit> {
            self.calls.push(args.to_vec());
            self.answers.remove(0)
        }
    }

    fn answer(stdout: &str) -> Result<Exit> {
        Ok(Exit::Success(Ran {
            stdout: stdout.into(),
            stderr: String::new(),
        }))
    }

    fn failure(message: &str) -> Result<Exit> {
        Ok(Exit::Failure {
            status: "exit status: 1".into(),
            message: message.into(),
        })
    }

    #[test]
    fn the_agent_list_names_claude_sessions_and_ignores_the_rest() {
        let json = format!(
            r#"{{"agents":[
              {{"session_id":"session:a","session_label":"web","window_id":"w","block_id":"block:1",
                "label":"","state":"idle","kind":null,"msg":"","updated_at":"t","pid":42,
                "claude_session_id":"{ID}","name":null,"cwd":"/w/a","placed":true,"focused":false}},
              {{"session_id":"session:b","block_id":"block:2","claude_session_id":null,"cwd":null}},
              {{"session_id":"session:c","block_id":"block:3","claude_session_id":"not-a-uuid","state":"dreaming"}}
            ]}}"#
        );
        let agents = parse_agent_list(&json).unwrap();
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].block_id, "block:1");
        assert_eq!(agents[0].rex_session_id, "session:a");
        assert_eq!(agents[0].session, Some(SessionId::new(Agent::Claude, ID)));
        assert_eq!(agents[0].cwd.as_deref(), Some(Path::new("/w/a")));
        assert_eq!(agents[0].state, Some(AgentState::Idle));
        assert_eq!(agents[1].session, None);
        assert_eq!(agents[1].state, None, "no state reported");
        assert_eq!(agents[2].session, None, "only a UUID names a session");
        assert_eq!(agents[2].state, None, "an unknown state is not guessed");
        for (raw, state) in [
            ("working", AgentState::Working),
            ("blocked", AgentState::Blocked),
            ("done", AgentState::Done),
            ("idle", AgentState::Idle),
            ("error", AgentState::Error),
        ] {
            assert_eq!(AgentState::parse(raw), Some(state));
        }
        assert!(parse_agent_list(r#"{"agents":[]}"#).unwrap().is_empty());
        assert!(parse_agent_list("not json").is_err());
    }

    #[test]
    fn commands_are_argv_with_values_bound_to_their_options() {
        let mut cli = RexAgentCli::new(Queued {
            answers: vec![
                answer(r#"{"agents":[]}"#),
                answer(""),
                answer("{\"session_id\": \"session:n\", \"block_id\": \"block:n\"}\n"),
            ],
            ..Queued::default()
        });
        cli.agents().unwrap();
        cli.focus("block:1").unwrap();
        let argv = vec!["claude".to_string(), "--resume".into(), ID.into()];
        let opened = cli
            .open(None, Path::new("/w/-odd dir; rm -rf ~"), "-odd", &argv)
            .unwrap();
        assert_eq!(
            opened,
            Opened {
                rex_session_id: "session:n".into(),
                block_id: "block:n".into()
            }
        );
        let calls = cli.into_inner().calls;
        assert_eq!(calls[0], ["list", "--json"]);
        assert_eq!(calls[1], ["focus", "block:1"]);
        assert_eq!(
            calls[2],
            [
                "open",
                "--cwd=/w/-odd dir; rm -rf ~",
                "--label=-odd",
                "--",
                "claude",
                "--resume",
                ID
            ]
        );
    }

    #[test]
    fn a_resume_opens_as_a_tab_of_the_one_session_working_in_its_directory() {
        let mut cli = RexAgentCli::new(Queued {
            answers: vec![
                answer("{\"session_id\": \"session:web\", \"label\": \"web\"}\n"),
                answer(r#"{"session_id":"session:web","block_id":"block:n"}"#),
                failure("2 sessions work in /w/a"),
                failure("0 sessions work in /w/a"),
                answer(r#"{"session_id":"\u001b[31m","label":""}"#),
            ],
            ..Queued::default()
        });
        let into = cli.session_for(Path::new("/w/-a b")).unwrap().unwrap();
        assert_eq!(
            into,
            RexSession {
                session_id: "session:web".into(),
                label: "web".into()
            }
        );
        let argv = vec!["claude".to_string(), "--resume".into(), ID.into()];
        cli.open(Some(&into), Path::new("/w/-a b"), "claude", &argv)
            .unwrap();
        // Several sessions, or none: not an error, just no session to use.
        assert_eq!(cli.session_for(Path::new("/w/a")).unwrap(), None);
        assert_eq!(cli.session_for(Path::new("/w/a")).unwrap(), None);
        assert!(cli.session_for(Path::new("/w/a")).is_err());
        let calls = cli.into_inner().calls;
        assert_eq!(calls[0], ["session-for", "--", "/w/-a b"]);
        assert_eq!(
            calls[1],
            [
                "open",
                "--session=session:web",
                "--cwd=/w/-a b",
                "--label=claude",
                "--",
                "claude",
                "--resume",
                ID
            ]
        );
    }

    #[test]
    fn a_block_id_that_reads_as_an_option_is_never_passed() {
        let mut cli = RexAgentCli::new(Queued::default());
        for bad in ["", "--help", "block:\u{1b}"] {
            assert!(cli.focus(bad).is_err(), "{bad:?}");
        }
        assert!(cli.into_inner().calls.is_empty());
    }

    #[test]
    fn a_warning_on_success_is_kept_for_the_notice() {
        let mut cli = RexAgentCli::new(Queued {
            answers: vec![Ok(Exit::Success(Ran {
                stdout: String::new(),
                stderr: "focused the pane, but the app did not switch to it".into(),
            }))],
            ..Queued::default()
        });
        cli.focus("block:1").unwrap();
        assert!(cli
            .take_warning()
            .is_some_and(|w| w.contains("did not switch")));
        assert_eq!(cli.take_warning(), None, "a warning is reported once");
    }

    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("rex-agent");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// The real process path against a stand-in `rex-agent`: arguments arrive
    /// one per argv slot, a warning on stderr is not a failure, and a failure
    /// carries the tool's own message without its prefix.
    #[test]
    fn the_process_runner_passes_argv_and_reports_failures() {
        let temp = agent_history_core::test_support::TempDir::new("rex-runner").unwrap();
        let log = temp.path().join("argv");
        let ok = script(
            temp.path(),
            &format!(
                "for a in \"$@\"; do printf '%s\\n' \"$a\" >> '{}'; done\n\
                 echo 'rex-agent: Remote Control is off' >&2\n\
                 echo '{{\"session_id\":\"session:n\",\"block_id\":\"block:n\"}}'",
                log.display()
            ),
        );
        let mut cli = RexAgentCli::new(ProcessRunner::new(&ok));
        let argv = vec!["codex".to_string(), "resume".into(), ID.into()];
        let opened = cli
            .open(None, Path::new("/w/a b"), "a b; $(x)", &argv)
            .unwrap();
        assert_eq!(opened.block_id, "block:n");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("open\n--cwd=/w/a b\n--label=a b; $(x)\n--\ncodex\nresume\n{ID}\n")
        );
        assert_eq!(cli.take_warning().as_deref(), Some("Remote Control is off"));

        let failing = script(
            temp.path(),
            "echo 'rex-agent: no agent matches block:9' >&2\nexit 1",
        );
        let error = RexAgentCli::new(ProcessRunner::new(&failing))
            .focus("block:9")
            .unwrap_err()
            .to_string();
        assert!(error.contains("`rex-agent focus` failed"), "{error}");
        assert!(error.contains(": no agent matches block:9"), "{error}");

        let missing = RexAgentCli::new(ProcessRunner::new(temp.path().join("absent")))
            .agents()
            .unwrap_err()
            .to_string();
        assert!(missing.contains("REX_AGENT_BIN"), "{missing}");

        // A nonzero `session-for` means no single session, not a failure.
        let none = script(
            temp.path(),
            "echo 'rex-agent: 2 sessions work in /w' >&2\nexit 1",
        );
        assert_eq!(
            RexAgentCli::new(ProcessRunner::new(&none))
                .session_for(Path::new("/w"))
                .unwrap(),
            None
        );
    }
}

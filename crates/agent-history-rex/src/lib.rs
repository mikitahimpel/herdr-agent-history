//! Rex host boundary: finds Claude Code agents Rex is running, focuses their
//! panes, and opens a resumed native session as a tab of the Rex session
//! already working in its directory, or as a new Rex session. Every host
//! operation goes through the `rex-agent` CLI ([`cli`]).
use agent_history_core::{Agent, CoreError, Priorities, Result, Session, SessionId};
use agent_history_restore::{live, plan::NativeResumePlan, Resumed};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub mod cli;
pub mod integration;

/// A Claude Code agent running in a Rex terminal block, as `rex-agent list`
/// reports it. Rex only detects Claude Code (through the terminal's Program
/// Status Protocol), so Codex agents never appear here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RexAgent {
    pub rex_session_id: String,
    pub block_id: String,
    /// The Claude session the agent is serving, once Claude has said so.
    pub session: Option<SessionId>,
    pub cwd: Option<PathBuf>,
    /// What the agent last reported through the Program Status Protocol;
    /// `None` when Rex reported a state this crate does not know.
    pub state: Option<AgentState>,
}

/// A Claude Code agent's state as `rex-agent list` reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentState {
    /// Running a turn.
    Working,
    /// Waiting for the user to grant a permission or answer a question.
    Blocked,
    /// Finished a turn the user has not looked at yet.
    Done,
    /// Waiting for a prompt.
    Idle,
    Error,
}

impl AgentState {
    pub fn parse(state: &str) -> Option<Self> {
        Some(match state {
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "idle" => Self::Idle,
            "error" => Self::Error,
            _ => return None,
        })
    }

    /// The priority group: the states that want the user first.
    pub fn group(self) -> u32 {
        match self {
            Self::Blocked => BLOCKED,
            Self::Error => ERROR,
            Self::Done => DONE,
            Self::Working => WORKING,
            Self::Idle => IDLE,
        }
    }
}

/// The Rex session `rex-agent session-for` names as the one whose terminals
/// work in a directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RexSession {
    pub session_id: String,
    pub label: String,
}

/// The Rex session and block `rex-agent open` created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opened {
    pub rex_session_id: String,
    pub block_id: String,
}

/// The host operations resuming needs; mockable so tests run without Rex.
pub trait RexHost {
    fn agents(&mut self) -> Result<Vec<RexAgent>>;
    fn focus(&mut self, block_id: &str) -> Result<()>;
    /// The one Rex session working in `cwd`; `None` when there is none, or
    /// more than one.
    fn session_for(&mut self, cwd: &Path) -> Result<Option<RexSession>>;
    /// Opens `argv` in `cwd` and focuses it: as a tab named `label` in
    /// `session`, or as a new Rex session named `label`.
    fn open(
        &mut self,
        session: Option<&RexSession>,
        cwd: &Path,
        label: &str,
        argv: &[String],
    ) -> Result<Opened>;
    /// A warning a command that otherwise succeeded printed since the last
    /// call, such as a focus the app did not follow.
    fn take_warning(&mut self) -> Option<String> {
        None
    }
}

/// Focuses the pane already running the session, or resumes it in a new tab
/// of the one Rex session working in its directory, or else in a new Rex
/// session. A live agent with another session ID is never a match.
pub fn resume_in_rex<H: RexHost + ?Sized>(host: &mut H, session: &Session) -> Result<Resumed> {
    let running = live::running_sessions();
    resume_in_rex_with(host, session, |path| path.is_dir(), &running)
}

/// `running` names the sessions an agent process on this machine is already
/// serving, from [`live::running_sessions`].
pub fn resume_in_rex_with<H, F>(
    host: &mut H,
    session: &Session,
    exists: F,
    running: &HashSet<SessionId>,
) -> Result<Resumed>
where
    H: RexHost + ?Sized,
    F: Fn(&Path) -> bool,
{
    let plan = NativeResumePlan::for_session(session)?;
    let cwd = session
        .cwd
        .as_deref()
        .ok_or_else(|| CoreError::Unsupported("the session has no recorded directory".into()))?;
    if !exists(cwd) {
        return Err(CoreError::Unsupported(
            "the session's directory does not exist; refusing to open it".into(),
        ));
    }
    if session.id.agent == Agent::Claude {
        let agents = host.agents()?;
        let mut matching: Vec<&RexAgent> = agents
            .iter()
            .filter(|agent| agent.session.as_ref() == Some(&session.id))
            .collect();
        if matching.len() > 1 {
            return Err(CoreError::Unsupported(
                "multiple live agents match the native session".into(),
            ));
        }
        if let Some(agent) = matching.pop() {
            host.focus(&agent.block_id)?;
            return Ok(Resumed::Focused);
        }
        if running.contains(&session.id) {
            // A process serves this conversation under an ID Rex does not
            // report. The directory identifies its pane only when exactly one
            // agent runs there; two would be a guess.
            let mut here = agents.iter().filter(|a| a.cwd.as_deref() == Some(cwd));
            if let (Some(agent), None) = (here.next(), here.next()) {
                host.focus(&agent.block_id)?;
                return Ok(Resumed::Focused);
            }
            return Ok(Resumed::AlreadyRunning);
        }
    } else if running.contains(&session.id) {
        // Rex does not list Codex agents, so its pane cannot be found; a
        // second process on one transcript is still worse than not starting.
        return Ok(Resumed::AlreadyRunning);
    }
    match host.session_for(cwd)? {
        // A tab in a session is named for what runs in it; the session
        // already says where.
        Some(into) => {
            let tab = match plan.agent {
                Agent::Claude => "claude",
                Agent::Codex => "codex",
            };
            host.open(Some(&into), cwd, tab, &plan.argv)?
        }
        None => host.open(None, cwd, &label(cwd), &plan.argv)?,
    };
    Ok(Resumed::Started)
}

/// The Rex session label for a resume: the directory's name, without
/// control characters.
fn label(cwd: &Path) -> String {
    let name: String = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if name.is_empty() {
        "agent".into()
    } else {
        name
    }
}

/// Priority groups, most urgent first: an agent waiting on the user, one
/// that failed, one with a finished turn nobody has read, one at work, and
/// one waiting for a prompt.
pub const BLOCKED: u32 = 0;
pub const ERROR: u32 = 1;
pub const DONE: u32 = 2;
pub const WORKING: u32 = 3;
pub const IDLE: u32 = 4;
/// A running agent whose state Rex does not report: one found only by the
/// process scan (every Codex agent, and Claude outside Rex), or a Rex agent
/// in a state this crate does not know.
pub const RUNNING: u32 = 5;

/// Sessions for which Enter would show a running agent instead of starting
/// one, with their group: Claude sessions exactly one Rex agent reports, by
/// that agent's state, and those a local process serves.
pub fn live_sessions(
    sessions: &[Session],
    agents: &[RexAgent],
    running: &HashSet<SessionId>,
) -> Vec<(SessionId, u32)> {
    sessions
        .iter()
        .filter_map(|session| {
            let mut serving = agents
                .iter()
                .filter(|agent| agent.session.as_ref() == Some(&session.id));
            let group = match (serving.next(), serving.next()) {
                (Some(agent), None) => agent.state.map_or(RUNNING, AgentState::group),
                _ if running.contains(&session.id) => RUNNING,
                _ => return None,
            };
            Some((session.id.clone(), group))
        })
        .collect()
}

/// [`live_sessions`] as the browser asks for it, with Rex's agent list and
/// the process scan gathered concurrently. A Rex that cannot answer
/// prioritizes nothing; it does not make the browser fail.
pub fn live_priorities(host: &mut dyn RexHost, sessions: &[Session]) -> Priorities {
    live_priorities_with(host, sessions, live::running_sessions)
}

pub fn live_priorities_with<S>(host: &mut dyn RexHost, sessions: &[Session], scan: S) -> Priorities
where
    S: FnOnce() -> HashSet<SessionId> + Send,
{
    let (running, agents) = std::thread::scope(|scope| {
        let scan = scope.spawn(scan);
        let agents = host.agents();
        (scan.join().unwrap_or_default(), agents)
    });
    let Ok(agents) = agents else {
        return Priorities::default();
    };
    live_sessions(sessions, &agents, &running)
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::SourceRef;

    fn session(agent: Agent, n: u8, cwd: &str) -> Session {
        Session {
            id: SessionId::new(agent, format!("00000000-0000-4000-8000-0000000000{n:02}")),
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

    fn agent(block: &str, serving: Option<&Session>, cwd: &str) -> RexAgent {
        RexAgent {
            rex_session_id: format!("session:{block}"),
            block_id: block.into(),
            session: serving.map(|s| s.id.clone()),
            cwd: Some(cwd.into()),
            state: Some(AgentState::Idle),
        }
    }

    #[derive(Default)]
    struct Host {
        agents: Vec<RexAgent>,
        effects: Vec<String>,
        unreachable: bool,
        /// What `session_for` answers, for any directory.
        working_here: Option<RexSession>,
    }

    impl RexHost for Host {
        fn agents(&mut self) -> Result<Vec<RexAgent>> {
            self.effects.push("list".into());
            if self.unreachable {
                return Err(CoreError::Unsupported("rex is not running".into()));
            }
            Ok(self.agents.clone())
        }
        fn focus(&mut self, block_id: &str) -> Result<()> {
            self.effects.push(format!("focus {block_id}"));
            Ok(())
        }
        fn session_for(&mut self, cwd: &Path) -> Result<Option<RexSession>> {
            self.effects.push(format!("session-for {}", cwd.display()));
            Ok(self.working_here.clone())
        }
        fn open(
            &mut self,
            session: Option<&RexSession>,
            cwd: &Path,
            label: &str,
            argv: &[String],
        ) -> Result<Opened> {
            let into = session.map_or("new".into(), |s| s.session_id.clone());
            self.effects.push(format!(
                "open {into} {} {label} {}",
                cwd.display(),
                argv.join(" ")
            ));
            Ok(Opened {
                rex_session_id: "session:new".into(),
                block_id: "block:new".into(),
            })
        }
    }

    fn none() -> HashSet<SessionId> {
        HashSet::new()
    }

    fn resume(host: &mut Host, s: &Session, running: &HashSet<SessionId>) -> Result<Resumed> {
        resume_in_rex_with(host, s, |_| true, running)
    }

    #[test]
    fn a_claude_session_rex_is_running_is_focused_not_started() {
        let s = session(Agent::Claude, 1, "/w/a");
        let other = session(Agent::Claude, 2, "/w/a");
        let mut host = Host {
            agents: vec![
                agent("block:o", Some(&other), "/w/a"),
                agent("block:s", Some(&s), "/w/b"),
            ],
            ..Host::default()
        };
        assert_eq!(resume(&mut host, &s, &none()).unwrap(), Resumed::Focused);
        assert_eq!(host.effects, ["list", "focus block:s"]);
    }

    #[test]
    fn a_session_nothing_runs_opens_a_new_rex_session_with_the_native_argv() {
        let s = session(Agent::Claude, 1, "/w/project");
        let mut host = Host {
            agents: vec![agent(
                "block:o",
                Some(&session(Agent::Claude, 2, "/w/project")),
                "/w/project",
            )],
            ..Host::default()
        };
        assert_eq!(resume(&mut host, &s, &none()).unwrap(), Resumed::Started);
        assert_eq!(
            host.effects,
            [
                "list",
                "session-for /w/project",
                "open new /w/project project claude --resume 00000000-0000-4000-8000-000000000001"
            ]
        );
    }

    #[test]
    fn a_session_resumes_as_a_tab_of_the_rex_session_working_in_its_directory() {
        let into = RexSession {
            session_id: "session:web".into(),
            label: "web".into(),
        };
        for (agent, argv) in [
            (Agent::Claude, "claude claude --resume"),
            (Agent::Codex, "codex codex resume"),
        ] {
            let s = session(agent, 1, "/w/project");
            let mut host = Host {
                working_here: Some(into.clone()),
                ..Host::default()
            };
            assert_eq!(resume(&mut host, &s, &none()).unwrap(), Resumed::Started);
            assert_eq!(
                host.effects.last().unwrap(),
                &format!("open session:web /w/project {argv} 00000000-0000-4000-8000-000000000001")
            );
            assert!(host.effects.contains(&"session-for /w/project".to_string()));
        }
    }

    #[test]
    fn a_running_session_is_focused_without_asking_which_session_works_there() {
        let s = session(Agent::Claude, 1, "/w/a");
        let mut host = Host {
            agents: vec![agent("block:s", Some(&s), "/w/a")],
            working_here: Some(RexSession {
                session_id: "session:web".into(),
                label: "web".into(),
            }),
            ..Host::default()
        };
        assert_eq!(resume(&mut host, &s, &none()).unwrap(), Resumed::Focused);
        assert_eq!(host.effects, ["list", "focus block:s"]);
    }

    #[test]
    fn codex_is_resumed_without_asking_rex_which_cannot_see_it() {
        let s = session(Agent::Codex, 1, "/w/a");
        let mut host = Host::default();
        assert_eq!(resume(&mut host, &s, &none()).unwrap(), Resumed::Started);
        assert_eq!(
            host.effects,
            [
                "session-for /w/a",
                "open new /w/a a codex resume 00000000-0000-4000-8000-000000000001"
            ]
        );
        // A local process already serves it: nothing is started twice.
        let mut host = Host::default();
        let running = [s.id.clone()].into_iter().collect();
        assert_eq!(
            resume(&mut host, &s, &running).unwrap(),
            Resumed::AlreadyRunning
        );
        assert!(host.effects.is_empty());
    }

    /// Claude's ID moves on after `/resume` or a fork, so Rex can report the
    /// pane under another ID while a process still serves this one.
    #[test]
    fn a_running_session_rex_reports_under_another_id_is_found_by_directory_only_when_unambiguous()
    {
        let s = session(Agent::Claude, 1, "/w/a");
        let stale = session(Agent::Claude, 9, "/w/a");
        let running: HashSet<_> = [s.id.clone()].into_iter().collect();
        let mut host = Host {
            agents: vec![
                agent("block:p", Some(&stale), "/w/a"),
                agent("block:q", None, "/w/b"),
            ],
            ..Host::default()
        };
        assert_eq!(resume(&mut host, &s, &running).unwrap(), Resumed::Focused);
        assert_eq!(host.effects, ["list", "focus block:p"]);

        let mut host = Host {
            agents: vec![
                agent("block:p", Some(&stale), "/w/a"),
                agent("block:q", None, "/w/a"),
            ],
            ..Host::default()
        };
        assert_eq!(
            resume(&mut host, &s, &running).unwrap(),
            Resumed::AlreadyRunning
        );
        assert_eq!(
            host.effects,
            ["list"],
            "no pane picked at random, nothing started"
        );
    }

    #[test]
    fn two_panes_reporting_one_session_are_an_error_not_a_guess() {
        let s = session(Agent::Claude, 1, "/w/a");
        let mut host = Host {
            agents: vec![
                agent("block:1", Some(&s), "/w/a"),
                agent("block:2", Some(&s), "/w/a"),
            ],
            ..Host::default()
        };
        assert!(resume(&mut host, &s, &none()).is_err());
        assert_eq!(host.effects, ["list"]);
    }

    #[test]
    fn a_missing_directory_or_invalid_id_has_no_host_effect() {
        let s = session(Agent::Claude, 1, "/w/gone");
        let mut host = Host::default();
        assert!(resume_in_rex_with(&mut host, &s, |_| false, &none()).is_err());
        let mut bad = session(Agent::Claude, 1, "/w/a");
        bad.id.native_id = "bad\n-id".into();
        assert!(resume(&mut host, &bad, &none()).is_err());
        assert!(host.effects.is_empty());
    }

    #[test]
    fn labels_are_the_directory_name_without_control_characters() {
        assert_eq!(label(Path::new("/w/my repo")), "my repo");
        assert_eq!(label(Path::new("/w/a\u{1b}[31mb")), "a[31mb");
        assert_eq!(label(Path::new("/")), "agent");
    }

    #[test]
    fn rex_agents_are_grouped_by_state_most_urgent_first() {
        let states = [
            (Some(AgentState::Idle), IDLE),
            (Some(AgentState::Working), WORKING),
            (Some(AgentState::Done), DONE),
            (Some(AgentState::Error), ERROR),
            (Some(AgentState::Blocked), BLOCKED),
            (None, RUNNING),
        ];
        let sessions: Vec<Session> = (0..states.len() as u8)
            .map(|n| session(Agent::Claude, n, "/w/a"))
            .collect();
        let agents: Vec<RexAgent> = sessions
            .iter()
            .zip(states)
            .map(|(s, (state, _))| RexAgent {
                state,
                ..agent(&format!("block:{}", s.id.native_id), Some(s), "/w/a")
            })
            .collect();
        let live = live_sessions(&sessions, &agents, &none());
        let expected: Vec<_> = sessions
            .iter()
            .zip(states)
            .map(|(s, (_, group))| (s.id.clone(), group))
            .collect();
        assert_eq!(live, expected);
        let order = [BLOCKED, ERROR, DONE, WORKING, IDLE, RUNNING];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");

        // The process scan alone knows no state; two panes on one session
        // say nothing reliable about it either.
        let scanned = session(Agent::Codex, 40, "/w/b");
        let doubled = session(Agent::Claude, 41, "/w/b");
        let agents = [
            agent("block:x", Some(&doubled), "/w/b"),
            RexAgent {
                state: Some(AgentState::Blocked),
                ..agent("block:y", Some(&doubled), "/w/b")
            },
        ];
        let running = [scanned.id.clone(), doubled.id.clone()]
            .into_iter()
            .collect();
        assert_eq!(
            live_sessions(&[scanned.clone(), doubled.clone()], &agents, &running),
            [(scanned.id, RUNNING), (doubled.id, RUNNING)]
        );
    }

    #[test]
    fn priorities_count_rex_reports_and_local_processes() {
        let reported = session(Agent::Claude, 1, "/w/a");
        let codex = session(Agent::Codex, 2, "/w/b");
        let closed = session(Agent::Claude, 3, "/w/a");
        let sessions = [reported.clone(), codex.clone(), closed.clone()];
        let mut host = Host {
            agents: vec![agent("block:1", Some(&reported), "/w/a")],
            ..Host::default()
        };
        let p = live_priorities_with(&mut host, &sessions, || {
            [codex.id.clone()].into_iter().collect()
        });
        assert_eq!(p.len(), 2);
        assert_eq!(p.group(&reported.id), Some(IDLE));
        assert_eq!(p.group(&codex.id), Some(RUNNING));
        assert_eq!(p.group(&closed.id), None);

        let mut silent = Host {
            unreachable: true,
            ..Host::default()
        };
        assert!(
            live_priorities_with(&mut silent, &sessions, || [codex.id.clone()]
                .into_iter()
                .collect())
            .is_empty()
        );
    }
}

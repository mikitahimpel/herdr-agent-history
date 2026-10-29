//! Small host boundary; Herdr integration owns workspace and process behavior.
use agent_history_core::{CoreError, Result, Session};
use std::path::Path;

pub mod integration;
pub mod live;
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

/// What resuming did, so the browser can say so rather than leave the reader
/// guessing which of these happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resumed {
    /// A pane already running this session was brought to the front.
    Focused,
    /// The session was not running, so the agent was started for it.
    Started,
    /// A process on this machine is already serving this session and no pane
    /// could be identified for it, so nothing was started.
    AlreadyRunning,
}

/// Finds the exact native session before starting anything. In particular, a
/// live agent with another session ID is never treated as a match.
pub fn resume_in_host<H: HostRuntime>(host: &mut H, session: &Session) -> Result<Resumed> {
    let running = live::running_sessions();
    resume_in_host_with(host, session, |path| path.exists(), &running)
}

pub fn resume_in_host_with_checker<H, F>(
    host: &mut H,
    session: &Session,
    exists: F,
) -> Result<Resumed>
where
    H: HostRuntime,
    F: Fn(&Path) -> bool,
{
    resume_in_host_with(host, session, exists, &std::collections::HashSet::new())
}

/// `running` names the sessions an agent process on this machine is already
/// serving, from [`live::running_sessions`].
pub fn resume_in_host_with<H, F>(
    host: &mut H,
    session: &Session,
    exists: F,
    running: &std::collections::HashSet<agent_history_core::SessionId>,
) -> Result<Resumed>
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
        return Ok(Resumed::Focused);
    }
    if running.contains(&session.id) {
        // A process is serving this conversation under an ID the host does
        // not report, so starting the agent would put a second process on one
        // transcript. The pane cannot be identified by session ID here; the
        // directory is what is left, and it is only trusted when it picks out
        // exactly one agent of the right kind.
        if let Some(agent) = sole_agent_in_cwd(session, &workspace.id, &agents) {
            host.focus_agent(&agent.id)?;
            return Ok(Resumed::Focused);
        }
        return Ok(Resumed::AlreadyRunning);
    }
    host.start_agent(&workspace, session, &plan)?;
    Ok(Resumed::Started)
}

/// The one agent of the session's kind running in the session's own directory
/// within `workspace_id`, if it is the only one. Two candidates name no pane
/// in particular, and focusing the wrong one hides the conversation the
/// reader asked for behind another.
pub fn sole_agent_in_cwd<'a>(
    session: &Session,
    workspace_id: &str,
    agents: &'a [resume::LiveAgent],
) -> Option<&'a resume::LiveAgent> {
    let cwd = session.cwd.as_deref()?;
    let mut candidates = agents.iter().filter(|agent| {
        agent.workspace_id == workspace_id
            && agent.agent == session.id.agent
            && agent.cwd.as_deref() == Some(cwd)
    });
    let found = candidates.next()?;
    candidates.next().is_none().then_some(found)
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
                    // Herdr reports no session ID for a pane until the agent's
                    // integration does: seconds after `claude --resume`, never for
                    // an agent stopped at a startup prompt, and on older Codex not
                    // before the first turn. A pane this integration started
                    // carries the session ID in its agent name instead.
                    None => agent.name.as_deref() == Some(plan.agent_name.as_str()),
                }
        })
        .collect()
}

/// [`live_sessions`] as the browser asks for it: the host's view and the
/// process table, gathered together, as the one priority group [`RUNNING`].
///
/// The two questions are independent — one is a handful of commands to Herdr,
/// the other a scan of the process table — so the scan runs while the
/// commands do rather than after them. A host that cannot answer prioritizes
/// nothing; it does not make the browser fail.
///
/// [`RUNNING`]: integration::RUNNING
pub fn live_priorities(
    host: &mut dyn HostRuntime,
    sessions: &[Session],
) -> agent_history_core::Priorities {
    live_priorities_with(host, sessions, live::running_sessions)
}

/// [`live_priorities`] with the process scan given rather than run, so that a
/// test can say what this machine is serving without starting anything.
pub fn live_priorities_with<S>(
    host: &mut dyn HostRuntime,
    sessions: &[Session],
    scan: S,
) -> agent_history_core::Priorities
where
    S: FnOnce() -> std::collections::HashSet<agent_history_core::SessionId> + Send,
{
    let (running, hosted) = std::thread::scope(|scope| {
        let scan = scope.spawn(scan);
        // Asked in this order so a failure costs no second command.
        let hosted = host.workspaces().and_then(|w| Ok((w, host.agents()?)));
        (scan.join().unwrap_or_default(), hosted)
    });
    let Ok((workspaces, agents)) = hosted else {
        return agent_history_core::Priorities::default();
    };
    live_sessions(sessions, &workspaces, &agents, &running)
        .into_iter()
        .map(|id| (id, integration::RUNNING))
        .collect()
}

/// Sessions for which Enter would show a running agent instead of starting
/// one: those a process on this machine is already serving, and those with
/// exactly one matching agent in the workspace open at the session's recorded
/// working directory, the same rules `resume_in_host` applies.
pub fn live_sessions(
    sessions: &[Session],
    workspaces: &[resume::WorkspaceRecord],
    agents: &[resume::LiveAgent],
    running: &std::collections::HashSet<agent_history_core::SessionId>,
) -> Vec<agent_history_core::SessionId> {
    sessions
        .iter()
        .filter(|session| {
            // A process serving the session is enough on its own: the pane it
            // belongs to may report a different ID, or lie outside Herdr.
            if running.contains(&session.id) {
                return true;
            }
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
            cwd: session.and_then(|s| s.cwd.clone()),
        }
    }

    fn nothing_running() -> std::collections::HashSet<SessionId> {
        std::collections::HashSet::new()
    }

    fn running(sessions: [&Session; 1]) -> std::collections::HashSet<SessionId> {
        sessions.iter().map(|s| s.id.clone()).collect()
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
                created: false,
            },
            WorkspaceRecord {
                id: "wb".into(),
                cwd: "/w/b".into(),
                root_pane_id: None,
                root_pane_occupied: false,
                created: false,
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
            &nothing_running(),
        );
        assert_eq!(live, [reported.id, named.id]);
    }

    /// Herdr names the session a pane started with, and Claude's ID moves on
    /// from there, so the host reports a live conversation as a stale ID in a
    /// pane it cannot connect to it.
    #[test]
    fn a_session_its_pane_no_longer_names_is_still_live() {
        let moved_on = session(1, "/w/a");
        let stale = session(9, "/w/a");
        let workspaces = [WorkspaceRecord {
            id: "wa".into(),
            cwd: "/w/a".into(),
            root_pane_id: None,
            root_pane_occupied: false,
            created: false,
        }];
        // The pane reports the ID it launched with, not the one it serves.
        let agents = [agent("1", "wa", Some(&stale), None)];
        let live = live_sessions(
            std::slice::from_ref(&moved_on),
            &workspaces,
            &agents,
            &running([&moved_on]),
        );
        assert_eq!(live, [moved_on.id]);
    }

    #[test]
    fn a_live_session_is_shown_rather_than_started_twice() {
        let moved_on = session(1, "/w/a");
        let stale = session(9, "/w/a");
        let mut host = TestHost {
            workspaces: vec![WorkspaceRecord {
                id: "wa".into(),
                cwd: "/w/a".into(),
                root_pane_id: Some("p1".into()),
                root_pane_occupied: true,
                created: false,
            }],
            agents: vec![agent("p1", "wa", Some(&stale), None)],
            effects: Vec::new(),
        };
        let outcome =
            resume_in_host_with(&mut host, &moved_on, |_| true, &running([&moved_on])).unwrap();
        assert_eq!(outcome, Resumed::Focused);
        assert_eq!(host.effects, ["focus workspace wa", "focus agent p1"]);
    }

    #[test]
    fn a_pane_that_cannot_be_told_apart_is_never_guessed_at() {
        let moved_on = session(1, "/w/a");
        let one = session(8, "/w/a");
        let other = session(9, "/w/a");
        let mut host = TestHost {
            workspaces: vec![WorkspaceRecord {
                id: "wa".into(),
                cwd: "/w/a".into(),
                root_pane_id: Some("p1".into()),
                root_pane_occupied: true,
                created: false,
            }],
            agents: vec![
                agent("p1", "wa", Some(&one), None),
                agent("p2", "wa", Some(&other), None),
            ],
            effects: Vec::new(),
        };
        let outcome =
            resume_in_host_with(&mut host, &moved_on, |_| true, &running([&moved_on])).unwrap();
        // Nothing started, and no pane picked at random.
        assert_eq!(outcome, Resumed::AlreadyRunning);
        assert_eq!(host.effects, ["focus workspace wa"]);
    }

    #[test]
    fn a_session_nothing_is_serving_is_still_started() {
        let idle = session(1, "/w/a");
        let mut host = TestHost {
            workspaces: vec![WorkspaceRecord {
                id: "wa".into(),
                cwd: "/w/a".into(),
                root_pane_id: Some("p1".into()),
                root_pane_occupied: true,
                created: false,
            }],
            agents: Vec::new(),
            effects: Vec::new(),
        };
        let outcome = resume_in_host_with(&mut host, &idle, |_| true, &nothing_running()).unwrap();
        assert_eq!(outcome, Resumed::Started);
        assert_eq!(host.effects, ["focus workspace wa", "start agent in wa"]);
    }

    /// The browser's whole priority query: what Herdr reports and what the
    /// process table shows, which are not the same set. A conversation a
    /// process is serving under an ID no pane reports is still running, and
    /// dropping the scan here would quietly lose it.
    #[test]
    fn the_priority_query_counts_both_what_the_host_reports_and_what_runs() {
        let reported = session(1, "/w/a");
        let unreported = session(2, "/w/b");
        let mut host = TestHost {
            workspaces: vec![WorkspaceRecord {
                id: "wa".into(),
                cwd: "/w/a".into(),
                root_pane_id: None,
                root_pane_occupied: false,
                created: false,
            }],
            agents: vec![agent("a", "wa", Some(&reported), None)],
            effects: Vec::new(),
        };
        let sessions = [reported.clone(), unreported.clone()];

        let both = live_priorities_with(&mut host, &sessions, || running([&unreported]));
        assert_eq!(both.group(&reported.id), Some(integration::RUNNING));
        assert_eq!(
            both.group(&unreported.id),
            Some(integration::RUNNING),
            "a process serving it is enough on its own"
        );

        // Nothing serving it, and no pane for it: only the reported one.
        let hosted_only = live_priorities_with(&mut host, &sessions, nothing_running);
        assert_eq!(hosted_only.len(), 1);
        assert_eq!(hosted_only.group(&reported.id), Some(integration::RUNNING));
    }

    /// A host that cannot be asked prioritizes nothing, and says so by
    /// leaving the results alone rather than by failing.
    #[test]
    fn an_unreachable_host_prioritizes_nothing() {
        struct Silent;
        impl HostRuntime for Silent {
            fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>> {
                Err(CoreError::Unsupported("herdr is not running".into()))
            }
            fn agents(&mut self) -> Result<Vec<LiveAgent>> {
                unreachable!("a failed first command costs no second one")
            }
            fn focus_workspace(&mut self, _: &str) -> Result<()> {
                unreachable!()
            }
            fn focus_agent(&mut self, _: &str) -> Result<()> {
                unreachable!()
            }
            fn open_workspace(&mut self, _: &Path) -> Result<WorkspaceRecord> {
                unreachable!()
            }
            fn start_agent(
                &mut self,
                _: &WorkspaceRecord,
                _: &Session,
                _: &resume::NativeResumePlan,
            ) -> Result<()> {
                unreachable!()
            }
        }
        let session = session(1, "/w/a");
        let priorities = live_priorities_with(&mut Silent, std::slice::from_ref(&session), || {
            running([&session])
        });
        assert!(priorities.is_empty());
    }

    struct TestHost {
        workspaces: Vec<WorkspaceRecord>,
        agents: Vec<LiveAgent>,
        effects: Vec<String>,
    }

    impl HostRuntime for TestHost {
        fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>> {
            Ok(self.workspaces.clone())
        }
        fn agents(&mut self) -> Result<Vec<LiveAgent>> {
            Ok(self.agents.clone())
        }
        fn focus_workspace(&mut self, id: &str) -> Result<()> {
            self.effects.push(format!("focus workspace {id}"));
            Ok(())
        }
        fn focus_agent(&mut self, id: &str) -> Result<()> {
            self.effects.push(format!("focus agent {id}"));
            Ok(())
        }
        fn open_workspace(&mut self, _: &Path) -> Result<WorkspaceRecord> {
            unreachable!("every test opens a workspace that already exists")
        }
        fn start_agent(
            &mut self,
            workspace: &WorkspaceRecord,
            _: &Session,
            _: &resume::NativeResumePlan,
        ) -> Result<()> {
            self.effects
                .push(format!("start agent in {}", workspace.id));
            Ok(())
        }
    }
}

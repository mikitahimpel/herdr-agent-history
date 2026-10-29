//! Rex-specific actions for the shared Agent History browser.
use crate::{resume_in_rex, RexHost, BLOCKED, DONE, ERROR, IDLE, RUNNING, WORKING};
use agent_history_core::{
    availability::Availability, preview::preview_source, CoreError, Priorities, Result,
    ResultPrioritizer, Session, SqliteStore,
};
use agent_history_restore::{already_running_notice, restore, resumed_notice, Resumed};
use agent_history_tui::{
    BackgroundPrioritizer, BrowserState, Integration, Key, Mode, SessionState,
};
use std::sync::Arc;

/// The browser's title. Rex users know they are in Rex, so it is the
/// product's name alone.
pub const TITLE: &str = "Agent History";

/// Opens a separate host for the priority query the browser runs on a
/// worker thread, so neither the keystrokes nor that query wait on the other.
pub type HostFactory = Arc<dyn Fn() -> Box<dyn RexHost + Send> + Send + Sync>;

/// Owns restoration choices and Rex side effects. Search, selection, and
/// rendering remain in `agent-history-tui`.
pub struct RexIntegration<H> {
    host: H,
    recovery: Vec<restore::RecoveryChoice>,
    confirmation_text: Option<String>,
    background_host: Option<HostFactory>,
}

impl<H> RexIntegration<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            recovery: Vec::new(),
            confirmation_text: None,
            background_host: None,
        }
    }

    /// Lets the browser ask Rex which sessions are running on a worker
    /// thread. `rex-agent list` asks Rex about every block, which takes long
    /// enough that the browser should not wait for it to open.
    pub fn with_background_host(
        mut self,
        open: impl Fn() -> Box<dyn RexHost + Send> + Send + Sync + 'static,
    ) -> Self {
        self.background_host = Some(Arc::new(open));
        self
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    fn session(&self, state: &BrowserState, store: &SqliteStore) -> Result<Session> {
        let result = state
            .selected_result()
            .ok_or_else(|| CoreError::Unsupported("no session selected".into()))?;
        preview_source(store, &result.source, 0)?;
        let session = agent_history_core::preview::session_for_source(store, &result.source)?;
        if session.id != result.session_id {
            return Err(CoreError::Unsupported(
                "selected session metadata is stale".into(),
            ));
        }
        Ok(session)
    }

    fn resume(&mut self, state: &mut BrowserState, store: &SqliteStore) -> Result<()>
    where
        H: RexHost,
    {
        let session = match self.session(state, store) {
            Err(CoreError::StaleSource) => {
                // Usually a running agent that has written since the last scan.
                // Nothing is done to the host until the index has caught up.
                state.reindex_wanted = true;
                state.notice =
                    Some("Reading this conversation again; press Enter in a moment".into());
                return Ok(());
            }
            session => session?,
        };
        if !session.cwd.as_ref().is_some_and(|p| p.is_dir()) {
            self.recovery = restore::recovery_options(
                &session,
                session.repository_root.as_ref().is_some_and(|p| p.is_dir()),
                session.worktree.as_ref().is_some_and(|p| p.exists()),
            )
            .choices;
            state.mode = Mode::Action;
            return Ok(());
        }
        self.resume_session(state, store, &session)
    }

    /// The browser floats over the window it was opened from, so it closes once
    /// the session is open or focused elsewhere. It stays open when it has
    /// something to show: the conversation of a session it couldn't focus, or a
    /// Rex warning such as the app not switching to the new session.
    fn resume_session(
        &mut self,
        state: &mut BrowserState,
        store: &SqliteStore,
        session: &Session,
    ) -> Result<()>
    where
        H: RexHost,
    {
        let outcome = resume_in_rex(&mut self.host, session)?;
        let shown_here = outcome == Resumed::AlreadyRunning;
        let notice = match outcome {
            Resumed::AlreadyRunning => {
                state.show_preview(store)?;
                already_running_notice(session)
            }
            outcome => resumed_notice(session, outcome),
        };
        let warning = self.host.take_warning();
        state.closed = warning.is_none() && !shown_here;
        state.notice = Some(match warning {
            Some(warning) => format!("{notice}. Rex: {warning}"),
            None => notice,
        });
        Ok(())
    }
}

/// Surfaces sessions whose agent is running, ahead of relevance order.
impl<H: RexHost> ResultPrioritizer for RexIntegration<H> {
    /// A failed query only means nothing is prioritized. Enter does not rely
    /// on the answer: resuming asks Rex again.
    fn prioritize(&mut self, sessions: &[Session]) -> Priorities {
        crate::live_priorities(&mut self.host, sessions)
    }
}

impl<H: RexHost> Integration for RexIntegration<H> {
    fn title(&self) -> &str {
        TITLE
    }

    fn enter_label(&self) -> &str {
        "Resume"
    }

    /// The browser floats over a Rex window; leaving it closes the layer.
    fn close_label(&self) -> &str {
        "close"
    }

    /// Spelled as a Mac keyboard and its menus spell Control.
    fn key_name(&self, key: &'static str) -> &'static str {
        match key {
            "^C" => "⌃C",
            key => key,
        }
    }

    fn preview_enter_label(&self) -> Option<&str> {
        Some("Resume")
    }

    fn prioritizer(&mut self) -> Option<&mut dyn ResultPrioritizer> {
        Some(self)
    }

    fn background_prioritizer(&mut self) -> Option<BackgroundPrioritizer> {
        let open = self.background_host.clone()?;
        Some(Box::new(move |sessions| {
            crate::live_priorities(open().as_mut(), sessions)
        }))
    }

    fn availability_label(&self, state: SessionState) -> &str {
        match state {
            SessionState::Prioritized(BLOCKED) => "blocked",
            SessionState::Prioritized(ERROR) => "error",
            SessionState::Prioritized(DONE) => "done",
            SessionState::Prioritized(WORKING) => "working",
            SessionState::Prioritized(IDLE) => "idle",
            SessionState::Prioritized(RUNNING) | SessionState::Prioritized(_) => "running",
            SessionState::Stored(Availability::OnDisk) => "resumable",
            SessionState::Stored(Availability::Recoverable) => "recoverable",
            SessionState::Stored(Availability::RepositoryKnown) => "repo gone",
            SessionState::Stored(Availability::TranscriptOnly) => "transcript",
        }
    }

    fn action_lines(&self) -> Vec<String> {
        let mut lines = vec!["The session's directory no longer exists.".to_string()];
        if let Some(text) = &self.confirmation_text {
            lines.push(text.clone());
            return lines;
        }
        if self
            .recovery
            .contains(&restore::RecoveryChoice::RecreateWorktree)
        {
            lines.push("w — Recreate worktree and resume (confirmation follows)".into());
        }
        if self
            .recovery
            .contains(&restore::RecoveryChoice::ExistingRepository)
        {
            lines.push("r — Resume in the repository".into());
        }
        lines.push("v — View original conversation    c/Esc — Cancel".into());
        lines
    }

    fn handle(&mut self, key: Key, state: &mut BrowserState, store: &SqliteStore) -> Result<bool> {
        state.error = None;
        match state.mode {
            Mode::Action => match key {
                Key::Char('y') if self.confirmation_text.is_some() => {
                    let session = self.session(state, store)?;
                    let plan = restore::plan_recreate(&session)?;
                    restore::execute_recreate(&plan, restore::Confirmation::confirmed())?;
                    self.confirmation_text = None;
                    self.resume(state, store)?;
                    Ok(true)
                }
                Key::Char('n') | Key::Esc if self.confirmation_text.is_some() => {
                    self.confirmation_text = None;
                    Ok(true)
                }
                _ if self.confirmation_text.is_some() => Ok(true),
                Key::Char('w')
                    if self
                        .recovery
                        .contains(&restore::RecoveryChoice::RecreateWorktree) =>
                {
                    let session = self.session(state, store)?;
                    let plan = restore::plan_recreate(&session)?;
                    self.confirmation_text = Some(format!(
                        "Create {} at commit {}? y = confirm, n/Esc = back",
                        plan.worktree.display(),
                        session.commit.as_deref().unwrap_or("unknown")
                    ));
                    Ok(true)
                }
                Key::Char('r')
                    if self
                        .recovery
                        .contains(&restore::RecoveryChoice::ExistingRepository) =>
                {
                    let mut session = self.session(state, store)?;
                    session.cwd = session.repository_root.clone();
                    state.mode = Mode::Results;
                    self.resume_session(state, store, &session)?;
                    Ok(true)
                }
                Key::Char('v') => {
                    state.show_preview(store)?;
                    Ok(true)
                }
                Key::Char('c') | Key::Esc => {
                    state.mode = Mode::Results;
                    self.confirmation_text = None;
                    Ok(true)
                }
                _ => Ok(true),
            },
            Mode::Preview | Mode::Query | Mode::Results if matches!(key, Key::Enter) => {
                self.resume(state, store)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentState, Opened, RexAgent, RexSession};
    use agent_history_core::{
        adapters::ClaudeAdapter, git_command, index::index_all, test_support::TempDir, AgentAdapter,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    #[derive(Clone, Default)]
    struct Host {
        agents: Vec<RexAgent>,
        effects: Vec<String>,
        opened: Vec<(PathBuf, Vec<String>)>,
        warning: Option<String>,
        working_here: Option<RexSession>,
    }

    impl RexHost for Host {
        fn agents(&mut self) -> Result<Vec<RexAgent>> {
            self.effects.push("list".into());
            Ok(self.agents.clone())
        }
        fn focus(&mut self, block_id: &str) -> Result<()> {
            self.effects.push(format!("focus {block_id}"));
            Ok(())
        }
        fn session_for(&mut self, _: &Path) -> Result<Option<RexSession>> {
            self.effects.push("session-for".into());
            Ok(self.working_here.clone())
        }
        fn open(
            &mut self,
            session: Option<&RexSession>,
            cwd: &Path,
            _: &str,
            argv: &[String],
        ) -> Result<Opened> {
            self.effects.push(match session {
                Some(session) => format!("open in {}", session.session_id),
                None => "open".into(),
            });
            self.opened.push((cwd.into(), argv.to_vec()));
            Ok(Opened {
                rex_session_id: "session:n".into(),
                block_id: "block:n".into(),
            })
        }
        fn take_warning(&mut self) -> Option<String> {
            self.warning.take()
        }
    }

    fn temp() -> (TempDir, PathBuf) {
        let t = TempDir::new("rex-integration").unwrap();
        let root = t.path().canonicalize().unwrap();
        (t, root)
    }

    fn fixture(root: &Path, cwd: &Path) -> SqliteStore {
        let histories = root.join("histories");
        fs::create_dir(&histories).unwrap();
        fs::write(
            histories.join("native.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::json!({"type":"user","sessionId":"00000000-0000-4000-8000-000000000001","cwd":cwd,"message":{"content":"portfolio visibility original question"}}),
                serde_json::json!({"type":"assistant","message":{"content":"full original answer"}})
            ),
        )
        .unwrap();
        let mut db = SqliteStore::open(root.join("private/index.sqlite")).unwrap();
        let adapters: Vec<Box<dyn AgentAdapter>> =
            vec![Box::new(ClaudeAdapter::with_root(histories))];
        index_all(&mut db, &adapters).unwrap();
        db
    }

    fn search(state: &mut BrowserState, db: &SqliteStore) {
        for c in "portfolio visibility".chars() {
            state
                .handle(if c == ' ' { Key::Space } else { Key::Char(c) }, db)
                .unwrap();
        }
    }

    fn running(db: &SqliteStore, cwd: &Path) -> Host {
        let session = db.sessions().unwrap().remove(0);
        Host {
            agents: vec![RexAgent {
                rex_session_id: "session:a".into(),
                block_id: "block:a".into(),
                session: Some(session.id),
                cwd: Some(cwd.into()),
                state: Some(AgentState::Working),
            }],
            ..Host::default()
        }
    }

    #[test]
    fn enter_resumes_in_a_new_rex_session_and_closes_the_browser() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        search(&mut s, &db);
        assert!(i.handle(Key::Enter, &mut s, &db).unwrap());
        assert!(s.closed);
        assert_eq!(i.host().effects, ["list", "session-for", "open"]);
        assert_eq!(
            i.host().opened[0],
            (
                root.clone(),
                vec![
                    "claude".to_string(),
                    "--resume".into(),
                    "00000000-0000-4000-8000-000000000001".into()
                ]
            )
        );
        assert!(s.notice.as_deref().is_some_and(|n| n.contains("Resumed")));
        assert_eq!(s.query, "portfolio visibility");
    }

    #[test]
    fn enter_on_a_running_session_focuses_its_pane_and_closes_the_browser() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(running(&db, &root));
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        assert_eq!(i.host().effects, ["list", "focus block:a"]);
        assert!(s.closed);
    }

    #[test]
    fn enter_on_a_running_session_focuses_its_pane_and_stays_open_to_show_rex_warnings() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host {
            warning: Some("Remote Control is off".into()),
            ..running(&db, &root)
        });
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        assert_eq!(i.host().effects, ["list", "focus block:a"]);
        assert!(!s.closed);
        let notice = s.notice.clone().unwrap();
        assert!(notice.contains("Opened running"), "{notice}");
        assert!(notice.contains("Remote Control is off"), "{notice}");
    }

    #[test]
    fn running_sessions_are_prioritized_in_the_background_as_in_place() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let sessions = db.sessions().unwrap();
        let host = running(&db, &root);
        let mut i =
            RexIntegration::new(host.clone()).with_background_host(move || Box::new(host.clone()));
        let direct = i.prioritize(&sessions);
        assert_eq!(direct.group(&sessions[0].id), Some(WORKING));
        let mut background = i.background_prioritizer().unwrap();
        assert_eq!(background(&sessions), direct);
        let mut s = BrowserState::default();
        s.set_priorities(direct, &db);
        search(&mut s, &db);
        let state = s.availability.state(&sessions[0].id).unwrap();
        assert_eq!(i.availability_label(state), "working");
    }

    #[test]
    fn markers_name_the_rex_state_or_plain_running() {
        let i = RexIntegration::new(Host::default());
        let label = |group| i.availability_label(SessionState::Prioritized(group));
        assert_eq!(
            [BLOCKED, ERROR, DONE, WORKING, IDLE, RUNNING].map(label),
            ["blocked", "error", "done", "working", "idle", "running"]
        );
    }

    #[test]
    fn the_rex_ui_says_nothing_of_herdr() {
        let i = RexIntegration::new(Host::default());
        let mut text = vec![i.title().to_string(), i.enter_label().to_string()];
        text.extend(i.action_lines());
        for group in 0..=RUNNING + 1 {
            text.push(
                i.availability_label(SessionState::Prioritized(group))
                    .into(),
            );
        }
        for text in text {
            assert!(!text.to_lowercase().contains("herdr"), "{text}");
            assert!(!text.to_lowercase().contains("workspace"), "{text}");
        }
    }

    #[test]
    fn esc_in_the_search_box_closes_the_browser() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        assert_eq!(s.mode, Mode::Query);
        assert!(
            !i.handle(Key::Esc, &mut s, &db).unwrap(),
            "left to the browser"
        );
        s.handle(Key::Esc, &db).unwrap();
        assert!(s.closed);
        assert!(i.host().effects.is_empty());
    }

    #[test]
    fn enter_resumes_as_a_tab_of_the_rex_session_working_in_the_directory() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host {
            working_here: Some(RexSession {
                session_id: "session:web".into(),
                label: "web".into(),
            }),
            ..Host::default()
        });
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        assert_eq!(
            i.host().effects,
            ["list", "session-for", "open in session:web"]
        );
        assert_eq!(i.host().opened[0].0, root);
        assert!(s.closed);
    }

    #[test]
    fn a_conversation_written_since_the_scan_has_no_host_effect() {
        let (_t, root) = temp();
        let db = fixture(&root, &root);
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        search(&mut s, &db);
        fs::write(root.join("histories/native.jsonl"), "replacement\n").unwrap();
        assert!(i.handle(Key::Enter, &mut s, &db).unwrap());
        assert!(s.reindex_wanted);
        assert!(i.host().effects.is_empty());
    }

    #[test]
    fn missing_workspace_view_and_cancel_have_no_host_effects() {
        let (_t, root) = temp();
        let db = fixture(&root, &root.join("missing"));
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Action);
        i.handle(Key::Char('v'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Preview);
        s.handle(Key::Esc, &db).unwrap();
        i.handle(Key::Char('c'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Results);
        assert!(i.host().effects.is_empty());
    }

    fn linked(root: &Path) -> (PathBuf, PathBuf) {
        let repo = root.join("repo");
        fs::create_dir(&repo).unwrap();
        let git = |a: &[&str]| assert!(git_command(&repo).args(a).status().unwrap().success());
        git(&["init", "--quiet"]);
        fs::write(repo.join("tracked"), "preserved").unwrap();
        git(&["add", "tracked"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
            "--quiet",
        ]);
        let target = root.join("linked");
        git(&[
            "worktree",
            "add",
            "--detach",
            target.to_str().unwrap(),
            "HEAD",
        ]);
        (repo, target)
    }

    fn in_web() -> Host {
        Host {
            working_here: Some(RexSession {
                session_id: "session:web".into(),
                label: "web".into(),
            }),
            ..Host::default()
        }
    }

    #[test]
    fn recovery_resumes_in_the_rex_session_working_in_the_directory() {
        let (_t, root) = temp();
        let (repo, target) = linked(&root);
        let db = fixture(&root, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(in_web());
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        i.handle(Key::Char('r'), &mut s, &db).unwrap();
        assert_eq!(
            i.host().effects,
            ["list", "session-for", "open in session:web"]
        );
        assert_eq!(i.host().opened[0].0, repo);

        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(in_web());
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        i.handle(Key::Char('w'), &mut s, &db).unwrap();
        i.handle(Key::Char('y'), &mut s, &db).unwrap();
        assert_eq!(
            i.host().effects,
            ["list", "session-for", "open in session:web"]
        );
        assert_eq!(i.host().opened[0].0, target);
    }

    #[test]
    fn missing_worktree_resumes_in_the_repository() {
        let (_t, root) = temp();
        let (repo, target) = linked(&root);
        let db = fixture(&root, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        i.handle(Key::Char('r'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Results);
        assert_eq!(i.host().effects, ["list", "session-for", "open"]);
        assert_eq!(i.host().opened[0].0, repo);
        assert!(!target.exists());
    }

    #[test]
    fn recreating_a_worktree_needs_yes_then_opens_it_in_rex() {
        let (_t, root) = temp();
        let (_repo, target) = linked(&root);
        let db = fixture(&root, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut s = BrowserState::default();
        let mut i = RexIntegration::new(Host::default());
        search(&mut s, &db);
        i.handle(Key::Enter, &mut s, &db).unwrap();
        i.handle(Key::Char('w'), &mut s, &db).unwrap();
        assert!(i
            .action_lines()
            .iter()
            .any(|l| l.contains(target.to_str().unwrap())));
        // Recovery keys are ignored until the question is answered.
        i.handle(Key::Char('r'), &mut s, &db).unwrap();
        i.handle(Key::Char('n'), &mut s, &db).unwrap();
        assert!(!target.exists());
        assert!(i.host().effects.is_empty());
        i.handle(Key::Char('w'), &mut s, &db).unwrap();
        i.handle(Key::Char('y'), &mut s, &db).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("tracked")).unwrap(),
            "preserved"
        );
        assert_eq!(i.host().effects, ["list", "session-for", "open"]);
        assert_eq!(i.host().opened[0].0, target);
        assert!(s.notice.as_deref().is_some_and(|n| n.contains("Resumed")));
    }
}

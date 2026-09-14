//! Terminal-independent interaction controller; effects are exercised with a mock host.
use crate::{
    restore::{self, Confirmation, RecoveryChoice},
    resume_in_host, HostRuntime,
};
use agent_history_core::{
    preview::preview_source, CoreError, IndexStore, Result, SearchResult, Session, Store,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayKey {
    Char(char),
    Backspace,
    Up,
    Down,
    Space,
    Enter,
    Esc,
    Tab,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Query,
    Results,
    Preview,
    Recovery,
    ConfirmRecreate,
}
#[derive(Clone, Debug, Default)]
pub struct OverlayState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected: usize,
    pub mode: Mode,
    pub preview: String,
    pub preview_scroll: usize,
    pub confirmation_text: String,
    pub error: Option<String>,
    pub closed: bool,
    pub recovery: Vec<RecoveryChoice>,
    pub status: String,
}
impl OverlayState {
    pub fn refresh<S: Store>(&mut self, store: &S) {
        match store.search(&self.query, 50) {
            Ok(items) => {
                self.results = items;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
                self.error = None
            }
            Err(e) => {
                self.results.clear();
                self.error = Some(e.to_string())
            }
        }
    }
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }
    fn selected_source(&self) -> Result<&SearchResult> {
        self.selected_result()
            .ok_or_else(|| CoreError::Unsupported("no session selected".into()))
    }
    fn show_preview<S: IndexStore>(&mut self, store: &S) -> Result<()> {
        let result = self.selected_source()?;
        let p = preview_source(store, &result.source, 64 * 1024)?;
        self.preview = p.text;
        self.preview_scroll = 0;
        if p.truncated_before || p.truncated_after {
            self.preview
                .push_str("\n\n[Surrounding context is limited]")
        }
        self.mode = Mode::Preview;
        Ok(())
    }
    fn session<S: IndexStore>(&self, store: &S) -> Result<Session> {
        let result = self.selected_source()?;
        preview_source(store, &result.source, 0)?;
        let session = agent_history_core::preview::session_for_source(store, &result.source)?;
        if session.id != result.session_id {
            return Err(CoreError::Unsupported(
                "selected session metadata is stale".into(),
            ));
        }
        Ok(session)
    }
    fn resume<S: IndexStore, H: HostRuntime>(&mut self, store: &S, host: &mut H) -> Result<()> {
        let session = self.session(store)?;
        if !session.cwd.as_ref().is_some_and(|p| p.is_dir()) {
            self.recovery = restore::recovery_options(
                &session,
                session.repository_root.as_ref().is_some_and(|p| p.is_dir()),
                session.worktree.as_ref().is_some_and(|p| p.exists()),
            )
            .choices;
            self.mode = Mode::Recovery;
            return Ok(());
        }
        resume_in_host(host, &session)?;
        self.closed = true;
        Ok(())
    }
    pub fn handle<S: IndexStore, H: HostRuntime>(
        &mut self,
        key: OverlayKey,
        store: &S,
        host: &mut H,
    ) {
        self.error = None;
        if let Err(e) = self.dispatch(key, store, host) {
            self.error = Some(e.to_string())
        }
    }
    fn dispatch<S: IndexStore, H: HostRuntime>(
        &mut self,
        key: OverlayKey,
        store: &S,
        host: &mut H,
    ) -> Result<()> {
        match self.mode {
            Mode::ConfirmRecreate => {
                if matches!(key, OverlayKey::Char('y') | OverlayKey::Char('Y')) {
                    let session = self.session(store)?;
                    let plan = restore::plan_recreate(&session)?;
                    restore::execute_recreate(&plan, Confirmation::confirmed())?;
                    self.resume(store, host)?;
                } else if matches!(
                    key,
                    OverlayKey::Esc | OverlayKey::Char('n') | OverlayKey::Char('N')
                ) {
                    self.mode = Mode::Recovery
                }
                return Ok(());
            }
            Mode::Recovery => {
                match key {
                    OverlayKey::Char('w')
                        if self.recovery.contains(&RecoveryChoice::RecreateWorktree) =>
                    {
                        let session = self.session(store)?;
                        let plan = restore::plan_recreate(&session)?;
                        self.confirmation_text = format!(
                            "Create {} at commit {}",
                            plan.worktree.display(),
                            session.commit.as_deref().unwrap_or("unknown")
                        );
                        self.mode = Mode::ConfirmRecreate
                    }
                    OverlayKey::Char('r')
                        if self.recovery.contains(&RecoveryChoice::ExistingRepository) =>
                    {
                        let mut session = self.session(store)?;
                        session.cwd = session.repository_root.clone();
                        resume_in_host(host, &session)?;
                        self.closed = true;
                    }
                    OverlayKey::Char('v') => self.show_preview(store)?,
                    OverlayKey::Esc | OverlayKey::Char('c') => self.mode = Mode::Results,
                    _ => {}
                }
                return Ok(());
            }
            Mode::Preview => {
                match key {
                    OverlayKey::Esc => self.mode = Mode::Results,
                    OverlayKey::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                    OverlayKey::Down => {
                        self.preview_scroll = (self.preview_scroll + 1)
                            .min(self.preview.lines().count().saturating_sub(1))
                    }
                    OverlayKey::Enter => self.resume(store, host)?,
                    _ => {}
                }
                return Ok(());
            }
            _ => {}
        }
        match key {
            OverlayKey::Esc if self.mode == Mode::Results => self.mode = Mode::Query,
            OverlayKey::Esc => self.closed = true,
            OverlayKey::Down => {
                if self.mode == Mode::Results {
                    self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1))
                }
                self.mode = Mode::Results;
            }
            OverlayKey::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.mode = Mode::Results;
            }
            OverlayKey::Tab => {
                self.mode = if self.mode == Mode::Query {
                    Mode::Results
                } else {
                    Mode::Query
                }
            }
            OverlayKey::Space if self.mode == Mode::Results => self.show_preview(store)?,
            OverlayKey::Space => {
                self.query.push(' ');
                self.refresh(store)
            }
            OverlayKey::Char(c) => {
                self.mode = Mode::Query;
                self.query.push(c);
                self.refresh(store)
            }
            OverlayKey::Backspace => {
                self.mode = Mode::Query;
                self.query.pop();
                self.refresh(store)
            }
            OverlayKey::Enter => self.resume(store, host)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resume::{LiveAgent, NativeResumePlan, WorkspaceRecord};
    use agent_history_core::{
        adapters::ClaudeAdapter, index::index_all, AgentAdapter, SqliteStore,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "history-overlay-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[derive(Default)]
    struct Host {
        effects: Vec<String>,
        session: Option<Session>,
    }
    impl HostRuntime for Host {
        fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>> {
            self.effects.push("list".into());
            Ok(vec![])
        }
        fn agents(&mut self) -> Result<Vec<LiveAgent>> {
            Ok(vec![])
        }
        fn focus_workspace(&mut self, _: &str) -> Result<()> {
            self.effects.push("focus".into());
            Ok(())
        }
        fn focus_agent(&mut self, _: &str) -> Result<()> {
            self.effects.push("focus agent".into());
            Ok(())
        }
        fn open_workspace(&mut self, cwd: &Path) -> Result<WorkspaceRecord> {
            self.effects.push("open".into());
            Ok(WorkspaceRecord {
                id: "w".into(),
                cwd: cwd.into(),
                root_pane_id: None,
                root_pane_occupied: false,
            })
        }
        fn start_agent(
            &mut self,
            _: &WorkspaceRecord,
            s: &Session,
            plan: &NativeResumePlan,
        ) -> Result<()> {
            assert_eq!(plan.argv.last(), Some(&s.id.native_id));
            self.effects.push("resume".into());
            self.session = Some(s.clone());
            Ok(())
        }
    }
    fn fixture(d: &Temp, cwd: &Path) -> (SqliteStore, PathBuf) {
        let root = d.0.join("histories");
        fs::create_dir(&root).unwrap();
        let source = root.join("native.jsonl");
        fs::write(&source,format!("{}\n{}\n",serde_json::json!({"type":"user","sessionId":"00000000-0000-4000-8000-000000000001","cwd":cwd,"message":{"content":"portfolio visibility original question"}}),serde_json::json!({"type":"assistant","message":{"content":"full original answer not protocol"}}))).unwrap();
        let mut db = SqliteStore::open(d.0.join("private/index.sqlite")).unwrap();
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![Box::new(ClaudeAdapter::with_root(root))];
        assert_eq!(index_all(&mut db, &adapters).unwrap().failed_files, 0);
        (db, source)
    }
    fn search(s: &mut OverlayState, db: &SqliteStore, host: &mut Host) {
        for c in "portfolio visibility".chars() {
            s.handle(
                if c == ' ' {
                    OverlayKey::Space
                } else {
                    OverlayKey::Char(c)
                },
                db,
                host,
            )
        }
    }
    #[test]
    fn search_preview_resume_uses_original_source_and_preserves_query() {
        let d = Temp::new();
        let (db, _) = fixture(&d, &d.0);
        let mut host = Host::default();
        let mut state = OverlayState::default();
        search(&mut state, &db, &mut host);
        assert_eq!(state.query, "portfolio visibility");
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.mode, Mode::Query);
        state.handle(OverlayKey::Down, &db, &mut host);
        state.handle(OverlayKey::Space, &db, &mut host);
        assert_eq!(state.mode, Mode::Preview);
        assert!(state.preview.contains("User: portfolio"));
        assert!(state.preview.contains("Assistant: full original answer"));
        assert!(!state.preview.contains("sessionId"));
        assert!(host.effects.is_empty());
        state.handle(OverlayKey::Esc, &db, &mut host);
        assert_eq!(state.mode, Mode::Results);
        assert_eq!(state.query, "portfolio visibility");
        state.handle(OverlayKey::Enter, &db, &mut host);
        assert!(state.closed);
        assert_eq!(host.effects, ["list", "open", "resume"]);
        assert_eq!(host.session.unwrap().cwd, Some(d.0.clone()));
    }
    #[test]
    fn replacement_blocks_resume_before_any_host_effect() {
        let d = Temp::new();
        let (db, source) = fixture(&d, &d.0);
        let mut state = OverlayState::default();
        let mut host = Host::default();
        search(&mut state, &db, &mut host);
        fs::write(source, "replacement\n").unwrap();
        state.handle(OverlayKey::Enter, &db, &mut host);
        assert!(state.error.is_some());
        assert!(!state.closed);
        assert!(host.effects.is_empty());
    }
    #[test]
    fn missing_workspace_view_and_cancel_have_no_host_effects() {
        let d = Temp::new();
        let (db, _) = fixture(&d, &d.0.join("missing"));
        let mut state = OverlayState::default();
        let mut host = Host::default();
        search(&mut state, &db, &mut host);
        state.handle(OverlayKey::Enter, &db, &mut host);
        assert_eq!(state.mode, Mode::Recovery);
        assert_eq!(
            state.recovery,
            [RecoveryChoice::ViewConversation, RecoveryChoice::Cancel]
        );
        state.handle(OverlayKey::Char('v'), &db, &mut host);
        assert_eq!(state.mode, Mode::Preview);
        state.handle(OverlayKey::Esc, &db, &mut host);
        state.handle(OverlayKey::Enter, &db, &mut host);
        state.handle(OverlayKey::Char('c'), &db, &mut host);
        assert_eq!(state.mode, Mode::Results);
        assert!(host.effects.is_empty());
    }
    #[test]
    fn declining_confirmation_never_creates_a_target() {
        let d = Temp::new();
        let (db, _) = fixture(&d, &d.0.join("missing"));
        let mut state = OverlayState {
            mode: Mode::ConfirmRecreate,
            ..Default::default()
        };
        let mut host = Host::default();
        state.handle(OverlayKey::Char('n'), &db, &mut host);
        assert_eq!(state.mode, Mode::Recovery);
        assert!(!d.0.join("missing").exists());
        assert!(host.effects.is_empty());
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn linked(d: &Temp) -> (PathBuf, PathBuf) {
        let root = d.0.join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        fs::write(root.join("tracked"), "preserved").unwrap();
        git(&root, &["add", "tracked"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "fixture",
                "--quiet",
            ],
        );
        let target = d.0.join("linked");
        git(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                target.to_str().unwrap(),
                "HEAD",
            ],
        );
        (root, target)
    }
    #[test]
    fn deleted_registered_worktree_requires_yes_then_resumes_without_changing_main() {
        let d = Temp::new();
        let (root, target) = linked(&d);
        let before = git(&root, &["rev-parse", "HEAD"]);
        let (db, _) = fixture(&d, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut state = OverlayState::default();
        let mut host = Host::default();
        search(&mut state, &db, &mut host);
        state.handle(OverlayKey::Enter, &db, &mut host);
        assert!(state.recovery.contains(&RecoveryChoice::RecreateWorktree));
        state.handle(OverlayKey::Char('w'), &db, &mut host);
        assert_eq!(state.mode, Mode::ConfirmRecreate);
        assert!(state.confirmation_text.contains(target.to_str().unwrap()));
        assert!(!target.exists());
        state.handle(OverlayKey::Char('n'), &db, &mut host);
        assert!(!target.exists());
        assert!(host.effects.is_empty());
        state.handle(OverlayKey::Char('w'), &db, &mut host);
        state.handle(OverlayKey::Char('y'), &db, &mut host);
        assert!(state.error.is_none(), "{:?}", state.error);
        assert!(state.closed);
        assert_eq!(host.effects, ["list", "open", "resume"]);
        assert_eq!(
            fs::read_to_string(target.join("tracked")).unwrap(),
            "preserved"
        );
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), before);
        assert_eq!(
            fs::read_to_string(root.join("tracked")).unwrap(),
            "preserved"
        );
        assert_eq!(git(&target, &["rev-parse", "HEAD"]), before);
    }
    #[test]
    fn locked_registration_existing_target_and_missing_commit_are_not_mutated() {
        let d = Temp::new();
        let (root, target) = linked(&d);
        let (db, _) = fixture(&d, &target);
        let mut state = OverlayState::default();
        let mut host = Host::default();
        search(&mut state, &db, &mut host);
        let session = state.session(&db).unwrap();
        let plan = restore::plan_recreate(&session).unwrap();
        assert!(restore::execute_recreate(&plan, Confirmation::confirmed()).is_err());
        assert_eq!(
            fs::read_to_string(target.join("tracked")).unwrap(),
            "preserved"
        );
        git(&root, &["worktree", "lock", target.to_str().unwrap()]);
        fs::remove_dir_all(&target).unwrap();
        assert!(restore::execute_recreate(&plan, Confirmation::confirmed())
            .unwrap_err()
            .to_string()
            .contains("locked"));
        assert!(!target.exists());
        git(&root, &["worktree", "unlock", target.to_str().unwrap()]);
        let mut invalid = session.clone();
        invalid.commit = Some("f".repeat(40));
        assert!(restore::execute_recreate(
            &restore::plan_recreate(&invalid).unwrap(),
            Confirmation::confirmed()
        )
        .is_err());
        assert!(!target.exists());
        std::os::unix::fs::symlink(&root, &target).unwrap();
        assert!(restore::execute_recreate(&plan, Confirmation::confirmed()).is_err());
        assert_eq!(
            fs::read_to_string(root.join("tracked")).unwrap(),
            "preserved"
        );
    }
    #[test]
    fn missing_workspace_can_resume_in_recorded_existing_repository() {
        let d = Temp::new();
        let (root, target) = linked(&d);
        let (db, _) = fixture(&d, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut state = OverlayState::default();
        let mut host = Host::default();
        search(&mut state, &db, &mut host);
        state.handle(OverlayKey::Enter, &db, &mut host);
        state.handle(OverlayKey::Char('r'), &db, &mut host);
        assert!(state.error.is_none());
        assert!(state.closed);
        assert_eq!(host.session.unwrap().cwd, Some(root));
        assert!(!target.exists());
    }
}

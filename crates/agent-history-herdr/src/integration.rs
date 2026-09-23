//! Herdr-specific actions for the shared Agent History browser.
use crate::{restore, resume_in_host, HostRuntime};
use agent_history_core::{preview::preview_source, CoreError, Result, SqliteStore};
use agent_history_tui::{BrowserState, Integration, Key, Mode, Palette};

/// The Herdr adapter owns restoration choices and host side effects. Search,
/// selection, and rendering remain in `agent-history-tui`.
pub struct HerdrIntegration<H> {
    host: H,
    recovery: Vec<restore::RecoveryChoice>,
    confirmation_text: Option<String>,
    palette: Palette,
}

impl<H> HerdrIntegration<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            recovery: Vec::new(),
            confirmation_text: None,
            palette: Palette::terminal(),
        }
    }

    /// Colors the browser with `palette`, normally Herdr's configured theme
    /// from [`crate::theme::palette_from_env`].
    pub fn with_palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    fn session(
        &self,
        state: &BrowserState,
        store: &SqliteStore,
    ) -> Result<agent_history_core::Session> {
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
        H: HostRuntime,
    {
        let session = self.session(state, store)?;
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
        resume_in_host(&mut self.host, &session)?;
        state.closed = true;
        Ok(())
    }
}

impl<H: HostRuntime> Integration for HerdrIntegration<H> {
    fn title(&self) -> &str {
        "Agent History — Herdr"
    }

    fn enter_label(&self) -> &str {
        "Resume"
    }

    fn preview_enter_label(&self) -> Option<&str> {
        Some("Resume")
    }

    fn palette(&self) -> Palette {
        self.palette
    }

    fn action_lines(&self) -> Vec<String> {
        if let Some(text) = &self.confirmation_text {
            return vec![
                "The recorded workspace is unavailable.".into(),
                text.clone(),
            ];
        }
        let mut lines = vec!["The recorded workspace is unavailable.".into()];
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
            lines.push("r — Resume in existing repository".into());
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
                    resume_in_host(&mut self.host, &session)?;
                    state.closed = true;
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
            Mode::Preview if matches!(key, Key::Enter) => {
                self.resume(state, store)?;
                Ok(true)
            }
            Mode::Query | Mode::Results if matches!(key, Key::Enter) => {
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
    use agent_history_core::{
        adapters::ClaudeAdapter, git_command, index::index_all, AgentAdapter, Session, SqliteStore,
    };
    use agent_history_tui::{BrowserState, Key, Mode};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "herdr-adapter-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p.canonicalize().unwrap())
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
        fn workspaces(&mut self) -> Result<Vec<crate::resume::WorkspaceRecord>> {
            self.effects.push("list".into());
            Ok(vec![])
        }
        fn agents(&mut self) -> Result<Vec<crate::resume::LiveAgent>> {
            Ok(vec![])
        }
        fn focus_workspace(&mut self, _: &str) -> Result<()> {
            self.effects.push("focus".into());
            Ok(())
        }
        fn focus_agent(&mut self, _: &str) -> Result<()> {
            self.effects.push("agent focus".into());
            Ok(())
        }
        fn open_workspace(&mut self, cwd: &Path) -> Result<crate::resume::WorkspaceRecord> {
            self.effects.push("open".into());
            Ok(crate::resume::WorkspaceRecord {
                id: "w".into(),
                cwd: cwd.into(),
                root_pane_id: None,
                root_pane_occupied: false,
            })
        }
        fn start_agent(
            &mut self,
            _: &crate::resume::WorkspaceRecord,
            s: &Session,
            p: &crate::resume::NativeResumePlan,
        ) -> Result<()> {
            assert_eq!(p.argv.last(), Some(&s.id.native_id));
            self.effects.push("resume".into());
            self.session = Some(s.clone());
            Ok(())
        }
    }
    fn fixture(t: &Temp, cwd: &Path) -> (SqliteStore, PathBuf) {
        let root = t.0.join("histories");
        fs::create_dir(&root).unwrap();
        let source = root.join("native.jsonl");
        fs::write(&source, format!("{}\n{}\n", serde_json::json!({"type":"user","sessionId":"00000000-0000-4000-8000-000000000001","cwd":cwd,"message":{"content":"portfolio visibility original question"}}), serde_json::json!({"type":"assistant","message":{"content":"full original answer not protocol"}}))).unwrap();
        let mut db = SqliteStore::open(t.0.join("private/index.sqlite")).unwrap();
        let adapters: Vec<Box<dyn AgentAdapter>> = vec![Box::new(ClaudeAdapter::with_root(root))];
        index_all(&mut db, &adapters).unwrap();
        (db, source)
    }
    fn search(state: &mut BrowserState, db: &SqliteStore) {
        for c in "portfolio visibility".chars() {
            state
                .handle(if c == ' ' { Key::Space } else { Key::Char(c) }, db)
                .unwrap();
        }
    }
    fn enter(state: &mut BrowserState, db: &SqliteStore, integration: &mut HerdrIntegration<Host>) {
        integration.handle(Key::Enter, state, db).unwrap();
    }

    #[test]
    fn search_preview_resume_uses_original_source_and_preserves_query() {
        let t = Temp::new();
        let (db, _) = fixture(&t, &t.0);
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        s.handle(Key::Down, &db).unwrap();
        s.handle(Key::Space, &db).unwrap();
        assert_eq!(s.mode, Mode::Preview);
        assert!(s.preview.contains("User: portfolio"));
        s.handle(Key::Esc, &db).unwrap();
        enter(&mut s, &db, &mut i);
        assert!(s.closed);
        assert_eq!(s.query, "portfolio visibility");
    }
    #[test]
    fn replacement_blocks_resume_before_any_host_effect() {
        let t = Temp::new();
        let (db, source) = fixture(&t, &t.0);
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        fs::write(source, "replacement\n").unwrap();
        assert!(i.handle(Key::Enter, &mut s, &db).is_err());
        assert!(!s.closed);
        assert!(i.host().effects.is_empty());
    }
    #[test]
    fn missing_workspace_view_and_cancel_have_no_host_effects() {
        let t = Temp::new();
        let (db, _) = fixture(&t, &t.0.join("missing"));
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        enter(&mut s, &db, &mut i);
        assert_eq!(s.mode, Mode::Action);
        i.handle(Key::Char('v'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Preview);
        s.handle(Key::Esc, &db).unwrap();
        i.handle(Key::Char('c'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Results);
        assert!(i.host().effects.is_empty());
    }
    #[test]
    fn declining_confirmation_never_creates_a_target() {
        let t = Temp::new();
        let (db, _) = fixture(&t, &t.0.join("missing"));
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        enter(&mut s, &db, &mut i);
        i.confirmation_text = Some("confirm".into());
        i.handle(Key::Char('n'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Action);
        assert!(!t.0.join("missing").exists());
    }
    #[test]
    fn confirmation_ignores_recovery_keys_until_decided() {
        let t = Temp::new();
        let (db, _) = fixture(&t, &t.0.join("missing"));
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        enter(&mut s, &db, &mut i);
        i.confirmation_text = Some("confirm".into());
        i.handle(Key::Char('r'), &mut s, &db).unwrap();
        i.handle(Key::Char('v'), &mut s, &db).unwrap();
        assert_eq!(s.mode, Mode::Action);
        assert!(i.host().effects.is_empty());
    }
    fn linked(t: &Temp) -> (PathBuf, PathBuf) {
        let root = t.0.join("repo");
        fs::create_dir(&root).unwrap();
        // Fixtures use the same isolation as production: a suite run from a
        // Git hook inherits GIT_DIR, which overrides `-C` and would point these
        // commands at the surrounding repository.
        let git = |a: &[&str]| {
            assert!(git_command(&root).args(a).status().unwrap().success());
        };
        git(&["init", "--quiet"]);
        fs::write(root.join("tracked"), "preserved").unwrap();
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
        let target = t.0.join("linked");
        assert!(git_command(&root)
            .args([
                "worktree",
                "add",
                "--detach",
                target.to_str().unwrap(),
                "HEAD"
            ])
            .status()
            .unwrap()
            .success());
        (root, target)
    }
    #[test]
    fn missing_workspace_can_resume_in_recorded_existing_repository() {
        let t = Temp::new();
        let (root, target) = linked(&t);
        let (db, _) = fixture(&t, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        enter(&mut s, &db, &mut i);
        i.handle(Key::Char('r'), &mut s, &db).unwrap();
        assert!(s.closed);
        assert_eq!(i.host().effects, ["list", "open", "resume"]);
        assert_eq!(i.host().session.clone().unwrap().cwd, Some(root));
    }
    #[test]
    fn deleted_registered_worktree_requires_yes_then_resumes_without_changing_main() {
        let t = Temp::new();
        let (root, target) = linked(&t);
        let before = String::from_utf8(
            git_command(&root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let (db, _) = fixture(&t, &target);
        fs::remove_dir_all(&target).unwrap();
        let mut s = BrowserState::default();
        let mut i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        enter(&mut s, &db, &mut i);
        assert!(i
            .recovery
            .contains(&restore::RecoveryChoice::RecreateWorktree));
        i.handle(Key::Char('w'), &mut s, &db).unwrap();
        assert!(i.confirmation_text.is_some());
        assert!(i
            .confirmation_text
            .as_ref()
            .unwrap()
            .contains(target.to_str().unwrap()));
        assert!(!target.exists());
        i.handle(Key::Char('n'), &mut s, &db).unwrap();
        assert!(!target.exists());
        assert!(i.host().effects.is_empty());
        i.handle(Key::Char('w'), &mut s, &db).unwrap();
        i.handle(Key::Char('y'), &mut s, &db).unwrap();
        assert!(s.closed);
        assert_eq!(i.host().effects, ["list", "open", "resume"]);
        assert_eq!(
            fs::read_to_string(target.join("tracked")).unwrap(),
            "preserved"
        );
        let after = String::from_utf8(
            git_command(&root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            fs::read_to_string(root.join("tracked")).unwrap(),
            "preserved"
        );
        let recreated = git_command(&target)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(recreated.status.success());
        assert_eq!(String::from_utf8(recreated.stdout).unwrap(), before);
    }
    #[test]
    fn locked_registration_existing_target_and_missing_commit_are_not_mutated() {
        let t = Temp::new();
        let (root, target) = linked(&t);
        let (db, _) = fixture(&t, &target);
        let mut s = BrowserState::default();
        let i = HerdrIntegration::new(Host::default());
        search(&mut s, &db);
        let session = i.session(&s, &db).unwrap();
        let plan = restore::plan_recreate(&session).unwrap();
        assert!(restore::execute_recreate(&plan, restore::Confirmation::confirmed()).is_err());
        assert_eq!(
            fs::read_to_string(target.join("tracked")).unwrap(),
            "preserved"
        );
        assert!(git_command(&root)
            .args(["worktree", "lock", target.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        fs::remove_dir_all(&target).unwrap();
        assert!(
            restore::execute_recreate(&plan, restore::Confirmation::confirmed())
                .unwrap_err()
                .to_string()
                .contains("locked")
        );
        assert!(!target.exists());
        assert!(git_command(&root)
            .args(["worktree", "unlock", target.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        let mut invalid = session.clone();
        invalid.commit = Some("f".repeat(40));
        assert!(restore::execute_recreate(
            &restore::plan_recreate(&invalid).unwrap(),
            restore::Confirmation::confirmed()
        )
        .is_err());
        assert!(!target.exists());
        std::os::unix::fs::symlink(&root, &target).unwrap();
        assert!(restore::execute_recreate(&plan, restore::Confirmation::confirmed()).is_err());
        assert_eq!(
            fs::read_to_string(root.join("tracked")).unwrap(),
            "preserved"
        );
    }
}

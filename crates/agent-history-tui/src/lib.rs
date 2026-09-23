//! Terminal UI for searching and previewing Agent History.
//! This crate deliberately contains no host or workspace integration.
use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::{index_all_with_progress, IndexProgress},
    preview::preview_source,
    CoreError, EventKind, Result, SearchResult, SourceRef, SqliteStore,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

mod markdown;
mod terminal;
mod text;
mod theme;
mod ui;

pub use text::{safe, wrap};
pub use theme::{Color, Palette};
pub use ui::draw;

/// Maximum results fetched per query.
pub const RESULT_LIMIT: usize = 50;
const PREVIEW_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Backspace,
    Up,
    Down,
    PageUp,
    PageDown,
    Space,
    Enter,
    Esc,
    Tab,
    F2,
}

/// Which pane has keyboard focus, or the integration's action screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Query,
    Results,
    Preview,
    Action,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RoleFilter {
    #[default]
    All,
    User,
    Assistant,
}
impl RoleFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::User, Self::Assistant];
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::User,
            Self::User => Self::Assistant,
            Self::Assistant => Self::All,
        }
    }
    fn kind(self) -> Option<EventKind> {
        match self {
            Self::All => None,
            Self::User => Some(EventKind::User),
            Self::Assistant => Some(EventKind::Assistant),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::User => "User",
            Self::Assistant => "Assistant",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BrowserState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected: usize,
    pub mode: Mode,
    /// Original conversation around the selected result.
    pub preview: String,
    /// The source `preview` was read from; `None` when nothing is loaded.
    pub preview_for: Option<SourceRef>,
    /// Why the selected result's conversation could not be read.
    pub preview_error: Option<String>,
    pub preview_scroll: usize,
    /// Scroll the preview to the first matched term on the next render.
    pub preview_anchor: bool,
    /// First result shown in the results pane.
    pub list_offset: usize,
    pub error: Option<String>,
    pub closed: bool,
    pub status: String,
    pub role_filter: RoleFilter,
}
impl BrowserState {
    pub fn refresh(&mut self, store: &SqliteStore) {
        match store.search_with_role(&self.query, RESULT_LIMIT, self.role_filter.kind()) {
            Ok(items) => {
                self.results = items;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
                self.error = None;
            }
            Err(e) => {
                self.results.clear();
                self.selected = 0;
                self.error = Some(e.to_string());
            }
        }
        self.sync_preview(store);
    }
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }

    /// Loads the conversation for the selected result unless it is already
    /// loaded. Failures are kept for the preview pane rather than reported as
    /// errors, because browsing past an unreadable result is normal.
    pub fn sync_preview(&mut self, store: &SqliteStore) {
        match self.selected_result().map(|r| r.source.clone()) {
            Some(source) if self.preview_for.as_ref() != Some(&source) => {
                let _ = self.load_preview(store, source);
            }
            Some(_) => {}
            None => {
                self.preview.clear();
                self.preview_for = None;
                self.preview_error = None;
                self.preview_scroll = 0;
            }
        }
    }

    fn load_preview(&mut self, store: &SqliteStore, source: SourceRef) -> Result<()> {
        self.preview.clear();
        self.preview_error = None;
        self.preview_scroll = 0;
        self.preview_anchor = true;
        self.preview_for = Some(source.clone());
        match preview_source(store, &source, PREVIEW_BYTES) {
            Ok(p) => {
                self.preview = p.text;
                if p.truncated_before || p.truncated_after {
                    self.preview
                        .push_str("\n\n[Surrounding context is limited]");
                }
                Ok(())
            }
            Err(e) => {
                self.preview_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// Focuses the preview of the selected result, re-reading the source if
    /// the loaded preview belongs to another result or failed.
    pub fn show_preview(&mut self, store: &SqliteStore) -> Result<()> {
        let source = self
            .selected_result()
            .ok_or_else(|| CoreError::Unsupported("no session selected".into()))?
            .source
            .clone();
        if self.preview_for.as_ref() != Some(&source) || self.preview_error.is_some() {
            self.load_preview(store, source)?;
        }
        self.mode = Mode::Preview;
        Ok(())
    }

    fn requery(&mut self, store: &SqliteStore) {
        self.selected = 0;
        self.list_offset = 0;
        self.refresh(store);
    }

    fn select(&mut self, index: usize, store: &SqliteStore) {
        self.selected = index.min(self.results.len().saturating_sub(1));
        self.sync_preview(store);
    }

    pub fn handle(&mut self, key: Key, store: &SqliteStore) -> Result<()> {
        self.error = None;
        match self.mode {
            Mode::Preview => match key {
                Key::Esc => self.mode = Mode::Results,
                Key::Tab => self.mode = Mode::Query,
                Key::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                Key::Down => self.preview_scroll = self.preview_scroll.saturating_add(1),
                Key::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(10),
                Key::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(10),
                Key::F2 => {
                    self.role_filter = self.role_filter.next();
                    self.mode = Mode::Results;
                    self.requery(store);
                }
                _ => {}
            },
            Mode::Action => {}
            _ => match key {
                Key::Esc if self.mode == Mode::Results => self.mode = Mode::Query,
                Key::Esc => self.closed = true,
                Key::Down => {
                    if self.mode == Mode::Results {
                        self.select(self.selected.saturating_add(1), store);
                    }
                    self.mode = Mode::Results;
                }
                Key::Up => {
                    self.mode = Mode::Results;
                    self.select(self.selected.saturating_sub(1), store);
                }
                Key::PageDown => {
                    self.mode = Mode::Results;
                    self.select(self.selected.saturating_add(5), store);
                }
                Key::PageUp => {
                    self.mode = Mode::Results;
                    self.select(self.selected.saturating_sub(5), store);
                }
                Key::Tab if self.mode == Mode::Query => self.mode = Mode::Results,
                Key::Tab => {
                    if self.show_preview(store).is_err() {
                        self.mode = Mode::Query;
                    }
                }
                Key::F2 => {
                    self.role_filter = self.role_filter.next();
                    self.requery(store);
                }
                Key::Space if self.mode == Mode::Results => self.show_preview(store)?,
                Key::Space => {
                    self.query.push(' ');
                    self.requery(store);
                }
                Key::Char(c) => {
                    self.mode = Mode::Query;
                    self.query.push(c);
                    self.requery(store);
                }
                Key::Backspace => {
                    self.mode = Mode::Query;
                    self.query.pop();
                    self.requery(store);
                }
                Key::Enter => self.show_preview(store)?,
            },
        }
        Ok(())
    }
}

pub trait Integration {
    fn title(&self) -> &str;
    fn enter_label(&self) -> &str;
    /// What Enter does while the preview pane is focused, if anything.
    fn preview_enter_label(&self) -> Option<&str> {
        None
    }
    /// Colors for the browser. The default follows the terminal's own
    /// ANSI palette; integrations may supply their host's theme.
    fn palette(&self) -> Palette {
        Palette::terminal()
    }
    fn handle(&mut self, key: Key, state: &mut BrowserState, store: &SqliteStore) -> Result<bool>;
    fn action_lines(&self) -> Vec<String>;
}

#[derive(Default)]
pub struct Standalone;
impl Integration for Standalone {
    fn title(&self) -> &str {
        "Agent History (Standalone)"
    }
    fn enter_label(&self) -> &str {
        "Preview"
    }
    fn handle(&mut self, _: Key, _: &mut BrowserState, _: &SqliteStore) -> Result<bool> {
        Ok(false)
    }
    fn action_lines(&self) -> Vec<String> {
        Vec::new()
    }
}

pub fn run(args: Vec<String>, integration: &mut impl Integration) -> io::Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}\n\nOptions: --db PATH --claude-root PATH --codex-root PATH\nExplicit root options disable default history discovery for both agents.\nType a query; arrows/Tab focus results and preview; F2 filters role; Space previews; Enter {}; Esc goes back; Ctrl-C closes.", integration.title(), integration.enter_label().to_lowercase());
        return Ok(());
    }
    let (db, claude, codex) = parse_args(args)?;
    let custom = claude.is_some() || codex.is_some();
    let adapters: Vec<Box<dyn agent_history_core::AgentAdapter>> = if custom {
        vec![
            Box::new(ClaudeAdapter::new(claude)),
            Box::new(CodexAdapter::new(codex)),
        ]
    } else {
        vec![
            Box::new(ClaudeAdapter::default()),
            Box::new(CodexAdapter::default()),
        ]
    };
    let palette = integration.palette();
    let title = integration.title().to_string();
    terminal::install_signal_handlers();
    let mut screen = terminal::Screen::enter()?;

    let started = Instant::now();
    let progress: Arc<Mutex<Option<IndexProgress>>> = Arc::new(Mutex::new(None));
    let done = Arc::new(AtomicBool::new(false));
    let ticker = {
        let progress = Arc::clone(&progress);
        let done = Arc::clone(&done);
        let mut term = screen.take();
        thread::spawn(move || {
            let mut typed = Vec::new();
            let mut frame = 0usize;
            while !done.load(Ordering::Acquire) {
                let snapshot = progress.lock().map(|p| p.clone()).unwrap_or_default();
                let _ = term.draw(|f| {
                    ui::draw_progress(
                        f,
                        &palette,
                        &title,
                        frame,
                        started.elapsed(),
                        snapshot.as_ref(),
                    )
                });
                frame += 1;
                if terminal::interrupted() {
                    terminal::abort(term);
                }
                if event::poll(Duration::from_millis(120)).unwrap_or(false) {
                    if let Ok(Event::Key(k)) = event::read() {
                        if is_interrupt(&k) {
                            terminal::abort(term);
                        }
                        if k.kind != KeyEventKind::Release {
                            typed.extend(map_key(k.code));
                        }
                    }
                }
            }
            (term, typed)
        })
    };
    let finish = |ticker: thread::JoinHandle<_>| {
        done.store(true, Ordering::Release);
        ticker
            .join()
            .map_err(|_| io::Error::other("progress display failed"))
    };
    let mut store = match SqliteStore::open(db) {
        Ok(store) => store,
        Err(e) => {
            finish(ticker)?;
            return Err(core_io(e));
        }
    };
    let progress_for_index = Arc::clone(&progress);
    let report = index_all_with_progress(&mut store, &adapters, |p: IndexProgress| {
        if let Ok(mut progress) = progress_for_index.lock() {
            *progress = Some(p);
        }
    });
    let (term, typed) = finish(ticker)?;
    screen.put_back(term);
    let report = report.map_err(core_io)?;
    let mut state = BrowserState {
        status: format!(
            "{} files · {} chunks · {} failed · {} malformed · {:.1}s",
            report.files,
            report.chunks,
            report.failed_files,
            report.malformed_records,
            started.elapsed().as_secs_f32()
        ),
        ..Default::default()
    };
    if !report.errors.is_empty() {
        state.status.push_str(" — ");
        state.status.push_str(&report.errors.join("; "));
    }
    // Replay what was typed while indexing, but never an action key.
    for key in typed {
        if matches!(key, Key::Char(_) | Key::Space | Key::Backspace) {
            let _ = state.handle(key, &store);
        }
    }
    let term = screen.terminal();
    while !state.closed {
        term.draw(|f| draw(f, &mut state, integration))?;
        if terminal::interrupted() {
            break;
        }
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        if is_interrupt(&k) {
            break;
        }
        if let Some(key) = map_key(k.code) {
            let handled = match integration.handle(key, &mut state, &store) {
                Ok(handled) => handled,
                Err(e) => {
                    state.error = Some(e.to_string());
                    continue;
                }
            };
            if !handled {
                if let Err(e) = state.handle(key, &store) {
                    state.error = Some(e.to_string());
                }
            }
            state.sync_preview(&store);
        }
    }
    Ok(())
}

fn is_interrupt(k: &KeyEvent) -> bool {
    k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c')
}

fn parse_args(args: Vec<String>) -> io::Result<(PathBuf, Option<PathBuf>, Option<PathBuf>)> {
    let mut db = std::env::var_os("AGENT_HISTORY_DB").map(PathBuf::from);
    let mut claude = None;
    let mut codex = None;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                return Err(io::Error::other(
                    "agent-history tui [--db PATH] [--claude-root PATH] [--codex-root PATH]",
                ))
            }
            "--db" | "--claude-root" | "--codex-root" => {
                let value = PathBuf::from(
                    it.next()
                        .ok_or_else(|| io::Error::other("option requires a path"))?,
                );
                match arg.as_str() {
                    "--db" => db = Some(value),
                    "--claude-root" => claude = Some(value),
                    _ => codex = Some(value),
                }
            }
            _ => return Err(io::Error::other(format!("unknown option: {arg}"))),
        }
    }
    let db = db
        .or_else(agent_history_core::default_index_path)
        .ok_or_else(|| io::Error::other("HOME is unavailable; specify --db"))?;
    Ok((db, claude, codex))
}
fn core_io(e: agent_history_core::CoreError) -> io::Error {
    io::Error::other(e.to_string())
}
fn map_key(k: KeyCode) -> Option<Key> {
    Some(match k {
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::F(2) => Key::F2,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::index::index_all;
    use std::fs;

    #[test]
    fn role_filter_cycles_and_maps() {
        assert_eq!(RoleFilter::All.next(), RoleFilter::User);
        assert_eq!(RoleFilter::User.next(), RoleFilter::Assistant);
        assert_eq!(RoleFilter::Assistant.next(), RoleFilter::All);
        assert_eq!(RoleFilter::User.kind(), Some(EventKind::User));
    }

    pub(crate) fn fixture_store(root: &std::path::Path, records: &[&str]) -> SqliteStore {
        fs::create_dir_all(root.join("history")).unwrap();
        fs::create_dir(root.join("private")).unwrap();
        let mut permissions = fs::metadata(root.join("private")).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o700);
        fs::set_permissions(root.join("private"), permissions).unwrap();
        let mut body = String::new();
        for record in records {
            body.push_str(record);
            body.push('\n');
        }
        fs::write(root.join("history/session.jsonl"), body).unwrap();
        let mut store = SqliteStore::open(root.join("private/index.sqlite")).unwrap();
        index_all(
            &mut store,
            &[Box::new(ClaudeAdapter::with_root(root.join("history")))],
        )
        .unwrap();
        store
    }

    pub(crate) fn rememberable(root: &std::path::Path) -> SqliteStore {
        let cwd = root.to_string_lossy();
        let user = format!(
            r#"{{"type":"user","sessionId":"00000000-0000-4000-8000-000000000001","cwd":"{cwd}","message":{{"content":"rememberable topic"}}}}"#
        );
        fixture_store(
            root,
            &[
                &user,
                r#"{"type":"assistant","message":{"content":"rememberable answer"}}"#,
            ],
        )
    }

    #[test]
    fn standalone_has_no_actions_and_enter_opens_preview() {
        let temp = agent_history_core::test_support::TempDir::new("tui").unwrap();
        let store = rememberable(temp.path());
        let mut state = BrowserState {
            query: "rememberable".into(),
            ..Default::default()
        };
        state.refresh(&store);
        assert_eq!(state.results.len(), 2);
        state.handle(Key::Down, &store).unwrap();
        assert_eq!(state.selected, 0, "first arrow focuses the first result");
        state.handle(Key::Down, &store).unwrap();
        assert_eq!(state.selected, 1);
        state.handle(Key::F2, &store).unwrap();
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.results[0].kind, EventKind::User);
        state.handle(Key::Enter, &store).unwrap();
        assert_eq!(state.mode, Mode::Preview);
        assert!(state.preview.contains("User: rememberable topic"));
        assert!(Standalone.action_lines().is_empty());
        assert_eq!(Standalone.preview_enter_label(), None);
        state.handle(Key::Esc, &store).unwrap();
        assert_eq!(state.query, "rememberable");
        assert_eq!(state.mode, Mode::Results);
        state.handle(Key::F2, &store).unwrap();
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.results[0].kind, EventKind::Assistant);
        assert_eq!(state.role_filter, RoleFilter::Assistant);
    }

    #[test]
    fn preview_follows_selection_and_tab_cycles_focus() {
        let temp = agent_history_core::test_support::TempDir::new("tui-follow").unwrap();
        let store = rememberable(temp.path());
        let mut state = BrowserState::default();
        for c in "rememberable".chars() {
            state.handle(Key::Char(c), &store).unwrap();
        }
        assert_eq!(state.mode, Mode::Query);
        assert_eq!(
            state.preview_for.as_ref(),
            Some(&state.results[0].source),
            "preview is loaded without leaving the query box"
        );
        state.handle(Key::Tab, &store).unwrap();
        assert_eq!(state.mode, Mode::Results);
        state.handle(Key::Down, &store).unwrap();
        assert_eq!(state.preview_for.as_ref(), Some(&state.results[1].source));
        state.handle(Key::Tab, &store).unwrap();
        assert_eq!(state.mode, Mode::Preview);
        state.handle(Key::Tab, &store).unwrap();
        assert_eq!(state.mode, Mode::Query);
        state.handle(Key::Backspace, &store).unwrap();
        assert_eq!(state.selected, 0, "a new query starts at the top");
        state.query = "absent".into();
        state.refresh(&store);
        assert!(state.results.is_empty());
        assert!(state.preview_for.is_none() && state.preview.is_empty());
    }

    #[test]
    fn unreadable_source_is_reported_in_the_preview_not_as_a_crash() {
        let temp = agent_history_core::test_support::TempDir::new("tui-stale").unwrap();
        let store = rememberable(temp.path());
        fs::write(temp.path().join("history/session.jsonl"), "replaced\n").unwrap();
        let mut state = BrowserState {
            query: "rememberable".into(),
            ..Default::default()
        };
        state.refresh(&store);
        assert!(state.preview_error.is_some());
        assert!(
            state.handle(Key::Space, &store).is_ok(),
            "space types in the query"
        );
        state.mode = Mode::Results;
        assert!(state.handle(Key::Space, &store).is_err());
        assert_eq!(state.mode, Mode::Results);
    }
}

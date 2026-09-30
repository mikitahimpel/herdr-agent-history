//! Terminal UI for searching and previewing Agent History.
//! This crate deliberately contains no host or workspace integration.
use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    availability::Availability,
    background::{BackgroundIndex, ScanSnapshot},
    preview::preview_source,
    AgentAdapter, CoreError, EventKind, Result, SearchResult, Session, SessionId, SourceRef,
    SqliteStore, Widening,
};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::Position;
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

mod availability;
mod markdown;
pub mod report;
mod terminal;
mod text;
mod theme;
mod ui;

pub use availability::{Availabilities, SessionState};
pub use text::{safe, wrap};
pub use theme::{Color, Palette};
pub use ui::draw;

/// Maximum results fetched per query.
pub const RESULT_LIMIT: usize = 50;
const PREVIEW_BYTES: u64 = 64 * 1024;
/// While indexing runs, results are re-queried at most this often, so that a
/// stream of commits does not reshuffle the list under the reader.
const RESULTS_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

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
    F3,
}

/// A mouse action at a screen cell. Mouse input is additive: every action
/// here also has a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mouse {
    Click { column: u16, row: u16 },
    ScrollUp { column: u16, row: u16 },
    ScrollDown { column: u16, row: u16 },
}
impl Mouse {
    fn position(self) -> Position {
        match self {
            Self::Click { column, row }
            | Self::ScrollUp { column, row }
            | Self::ScrollDown { column, row } => Position::new(column, row),
        }
    }
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

/// Whether the index the results come from has caught up with the history on
/// disk.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Indexing {
    /// The last scan finished; the index holds every file it found.
    #[default]
    Settled,
    /// A scan is still running, so recently changed conversations may be
    /// missing. `checked` of `total` files are done; `total` is 0 until the
    /// files have been found.
    Running { checked: u64, total: u64 },
    /// The last scan did not finish; `BrowserState::status` says why.
    Incomplete,
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
    /// Non-empty when nothing matched the query exactly and `results` are
    /// near matches found by widening these words.
    pub widened: Vec<Widening>,
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
    /// Confirms something that happened elsewhere, such as a resumed session.
    /// The host usually takes focus at that moment, so without this the browser
    /// would look as though nothing had happened. Cleared by the next keystroke.
    pub notice: Option<String>,
    pub closed: bool,
    pub status: String,
    pub indexing: Indexing,
    pub role_filter: RoleFilter,
    /// Whether each result's session still exists on disk, filled in by a
    /// background check.
    pub availability: Availabilities,
    /// Whether the browser receives mouse events. Off hands selection back
    /// to the terminal; F3 toggles it.
    pub mouse_capture: bool,
    /// The results list was scrolled by the wheel, so it no longer follows
    /// the selection until the selection moves.
    pub list_scrolled: bool,
    /// Where the last frame drew each clickable region.
    pub(crate) hits: ui::Hits,
}
impl BrowserState {
    pub fn refresh(&mut self, store: &SqliteStore) {
        self.search(store);
        self.sync_preview(store);
    }

    /// Re-runs the query after the index changed underneath it. The selected
    /// conversation stays selected while it is still among the results, and
    /// an error the reader has not dismissed stays shown.
    pub fn refresh_in_place(&mut self, store: &SqliteStore) {
        let selected = self.selected_result().map(|r| r.source.clone());
        let error = self.error.take();
        self.search(store);
        if self.error.is_none() {
            self.error = error;
        }
        if let Some(i) = selected.and_then(|s| self.results.iter().position(|r| r.source == s)) {
            self.selected = i;
        }
        self.sync_preview(store);
    }

    fn search(&mut self, store: &SqliteStore) {
        match store.search_with_fallback(&self.query, RESULT_LIMIT, self.role_filter.kind()) {
            Ok(outcome) => {
                self.results = outcome.results;
                self.widened = outcome.widened;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
                self.error = None;
            }
            Err(e) => {
                self.results.clear();
                self.widened.clear();
                self.selected = 0;
                self.error = Some(e.to_string());
            }
        }
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
        self.list_scrolled = false;
        self.refresh(store);
    }

    fn select(&mut self, index: usize, store: &SqliteStore) {
        self.list_scrolled = false;
        self.selected = index.min(self.results.len().saturating_sub(1));
        self.sync_preview(store);
    }

    /// Applies a mouse action against the regions of the last drawn frame.
    /// A click anywhere in a pane focuses it; a click on a result also
    /// selects it. The recovery screen ignores the mouse so that a stray
    /// click can never answer it.
    pub fn mouse(&mut self, event: Mouse, store: &SqliteStore) {
        if self.mode == Mode::Action {
            return;
        }
        self.error = None;
        let at = event.position();
        let hits = std::mem::take(&mut self.hits);
        let in_results = hits.results.is_some_and(|r| r.contains(at));
        let in_preview = hits.preview.is_some_and(|r| r.contains(at));
        match event {
            Mouse::Click { .. } => {
                if hits.search.contains(at) {
                    self.mode = Mode::Query;
                } else if let Some(&(_, filter)) = hits.tabs.iter().find(|(r, _)| r.contains(at)) {
                    if filter != self.role_filter {
                        self.role_filter = filter;
                        self.requery(store);
                    }
                    if self.mode == Mode::Preview {
                        self.mode = Mode::Results;
                    }
                } else if in_results {
                    self.mode = Mode::Results;
                    if let Some(&(_, index)) = hits.rows.iter().find(|(r, _)| r.contains(at)) {
                        self.select(index, store);
                    }
                } else if in_preview {
                    self.mode = Mode::Preview;
                }
            }
            Mouse::ScrollUp { .. } if in_results => {
                self.list_scrolled = true;
                self.list_offset = self.list_offset.saturating_sub(1);
            }
            Mouse::ScrollDown { .. } if in_results => {
                self.list_scrolled = true;
                self.list_offset = self.list_offset.saturating_add(1);
            }
            Mouse::ScrollUp { .. } if in_preview => {
                self.preview_anchor = false;
                self.preview_scroll = self.preview_scroll.saturating_sub(3);
            }
            Mouse::ScrollDown { .. } if in_preview => {
                self.preview_anchor = false;
                self.preview_scroll = self.preview_scroll.saturating_add(3);
            }
            _ => {}
        }
        self.hits = hits;
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
                // Mouse capture is terminal state; the run loop toggles it.
                Key::F3 => {}
            },
        }
        Ok(())
    }
}

pub trait Integration {
    fn title(&self) -> &str;
    fn enter_label(&self) -> &str;
    /// Whether the host draws a titled frame around the browser. Whoever owns
    /// the chrome owns the title, so the browser then leaves its own out.
    fn host_draws_title(&self) -> bool {
        false
    }
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
    /// Sessions the host already runs, asked once when the browser opens.
    fn live_sessions(&mut self, _sessions: &[Session]) -> Vec<SessionId> {
        Vec::new()
    }
    /// Short wording for a result's availability marker. The default only
    /// describes what exists on disk; it never promises that Enter resumes.
    fn availability_label(&self, state: SessionState) -> &str {
        match state {
            SessionState::Live => "running",
            SessionState::Stored(Availability::OnDisk) => "on disk",
            SessionState::Stored(Availability::Recoverable) => "repo only",
            SessionState::Stored(Availability::RepositoryKnown) => "repo gone",
            SessionState::Stored(Availability::TranscriptOnly) => "transcript",
        }
    }
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

/// Carries the background scan into the browser: progress into the header,
/// and each batch of commits into the results.
struct ScanFollower {
    started: Instant,
    seen_chunks: u64,
    refreshed: Instant,
    /// Commits the results have not been re-queried for yet.
    stale: bool,
    finished: bool,
}

impl ScanFollower {
    fn new(started: Instant) -> Self {
        Self {
            started,
            seen_chunks: 0,
            refreshed: started,
            stale: false,
            finished: false,
        }
    }

    /// Returns the scan's report the first time it is seen finished.
    fn apply(
        &mut self,
        scan: &ScanSnapshot,
        state: &mut BrowserState,
        store: &SqliteStore,
    ) -> Option<agent_history_core::index::IndexReport> {
        if scan.committed_chunks > self.seen_chunks {
            self.seen_chunks = scan.committed_chunks;
            self.stale = true;
        }
        let mut report = None;
        match (&scan.outcome, self.finished) {
            (None, _) => {
                let (checked, total) = scan
                    .progress
                    .as_ref()
                    .map_or((0, 0), |p| (p.completed_files, p.total_files));
                state.indexing = Indexing::Running { checked, total };
            }
            (Some(outcome), false) => {
                self.finished = true;
                // A finished scan may also have removed or rewritten chunks.
                self.stale = true;
                match outcome {
                    Ok(r) => {
                        state.status = format!(
                            "{} files · {} chunks · {} failed · {} malformed · {:.1}s",
                            r.files,
                            r.chunks,
                            r.failed_files,
                            r.malformed_records,
                            self.started.elapsed().as_secs_f32()
                        );
                        if !r.errors.is_empty() {
                            state.status.push_str(" — ");
                            state.status.push_str(&r.errors.join("; "));
                        }
                        state.indexing = if r.cancelled {
                            Indexing::Incomplete
                        } else {
                            Indexing::Settled
                        };
                        report = Some(r.clone());
                    }
                    Err(e) => {
                        state.status = format!("Index not updated: {e}");
                        state.indexing = Indexing::Incomplete;
                    }
                }
            }
            (Some(_), true) => {}
        }
        // Never while the integration's action screen is open: the action
        // belongs to the result that was selected when it opened.
        let due = self.finished || self.refreshed.elapsed() >= RESULTS_REFRESH_INTERVAL;
        if self.stale && due && state.mode != Mode::Action {
            state.refresh_in_place(store);
            self.stale = false;
            self.refreshed = Instant::now();
        }
        report
    }
}

fn adapters(claude: Option<PathBuf>, codex: Option<PathBuf>) -> Vec<Box<dyn AgentAdapter>> {
    if claude.is_some() || codex.is_some() {
        vec![
            Box::new(ClaudeAdapter::new(claude)),
            Box::new(CodexAdapter::new(codex)),
        ]
    } else {
        vec![
            Box::new(ClaudeAdapter::default()),
            Box::new(CodexAdapter::default()),
        ]
    }
}

pub fn run(args: Vec<String>, integration: &mut impl Integration) -> io::Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}\n\nOptions: --db PATH --claude-root PATH --codex-root PATH\nExplicit root options disable default history discovery for both agents.\nType a query; arrows/Tab focus results and preview; F2 filters role; Space previews; Enter {}; Esc goes back; Ctrl-C closes.", integration.title(), integration.enter_label().to_lowercase());
        return Ok(());
    }
    let (db, claude, codex) = parse_args(args)?;
    terminal::install_signal_handlers();
    let mut screen = terminal::Screen::enter()?;
    let started = Instant::now();
    let mut state = BrowserState {
        status: "Opening the index…".into(),
        indexing: Indexing::Running {
            checked: 0,
            total: 0,
        },
        mouse_capture: true,
        ..Default::default()
    };
    let term = screen.terminal();
    // Opening can migrate an old schema, which takes a moment once.
    term.draw(|f| draw(f, &mut state, integration))?;
    let store = SqliteStore::open(&db).map_err(core_io)?;
    // The existing index is searchable from here on; the scan only adds to it.
    let scan = BackgroundIndex::spawn(db, move || adapters(claude, codex))?;
    let mut follower = ScanFollower::new(started);
    // One read of the session table and one host query; after this, only the
    // worker touches the filesystem, and only for sessions not seen before.
    let sessions = store.sessions().unwrap_or_default();
    state
        .availability
        .set_live(integration.live_sessions(&sessions));
    let mut worker =
        availability::Worker::spawn(sessions, agent_history_core::availability::availability);
    while !state.closed {
        let report = follower.apply(&scan.snapshot(), &mut state, &store);
        if report.is_some_and(|r| r.bytes_read > 0) {
            // The scan may have added sessions, or moved their sources.
            let sessions = store.sessions().unwrap_or_default();
            state.availability = Availabilities::default();
            state
                .availability
                .set_live(integration.live_sessions(&sessions));
            worker = availability::Worker::spawn(
                sessions,
                agent_history_core::availability::availability,
            );
        }
        worker.drain(&mut state.availability);
        worker.request(state.availability.wanted(&state.results));
        term.draw(|f| draw(f, &mut state, integration))?;
        if terminal::interrupted() {
            break;
        }
        let wait = if matches!(state.indexing, Indexing::Running { .. }) {
            120
        } else if state.availability.pending() {
            50
        } else {
            200
        };
        if !event::poll(Duration::from_millis(wait))? {
            continue;
        }
        let k = match event::read()? {
            Event::Key(k) => k,
            Event::Mouse(m) => {
                if let Some(action) = map_mouse(m) {
                    state.mouse(action, &store);
                }
                continue;
            }
            _ => continue,
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        if is_interrupt(&k) {
            break;
        }
        if map_key(k.code) == Some(Key::F3) {
            // Handled before the integration so it works on every screen.
            state.mouse_capture = !state.mouse_capture;
            terminal::set_mouse_capture(&mut io::stdout(), state.mouse_capture)?;
            continue;
        }
        // A confirmation describes the keystroke that caused it, so it is
        // cleared before the next one rather than lingering over new results.
        state.notice = None;
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
    // Abandons the file in progress before its commit; everything committed stays.
    scan.stop();
    Ok(())
}

fn map_mouse(m: MouseEvent) -> Option<Mouse> {
    let (column, row) = (m.column, m.row);
    Some(match m.kind {
        MouseEventKind::Down(MouseButton::Left) => Mouse::Click { column, row },
        MouseEventKind::ScrollUp => Mouse::ScrollUp { column, row },
        MouseEventKind::ScrollDown => Mouse::ScrollDown { column, row },
        _ => return None,
    })
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
        KeyCode::F(3) => Key::F3,
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

    fn claude_line(session: &str, text: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"sessionId\":\"{session}\",\"message\":{{\"content\":\"{text}\"}}}}\n"
        )
    }

    /// Claude parsing, except that the record containing `gate` waits for the test.
    struct Gated {
        root: PathBuf,
        gate: &'static str,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl AgentAdapter for Gated {
        fn agent(&self) -> agent_history_core::Agent {
            agent_history_core::Agent::Claude
        }
        fn discover(&self) -> Result<Vec<agent_history_core::SessionFile>> {
            ClaudeAdapter::with_root(self.root.clone()).discover()
        }
        fn parse_record(
            &self,
            s: &Session,
            r: &[u8],
            src: SourceRef,
        ) -> Result<agent_history_core::ParsedRecord> {
            if String::from_utf8_lossy(r).contains(self.gate) {
                let _ = self.entered.send(());
                let _ = self.release.recv();
            }
            ClaudeAdapter::with_root(self.root.clone()).parse_record(s, r, src)
        }
    }

    /// An index holding one `topic` conversation, a second one on disk that
    /// is not indexed yet, and a scan held just before committing it.
    struct Scanning {
        _temp: agent_history_core::test_support::TempDir,
        store: SqliteStore,
        scan: BackgroundIndex,
        release: std::sync::mpsc::Sender<()>,
    }
    fn scanning(name: &str) -> Scanning {
        let temp = agent_history_core::test_support::TempDir::new(name).unwrap();
        let root = temp.path().to_path_buf();
        let store = fixture_store(&root, &[claude_line("old", "topic alpha").trim_end()]);
        fs::write(
            root.join("history/new.jsonl"),
            claude_line("new", "topic beta"),
        )
        .unwrap();
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let history = root.join("history");
        let scan = BackgroundIndex::spawn(root.join("private/index.sqlite"), move || {
            vec![Box::new(Gated {
                root: history,
                gate: "beta",
                entered,
                release: release_rx,
            }) as Box<dyn AgentAdapter>]
        })
        .unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the scan reaches the new conversation");
        Scanning {
            _temp: temp,
            store,
            scan,
            release,
        }
    }
    fn finished(scan: &BackgroundIndex) -> ScanSnapshot {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let snapshot = scan.snapshot();
            if snapshot.finished() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "scan did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn typing_searches_the_existing_index_while_the_scan_runs_and_results_follow_its_commit() {
        let t = scanning("tui-scan");
        let mut state = BrowserState::default();
        let mut follower = ScanFollower::new(Instant::now());
        assert!(follower
            .apply(&t.scan.snapshot(), &mut state, &t.store)
            .is_none());
        assert!(matches!(state.indexing, Indexing::Running { .. }));
        for c in "topic".chars() {
            state.handle(Key::Char(c), &t.store).unwrap();
        }
        assert_eq!(state.results.len(), 1, "served from the existing index");
        state.handle(Key::Down, &t.store).unwrap();
        let selected = state.selected_result().unwrap().source.clone();
        state.error = Some("resume failed".into());

        t.release.send(()).unwrap();
        let report = follower
            .apply(&finished(&t.scan), &mut state, &t.store)
            .expect("the finish is reported once");
        assert!(report.bytes_read > 0);
        assert_eq!(state.indexing, Indexing::Settled);
        assert!(state.status.contains("0 failed"), "{}", state.status);
        assert_eq!(state.results.len(), 2, "refreshed without a keystroke");
        assert_eq!(
            state.selected_result().unwrap().source,
            selected,
            "the reader's selection survives the refresh"
        );
        assert_eq!(state.error.as_deref(), Some("resume failed"));
        assert_eq!(state.mode, Mode::Results);
        assert!(follower
            .apply(&t.scan.snapshot(), &mut state, &t.store)
            .is_none());
    }

    #[test]
    fn results_never_change_under_an_open_action_screen() {
        let t = scanning("tui-action");
        let mut state = BrowserState {
            query: "topic".into(),
            ..Default::default()
        };
        state.refresh(&t.store);
        state.mode = Mode::Action;
        let mut follower = ScanFollower::new(Instant::now());
        t.release.send(()).unwrap();
        follower.apply(&finished(&t.scan), &mut state, &t.store);
        assert_eq!(state.results.len(), 1, "deferred while the action is open");
        state.mode = Mode::Results;
        follower.apply(&t.scan.snapshot(), &mut state, &t.store);
        assert_eq!(state.results.len(), 2, "applied once it closes");
    }

    #[test]
    fn a_scan_that_does_not_finish_marks_the_index_incomplete() {
        let t = scanning("tui-incomplete");
        let mut state = BrowserState::default();
        let mut follower = ScanFollower::new(Instant::now());
        t.scan.request_stop();
        t.release.send(()).unwrap();
        let stopped = t.scan.stop();
        follower.apply(&stopped, &mut state, &t.store);
        assert_eq!(state.indexing, Indexing::Incomplete);

        let mut failed = BrowserState::default();
        let snapshot = ScanSnapshot {
            outcome: Some(Err("storage error: disk full".into())),
            ..Default::default()
        };
        ScanFollower::new(Instant::now()).apply(&snapshot, &mut failed, &t.store);
        assert_eq!(failed.indexing, Indexing::Incomplete);
        assert!(failed.status.contains("disk full"), "{}", failed.status);
    }
}

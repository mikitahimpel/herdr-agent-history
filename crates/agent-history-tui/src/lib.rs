//! Terminal UI for searching and previewing Agent History.
//! This crate deliberately contains no host or workspace integration.
use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::{index_all_with_progress, IndexProgress},
    preview::preview_source,
    Agent, CoreError, EventKind, Result, SearchResult, SqliteStore,
};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Backspace,
    Up,
    Down,
    Space,
    Enter,
    Esc,
    Tab,
    F2,
}

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
    pub preview: String,
    pub preview_scroll: usize,
    pub error: Option<String>,
    pub closed: bool,
    pub status: String,
    pub role_filter: RoleFilter,
}
impl BrowserState {
    pub fn refresh(&mut self, store: &SqliteStore) {
        match store.search_with_role(&self.query, 50, self.role_filter.kind()) {
            Ok(items) => {
                self.results = items;
                self.selected = self.selected.min(self.results.len().saturating_sub(1));
                self.error = None;
            }
            Err(e) => {
                self.results.clear();
                self.error = Some(e.to_string());
            }
        }
    }
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }
    pub fn show_preview(&mut self, store: &SqliteStore) -> Result<()> {
        let source = self
            .selected_result()
            .ok_or_else(|| CoreError::Unsupported("no session selected".into()))?
            .source
            .clone();
        let p = preview_source(store, &source, 64 * 1024)?;
        self.preview = p.text;
        self.preview_scroll = 0;
        if p.truncated_before || p.truncated_after {
            self.preview
                .push_str("\n\n[Surrounding context is limited]");
        }
        self.mode = Mode::Preview;
        Ok(())
    }
    pub fn handle(&mut self, key: Key, store: &SqliteStore) -> Result<()> {
        self.error = None;
        match self.mode {
            Mode::Preview => match key {
                Key::Esc => self.mode = Mode::Results,
                Key::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                Key::Down => self.preview_scroll = self.preview_scroll.saturating_add(1),
                _ => {}
            },
            Mode::Action => {}
            _ => match key {
                Key::Esc if self.mode == Mode::Results => self.mode = Mode::Query,
                Key::Esc => self.closed = true,
                Key::Down => {
                    if self.mode == Mode::Results {
                        self.selected = self
                            .selected
                            .saturating_add(1)
                            .min(self.results.len().saturating_sub(1));
                    }
                    self.mode = Mode::Results;
                }
                Key::Up => {
                    self.mode = Mode::Results;
                    self.selected = self.selected.saturating_sub(1);
                }
                Key::Tab => {
                    self.mode = if self.mode == Mode::Query {
                        Mode::Results
                    } else {
                        Mode::Query
                    }
                }
                Key::F2 => {
                    self.role_filter = self.role_filter.next();
                    self.selected = 0;
                    self.refresh(store);
                }
                Key::Space if self.mode == Mode::Results => self.show_preview(store)?,
                Key::Space => {
                    self.query.push(' ');
                    self.refresh(store);
                }
                Key::Char(c) => {
                    self.mode = Mode::Query;
                    self.query.push(c);
                    self.refresh(store);
                }
                Key::Backspace => {
                    self.mode = Mode::Query;
                    self.query.pop();
                    self.refresh(store);
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
        println!("{}\n\nOptions: --db PATH --claude-root PATH --codex-root PATH\nExplicit root options disable default history discovery for both agents.\nType a query; arrows/Tab focus results; F2 filters role; Space previews; Enter {}; Esc goes back.", integration.title(), integration.enter_label().to_lowercase());
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
    let started = Instant::now();
    let progress = Arc::new(Mutex::new(String::from("Preparing conversation index…")));
    let done = Arc::new(AtomicBool::new(false));
    let ticker_progress = Arc::clone(&progress);
    let ticker_done = Arc::clone(&done);
    let ticker_started = started;
    let ticker = thread::spawn(move || {
        let glyphs = ['·', '•', '●', '•'];
        let mut i = 0;
        while !ticker_done.load(Ordering::Acquire) {
            let text = ticker_progress
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
            let width = terminal::size()
                .map(|(w, _)| w.saturating_sub(1) as usize)
                .unwrap_or(100);
            eprint!(
                "\r\x1b[2K{}",
                safe(
                    &format!(
                        "Agent History {} ({:.1}s) {}",
                        glyphs[i % glyphs.len()],
                        ticker_started.elapsed().as_secs_f32(),
                        text
                    ),
                    width
                )
            );
            let _ = io::stderr().flush();
            i += 1;
            thread::sleep(Duration::from_millis(150));
        }
    });
    let mut store = match SqliteStore::open(db) {
        Ok(store) => store,
        Err(e) => {
            done.store(true, Ordering::Release);
            let _ = ticker.join();
            eprintln!();
            return Err(core_io(e));
        }
    };
    let progress_for_index = Arc::clone(&progress);
    let report = index_all_with_progress(&mut store, &adapters, |p: IndexProgress| {
        if let Ok(mut progress) = progress_for_index.lock() {
            *progress = format!(
                "{} files {}/{} · {} MB read · {} records · {} chunks",
                agent_name(p.agent),
                p.agent_completed_files,
                p.agent_total_files,
                p.bytes_read / (1024 * 1024),
                p.records,
                p.chunks
            );
        }
    });
    done.store(true, Ordering::Release);
    let _ = ticker.join();
    eprintln!();
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
    let _screen = Screen::enter()?;
    let mut out = io::stdout();
    while !state.closed {
        render(&mut out, &mut state, integration)?;
        if let Event::Key(k) = event::read()? {
            if k.kind == KeyEventKind::Release {
                continue;
            }
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
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
            }
        }
    }
    Ok(())
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
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h).join("Library/Application Support/Agent History/index.sqlite")
            })
        })
        .ok_or_else(|| io::Error::other("HOME is unavailable; specify --db"))?;
    Ok((db, claude, codex))
}
fn core_io(e: agent_history_core::CoreError) -> io::Error {
    io::Error::other(e.to_string())
}
fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "Claude",
        Agent::Codex => "Codex",
    }
}
fn map_key(k: KeyCode) -> Option<Key> {
    Some(match k {
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::F(2) => Key::F2,
        _ => return None,
    })
}

struct Screen;
impl Screen {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        Ok(guard)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

pub fn render(
    out: &mut impl Write,
    state: &mut BrowserState,
    integration: &impl Integration,
) -> io::Result<()> {
    let (width, height) = terminal::size()?;
    let width = width.saturating_sub(1) as usize;
    let mut lines = vec![
        format!("{}  {}", integration.title(), state.query),
        format!(
            "Role: {}   (F2 changes)   {}",
            state.role_filter.label(),
            state.status
        ),
        format!(
            "Type query · ↑↓/Tab results · F2 role · Space preview · Enter {} · Esc back · Ctrl-C close",
            integration.enter_label()
        ),
    ];
    if let Some(e) = &state.error {
        lines.push(format!("Error: {e}"));
    }
    match state.mode {
        Mode::Preview => {
            lines.push(format!(
                "Original conversation — Enter {} · Esc results",
                integration.enter_label()
            ));
            if let Some(r) = state.selected_result() {
                lines.push(format!(
                    "{} · {}",
                    agent_name(r.agent),
                    r.repository.as_deref().map(compact_repo).unwrap_or("-")
                ));
            }
            let wrapped: Vec<String> = state
                .preview
                .lines()
                .flat_map(|l| wrap(l, width.saturating_sub(1).max(12)))
                .collect();
            let visible = height.saturating_sub(lines.len() as u16 + 1) as usize;
            let skip = state
                .preview_scroll
                .min(wrapped.len().saturating_sub(visible.max(1)));
            state.preview_scroll = skip;
            lines.extend(wrapped.into_iter().skip(skip));
        }
        Mode::Action => lines.extend(integration.action_lines()),
        _ => {
            if state.results.is_empty() {
                lines.push(
                    if state.query.is_empty() {
                        "Type words from a conversation."
                    } else {
                        "No matching sessions."
                    }
                    .into(),
                );
            }
            let visible = height.saturating_sub(lines.len() as u16 + 1) as usize / 4;
            let start = state.selected.saturating_sub(visible.saturating_sub(1));
            for (i, r) in state.results.iter().enumerate().skip(start).take(visible) {
                let date = r
                    .timestamp
                    .map(|t| {
                        let d: chrono::DateTime<chrono::Utc> = t.into();
                        d.format("%Y-%m-%d").to_string()
                    })
                    .unwrap_or_else(|| "unknown date".into());
                let context = match (r.repository.as_deref(), r.branch.as_deref()) {
                    (Some(a), Some(b)) => format!("{} / {b}", compact_repo(a)),
                    (Some(a), None) => compact_repo(a).into(),
                    (None, Some(b)) => b.into(),
                    _ => "-".into(),
                };
                lines.push(format!(
                    "{} {} · {} · {}",
                    if i == state.selected && state.mode == Mode::Results {
                        "▶"
                    } else {
                        " "
                    },
                    agent_name(r.agent),
                    role_name(r.kind),
                    context
                ));
                lines.push(format!("  {date}"));
                let mut snippet = wrap(&r.snippet, width.saturating_sub(6).max(12));
                snippet.truncate(2);
                lines.extend(snippet.into_iter().map(|x| format!("    {x}")));
            }
        }
    }
    execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
    for line in lines.into_iter().take(height.saturating_sub(1) as usize) {
        writeln!(out, "{}\r", safe(&line, width))?;
    }
    out.flush()
}
pub fn safe(value: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in value.chars() {
        let c = if c.is_control() { ' ' } else { c };
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out
}
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for p in text.lines() {
        if p.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in p.split_whitespace() {
            if UnicodeWidthStr::width(word) > width {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let mut part = String::new();
                let mut used = 0;
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > width && !part.is_empty() {
                        out.push(std::mem::take(&mut part));
                        used = 0;
                    }
                    part.push(c);
                    used += cw;
                }
                line = part;
                continue;
            }
            if !line.is_empty()
                && UnicodeWidthStr::width(line.as_str()) + 1 + UnicodeWidthStr::width(word) > width
            {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
fn compact_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}
fn role_name(k: EventKind) -> &'static str {
    match k {
        EventKind::User => "User",
        EventKind::Assistant => "Assistant",
        EventKind::ToolResult => "Tool",
    }
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

    #[test]
    fn standalone_has_no_actions_and_enter_opens_preview() {
        let temp = agent_history_core::test_support::TempDir::new("tui").unwrap();
        let root = temp.path().to_path_buf();
        fs::create_dir_all(root.join("history")).unwrap();
        fs::create_dir(root.join("private")).unwrap();
        let mut permissions = fs::metadata(root.join("private")).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o700);
        fs::set_permissions(root.join("private"), permissions).unwrap();
        let cwd = root.to_string_lossy();
        let source = root.join("history/session.jsonl");
        fs::write(&source, format!(r##"{{"type":"user","sessionId":"00000000-0000-4000-8000-000000000001","cwd":"{cwd}","message":{{"content":"rememberable topic"}}}}
{{"type":"assistant","message":{{"content":"rememberable answer"}}}}
"##)).unwrap();
        let mut store = SqliteStore::open(root.join("private/index.sqlite")).unwrap();
        index_all(
            &mut store,
            &[Box::new(ClaudeAdapter::with_root(root.join("history")))],
        )
        .unwrap();
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
        state.handle(Key::Esc, &store).unwrap();
        assert_eq!(state.query, "rememberable");
        state.handle(Key::F2, &store).unwrap();
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.results[0].kind, EventKind::Assistant);
    }

    #[test]
    fn terminal_helpers_sanitize_and_wrap_wide_text() {
        assert_eq!(safe("a\n雪雪x", 4), "a 雪");
        assert!(wrap("abcdefghij 雪", 4)
            .iter()
            .all(|s| UnicodeWidthStr::width(s.as_str()) <= 4));
    }
}

use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::index_all,
    AgentAdapter, SqliteStore,
};
use agent_history_herdr::{
    overlay::{Mode, OverlayKey, OverlayState},
    restore::RecoveryChoice,
    socket::{HerdrCli, ProcessRunner},
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
};
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
fn safe(value: &str, width: usize) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(width)
        .collect()
}
fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut db = std::env::var_os("AGENT_HISTORY_DB").map(PathBuf::from);
    let mut claude = None;
    let mut codex = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("agent-history-overlay [--db PATH] [--claude-root PATH] [--codex-root PATH]\nExplicit root options disable default history discovery for both agents.\nType a query; arrows/Tab focus results; Space previews; Enter resumes; Esc goes back.");
                return Ok(());
            }
            "--db" | "--claude-root" | "--codex-root" => {
                let value = PathBuf::from(
                    args.next()
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
                PathBuf::from(h)
                    .join("Library/Application Support/Herdr Agent History/index.sqlite")
            })
        })
        .ok_or_else(|| io::Error::other("HOME is unavailable; specify --db"))?;
    let custom = claude.is_some() || codex.is_some();
    let adapters: Vec<Box<dyn AgentAdapter>> = if custom {
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
    let mut store = SqliteStore::open(db).map_err(|e| io::Error::other(e.to_string()))?;
    eprintln!("Agent History: updating local history…");
    let report = index_all(&mut store, &adapters).map_err(|e| io::Error::other(e.to_string()))?;
    let status = format!(
        "{} files processed; {} failed; {} malformed records skipped",
        report.files, report.failed_files, report.malformed_records
    );
    eprintln!("{status}");
    for error in &report.errors {
        eprintln!("{}", safe(error, 500))
    }
    let _screen = Screen::enter()?;
    let mut state = OverlayState {
        status,
        ..Default::default()
    };
    if !report.errors.is_empty() {
        state
            .status
            .push_str(&format!(" — {}", report.errors.join("; ")))
    }
    let mut host = HerdrCli::new(ProcessRunner);
    let mut stdout = io::stdout();
    while !state.closed {
        render(&mut stdout, &state)?;
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }
            let key = match key.code {
                KeyCode::Char(' ') => OverlayKey::Space,
                KeyCode::Char(c) => OverlayKey::Char(c),
                KeyCode::Backspace => OverlayKey::Backspace,
                KeyCode::Up => OverlayKey::Up,
                KeyCode::Down => OverlayKey::Down,
                KeyCode::Enter => OverlayKey::Enter,
                KeyCode::Esc => OverlayKey::Esc,
                KeyCode::Tab => OverlayKey::Tab,
                _ => continue,
            };
            state.handle(key, &store, &mut host);
        }
    }
    Ok(())
}
fn render(out: &mut impl Write, state: &OverlayState) -> io::Result<()> {
    let (width, height) = terminal::size()?;
    let width = width.saturating_sub(1) as usize;
    let mut lines = vec![
        format!("Agent History > {}", state.query),
        state.status.clone(),
    ];
    lines.push(
        "Type query · ↑↓/Tab results · Space preview · Enter resume · Esc back · Ctrl-C close"
            .into(),
    );
    if let Some(e) = &state.error {
        lines.push(format!("Error: {e}"))
    }
    match state.mode {
        Mode::Preview => {
            lines.push("Original conversation — Enter resume, Esc results".into());
            if let Some(r) = state.selected_result() {
                lines.push(format!(
                    "{:?} · {} · {}",
                    r.agent,
                    r.repository.as_deref().unwrap_or("unknown repository"),
                    r.branch.as_deref().unwrap_or("unknown branch")
                ));
            }
            lines.extend(
                state
                    .preview
                    .lines()
                    .skip(state.preview_scroll)
                    .map(str::to_owned),
            );
        }
        Mode::Recovery | Mode::ConfirmRecreate => {
            lines.push("The recorded workspace is unavailable.".into());
            if state.mode == Mode::ConfirmRecreate {
                lines.push(state.confirmation_text.clone());
                lines.push("Create the recorded worktree at its saved commit, then resume? y = confirm, n/Esc = back".into());
            } else {
                if state.recovery.contains(&RecoveryChoice::RecreateWorktree) {
                    lines.push("w — Recreate worktree and resume (confirmation follows)".into())
                }
                if state.recovery.contains(&RecoveryChoice::ExistingRepository) {
                    lines.push("r — Resume in existing repository".into())
                }
                lines.push("v — View original conversation    c/Esc — Cancel".into());
            }
        }
        _ => {
            if state.results.is_empty() {
                lines.push(
                    if state.query.is_empty() {
                        "Type words from a conversation."
                    } else {
                        "No matching sessions."
                    }
                    .into(),
                )
            }
            let visible = height.saturating_sub(lines.len() as u16 + 1) as usize / 2;
            let start = state.selected.saturating_sub(visible.saturating_sub(1));
            for (i, r) in state.results.iter().enumerate().skip(start).take(visible) {
                let date = r
                    .timestamp
                    .map(|t| {
                        let d: chrono::DateTime<chrono::Utc> = t.into();
                        d.format("%Y-%m-%d").to_string()
                    })
                    .unwrap_or_else(|| "unknown date".into());
                lines.push(format!(
                    "{} {:?} · {} · {} · {}",
                    if i == state.selected && state.mode == Mode::Results {
                        ">"
                    } else {
                        " "
                    },
                    r.agent,
                    r.repository.as_deref().unwrap_or("unknown repository"),
                    r.branch.as_deref().unwrap_or("unknown branch"),
                    date
                ));
                lines.push(format!("  {}", r.snippet));
            }
        }
    }
    execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
    for line in lines.into_iter().take(height as usize) {
        write!(out, "{}\r\n", safe(&line, width))?
    }
    out.flush()
}

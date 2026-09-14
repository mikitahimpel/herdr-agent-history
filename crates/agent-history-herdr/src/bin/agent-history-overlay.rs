use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::{index_all_with_progress, IndexProgress},
    AgentAdapter, EventKind, SqliteStore,
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
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
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
fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut db = std::env::var_os("AGENT_HISTORY_DB").map(PathBuf::from);
    let mut claude = None;
    let mut codex = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("agent-history-overlay [--db PATH] [--claude-root PATH] [--codex-root PATH]\nExplicit root options disable default history discovery for both agents.\nType a query; arrows/Tab focus results; F2 filters role; Space previews; Enter resumes; Esc goes back.");
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
    let indexing_started = Instant::now();
    let ticker_started = indexing_started;
    let progress = Arc::new(Mutex::new("Preparing conversation index…".to_string()));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ticker_progress = Arc::clone(&progress);
    let ticker_done = Arc::clone(&done);
    let ticker = thread::spawn(move || {
        let glyphs = ['·', '•', '●', '•'];
        let mut i = 0;
        while !ticker_done.load(std::sync::atomic::Ordering::Acquire) {
            let text = ticker_progress
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
            let line = format!(
                "Agent History {} ({:.1}s) {}",
                glyphs[i % glyphs.len()],
                ticker_started.elapsed().as_secs_f32(),
                text
            );
            let width = terminal::size()
                .map(|(w, _)| w.saturating_sub(1) as usize)
                .unwrap_or(100);
            eprint!("\r\x1b[2K{}", safe(&line, width));
            let _ = io::stderr().flush();
            i += 1;
            thread::sleep(Duration::from_millis(150));
        }
    });
    eprintln!("Agent History: preparing conversation index…");
    let mut store = match SqliteStore::open(db) {
        Ok(store) => store,
        Err(e) => {
            done.store(true, std::sync::atomic::Ordering::Release);
            let _ = ticker.join();
            eprintln!();
            return Err(io::Error::other(e.to_string()));
        }
    };
    if let Ok(mut p) = progress.lock() {
        *p = "Indexing discovered conversations…".into();
    }
    let progress_for_index = Arc::clone(&progress);
    let result = index_all_with_progress(&mut store, &adapters, |p: IndexProgress| {
        if let Ok(mut text) = progress_for_index.lock() {
            *text = format!(
                "{} files {}/{} · {} MB · {} records · {} chunks",
                match p.agent {
                    agent_history_core::Agent::Claude => "Claude",
                    agent_history_core::Agent::Codex => "Codex",
                },
                p.agent_completed_files,
                p.agent_total_files,
                p.bytes_read / (1024 * 1024),
                p.records,
                p.chunks
            );
        }
    });
    done.store(true, std::sync::atomic::Ordering::Release);
    let _ = ticker.join();
    eprintln!();
    let report = result.map_err(|e| io::Error::other(e.to_string()))?;
    let status = format!(
        "{} files · {} chunks · {} failed · {} malformed · {:.1}s",
        report.files,
        report.chunks,
        report.failed_files,
        report.malformed_records,
        indexing_started.elapsed().as_secs_f32()
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
        render(&mut stdout, &mut state)?;
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
                KeyCode::F(2) => OverlayKey::F2,
                _ => continue,
            };
            state.handle(key, &store, &mut host);
        }
    }
    Ok(())
}
fn render(out: &mut impl Write, state: &mut OverlayState) -> io::Result<()> {
    let (width, height) = terminal::size()?;
    let width = width.saturating_sub(1) as usize;
    let mut lines = vec![
        format!("Agent History  {}", state.query),
        format!(
            "Role: {}   (F2 changes)   {}",
            state.role_filter.label(),
            state.status
        ),
    ];
    lines.push(
        "Type query · ↑↓/Tab results · F2 role · Space preview · Enter resume · Esc back · Ctrl-C close"
            .into(),
    );
    if let Some(e) = &state.error {
        lines.push(format!("Error: {e}"))
    }
    match state.mode {
        Mode::Preview => {
            lines.push("Original conversation — Enter resume, Esc results".into());
            if let Some(r) = state.selected_result() {
                let mut context = vec![format!("{:?}", r.agent)];
                if let Some(repo) = r.repository.as_deref() {
                    context.push(compact_repo(repo).into());
                }
                if let Some(branch) = r.branch.as_deref() {
                    context.push(branch.into());
                }
                lines.push(context.join(" · "));
            }
            let preview_width = width.saturating_sub(1).max(12);
            let preview_lines: Vec<String> = state
                .preview
                .lines()
                .flat_map(|line| wrap(line, preview_width))
                .collect();
            let visible_lines = height.saturating_sub(lines.len() as u16 + 1) as usize;
            state.preview_scroll = state
                .preview_scroll
                .min(preview_lines.len().saturating_sub(visible_lines.max(1)));
            lines.extend(preview_lines.into_iter().skip(state.preview_scroll));
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
                    (None, None) => "-".to_string(),
                    (Some(repo), Some(branch)) => format!("{} / {branch}", compact_repo(repo)),
                    (Some(repo), None) => compact_repo(repo).to_string(),
                    (None, Some(branch)) => branch.to_string(),
                };
                lines.push(format!(
                    "{} {:?} · {} · {}",
                    if i == state.selected && state.mode == Mode::Results {
                        "▶"
                    } else {
                        " "
                    },
                    r.agent,
                    role_name(r.kind),
                    context,
                ));
                lines.push(format!("  {}", date));
                let mut wrapped = wrap(&r.snippet, width.saturating_sub(6).max(12));
                wrapped.truncate(2);
                for text in wrapped {
                    lines.push(format!("    {text}"));
                }
            }
        }
    }
    execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
    for line in lines.into_iter().take(height.saturating_sub(1) as usize) {
        write!(out, "{}\r\n", safe(&line, width))?
    }
    out.flush()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.lines() {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
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
                if !part.is_empty() {
                    line = part;
                }
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
fn role_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::User => "User",
        EventKind::Assistant => "Assistant",
        EventKind::ToolResult => "Tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_words_and_wide_characters_to_terminal_width() {
        let lines = wrap("abcdefghij 雪", 4);
        assert!(lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 4));
        assert_eq!(safe("雪雪x", 4), "雪雪");
    }
}

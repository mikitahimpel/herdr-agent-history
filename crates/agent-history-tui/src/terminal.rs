//! Raw-mode/alternate-screen lifetime. The terminal is restored when the
//! guard drops, when any thread panics, and when SIGINT or SIGTERM arrives.
use crossterm::{
    cursor, execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, Stdout},
    sync::{
        atomic::{AtomicBool, Ordering},
        Once,
    },
};

pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store: anything more is not async-signal-safe. The
    // event loops poll this flag and unwind normally.
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub(crate) fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: `on_signal` is async-signal-safe and lives for the process.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

pub(crate) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
}

fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}

/// Restores the terminal and exits as an interrupted process would. Used
/// while indexing, which cannot be cancelled cooperatively; SQLite
/// transactions keep the index consistent.
pub(crate) fn abort(term: Term) -> ! {
    drop(term);
    restore();
    std::process::exit(130)
}

pub(crate) struct Screen {
    term: Option<Term>,
}

impl Screen {
    pub(crate) fn enter() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut screen = Self { term: None };
        execute!(io::stdout(), EnterAlternateScreen)?;
        screen.term = Some(Terminal::new(CrosstermBackend::new(io::stdout()))?);
        Ok(screen)
    }

    /// Lends the terminal to another thread; hand it back with `put_back`.
    pub(crate) fn take(&mut self) -> Term {
        self.term.take().expect("terminal is lent out only once")
    }

    pub(crate) fn put_back(&mut self, term: Term) {
        self.term = Some(term);
    }

    pub(crate) fn terminal(&mut self) -> &mut Term {
        self.term.as_mut().expect("terminal was handed back")
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        drop(self.term.take());
        restore();
    }
}

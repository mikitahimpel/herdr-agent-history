//! Raw mode, alternate screen and mouse capture lifetime. The terminal is
//! restored when the guard drops, when any thread panics, and when SIGINT or
//! SIGTERM arrives.
use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, Stdout, Write},
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

/// Releases the mouse, leaves the alternate screen and shows the cursor.
/// Releasing capture that is already off is harmless, so this runs
/// unconditionally on every exit path.
pub(crate) fn write_restore(out: &mut impl Write) -> io::Result<()> {
    execute!(out, DisableMouseCapture, LeaveAlternateScreen, cursor::Show)
}

/// Turns mouse reporting on or off. Off returns drag-to-select to the
/// terminal.
pub(crate) fn set_mouse_capture(out: &mut impl Write, on: bool) -> io::Result<()> {
    if on {
        execute!(out, EnableMouseCapture)
    } else {
        execute!(out, DisableMouseCapture)
    }
}

fn restore() {
    let _ = disable_raw_mode();
    let _ = write_restore(&mut io::stdout());
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
        set_mouse_capture(&mut io::stdout(), true)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const MOUSE_OFF: &[&str] = &["\x1b[?1000l", "\x1b[?1002l", "\x1b[?1003l", "\x1b[?1006l"];

    fn written(f: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut out = Vec::new();
        f(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn restore_releases_the_mouse_before_leaving_the_screen() {
        let bytes = written(write_restore);
        for off in MOUSE_OFF {
            assert!(bytes.contains(off), "{off:?} missing from {bytes:?}");
        }
        let leave = bytes
            .find("\x1b[?1049l")
            .expect("leaves the alternate screen");
        assert!(bytes.find("\x1b[?1000l").unwrap() < leave);
        assert!(bytes.contains("\x1b[?25h"), "shows the cursor");
    }

    #[test]
    fn toggle_turns_reporting_off_and_back_on() {
        let off = written(|o| set_mouse_capture(o, false));
        for seq in MOUSE_OFF {
            assert!(off.contains(seq), "{seq:?} missing from {off:?}");
        }
        let on = written(|o| set_mouse_capture(o, true));
        for seq in ["\x1b[?1000h", "\x1b[?1006h"] {
            assert!(on.contains(seq), "{seq:?} missing from {on:?}");
        }
        assert!(!on.contains("?1000l"), "turning on never also turns off");
    }
}

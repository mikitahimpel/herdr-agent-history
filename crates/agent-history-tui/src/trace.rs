//! Phase timings for one startup, written to the file named by
//! `AGENT_HISTORY_TRACE`. The browser owns the terminal, so timings cannot go
//! to stdout or stderr; they are appended to a file the reader chooses, and
//! nothing is written or measured when the variable is unset.
use std::time::{Duration, Instant};

/// A startup being timed, or nothing at all when tracing is off.
pub(crate) struct Trace {
    inner: Option<Inner>,
}
struct Inner {
    path: std::path::PathBuf,
    name: String,
    started: Instant,
    last: Instant,
    line: String,
}

impl Trace {
    /// Starts timing when `path` names a file to append to, and measures
    /// nothing at all when it is `None`.
    pub(crate) fn start(name: &str, path: Option<std::ffi::OsString>) -> Self {
        let Some(path) = path else {
            return Self { inner: None };
        };
        let now = Instant::now();
        Self {
            inner: Some(Inner {
                path: path.into(),
                name: name.into(),
                started: now,
                last: now,
                line: format!("{name}:"),
            }),
        }
    }

    /// Records the time since the previous phase ended.
    pub(crate) fn phase(&mut self, name: &str) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let now = Instant::now();
        inner
            .line
            .push_str(&format!(" {name}={:.1}", ms(now - inner.last)));
        inner.last = now;
    }

    /// Writes what has been recorded so far as one line, with the total
    /// elapsed, and keeps timing. A startup that never reaches a mark writes
    /// nothing for it.
    pub(crate) fn mark(&mut self, label: &str) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let line = format!(
            "{} {label}={:.1}\n",
            std::mem::replace(&mut inner.line, format!("{}:", inner.name)),
            ms(inner.started.elapsed())
        );
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inner.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is measured and nothing is written unless a file is named,
    /// which is how every ordinary run goes.
    #[test]
    fn without_a_file_nothing_is_traced() {
        let mut trace = Trace::start("startup", None);
        trace.phase("screen");
        trace.mark("usable");
        assert!(trace.inner.is_none());
    }

    #[test]
    fn each_mark_is_one_line_naming_the_phases_before_it() {
        let dir = agent_history_core::test_support::TempDir::new("tui-trace").unwrap();
        let path = dir.path().join("trace.txt");
        let mut trace = Trace::start("startup", Some(path.clone().into()));
        trace.phase("screen");
        trace.mark("usable");
        trace.phase("scan");
        trace.mark("settled");
        let written = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2, "{written:?}");
        for (line, phases) in lines
            .iter()
            .zip([["screen", "usable"], ["scan", "settled"]])
        {
            assert!(line.starts_with("startup: "), "{line:?}");
            for phase in phases {
                assert!(line.contains(&format!("{phase}=")), "{line:?}");
            }
        }
        // Every phase of a mark is reported once, under that mark alone.
        assert!(!lines[1].contains("screen="));
    }
}

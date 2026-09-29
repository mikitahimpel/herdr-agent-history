//! Process-level output shared by every Agent History binary: the version
//! string and the one-line `name: message` form every error takes on stderr.
use std::{fmt::Display, process::ExitCode};

/// Package version, followed by the release identifier `scripts/package`
/// stamps through `AGENT_HISTORY_BUILD_ID`. Every 0.1.0 prerelease shares the
/// crate version, so only the identifier tells one build from another.
pub fn version() -> String {
    let build = option_env!("AGENT_HISTORY_BUILD_ID").filter(|id| !id.is_empty());
    format!(
        "{} ({})",
        env!("CARGO_PKG_VERSION"),
        sanitize(build.unwrap_or("development build"))
    )
}

/// Replaces control characters other than newline and tab, so an error that
/// quotes a path or transcript text cannot drive the terminal.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect()
}

/// Ends the process: success, or `name: message` on stderr and exit status 2.
/// The message is the error's `Display`, never its `Debug`.
pub fn exit<E: Display>(name: &str, result: Result<(), E>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error_line(name, &error));
            ExitCode::from(2)
        }
    }
}

fn error_line(name: &str, error: &impl Display) -> String {
    format!("{name}: {}", sanitize(&error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn io_errors_render_their_message_not_their_debug_struct() {
        let error = io::Error::other("cannot open index");
        assert_eq!(
            error_line("agent-history-overlay", &error),
            "agent-history-overlay: cannot open index"
        );
    }

    #[test]
    fn error_lines_neutralize_terminal_controls() {
        let error = io::Error::other("bad \u{1b}[31mpath\u{7}\r");
        let line = error_line("agent-history-herdr", &error);
        assert_eq!(line, "agent-history-herdr: bad �[31mpath��");
    }

    #[test]
    fn version_starts_with_the_package_version() {
        let version = version();
        assert!(version.starts_with(concat!(env!("CARGO_PKG_VERSION"), " (")));
        assert!(version.ends_with(')'));
        assert!(!version.chars().any(char::is_control));
    }
}

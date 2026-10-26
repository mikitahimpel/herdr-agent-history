//! Which native sessions a process on this machine is already running.
//!
//! Herdr names the session a pane started with, and Claude's own id moves on
//! from there: `/resume`, a fork or a continuation writes a new transcript
//! under a new id while the same pane keeps showing the conversation. Asking
//! the host alone therefore reports a live conversation as merely resumable,
//! and Enter starts a second process against a transcript that already has
//! one. The agent itself is not so forgetful: it carries the id it is serving
//! in its own command line, which is true whoever started it, Herdr or not.
use crate::resume::is_uuid;
use agent_history_core::{Agent, SessionId};
use std::collections::HashSet;

/// Every native session an agent process on this machine is serving.
///
/// A failure to ask is not a claim that nothing is running: it yields an
/// empty set, which only costs the caller the improvement this provides.
pub fn running_sessions() -> HashSet<SessionId> {
    // `-x` is this user's processes, which is as far as a resume could reach
    // anyway, and `ww` keeps the whole command line rather than a window's
    // worth of it.
    let output = std::process::Command::new("/bin/ps")
        .args(["-xww", "-o", "args="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            session_ids(String::from_utf8_lossy(&output.stdout).lines())
        }
        _ => HashSet::new(),
    }
}

/// The sessions named by a list of command lines, one per line.
pub fn session_ids<'a>(command_lines: impl Iterator<Item = &'a str>) -> HashSet<SessionId> {
    command_lines.filter_map(session_id).collect()
}

/// The session one command line serves, judged by the shape of its arguments
/// rather than by the program's name: Claude runs from a versioned path whose
/// file name is a version number, not `claude`.
fn session_id(command_line: &str) -> Option<SessionId> {
    let argv: Vec<&str> = command_line.split_whitespace().collect();
    let after = |flag: &str| {
        argv.iter()
            .rposition(|arg| *arg == flag)
            .and_then(|i| argv.get(i + 1))
            .copied()
            .filter(|value| is_uuid(value))
    };
    // The id the agent is writing under, which a fork sets while `--resume`
    // still names the transcript it forked from.
    if let Some(id) = after("--session-id") {
        return Some(SessionId::new(Agent::Claude, id));
    }
    // `claude --resume <id>`, the form this integration starts itself. A
    // `--resume <path>` is a transcript, not an id, and is left alone.
    if let Some(id) = after("--resume") {
        return Some(SessionId::new(Agent::Claude, id));
    }
    // `codex resume <id>`, whose subcommand is positional. The program is
    // checked here because `resume` is too ordinary a word to stand alone.
    let codex = argv
        .iter()
        .any(|arg| arg.rsplit('/').next() == Some("codex"));
    match after("resume") {
        Some(id) if codex => Some(SessionId::new(Agent::Codex, id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000002";

    fn one(command_line: &str) -> Option<SessionId> {
        session_id(command_line)
    }

    #[test]
    fn a_fork_is_serving_the_id_it_writes_not_the_one_it_read() {
        // The shape Claude's own background host runs in: the transcript it
        // resumed from is a path, and the live conversation is `--session-id`.
        let claude = format!(
            "/Users/x/.local/share/claude/versions/2.1.306 --session-id {ID} \
             --fork-session --resume /Users/x/.claude/projects/-p/{OTHER}.jsonl --model opus"
        );
        assert_eq!(one(&claude), Some(SessionId::new(Agent::Claude, ID)));
    }

    #[test]
    fn the_form_this_integration_starts_is_recognized() {
        assert_eq!(
            one(&format!("claude --resume {ID}")),
            Some(SessionId::new(Agent::Claude, ID))
        );
        assert_eq!(
            one(&format!("/usr/local/bin/codex resume {ID}")),
            Some(SessionId::new(Agent::Codex, ID))
        );
    }

    #[test]
    fn a_resume_that_names_no_session_claims_none() {
        // Nothing here identifies a conversation, and guessing one would
        // suppress a resume the user is entitled to.
        for argv in [
            "claude",
            "claude --resume",
            "codex resume --last",
            &format!("claude --resume /Users/x/.claude/projects/-p/{ID}.jsonl"),
            &format!("some-tool resume {ID}"),
            &format!("claude --session-id {}", &ID[..8]),
        ] {
            assert_eq!(one(argv), None, "{argv}");
        }
    }

    #[test]
    fn each_agent_keeps_its_own_identifier_space() {
        let ids = session_ids(
            [
                format!("claude --resume {ID}"),
                format!("codex resume {ID}"),
                format!("claude --resume {ID}"),
            ]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .into_iter(),
        );
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&SessionId::new(Agent::Claude, ID)));
        assert!(ids.contains(&SessionId::new(Agent::Codex, ID)));
    }
}

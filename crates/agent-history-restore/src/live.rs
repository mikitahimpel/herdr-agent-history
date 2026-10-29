//! Which native sessions a process on this machine is already running.
//!
//! A host names the session a pane started with, and Claude's own id moves on
//! from there: `/resume`, a fork or a continuation writes a new transcript
//! under a new id while the same pane keeps showing the conversation. Asking
//! the host alone therefore reports a live conversation as merely resumable,
//! and Enter starts a second process against a transcript that already has
//! one. The agent itself is not so forgetful: it carries the id it is serving
//! in its own command line, which is true whoever started it, Herdr or not.
use crate::plan::is_uuid;
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
    command_lines.flat_map(session_ids_in).collect()
}

/// Every session one command line serves, judged by the shape of its
/// arguments rather than by the program's name: Claude runs from a versioned
/// path whose file name is a version number, not `claude`.
///
/// One process can serve more than one of them. A fork writes a new
/// transcript under a new ID while reading the one it grew out of, and both
/// are the conversation the reader was looking at, so resuming either would
/// put a second agent beside the one already showing it.
fn session_ids_in(command_line: &str) -> Vec<SessionId> {
    let argv: Vec<&str> = command_line.split_whitespace().collect();
    let values = |flag: &'static str| {
        argv.iter()
            .enumerate()
            .filter(move |(_, arg)| **arg == flag)
            .filter_map(|(i, _)| argv.get(i + 1).copied())
    };
    // The ID the agent writes under, which a fork sets for itself.
    let written = values("--session-id")
        .filter(|value| is_uuid(value))
        .map(|id| SessionId::new(Agent::Claude, id));
    // What `--resume` was pointed at: an ID, as this integration starts it,
    // or the transcript a fork grew out of.
    let read = values("--resume")
        .filter_map(resumed_session)
        .map(|id| SessionId::new(Agent::Claude, id));
    // `codex resume <id>`, whose subcommand is positional. The program is
    // checked here because `resume` is too ordinary a word to stand alone.
    let codex = argv
        .iter()
        .any(|arg| arg.rsplit('/').next() == Some("codex"))
        .then(|| values("resume").filter(|value| is_uuid(value)))
        .into_iter()
        .flatten()
        .map(|id| SessionId::new(Agent::Codex, id));
    written.chain(read).chain(codex).collect()
}

/// The session a `--resume` value names: an ID, or the transcript file of one.
fn resumed_session(value: &str) -> Option<&str> {
    if is_uuid(value) {
        return Some(value);
    }
    let id = value.rsplit('/').next()?.strip_suffix(".jsonl")?;
    is_uuid(id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000002";

    fn one(command_line: &str) -> Vec<SessionId> {
        let mut ids = session_ids_in(command_line);
        ids.sort();
        ids
    }

    fn claude(id: &str) -> SessionId {
        SessionId::new(Agent::Claude, id)
    }

    /// The shape Claude's own background host runs in. Both conversations are
    /// the one on screen: resuming the transcript it grew out of would start a
    /// second agent beside the fork already showing it.
    #[test]
    fn a_fork_serves_the_conversation_it_read_as_well_as_the_one_it_writes() {
        let argv = format!(
            "/Users/x/.local/share/claude/versions/2.1.306 --session-id {ID} \
             --fork-session --resume /Users/x/.claude/projects/-p/{OTHER}.jsonl --model opus"
        );
        let mut expected = vec![claude(ID), claude(OTHER)];
        expected.sort();
        assert_eq!(one(&argv), expected);
    }

    #[test]
    fn the_form_this_integration_starts_is_recognized() {
        assert_eq!(one(&format!("claude --resume {ID}")), [claude(ID)]);
        assert_eq!(
            one(&format!("/usr/local/bin/codex resume {ID}")),
            [SessionId::new(Agent::Codex, ID)]
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
            "claude --resume /Users/x/.claude/projects/-p/latest.jsonl",
            &format!("claude --resume /Users/x/.claude/projects/-p/{ID}.json"),
            &format!("some-tool resume {ID}"),
            &format!("claude --session-id {}", &ID[..8]),
        ] {
            assert_eq!(one(argv), [], "{argv}");
        }
    }

    #[test]
    fn a_transcript_path_names_the_session_it_holds() {
        assert_eq!(
            one(&format!(
                "claude --resume /Users/x/.claude/projects/-p/{ID}.jsonl"
            )),
            [claude(ID)]
        );
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

//! Host-independent pieces of resuming a session: the native command, which
//! sessions a local process already serves, and confirmed worktree recovery.
//! Each host integration (Herdr, Rex) decides where the command runs.
use agent_history_core::{Agent, Session};

pub mod live;
pub mod plan;
pub mod restore;

/// What resuming did, so the browser can say so rather than leave the reader
/// guessing which of these happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resumed {
    /// A pane already running this session was brought to the front.
    Focused,
    /// The session was not running, so the agent was started for it.
    Started,
    /// A process on this machine is already serving this session and no pane
    /// could be identified for it, so nothing was started.
    AlreadyRunning,
}

fn agent_name(session: &Session) -> &'static str {
    match session.id.agent {
        Agent::Claude => "Claude",
        Agent::Codex => "Codex",
    }
}

/// Names the session that was resumed, so returning to the browser explains
/// what happened rather than looking as though nothing did. A session that
/// was already running was shown, not started, and saying "resumed" for both
/// would hide the difference that matters here.
pub fn resumed_notice(session: &Session, outcome: Resumed) -> String {
    let agent = agent_name(session);
    let verb = match outcome {
        Resumed::Focused => "Opened running",
        _ => "Resumed",
    };
    match session.repository.as_deref() {
        Some(repository) => format!("{verb} {agent} session in {repository}"),
        None => format!("{verb} {agent} session"),
    }
}

/// Explains the one case where Enter starts nothing: the conversation is
/// already being served, but not by a pane the integration can point to.
pub fn already_running_notice(session: &Session) -> String {
    format!(
        "This {} session is already running on this machine; showing it here instead of starting a second one",
        agent_name(session)
    )
}

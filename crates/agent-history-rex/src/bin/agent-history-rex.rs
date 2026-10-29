use agent_history_rex::{
    cli::{ProcessRunner, RexAgentCli},
    integration::RexIntegration,
};
use agent_history_tui::{report, run};
use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    report::exit("agent-history-rex", start())
}

fn start() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let integration = || RexIntegration::new(RexAgentCli::new(ProcessRunner::from_env()));
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return run(args, &mut integration());
    }
    // Rex sets REX_SESSION in every terminal it hosts.
    if std::env::var_os("REX_SESSION").is_none_or(|session| session.is_empty()) {
        return Err(io::Error::other(
            "Open a terminal in Rex, or use agent-history browse for standalone search.",
        ));
    }
    run(
        args,
        &mut integration()
            .with_background_host(|| Box::new(RexAgentCli::new(ProcessRunner::from_env()))),
    )
}

use agent_history_tui::report;
use std::process::ExitCode;

fn main() -> ExitCode {
    report::exit(
        "agent-history-overlay",
        agent_history_tui::run(
            std::env::args().skip(1).collect(),
            &mut agent_history_tui::Standalone,
        ),
    )
}

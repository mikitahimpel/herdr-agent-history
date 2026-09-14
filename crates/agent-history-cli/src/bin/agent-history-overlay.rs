fn main() -> std::io::Result<()> {
    agent_history_tui::run(
        std::env::args().skip(1).collect(),
        &mut agent_history_tui::Standalone,
    )
}

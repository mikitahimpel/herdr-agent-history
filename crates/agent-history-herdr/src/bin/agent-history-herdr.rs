use agent_history_herdr::{
    integration::HerdrIntegration,
    socket::{HerdrCli, ProcessRunner},
};
use agent_history_tui::run;
use std::io;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return run(
            args,
            &mut HerdrIntegration::new(HerdrCli::new(ProcessRunner)),
        );
    }
    if std::env::var_os("HERDR_ENV").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Err(io::Error::other(
            "Open a terminal pane in Herdr, or use agent-history browse for standalone search.",
        ));
    }
    run(
        args,
        &mut HerdrIntegration::new(HerdrCli::new(ProcessRunner)),
    )
}

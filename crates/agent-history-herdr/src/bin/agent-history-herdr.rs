use agent_history_herdr::{
    integration::HerdrIntegration,
    socket::{HerdrCli, ProcessRunner},
    theme,
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
        &mut HerdrIntegration::new(HerdrCli::new(ProcessRunner))
            .with_palette(theme::palette_from_env())
            // Herdr sets this only for plugin panes, which it frames with the
            // manifest's title; a manual run in an ordinary pane has no title.
            .with_host_title(std::env::var_os("HERDR_PLUGIN_ID").is_some()),
    )
}

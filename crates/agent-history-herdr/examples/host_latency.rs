//! What asking Herdr costs: each command the priority query runs, and the
//! process scan beside them. This is the work that used to stand between
//! launching the panel and its first keystroke.
//!
//! `cargo run --release -p agent-history-herdr --example host_latency`
//!
//! Requires a running Herdr. It only reads, and prints counts and timings,
//! never workspace paths or session identifiers.
use agent_history_herdr::socket::{parse_workspace_list, HerdrCli, ProcessRunner};
use std::time::{Duration, Instant};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let whole = Instant::now();
    let mut cli = HerdrCli::new(ProcessRunner);

    let t = Instant::now();
    let listed = cli.list_workspaces()?;
    println!("version + workspace list {:7.1} ms", ms(t.elapsed()));

    let workspaces = parse_workspace_list(&listed)?;
    let t = Instant::now();
    for workspace in &workspaces {
        let _ = cli.list_panes(&workspace.id)?;
    }
    println!(
        "pane list per workspace  {:7.1} ms  ({} workspaces)",
        ms(t.elapsed()),
        workspaces.len()
    );

    let t = Instant::now();
    let agents = cli.live_agents()?;
    println!(
        "agent list               {:7.1} ms  ({} agents)",
        ms(t.elapsed()),
        agents.len()
    );

    let t = Instant::now();
    let running = agent_history_herdr::live::running_sessions();
    println!(
        "process scan             {:7.1} ms  ({} sessions served)",
        ms(t.elapsed()),
        running.len()
    );

    println!("asked one after another  {:7.1} ms", ms(whole.elapsed()));
    Ok(())
}

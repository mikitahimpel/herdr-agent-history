use std::process::Command;

#[test]
fn refuses_non_herdr_invocation_before_opening_database() {
    let temp = agent_history_core::test_support::TempDir::new("herdr-preflight").unwrap();
    let root = temp.path();
    let db = root.join("private/index.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-history-herdr"))
        .args(["--db", db.to_str().unwrap()])
        .env_remove("HERDR_ENV")
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Open a terminal pane in Herdr"), "{stderr}");
    assert!(!db.exists(), "preflight must run before database creation");
}

#[test]
fn help_does_not_require_herdr_context_or_create_database() {
    let temp = agent_history_core::test_support::TempDir::new("herdr-help").unwrap();
    let root = temp.path();
    let db = root.join("private/index.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-history-herdr"))
        .args(["--help", "--db", db.to_str().unwrap()])
        .env_remove("HERDR_ENV")
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!db.exists());
}

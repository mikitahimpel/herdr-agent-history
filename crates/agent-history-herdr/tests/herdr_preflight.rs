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
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "agent-history-herdr: Open a terminal pane in Herdr, or use agent-history browse for standalone search.\n"
    );
    assert!(!db.exists(), "preflight must run before database creation");
}

#[test]
fn startup_errors_are_one_sanitized_line_without_debug_formatting() {
    let temp = agent_history_core::test_support::TempDir::new("herdr-error").unwrap();
    let root = temp.path();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-history-herdr"))
        .arg("--bo\u{1b}]0;title\u{7}gus")
        .env("HERDR_ENV", "1")
        .env("HOME", root.join("home"))
        .env_remove("AGENT_HISTORY_DB")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "agent-history-herdr: unknown option: --bo\u{fffd}]0;title\u{fffd}gus\n"
    );
    assert!(!root.join("home").exists());
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

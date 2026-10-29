use std::process::Command;

#[test]
fn refuses_to_run_outside_rex_before_opening_the_database() {
    let temp = agent_history_core::test_support::TempDir::new("rex-preflight").unwrap();
    let db = temp.path().join("private/index.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-history-rex"))
        .args(["--db", db.to_str().unwrap()])
        .env_remove("REX_SESSION")
        .env("HOME", temp.path().join("home"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "agent-history-rex: Open a terminal in Rex, or use agent-history browse for standalone search.\n"
    );
    assert!(!db.exists(), "preflight must run before database creation");
}

#[test]
fn help_works_anywhere_and_is_titled_with_the_product_name() {
    let temp = agent_history_core::test_support::TempDir::new("rex-help").unwrap();
    let db = temp.path().join("private/index.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_agent-history-rex"))
        .args(["--help", "--db", db.to_str().unwrap()])
        .env_remove("REX_SESSION")
        .env("HOME", temp.path().join("home"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.starts_with(&format!("{}\n", agent_history_rex::integration::TITLE)),
        "{help}"
    );
    assert!(!db.exists());
}

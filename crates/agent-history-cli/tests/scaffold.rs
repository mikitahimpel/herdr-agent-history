//! Process-level guard against falsely claiming unfinished commands succeeded.
use std::process::Command;

#[test]
fn unfinished_commands_fail_explicitly() {
    for args in [
        vec!["index"],
        vec!["search", "portfolio visibility"],
        vec!["status"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-history"))
            .args(args)
            .output()
            .expect("scaffold binary must launch");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented yet"));
    }
}

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> (agent_history_core::test_support::TempDir, PathBuf, PathBuf) {
    let root = agent_history_core::test_support::TempDir::new("cli-fixture").unwrap();
    let claude = root.path().join("claude");
    let codex = root.path().join("codex");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    fs::write(claude.join("claude-session.jsonl"), br#"{"type":"user","sessionId":"claude-session","timestamp":"2026-09-14T12:00:00Z","message":{"content":"portfolio visibility"}}
{"type":"assistant","sessionId":"claude-session","message":{"content":"Claude answer"}}
"#).unwrap();
    fs::write(
        codex.join("codex-session.jsonl"),
        br#"{"type":"session_meta","payload":{"id":"codex-session","cwd":"/tmp"}}
{"type":"response_item","role":"user","payload":{"type":"message","content":"portfolio visibility"}}
{"type":"response_item","role":"assistant","payload":{"type":"message","content":"Codex answer"}}
"#,
    )
    .unwrap();
    (root, claude, codex)
}

fn run(db: &Path, claude: &Path, codex: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agent-history"))
        .args(args)
        .args([
            "--db",
            db.to_str().unwrap(),
            "--claude-root",
            claude.to_str().unwrap(),
            "--codex-root",
            codex.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

#[test]
fn index_search_status_preview_and_append_work_across_processes() {
    let (root, claude, codex) = fixture();
    let db = root.path().join("private/index.sqlite");
    let out = run(&db, &claude, &codex, &["index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run(&db, &claude, &codex, &["search", "portfolio visibility"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(text.contains("Claude"));
    assert!(text.contains("Codex"));
    assert!(text.contains("2026-09-14T12:00:00"));
    for (query, role, matches) in [
        ("portfolio", "user", true),
        ("portfolio", "assistant", false),
        ("answer", "assistant", true),
        ("answer", "user", false),
    ] {
        let out = run(&db, &claude, &codex, &["search", query, "--role", role]);
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(text.contains("Claude"), matches);
        assert_eq!(text.contains("Codex"), matches);
    }
    let out = run(&db, &claude, &codex, &["status"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sessions: 2"));
    assert!(text.contains("Claude: 1"));
    assert!(text.contains("Codex: 1"));
    let out = run(
        &db,
        &claude,
        &codex,
        &["preview", "claude", "claude-session"],
    );
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("portfolio visibility"));
    fs::OpenOptions::new()
        .append(true)
        .open(claude.join("claude-session.jsonl"))
        .unwrap()
        .write_all(br#"{"type":"user","sessionId":"claude-session","message":{"content":"appended detail"}}
"#)
        .unwrap();
    let out = run(&db, &claude, &codex, &["index"]);
    assert!(out.status.success());
    let out = run(&db, &claude, &codex, &["search", "appended"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("appended"));
}

#[test]
fn help_version_and_argument_errors_are_deterministic() {
    let bin = env!("CARGO_BIN_EXE_agent-history");
    let out = Command::new(bin).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("agent-history 0.1.0"));
    let out = Command::new(bin).arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("search <query>"));
    let out = Command::new(bin)
        .args(["search", "--bogus", "--db", "/tmp/x"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option"));
}

#[test]
fn standalone_entrypoints_need_no_host_or_index_for_help() {
    let root = agent_history_core::test_support::TempDir::new("standalone-help").unwrap();
    for (binary, args) in [
        (
            env!("CARGO_BIN_EXE_agent-history"),
            vec!["browse", "--help"],
        ),
        (env!("CARGO_BIN_EXE_agent-history-overlay"), vec!["--help"]),
    ] {
        let output = Command::new(binary)
            .args(args)
            .env_remove("HERDR_ENV")
            .env_remove("HERDR_BIN_PATH")
            .env("HOME", root.path())
            .env("PATH", root.path().join("no-executables"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.to_lowercase().contains("preview"));
        assert!(!help.contains("Enter resume"));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn missing_home_and_removed_reset_fail_without_modifying_files() {
    let dir = agent_history_core::test_support::TempDir::new("cli-errors").unwrap();
    let native = dir.path().join("history.jsonl");
    fs::write(&native, "private fixture").unwrap();
    let bin = env!("CARGO_BIN_EXE_agent-history");
    let out = Command::new(bin)
        .arg("status")
        .env_remove("HOME")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let out = Command::new(bin)
        .args(["reset", "--db"])
        .arg(&native)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(native).unwrap(), "private fixture");
}

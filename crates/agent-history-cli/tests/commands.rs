use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn fixture() -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("agent-history-cli-{}", std::process::id()));
    let claude = root.join("claude");
    let codex = root.join("codex");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let mut permissions = fs::metadata(&root).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&root, permissions).unwrap();
    fs::write(claude.join("claude-session.jsonl"), br#"{"type":"user","sessionId":"claude-session","message":{"content":"portfolio visibility"}}
{"type":"assistant","sessionId":"claude-session","message":{"content":"Claude answer"}}
"#).unwrap();
    fs::write(
        codex.join("codex-session.jsonl"),
        br#"{"type":"session_meta","payload":{"id":"codex-session","cwd":"/tmp"}}
{"type":"response_item","role":"user","payload":{"content":"portfolio visibility"}}
{"type":"response_item","role":"assistant","payload":{"content":"Codex answer"}}
"#,
    )
    .unwrap();
    (root.clone(), claude, codex)
}

use std::os::unix::fs::PermissionsExt;

fn run(db: &PathBuf, claude: &PathBuf, codex: &PathBuf, args: &[&str]) -> std::process::Output {
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
    let db = root.join("index.sqlite");
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
    let _ = fs::remove_dir_all(root);
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

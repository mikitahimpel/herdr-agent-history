use agent_history_core::{
    adapters::{ClaudeAdapter, CodexAdapter},
    index::index_all,
    preview::preview_source,
    test_support::TempDir,
    AgentAdapter, EventKind, GitOrigin, SqliteStore,
};
use std::{fs, io::Write};

#[test]
fn conversation_filters_and_preview_exclude_tools_across_restart() {
    let dir = TempDir::new("conversation-flow").unwrap();
    let claude = dir.jsonl("claude.jsonl", &[
        r#"{"type":"user","sessionId":"claude-fixture","message":{"content":[{"type":"text","text":"usermarker commonmarker"},{"type":"tool_result","content":"toolsecret commonmarker"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"assistantmarker commonmarker: use `let value = 1;`"},{"type":"tool_use","name":"read_file","input":{"path":"toolsecret"}},{"type":"thinking","thinking":"reasonsecret"}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"toolsecret loaded file contents"}]}}"#,
    ]).unwrap();
    let codex = dir.jsonl("codex.jsonl", &[
        r#"{"type":"session_meta","payload":{"id":"codex-fixture"}}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"usermarker commonmarker"}]}}"#,
        r#"{"type":"response_item","payload":{"type":"function_call_output","output":"toolsecret commonmarker"}}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"output_text","text":"reasonsecret"}]}}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","channel":"final","content":[{"type":"output_text","text":"assistantmarker commonmarker: use `let value = 1;`"}]}}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"systemsecret"}]}}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","channel":"commentary","recipient":"functions.exec","content":[{"type":"output_text","text":"toolsecret"}]}}"#,
        r#"{"type":"response_item","channel":"analysis","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"reasonsecret"}]}}"#,
    ]).unwrap();
    let originals = [fs::read(&claude).unwrap(), fs::read(&codex).unwrap()];
    let adapters: Vec<Box<dyn AgentAdapter>> = vec![
        Box::new(ClaudeAdapter::with_root(&claude)),
        Box::new(CodexAdapter::with_root(&codex)),
    ];
    let db = dir.path().join("private/index.sqlite");
    let mut store = SqliteStore::open(&db).unwrap();
    let report = index_all(&mut store, &adapters).unwrap();
    assert_eq!(report.failed_files, 0);
    assert_eq!(report.malformed_records, 0);
    assert_eq!(store.search("commonmarker", 20).unwrap().len(), 4);
    for (role, own, other) in [
        (EventKind::User, "usermarker", "assistantmarker"),
        (EventKind::Assistant, "assistantmarker", "usermarker"),
    ] {
        let results = store
            .search_with_role("commonmarker", 20, Some(role))
            .unwrap();
        assert_eq!(results.len(), 2);
        for result in results {
            assert_eq!(result.kind, role);
            assert!(result.snippet.contains(own));
            assert!(!result.snippet.contains(other));
            let preview = preview_source(&store, &result.source, 4096).unwrap();
            assert!(preview.text.contains("User: usermarker"));
            assert!(preview.text.contains("Assistant: assistantmarker"));
            assert!(!preview.text.contains("toolsecret"));
            assert!(!preview.text.contains("reasonsecret"));
        }
        assert!(store
            .search_with_role(other, 20, Some(role))
            .unwrap()
            .is_empty());
    }
    for excluded in ["toolsecret", "reasonsecret", "systemsecret"] {
        assert!(store.search(excluded, 20).unwrap().is_empty());
    }
    assert_eq!(store.search("value", 20).unwrap().len(), 2);
    assert_eq!(fs::read(&claude).unwrap(), originals[0]);
    assert_eq!(fs::read(&codex).unwrap(), originals[1]);
    drop(store);
    let mut file = fs::OpenOptions::new().append(true).open(&codex).unwrap();
    let followup = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"followupmarker"}]}}"#;
    writeln!(file, "{followup}").unwrap();
    drop(file);
    let mut store = SqliteStore::open(db).unwrap();
    assert_eq!(index_all(&mut store, &adapters).unwrap().failed_files, 0);
    assert_eq!(
        store
            .search_with_role("followupmarker", 20, Some(EventKind::User))
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .search_with_role("followupmarker", 20, Some(EventKind::Assistant))
        .unwrap()
        .is_empty());
    assert_eq!(store.search("commonmarker", 20).unwrap().len(), 4);
}

#[test]
fn schema_two_rebuild_preserves_captured_context_and_invalidates_mixed_text() {
    let dir = TempDir::new("conversation-upgrade").unwrap();
    let source = dir.jsonl("history.jsonl", &[
        r#"{"type":"user","sessionId":"upgrade-session","cwd":"/missing/old-worktree","message":{"content":"remembermarker"}}"#,
        r#"{"type":"assistant","message":{"content":"answermarker"}}"#,
    ]).unwrap();
    let native = fs::read(&source).unwrap();
    let db = dir.path().join("private/index.sqlite");
    let adapters: Vec<Box<dyn AgentAdapter>> = vec![Box::new(ClaudeAdapter::with_root(&source))];
    let mut store = SqliteStore::open(&db).unwrap();
    index_all(&mut store, &adapters).unwrap();
    let old_result = store.search("remembermarker", 1).unwrap().remove(0);
    let state = store.indexed_file_state(&source).unwrap().unwrap();
    let mut checkpoint: serde_json::Value =
        serde_json::from_slice(&state.open_turn_state.unwrap()).unwrap();
    checkpoint.as_object_mut().unwrap().remove("format_version");
    checkpoint["session"]["repository_root"] = serde_json::json!("/captured/repository");
    checkpoint["session"]["repository"] = serde_json::json!("/captured/repository");
    checkpoint["session"]["branch"] = serde_json::json!("saved-branch");
    let builder_bytes: Vec<u8> = serde_json::from_value(checkpoint["builder"].clone()).unwrap();
    let mut builder: serde_json::Value = serde_json::from_slice(&builder_bytes).unwrap();
    builder["pending"].as_object_mut().unwrap().remove("kind");
    checkpoint["builder"] = serde_json::to_value(serde_json::to_vec(&builder).unwrap()).unwrap();
    let legacy = serde_json::to_vec(&checkpoint).unwrap();
    store
        .connection()
        .execute("UPDATE indexed_files SET open_turn_state=?", [&legacy])
        .unwrap();
    store.connection().execute_batch("UPDATE search_chunks SET text=text || ' toolsecret'; ALTER TABLE search_chunks DROP COLUMN kind; ALTER TABLE sessions DROP COLUMN git_origin; ALTER TABLE sessions DROP COLUMN repository_url; PRAGMA user_version=2;").unwrap();
    drop(store);

    let mut store = SqliteStore::open(&db).unwrap();
    assert!(store.search("toolsecret", 10).unwrap().is_empty());
    assert_eq!(
        store
            .indexed_file_state(&source)
            .unwrap()
            .unwrap()
            .open_turn_state,
        Some(legacy)
    );
    let report = index_all(&mut store, &adapters).unwrap();
    assert_eq!(report.failed_files, 0);
    assert_eq!(report.bytes_read, native.len() as u64);
    let user = store
        .search_with_role("remembermarker", 1, Some(EventKind::User))
        .unwrap()
        .remove(0);
    assert_ne!(user.source.generation, old_result.source.generation);
    let record =
        agent_history_core::preview::session_record_for_source(&store, &user.source).unwrap();
    let session = &record.session;
    assert_eq!(session.branch.as_deref(), Some("saved-branch"));
    assert_eq!(
        session.repository_root.as_deref(),
        Some(std::path::Path::new("/captured/repository"))
    );
    // Only a live `git` invocation could have filled those fields, so the upgrade must
    // label them as observations rather than leaving their origin unknown.
    assert_eq!(record.origin(), Some(GitOrigin::Observed));
    assert!(!record.is_recorded_only());
    assert_eq!(
        store.session_record(&session.id).unwrap().unwrap().origin(),
        Some(GitOrigin::Observed)
    );
    assert!(preview_source(&store, &old_result.source, 0).is_err());
    assert!(store.search("toolsecret", 10).unwrap().is_empty());
    assert_eq!(
        store
            .search_with_role("answermarker", 10, Some(EventKind::Assistant))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(index_all(&mut store, &adapters).unwrap().bytes_read, 0);
    assert_eq!(fs::read(source).unwrap(), native);
}

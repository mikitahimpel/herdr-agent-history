//! Indexing from one entry point and searching from another must reach the same
//! database. The CLI and the terminal UI each resolved `$HOME` themselves and
//! drifted onto different Application Support directories, so indexing wrote one
//! index while the overlay read another. The location lives in one place now, and
//! this scan keeps it there.

use std::{fs, path::PathBuf};

const MARKER: &str = "Application Support";

fn workspace_sources() -> Vec<(PathBuf, String)> {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate sits inside the workspace")
        .to_path_buf();
    let mut found = Vec::new();
    let mut pending = vec![crates];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("workspace directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).expect("source is readable");
                found.push((path, text));
            }
        }
    }
    assert!(!found.is_empty(), "found no sources to scan");
    found
}

#[test]
fn only_the_shared_constant_names_the_index_location() {
    let offenders = workspace_sources()
        .into_iter()
        .filter(|(path, text)| {
            text.contains(MARKER)
                && !path.ends_with("agent-history-core/src/storage.rs")
                // This scanner names the pattern it looks for.
                && !path.ends_with("agent-history-core/tests/index_path.rs")
        })
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "these name the index location directly instead of using \
         agent_history_core::default_index_path: {}",
        offenders.join(", ")
    );
}

#[test]
fn the_default_index_path_is_absolute_and_under_home() {
    let home = std::env::var_os("HOME").expect("HOME is set in the test environment");
    let path = agent_history_core::default_index_path().expect("HOME is set, so a path resolves");
    assert!(path.is_absolute(), "callers join this against nothing else");
    assert!(path.starts_with(PathBuf::from(home)));
    assert!(path.ends_with("index.sqlite"));
}

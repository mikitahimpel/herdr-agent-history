//! Recovery mutates Git, so every invocation in this crate must go through
//! `agent_history_core::git_command`. A raw `Command::new("git")` would inherit
//! the caller's Git variables, which override `-C`, and could then read or
//! write a repository other than the recorded one.

use std::{fs, path::PathBuf};

fn sources() -> Vec<(PathBuf, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let mut pending = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("crate directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                // This scanner names the pattern it looks for.
                && path.file_name().is_some_and(|name| name != "git_isolation.rs")
            {
                let text = fs::read_to_string(&path).expect("source is readable");
                found.push((path, text));
            }
        }
    }
    assert!(!found.is_empty(), "found no sources to scan");
    found
}

#[test]
fn no_source_invokes_git_outside_the_shared_helper() {
    let offenders = sources()
        .into_iter()
        .filter(|(_, text)| text.contains(r#"Command::new("git")"#))
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    assert!(
        offenders.is_empty(),
        "these build `git` directly instead of using agent_history_core::git_command: {}",
        offenders.join(", ")
    );
}

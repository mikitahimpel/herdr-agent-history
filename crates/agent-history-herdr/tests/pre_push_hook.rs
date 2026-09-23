//! The pre-push hook runs the gate with the pushing repository's Git variables
//! still exported, so it must clear exactly what `git_command` clears. Two
//! hand-maintained lists would otherwise drift apart silently.

use agent_history_core::INHERITED_GIT_ENVIRONMENT;
use std::{fs, path::PathBuf};

fn hook() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.githooks/pre-push")
        .canonicalize()
        .expect("pre-push hook is present");
    fs::read_to_string(path).expect("pre-push hook is readable")
}

/// Names passed to `unset`, following backslash line continuations.
fn unset_names(source: &str) -> Vec<String> {
    let joined = source.replace("\\\n", " ");
    joined
        .lines()
        .filter_map(|line| line.trim().strip_prefix("unset "))
        .flat_map(|names| names.split_whitespace())
        .map(str::to_owned)
        .collect()
}

#[test]
fn hook_clears_every_inherited_git_variable() {
    let cleared = unset_names(&hook());
    for name in INHERITED_GIT_ENVIRONMENT {
        assert!(
            cleared.iter().any(|found| found == name),
            "pre-push hook does not unset {name}"
        );
    }
}

#[test]
fn hook_resolves_the_root_before_clearing_and_still_runs_the_whole_gate() {
    let source = hook();
    let root_at = source.find("--show-toplevel").expect("resolves the root");
    let unset_at = source.find("\nunset").expect("clears the variables");
    let check_at = source.find("\nexec ").expect("runs the gate");
    assert!(
        source[check_at..].contains("scripts/check"),
        "the hook must run the project gate"
    );
    assert!(
        root_at < unset_at,
        "the root must be resolved while the variables still describe this worktree"
    );
    assert!(
        unset_at < check_at,
        "the variables must be cleared before the gate runs"
    );
    assert!(
        !source.contains("--no-verify") && !source.contains("SKIP"),
        "the hook must not weaken or skip the gate"
    );
}

//! Explicit, non-destructive recovery of a missing workspace.
use agent_history_core::{git_command, CoreError, Result, Session};
use std::path::{Component, Path, PathBuf};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryChoice {
    RecreateWorktree,
    ExistingRepository,
    ViewConversation,
    Cancel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryOptions {
    pub choices: Vec<RecoveryChoice>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRecreationPlan {
    pub repository_root: PathBuf,
    pub worktree: PathBuf,
    pub argv: Vec<String>,
}
pub fn recovery_options(
    session: &Session,
    repository_exists: bool,
    worktree_exists: bool,
) -> RecoveryOptions {
    let mut choices = Vec::new();
    if repository_exists && !worktree_exists && plan_recreate(session).is_ok() {
        choices.push(RecoveryChoice::RecreateWorktree)
    }
    if repository_exists {
        choices.push(RecoveryChoice::ExistingRepository)
    }
    choices.extend([RecoveryChoice::ViewConversation, RecoveryChoice::Cancel]);
    RecoveryOptions { choices }
}
fn fail(message: &str) -> CoreError {
    CoreError::Unsupported(message.into())
}
fn clean_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && !path.components().any(|c| c.as_os_str() == ".git")
}
pub fn plan_recreate(session: &Session) -> Result<GitRecreationPlan> {
    let root = session
        .repository_root
        .clone()
        .ok_or_else(|| fail("repository root is unavailable"))?;
    let target = session
        .worktree
        .clone()
        .ok_or_else(|| fail("recorded worktree path is unavailable"))?;
    let commit = session
        .commit
        .as_deref()
        .filter(|v| matches!(v.len(), 40 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| {
            fail("a recorded full commit hash is required for safe detached recreation")
        })?;
    if !clean_absolute(&root) || !clean_absolute(&target) || root == target {
        return Err(fail("invalid recorded repository or worktree path"));
    }
    if session
        .cwd
        .as_deref()
        .is_none_or(|cwd| !clean_absolute(cwd) || !cwd.starts_with(&target))
    {
        return Err(fail("session cwd is outside the recorded worktree"));
    }
    Ok(GitRecreationPlan {
        argv: vec![
            "git".into(),
            "-C".into(),
            root.to_string_lossy().into(),
            "worktree".into(),
            "add".into(),
            "--detach".into(),
            "--".into(),
            target.to_string_lossy().into(),
            commit.into(),
        ],
        repository_root: root,
        worktree: target,
    })
}
/// Only construct this token after an explicit affirmative runtime response.
#[derive(Clone, Copy, Debug)]
pub struct Confirmation;
impl Confirmation {
    pub fn confirmed() -> Self {
        Self
    }
}
pub fn execute_recreate(plan: &GitRecreationPlan, _confirmation: Confirmation) -> Result<()> {
    // Revalidate immediately before mutation, including symlink/path collisions.
    if !clean_absolute(&plan.repository_root)
        || !clean_absolute(&plan.worktree)
        || plan.repository_root == plan.worktree
    {
        return Err(fail("invalid recreation paths"));
    }
    if std::fs::symlink_metadata(&plan.worktree).is_ok() {
        return Err(fail(
            "worktree target already exists; nothing was overwritten",
        ));
    }
    let root = plan.repository_root.canonicalize()?;
    let parent = plan
        .worktree
        .parent()
        .ok_or_else(|| fail("worktree parent is unavailable"))?;
    if root != plan.repository_root || parent.canonicalize()? != parent {
        return Err(fail("recreation paths must not traverse symlinks"));
    }
    let commit = plan
        .argv
        .last()
        .filter(|v| matches!(v.len(), 40 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| fail("invalid commit hash"))?;
    let checked = git_command(&root)
        .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .output()?;
    if !checked.status.success() {
        return Err(fail("recorded commit is unavailable in the repository"));
    }
    let listed = git_command(&root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !listed.status.success() {
        return Err(fail("unable to verify existing worktree registrations"));
    }
    use std::os::unix::ffi::OsStrExt;
    let mut matches = false;
    let mut registered = false;
    for field in listed.stdout.split(|b| *b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            matches = path == plan.worktree.as_os_str().as_bytes();
            registered |= matches;
        } else if matches && (field == b"locked" || field.starts_with(b"locked ")) {
            return Err(fail(
                "recorded worktree registration is locked; no recreation attempted",
            ));
        }
    }
    if std::fs::symlink_metadata(&plan.worktree).is_ok() {
        return Err(fail(
            "worktree target appeared during validation; no recreation attempted",
        ));
    }
    // --force is used solely to replace the exact missing, unlocked registration.
    // It is never a branch force, and no existing path is accepted.
    let mut command = git_command(&root);
    command.args(["worktree", "add", "--detach"]);
    if registered {
        command.arg("--force");
    }
    let output = command.arg("--").arg(&plan.worktree).arg(commit).output()?;
    if !output.status.success() {
        return Err(fail("Git could not recreate the worktree. No prune, reset, checkout overwrite, or filesystem removal was attempted."));
    }
    Ok(())
}
pub fn path_exists(path: &Path) -> bool {
    path.is_dir()
}

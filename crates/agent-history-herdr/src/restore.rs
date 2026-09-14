use agent_history_core::{CoreError, Result, Session};
use std::path::{Path, PathBuf};

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
    if repository_exists && session.worktree.is_some() && !worktree_exists {
        choices.push(RecoveryChoice::RecreateWorktree);
    }
    if repository_exists {
        choices.push(RecoveryChoice::ExistingRepository);
    }
    choices.extend([RecoveryChoice::ViewConversation, RecoveryChoice::Cancel]);
    RecoveryOptions { choices }
}

pub fn plan_recreate(session: &Session) -> Result<GitRecreationPlan> {
    let root = session
        .repository_root
        .clone()
        .ok_or_else(|| CoreError::Unsupported("repository root is unavailable".into()))?;
    let path = session
        .worktree
        .clone()
        .ok_or_else(|| CoreError::Unsupported("worktree path is unavailable".into()))?;
    let revision = session
        .branch
        .clone()
        .or_else(|| session.commit.clone())
        .ok_or_else(|| CoreError::Unsupported("branch or commit is unavailable".into()))?;
    Ok(GitRecreationPlan {
        argv: vec![
            "git".into(),
            "-C".into(),
            root.display().to_string(),
            "worktree".into(),
            "add".into(),
            path.display().to_string(),
            revision,
        ],
        repository_root: root,
        worktree: path,
    })
}

/// Runtime confirmation is deliberately explicit at the call site.
#[derive(Clone, Copy, Debug)]
pub struct Confirmation;
impl Confirmation {
    pub fn confirmed() -> Self {
        Self
    }
}

pub fn execute_recreate(_plan: &GitRecreationPlan, _confirmation: Confirmation) -> Result<()> {
    Err(CoreError::Unsupported(
        "Git execution is host-runtime work and must be wired to an explicit confirmation flow"
            .into(),
    ))
}

pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

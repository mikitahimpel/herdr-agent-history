# Required agent workflow

All agents and contributors must follow these rules.

Read README.md, docs/RFC.md, and the relevant issue in docs/BACKLOG.md before editing. Scope implementation to an issue and its acceptance criteria.

## Mandatory validation

Run `./scripts/setup` once per checkout to enable the pre-push hook.

After code, dependency, build, or tooling changes, run `./scripts/check` before committing or handing off. Formatting, Clippy (warnings denied), all workspace tests, and the release build must pass with the committed lockfile. CI and the pre-push hook run this same command.

- Never skip tests, bypass hooks, disable checks, weaken assertions, or add blanket lint suppressions to hide failures.
- Fix failures introduced by your changes. If the environment blocks validation, report the exact failed check and reason; do not claim success or completed implementation.
- Add meaningful behavior/regression tests for new functionality, especially parsers, transactional offsets, file mutation, and restoration decisions.
- Use synthetic/sanitized fixtures and isolated temporary directories. Automated tests must not read private home histories, launch real agents, require running Herdr, or mutate user worktrees.
- Use `cargo fmt --all` to fix formatting, then rerun the complete gate.
- Handoffs must state the issue, changed behavior, validation performed, and material limitations.

## Architecture

Keep SEARCH / SESSION / RESTORE separate. Core must not depend on Herdr. Native transcripts are canonical and read-only. SQLite is disposable. No embeddings, cloud services, LLM processing, telemetry, or permanent daemon in V1. Never execute transcript content or interpolate it into shell commands. Worktree recreation requires explicit runtime confirmation.

Never commit transcripts, databases, credentials, or private benchmark histories.

## Enforcement

Instructions govern agent behavior. Local hooks are convenience checks. Required GitHub status checks on protected main enforce merges remotely; do not claim branch protection is enabled without verifying it.

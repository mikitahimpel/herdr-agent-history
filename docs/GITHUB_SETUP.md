# GitHub publication and enforcement

Remote: https://github.com/mikitahimpel/herdr-agent-history

The repository is public, the backlog issues are published, and CI runs on GitHub. The steps below are the original one-time publication procedure, kept for reference; they have already been carried out.

## Current state (verified 2026-09-29)

Read back with `gh api repos/mikitahimpel/herdr-agent-history/branches/main/protection` and `gh run list`:

| Setting | `.github/branch-protection.json` | Live on `main` |
| --- | --- | --- |
| Required check `Quality gate`, branch up to date | yes | yes |
| Force pushes and deletion blocked | yes | yes |
| Pull request required | yes, zero approvals | **no** |
| Conversations resolved | yes | **no** |
| Applies to administrators | yes | **no** |

`Quality gate` (`.github/workflows/ci.yml`, `./scripts/check` on `macos-latest`) runs on every push and pull request and was green on `main` at `4a25bcc`. The live rule is therefore weaker than the prepared policy: an administrator can push to `main` directly, and nothing requires a pull request. Applying the policy below would close that gap; it has not been applied.

## One-time publication

Run these steps from this checkout in a terminal with working GitHub access:

```sh
gh auth status
./scripts/check
git push -u origin main
python3 scripts/publish-issues.py --apply
```

If the remote already has commits, fetch and integrate them before pushing; do not force-push. If a command fails, resolve that error before continuing.

The publisher previews with no network activity when run without `--apply`. With `--apply`, it creates one tracker and 23 work issues, replaces dependency IDs with GitHub issue references, and reuses stable markers on retry. Existing implementation issue bodies are not overwritten. The generated tracker block is refreshed; text outside it is preserved. Publication failures are errors, not success.

## Enforce checks remotely

After the first CI run succeeds, enable the prepared protection policy:

```sh
gh api --method PUT repos/mikitahimpel/herdr-agent-history/branches/main/protection \
  --input .github/branch-protection.json
gh api repos/mikitahimpel/herdr-agent-history/branches/main/protection
```

This requires pull requests, a successful `Quality gate`, an up-to-date branch, resolved review conversations, and applies to administrators. Force pushes and deletion are blocked. Zero required human approvals keeps a solo-maintainer workflow possible; CI remains mandatory. The PUT replaces existing branch protection, so inspect and merge existing settings first if protection has since been configured.

Do not mark the quality-enforcement issue complete until the active remote rule and a green CI result are verified. Local hooks and AGENTS.md cannot prevent a user with sufficient permissions from bypassing or changing policy.

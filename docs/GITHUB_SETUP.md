# GitHub publication and enforcement

Remote: https://github.com/mikitahimpel/herdr-agent-history

Local setup is complete. GitHub publication was blocked by the session's approval policy. The repository was created by the owner and is public. No file writes, issue creation, or branch-protection changes were confirmed remotely.

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

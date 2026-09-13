#!/usr/bin/env python3
"""Preview by default; --apply publishes the checked-in backlog via authenticated gh."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
REPO = "mikitahimpel/herdr-agent-history"


def api(endpoint, method="GET", payload=None):
    args = ["gh", "api", endpoint, "--method", method]
    if payload is not None:
        args += ["--input", "-"]
    result = subprocess.run(args, input=json.dumps(payload) if payload is not None else None,
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    issues = json.loads((ROOT / "docs/issues.json").read_text())
    seen = set()
    for issue in issues:
        if issue["id"] in seen or any(dep not in seen for dep in issue["deps"]):
            raise SystemExit("Backlog has duplicate IDs or out-of-order dependencies")
        seen.add(issue["id"])
    if not args.apply:
        print(f"Would publish 1 tracker + {len(issues)} issues to {REPO}:")
        for issue in issues:
            print(f"  {issue['id']}: {issue['title']}")
        print("No network calls or writes. Pass --apply to publish.")
        return

    # Inspect all pages first so a retry never knowingly creates duplicate markers.
    existing = []
    page = 1
    while True:
        batch = api(f"repos/{REPO}/issues?state=all&per_page=100&page={page}")
        existing.extend(i for i in batch if "pull_request" not in i)
        if len(batch) < 100:
            break
        page += 1

    def find(marker):
        matches = [i for i in existing if marker in (i.get("body") or "")]
        if len(matches) > 1:
            raise SystemExit(f"Duplicate marker {marker}; resolve manually before retrying")
        return matches[0] if matches else None

    tracker_marker = "<!-- agent-history-issue:v1-tracker -->"
    tracker = find(tracker_marker)
    if tracker is None:
        tracker = api(f"repos/{REPO}/issues", "POST", {
            "title": "V1: Search, preview, and resume native agent sessions in Herdr",
            "body": "Delivery checklist will be populated by the backlog publisher.\n\n" + tracker_marker})
    published = {}
    for issue in issues:
        marker = f"<!-- agent-history-issue:{issue['id']} -->"
        remote = find(marker)
        if remote is None:
            body = issue["body"].replace("../../blob/main/docs/RFC.md",
                f"https://github.com/{REPO}/blob/main/docs/RFC.md")
            for dep in issue["deps"]:
                body = body.replace("- " + dep + "\n", f"- #{published[dep]['number']}\n")
            body += f"\nParent tracker: #{tracker['number']}\n"
            remote = api(f"repos/{REPO}/issues", "POST", {"title": issue["title"], "body": body})
        published[issue["id"]] = remote
        print(remote["html_url"], flush=True)

    start, end = "<!-- generated-backlog:start -->", "<!-- generated-backlog:end -->"
    generated = start + "\n## V1 delivery checklist\n\n"
    generated += "Find remembered terms, preview original context, and resume the same native session in Herdr. Existing worktrees require no extra decisions.\n\n"
    generated += "\n".join(f"- [ ] #{published[i['id']]['number']} — {i['title']}" for i in issues)
    generated += "\n\nNo daemon, vectors, cloud, or LLM processing in V1.\n" + end
    current = api(f"repos/{REPO}/issues/{tracker['number']}")["body"] or ""
    if start in current and end in current:
        before, rest = current.split(start, 1)
        _, after = rest.split(end, 1)
        updated = before + generated + after
    else:
        updated = current + "\n\n" + generated
    api(f"repos/{REPO}/issues/{tracker['number']}", "PATCH", {"body": updated})
    print("Tracker:", tracker["html_url"])


if __name__ == "__main__":
    main()

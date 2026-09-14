# V1 release status — local 0.1.0 candidate

V1 is **not released** and GitHub issue #13 remains open. The repository now contains a working local CLI, terminal overlay, verified-source preview, host resume adapter, confirmed Git worktree recreation, installable package scripts, and synthetic measurements. Implementation and local tests do not establish native-agent resume compatibility or clean-user installation acceptance.

## Architecture

`agent-history-core` owns native adapters, bounded conversation chunks, disposable SQLite/FTS5, transactional indexing, captured Git context, and source-verified previews. `agent-history-cli` provides index/search/status/preview. `agent-history-herdr` provides the terminal plugin, native-session command construction, public Herdr CLI operations, and recovery confirmation. Native files remain read-only; no daemon or transcript network service is introduced.

## Issue and PR map

| Issue | Local implementation and evidence | Remaining acceptance |
| --- | --- | --- |
| #7 Foundation | Shared types/contracts, isolated fixtures, CI definition, local quality gate | Remote CI execution not verified |
| #1 Ingestion | Claude/Codex adapters, synthetic fixtures, bounded resumable chunks | Real native-format compatibility evidence |
| #2 SQLite/FTS | Schema 2, external-content FTS, search/ranking, atomic source-scoped updates | Broader real-corpus measurements |
| #3 Incremental indexing | Committed offsets, partial-record retry, replacement handling, concurrent-writer checks | Documented interior rewrite/regrow limitation |
| #4 Git context | Main/linked/bare/separate Git directory tests; preserved observations | Index-time metadata is not proof of historical branch state |
| #5 CLI | Cross-process index/search/status/preview/append tests | Real-history user validation |
| #8 Core integration | Both agents through discovery → index → search → original preview; synthetic benchmark | Representative private-history validation without committing data |
| #9 Native resume | UUID-only argv, source validation, official session provenance, no plain-session fallback | Actual Claude/Codex resume after process exit on macOS |
| #6 Herdr | Functional terminal overlay, mocked host effects, synthetic PTY smoke, confirmed isolated Git recovery | Real active/closed Herdr workspace and native-agent acceptance |
| #11 Safeguards | Private index, safe open/sidecar rejection, rollback/corruption errors, confirmed recovery | Broader security review; see documented source-mutation limits |
| #10 Packaging | Apple Silicon archive/checksum, extracted installer, durable plugin path, isolated smoke | Clean-user Mac installation and GitHub release artifacts |
| #12 Documentation | README, indexing/privacy/recovery notes, plugin guide, benchmark and troubleshooting | Real UI screenshots/native acceptance evidence |
| #13 Release gate | Local quality gate and synthetic integration evidence | Blocked on the acceptance items above; no release tag |

All GitHub issues were inspected as the source backlog. No issue was closed, PR published, branch protection claimed, or release/tag created by this implementation run. Work is on local branch `codex/v1-integration`.

## Validation evidence

- `./scripts/setup` enabled the pre-push hook.
- `./scripts/check` passed formatting, Clippy with warnings denied, all 64 tests (45 core, 16 host/controller, 3 CLI), and the release build using the lockfile.
- Core regressions cover source identity, generation, partial Unicode records, rollback and competing writers, both adapters, normalized previews, and selected-source session context.
- Host tests cover exact live-session matching, safe command construction, UI effects, explicit recovery cancellation/confirmation, locked and existing worktree targets, and preserving the original checkout.
- The release overlay was exercised in a synthetic PTY: activation, a multiword query, result focus, original conversation preview, back navigation, and terminal cleanup. No native agent was launched.
- Packaging smoke installs/uninstalls into an isolated prefix and checks that unrelated output files and synthetic native history remain intact.

## Measurements

The reproducible synthetic corpus contains 200 files, 40,100 initial records, 20,000 chunks, and about 16.63 MiB of JSONL. The recorded Apple Silicon release run measured initial indexing at 839.899 ms; a uniquely identifiable 449-byte append at 59.716 ms; warm search p50/p95 at 16.145/17.241 ms; and maximum RSS at 12,353,536 bytes. SQLite plus WAL/SHM occupied 22,500,416 bytes, 1.290 times raw input. This is synthetic evidence, not representative private-history acceptance. See [PERFORMANCE.md](PERFORMANCE.md) for reproduction and scope.

## Exact external blocker and next step

This Codex task is outside a Herdr-managed pane: checking `HERDR_ENV=1` failed. The configured Herdr control skill prohibits inspecting or controlling the focused Herdr session from outside Herdr. Consequently, real native-agent launches, active/closed workspace focus and resume, and plugin operation in a live host were not attempted here.

Continue acceptance from a Herdr-managed task with installed Claude Code/Codex integrations and isolated native test sessions. Exercise both agents after their original processes exit, an open workspace, a closed workspace with an existing checkout, confirmed deleted-worktree recovery, and an unavailable repository. Also install the candidate under a clean supported macOS user account. Record results before publishing or tagging V1; mocked tests are not substitutes.

## Material limitations

- Only sampled boundaries verify prior content during append; arbitrary interior rewrite followed by regrowth can evade detection. Same-size changes and ordinary replacements/truncations are covered.
- Renamed sources retain unavailable old-path search rows alongside the new path.
- Initial activation indexing is synchronous and has no cancellation or per-file progress callback. Derived chunks are retained per file until commit, so memory grows with that file's extracted text.
- The overlay is a terminal plugin, not an in-process native Herdr widget. Herdr 0.7.1 is the target; later CLI changes require compatibility work.
- Safe worktree recreation uses the captured commit in detached HEAD state; it does not recreate uncommitted changes or reconstruct unavailable commits.
- The package is a local candidate, not a published, clean-install-certified release.

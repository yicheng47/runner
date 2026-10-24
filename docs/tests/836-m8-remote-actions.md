# 836 m8 — remote actions validation

Scope: host companion actions and feed, carried by the client core on `feat/836-m8-remote-actions`, based on `origin/nightly-remote` at `864bc82b`. No UI changed and no live checks apply; no app or non-test daemon is launched. All new behavior tests use the in-process relay, in-memory stores, temporary files and a fake session runtime. No network or sleeps.

## Selected checks and reasons

- Companion goldens: decode and re-encode every new request, ignore unknown fields and default omitted optional fields; verify `welcome.features` names every action. This settles the independent camelCase wire contract and preserves older message fixtures.
- Relay actions with a real `AppCore`: verify session start sizes, lifecycle, layouts, names, pins, moves, project operations and their durable rows/events/overview; reject Settings and invalid operations without mutating state. This proves the adapter uses the desktop operations rather than duplicating state writes.
- Relay catalog/folders: verify names-only projections, host paths, subdirectories only, file/missing-path refusal and start-path validation. Canary values cover configuration, account folders and file names.
- Relay missions/feed: verify start/stop/resume size, shared mission sidebar actions, newest-first cursor paging, follow/unfollow append order, answers and posts, and `remote/feed` delivery. These cover previously unserved v1 messages and the new client forwarding path.
- Client safety: requests are correlated with channel-owned IDs, results do not patch client state, Settings stays unknown, revoked channels stay refused, and `runner call` cannot reach `remote_request` or companion action names. Debug output and errors keep message bodies and terminal bytes private.
- Required gates: full workspace nextest with `CLAUDE_CONFIG_DIR` unset, workspace Clippy with and without updater, formatting and diff whitespace checks. These cover shared protocol/API generation and both daemon and app consumers. CLI help is unchanged, so guide generation is not selected.

## Implementation validation before review

Gate logs and separate `.exit` files live under this worktree’s `target/`. Each shell records its own command exit before printing a log excerpt.

| Check | Result | Exit | Evidence |
| --- | --- | --- | --- |
| Initial workspace nextest | Passed: 2,774 tests, 6 skipped, 1 slow | 0 | `target/m8-workspace-tests.log` |
| Focused protocol, relay and CLI admission suite before capability metadata | Passed: 53 tests | 0 | `target/m8-focused-final.log` |
| Workspace nextest, including capability metadata and field diagnostics | Passed: 2,778 tests, 6 skipped, 1 slow | 0 | `target/m8-workspace-tests-final2.log` |
| Relay action suite after the mission-event payload correction and startup synchronization update | Passed: 8 tests | 0 | `target/m8-actions-final2.log` |
| Final workspace Clippy | Passed | 0 | `target/m8-clippy-final4.log` |
| Final workspace Clippy with updater | Passed | 0 | `target/m8-clippy-updater-final4.log` |
| Formatting | Passed | 0 | `target/m8-fmt-check.log` |
| Diff whitespace | Passed | 0 | `target/m8-diff-check.log` |
| CLI guide generation | Not selected: CLI help unchanged | — | No command/help additions |
| Live regression cases | Not selected: m8 changes no UI and the brief prohibits live checks | — | Reviewed the regression-suite index; app/real-account cases remain m9’s scope |

The relay action suite was rechecked after the workspace build to verify the final mission rename/pin/archive event payloads exactly match the desktop and mission restart waits for handle installation. The final suite also covers a feed keeping its route alive without an overview follower, terminal traffic refusing the generic request entry point, role-folder fallback with explicit root scope, force versus running-member project deletion, capability discovery in `{result, features}`, and fixed action diagnostics without supplied values.

## Decisions and review

Jason’s mission-feed decision `01M5MJC1RF2VMYMQSRJHY3PX0S` settles the actual desktop start operations, host active-account inheritance and read-only catalog selection, and a shared sized mission resume with the existing desktop request delegating unchanged. The spec’s Remote actions section records the target IDs, pane tree, directory response, root project semantics, feed cursor/subscription behavior, lifecycle restrictions and the new client response contract.

The whole working-tree review is clean: reviewer message `01M5MPJVCCD1Z9VJ16B0G915KS` reports **NO REMAINING MUST-FIX ISSUES** after all five findings across two review rounds were resolved. The final verdict and candidate identity are recorded below.

## Development failures and recovery

Early focused runs returned 101 for test-fixture compilation and 100 for fixture assumptions: move orders included pinned nodes, tab deletion was attempted while populated, a direct-chat restart was treated as a mission-slot restart, and asynchronous mission startup was inspected before the fake runtime’s spawn notification. These were corrected to the desktop operations’ actual contracts; the subsequent 49-test and 53-test focused runs passed. The invalid Codex fork fixture was rejected by UUID validation before any materializer could launch; successful fork coverage uses the direct Claude fork path with an in-memory fake runtime and a temporary persisted session.

The first final-shape workspace/Clippy attempts returned 101 because an existing stalled-pair test still destructured the old three-field internal helper result after negotiated capabilities were retained. Its tuple was updated; those failures remain in `target/m8-workspace-tests-final.log` and `target/m8-clippy-final.log`. The final rerun results above supersede them without erasing this history. No known redraw-flake retry has been needed.

## Review round 1 — selected corrections

Reviewer message `01M5MMAN4410VQH5MHW379QS6Y` found four must-fix issues: mission members accidentally persisted as drawer sessions, hidden feed noise, unbounded feed messages/backlog, and lost drawer presentation. The correction checks are a mission layout containing a real member followed by archive; non-default tab/mission drawers followed by remote layout updates; mixed visible/noise pages and a noise-only suffix; and relay bursts above 1 MiB with bounded ordered delivery, an oversized-entry marker and a still-usable attached terminal. A direct adapter probe covers exhausted output-budget accounting before draining the same bounded backlog. These checks exercise the affected ownership, projection, pagination and backpressure contracts without live UI or services.

Review correction validation passed. The focused correction suite passed 62 tests (exit 0). The first workspace correction run exposed an extra notification when no feed was followed: `an_event_append_invalidates_pending_asks_in_the_overview` could consume that notification before its overview timer. `feed_changes` now leaves the existing no-follow wakeup path untouched. The failing attempt remains in `target/m8-review-fixes-workspace.log`; corrected checks and the final workspace rerun are recorded below. Re-review was requested on the whole corrected working-tree diff; no git write or PR had been made at that handoff.

Two workspace runs overlapped while the first finished its CLI goldens. The earlier run also reported isolated test-daemon startup timeouts in `project_missing` and `project_list`, finishing with 2,778 passed and 3 failed (exit 100); the corrected run hit the unchanged account fixture's create-before-write race in `add_and_login_keeps_each_request_bound_to_its_created_account` (the file existed but was still empty), finishing with 2,780 passed and 1 failed (exit 100). Both logs are retained. The workspace run without overlap passed all 2,781 tests, including those three fixtures, with 6 skipped and 1 slow (exit 0). No unrelated test or timeout implementation is changed.

| Correction check | Result | Exit | Evidence |
| --- | --- | --- | --- |
| Focused corrected suite | Passed: 62 tests | 0 | `target/m8-review-fixes-focused-verified.log` |
| Workspace nextest after corrections, without overlapping test runs | Passed: 2,781 tests, 6 skipped, 1 slow | 0 | `target/m8-review-fixes-workspace-solo.log` |
| Workspace Clippy | Passed | 0 | `target/m8-review-fixes-clippy-verified.log` |
| Workspace Clippy with updater | Passed | 0 | `target/m8-review-fixes-clippy-updater-verified.log` |
| Formatting | Passed | 0 | `target/m8-review-fixes-fmt-check.log` |
| Diff whitespace | Passed | 0 | `target/m8-review-fixes-diff-check.log` |

The layout correction preserves drawer height/open/selection and future drawer fields, including selection when a legacy row omits `active`; membership accepts shell sessions only. Mission layout calls the existing drawer-only operation and ignores requested member panes, so member IDs cannot prevent mission archive. The feed correction filters the desktop’s hidden signals before visible limits, advances raw cursors over filtered tails, and keeps both pages and append batches within 64 KiB. Follow subscriptions hold cursors instead of an accumulated backlog, respect output budget/credit and take turns with terminals and other missions. A too-large individual entry becomes the fixed `feedEntryTooLarge` signal at the same ID/time; the new golden fixes this additive signal form. The relay regression sends 512 ordinary 3 KiB messages, a message above 1 MiB and a later message, verifies every visible ID in append/page order, then verifies viewport, input and actual terminal output still work without reopening the route.

## Review round 2 — subscription wakeups

Reviewer message `01M5MP143RK7AHD84S1Y40N3X0` confirms the four original findings are addressed and identifies one remaining scheduler wakeup issue: an empty or failed feed can consume the wakeup while other subscriptions remain dirty. The selected regression follows 16 missions, deletes the first, appends only hidden noise to the second and one visible entry to the last, then emits a single `event/appended` notification. The last entry must arrive over the in-process relay without terminal output, another request or another event. Existing output-budget and terminal-sharing regressions cover the requirement to wait for actual credit when blocked.

The regression failed on the reviewed scheduler with a missed-delivery timeout (exit 100, `target/m8-review2-regression-before.log`) and passed after the correction (exit 0, `target/m8-review2-regression-after.log`). After consuming or removing a dirty subscription, the scheduler now re-arms its wakeup whenever any dirty subscription remains, including empty and error results. The credit and output-budget guards still return before consuming work or generating a wakeup, so blocked output waits for actual credit. The corrected focused suite and required gates passed; the whole uncommitted diff is submitted for re-review.

| Scheduler correction check | Result | Exit | Evidence |
| --- | --- | --- | --- |
| Focused companion, relay, CLI admission and mission-resume suite | Passed: 74 tests | 0 | `target/m8-review2-focused.log` |
| Workspace nextest | Passed: 2,782 tests, 6 skipped, 1 slow | 0 | `target/m8-review2-workspace.log` |
| Workspace Clippy | Passed | 0 | `target/m8-review2-clippy.log` |
| Workspace Clippy with updater | Passed | 0 | `target/m8-review2-clippy-updater.log` |
| Formatting | Passed | 0 | `target/m8-review2-fmt-check.log` |
| Diff whitespace | Passed | 0 | `target/m8-review2-diff-check.log` |

## Clean working-tree review

Reviewer message `01M5MPJVCCD1Z9VJ16B0G915KS` reports **NO REMAINING MUST-FIX ISSUES** on the whole working-tree diff. Reviewer round 3 passed the multi-subscription, large-feed/shared-terminal and no-follow overview regressions (3/3, exit 0, `target/m8-reviewer-round3.log`) and `git diff --check`. Earlier reviewer focused runs passed 59 and 62 tests. The reviewer also inspected all final coder gates above. All four original findings and the later scheduler wakeup issue are resolved; their failing attempts remain recorded.

The reviewer’s `target/m8-reviewer-round3-manifest.json` records SHA-256 hashes for all 56 changed/new files. Before recording this verdict, the coder verified all 56 still matched. The code and goldens remain that reviewed, fully tested candidate. Live checks are not needed for m8 and are explicitly prohibited by the brief; no app or non-test daemon, real account or remote service was operated.

The final history folds the brief/spec/plan commit into one implementation commit on `origin/nightly-remote` at `864bc82b`, confirmed current by a fetch after clean review. No separate untested intermediate implementation commits are retained. `cargo check --locked --workspace --profile ci` passed (exit 0, `target/m8-final-history-build.log`); the implementation and goldens match the reviewed candidate whose full workspace tests passed above. History/content checks are recorded under `target/m8-final-history-*` before pushing.

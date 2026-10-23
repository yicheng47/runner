# #836 remote live fixes

Date: 2026-10-23. Branch: `fix/836-remote-live-fixes`, based on `nightly-remote` at `b5869c0d`. Scope and authorization: [mission brief](../impls/briefs/836-remote-live-fixes.md). Live checks belong to Jason; neither crew slot launches or operates either app, contacts the remote service or reads credentials.

## Problem 1: transparent computer picker

Jason’s screenshot shows the computer picker with a directory-refresh warning and its footer hint drawn directly over sidebar and terminal content. Only the selected computer row has a background.

Cause: `render_computer_picker` supplies an unstyled content column to `popup_layer`. That helper positions and occludes a popup but intentionally leaves its appearance to the caller. The picker omitted the enclosing panel fill, border and corners that other menus provide.

Fix: give the complete picker a solid themed panel, border and rounded corners, enclosing computer rows, warnings and the hint. Keep the existing picker placement and actions. This fixes presentation of a failed directory refresh; it does not claim to resolve the connection failure that prompted the warning.

### Selected validation

- Fake-driven GPUI regression `picker_encloses_rows_errors_and_hint_in_an_opaque_panel`: successful and failed directory calls, dark/light themes, English/Simplified Chinese, Glass/Solid and 100%/150% app zoom; assert the active theme, effective material (Glass only on macOS), panel fill, row/warning/hint containment, width, placement, Escape dismissal, trigger-toggle dismissal and outside-click dismissal. The test installs the appearance global with Reduce transparency off before adding its window. These cover the missing popup surface and its existing interaction boundary without launching an app or using the network.
- Required workspace nextest, both Clippy configurations, formatting and whitespace gates from the brief. CLI help is unchanged, so `make guide-cli-check` is not applicable.
- Live follow-up for Jason, scoped to [WS-MENU-01](regression/workspace.md): open the computer picker in Glass and Solid, check its full background around rows/warnings/hint in dark and light, including the directory-error state shown; dismiss with Escape, an outside click and the same trigger. The crew does not run live checks.

### Results

| Check | Exit code | Evidence |
| --- | --- | --- |
| Targeted regression before the fix | 100 (expected failure) | `target/836-picker-before.log`: panel fill is `None` instead of the opaque theme panel |
| Targeted regression after the fix | 0 | `target/836-picker-after.log`: 1 passed |
| Targeted regression after review corrected the appearance precondition | 0 | `target/836-picker-review-fix.log`: 1 passed, with explicit active-theme and effective-material assertions |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 100 | `target/836-nextest.log`: 2,736 passed, 1 failed, 6 skipped; failure and retry below |
| Isolated retry of OpenCode fixture | 0 | `target/836-opencode-retry.log`: `model_query_waits_for_settlement_and_reaps_its_private_server` passed |
| Workspace Clippy, warnings denied | 0 | `target/836-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-fmt-check.log` |
| `git diff --check` | 0 | `target/836-diff-check.log` |
| Formatting and whitespace after the review correction | 0 each | `target/836-fmt-check-review-fix.log`, `target/836-diff-check-review-fix.log` |

The workspace run hit an unrelated failure in `runner-daemon`’s `model_query_waits_for_settlement_and_reaps_its_private_server`: parsing the fixture’s empty counter file at `runtimes/opencode/tests.rs:233` raised `ParseIntError { kind: Empty }`. Its isolated retry passed without code changes. The original workspace run is recorded as failed, not a clean full-suite pass. No OpenCode code is changed by this fix. Logs are kept under this worktree’s `target/836-*.log`.

The first review found no implementation defect but identified that the regression’s missing `Appearance` global forced every case to Solid. The correction adds that global and asserts the effective material and active theme, so Glass/Solid coverage is established by the corrected focused run; the earlier runs did not prove Glass coverage.

### Review verdict

On 2026-10-23 the reviewer reported **NO REMAINING MUST-FIX ISSUES** for Problem 1 in Runner message `01M5M09FPTVZ5RYYHA04DX41H1`. Its independent corrected regression and whitespace check both exited 0 (`target/836-reviewer-picker-corrected.log`, `target/836-reviewer-diff-check-corrected.log`). Live checks were skipped under the mission brief and remain Jason’s selected follow-up above. This verdict covers only the transparent-picker fix; at that point the signed-out picker design choice was pending.

## Problem 2: signed out appears as a refresh failure

Jason’s follow-up screenshot shows the opaque picker displaying a refresh warning while signed out. He requested a normal default dropdown, reviewed three Pencil variants and chose **A** on 2026-10-23. Design: `runner/desktop/specs/836-web-remote-control.pen` in `yeelinks_design`, frame `l2uAX`, **836 · 2.0 Signed-out computer picker · A approved**.

Cause: the picker did not distinguish signed-out presentation from a failed account directory refresh, and the shared directory worker requested `remote_computers` even without an activated account.

Fix: signed out shows a single-line **This computer** row, its local attention badge and a compact **Sign in to see your computers…** row. The action opens Account by mouse or keyboard without beginning enrollment. Cached account computers, access/directory errors and the remote hint are hidden in this state. Directory refreshes, including after local daemon reconnection, check the daemon’s current public account status first and skip listing while signed out. Existing signed-in states and genuine listing failures retain their behavior. No protocol or service contract changes.

### Selected validation

- Fake-driven `signed_out_picker_keeps_local_attention_and_opens_account_without_listing_or_enrolling`: English and Simplified Chinese; unknown status, Off and Stopped with retained account/host metadata; local row, attention badge and no duplicate subtitle; no stale rows/errors/hint; mouse, Tab/Enter and Tab/Space open Account without enrollment; no directory calls on picker opens or daemon reconnection.
- Fake-driven `account_sign_in_resumes_directory_refresh_and_signed_in_failures_remain_errors`: sign-out and sign-in transitions through Paused, Connecting, Connected and Reconnecting; listing resumes on sign-in and genuine directory failures remain errors.
- Fake-driven `picker_dismisses_at_tab_boundaries_and_escape_works_after_internal_traversal`: signed-in and signed-out pickers; forward/reverse boundary traversal dismisses and returns focus to the workspace root; Escape works after internal traversal, and subsequent Enter/Space do not activate underlying controls.
- Re-run the opaque-panel regression to retain signed-in error rendering, geometry and material coverage; run the brief’s required workspace gates. No CLI help changes.
- Live checks remain Jason’s: while signed out, open the picker and see only the local row and sign-in action; retain the local attention badge; activate sign-in by mouse or keyboard and reach Account; after signing in, see account computers and retain a real directory-error warning when a refresh fails. Neither crew slot operates the live apps.

### Results

| Check | Exit code | Evidence |
| --- | --- | --- |
| Signed-out regression before the fix | 100 (expected failure) | `target/836-signed-out-before.log`: an additional `remote_computers` call occurred |
| Final focused regressions | 0 | `target/836-signed-out-final-focused.log`: 3 passed |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `target/836-signed-out-nextest.log`: 2,739 passed, 6 skipped |
| Workspace Clippy, warnings denied | 0 | `target/836-signed-out-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-signed-out-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-signed-out-fmt-check.log` |
| `git diff --check` | 0 | `target/836-signed-out-diff-check.log` |
| Remote-client regressions after the Tab-boundary correction | 0 | `target/836-picker-tab-after.log`: 20 passed |
| Workspace nextest after the Tab-boundary correction | 100 | `target/836-picker-tab-nextest.log`: 2,739 passed, 1 failed, 6 skipped; isolated retry below |
| Isolated retry of daemon model-query fixture | 0 | `target/836-picker-tab-model-query-retry.log`: `concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded` passed |
| Clippy, with and without updater, after the Tab-boundary correction | 0 each | `target/836-picker-tab-clippy.log`, `target/836-picker-tab-clippy-updater.log` |
| Formatting and whitespace after the Tab-boundary correction | 0 each | `target/836-picker-tab-fmt-check.log`, `target/836-picker-tab-diff-check.log` |

Intermediate runs `target/836-signed-out-after.log` and `target/836-signed-out-fixed.log` exited 100 while the new test’s local-attention fixture was being corrected: a window/settings snapshot replaces manually seeded status, so each activation now has a fresh window and seeds attention after startup. The final test verifies a nonzero local name and badge before activating sign-in. The workspace suite for that candidate passes, including the OpenCode fixture that failed in the earlier Problem 1 run; that earlier failure remains preserved above.

The next review found a P2 in the new Tab handler: GPUI’s global focus traversal could leave the picker open while focus moved to the background. The boundary regression reproduces it (`target/836-picker-tab-before.log`, exit 100). The correction dismisses the picker and restores workspace-root focus whenever Tab or Shift-Tab leaves its focus subtree. The remote-client tests, both Clippy configurations, formatting and whitespace pass for the correction. Its workspace run hit an unrelated 30-second `query did not finish` failure in `runner-daemon`’s `runtime_status::models::tests::process_tests::concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded` at `runtime_status/models/tests.rs:183`; an isolated retry passed without code changes. This final workspace run is recorded as failed, not a clean full-suite pass.

### Whole-diff review verdict

On 2026-10-23 the reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete working-tree diff, including both problems and the keyboard correction, in Runner message `01M5M1ZP3EC21KVJZ07G5M3N81`. Its independent panel, signed-out and boundary regressions passed (3 tests, exit 0, `target/836-reviewer-tab-corrected.log`), as did whitespace validation (exit 0, `target/836-reviewer-tab-diff-check.log`). The reviewer skipped live checks under the brief and left the selected rechecks to Jason. The clean review authorizes the brief’s commit/push/PR workflow against `nightly-remote`; macOS and Windows CI must be green before the PR is called ready.

## Problem 3: blank remote sidebar tab labels

Jason connected the development client to the installed host successfully, then showed several chat rows with icons and status but no titles. This is a current m7c defect: the remote sidebar already mirrors projects, tabs and missions. Dimmed New chat, role and crew are intentional because companion protocol v1 has no creation requests.

Cause: the remote sidebar and workspace header rendered only the tab node’s explicit `name`. Normal tabs leave that name empty and take their visible label from the first session in pane order; the companion overview already includes those session titles.

Title fix: use the explicit tab name when present, otherwise the first populated pane’s session title in layout order. Untitled sessions fall back to their runtime (or the generic chat/terminal label), and empty tabs use the localized empty-tab label. The same resolution drives project rows, recent rows and the workspace header, and follows overview pushes. Jason then clarified that the whole remote project/tab structure felt unfinished and directed reuse of the current frontend sidebar components. The earlier title-only candidate passed 2,742 workspace tests (6 skipped), both Clippy variants, formatting and whitespace, all exit 0 (`target/836-sidebar-counts-*.log`); those results precede the shared-sidebar work below.

The fake-driven `remote_sidebar_uses_session_titles_for_unnamed_tabs_and_tracks_overview_updates` reproduces the missing project-row title before the fix (`target/836-sidebar-titles-before.log`, exit 100). It then checks project and recent titles, the active header, a pushed title change, an explicit-name override, split-pane order, empty tabs in English/Simplified Chinese and the untitled runtime fallback. The focused remote-client run before the final counter test passed 21 tests (`target/836-sidebar-counts-focused.log`, exit 0).

Structural cause: the remote view duplicated a simplified sidebar, ignored pin grouping, did not indent or collapse project children, and added a separate root-missions section. The overview's separate tab/mission arrays also lacked mixed sibling and pin order. The fix shares the local sidebar's tree model and row, section-header, workspace, scroll/layout and project-child components. Both sources adapt to the same node shape and presentation; remote row activation remains on the remote controller. Remote creation controls have no handlers, and remote collapse state never writes local settings. Project children and sections collapse, and existing runtime/split/mission marks, selection geometry and pin markers are reused.

Jason approved the optional overview `sidebar` node field through Runner messages `01M5M65NATDM421Y2VVAZZ1EP0` and `01M5M65NB689F8RYGMAANGJG54`, asking for the same kind/id/parent/position/pinned-position structure and matching companion goldens/spec. The host supplies its canonical node positions; the frontend uses them for mixed project, recent and pinned order. Missing metadata retains the older-host fallback, and future node kinds are ignored. No new request, service call, crate or migration. The protocol section and goldens are updated in this PR.

The structural regression reproduced the missing PINNED section before the fix (`target/836-sidebar-structure-before.log`, exit 100). Fake-driven checks cover pin exclusion from projects/recents, indented project tabs and missions, project/section collapse, selection, pushed pin changes, shuffled metadata with mixed row/pin order, moved membership, authoritative empty metadata, future kinds and older-host fallback. A shared-tree test compares local node rows with remote metadata. The shared disabled-controls regression verifies that mouse/keyboard activation opens no local creation modal and preserves local route/tabs. The daemon's in-process overview tests verify project/tab/mission metadata against canonical nodes; companion goldens verify optional-field compatibility.

Jason should recheck PINNED/PROJECTS/RECENTS, project indentation and collapse, provider/split/mission icons, the previously blank remote titles, selection, scrolling and host-driven order/title updates. Check that returning local preserves its sidebar state. Matching mixed row and pin order requires the updated host too; the older installed host retains fallback order. Live checks remain his; the crew operates neither app.

## Problem 4: remove picker attention counters

Jason removed the computer picker’s amber number and attention counters from this stage, and requested that design board `l2uAX` keep only A. This supersedes the initial badge-retention behavior and tests recorded under Problem 2; the earlier results remain historical evidence.

Fix: remove the trigger and row counters, the remote detail’s numeric attention suffix, their unused aggregation/localization and attention-only overview prefetches. Directory refreshes do not open remote overviews until a computer is selected. Preserve connection status, selection and chat/mission attention indicators. Design board `l2uAX` now contains only `pMdgR` (A), without either counter; B and C were deleted. The updated `runner/desktop/specs/836-web-remote-control.pen` was visually verified, explicitly saved and pushed in `yeelinks_design` commit `3042ccd`.

The revised signed-out fake regression fails before removal (`target/836-no-counters-before.log`, exit 100) with a nonzero local attention fixture. It now checks that the trigger and row counters are absent while sign-in behavior remains intact. `signed_in_picker_omits_local_and_remote_attention_counters` covers signed-in local and remote windows with nonzero local and remote attention. The guide and current spec reflect the decision.

Jason should check the trigger and dropdown while local, remote and signed out: no amber count or numeric attention suffix; connection state, selected computer and normal switching still work. No crew live checks ran.

### Combined Problems 3 and 4 validation

| Check | Exit code | Evidence |
| --- | --- | --- |
| Intermediate combined focused run | 100 | `target/836-sidebar-shared-final-focused.log`: 122 passed, 1 failed; fixture correction below |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 100 | `target/836-shared-sidebar-nextest.log`: 2,745 passed, 2 failed, 6 skipped, in 301.571 seconds; all sidebar and companion regressions passed |
| One isolated retry of both failing model-query fixtures, one test thread | 0 | `target/836-shared-sidebar-model-query-retry.log`: both passed without code changes |
| Workspace Clippy, warnings denied | 0 | `target/836-shared-sidebar-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-shared-sidebar-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-shared-sidebar-fmt-check.log` |
| `git diff --check` | 0 | `target/836-shared-sidebar-diff-check.log` |

The intermediate focused failure was in the new mission metadata fixture: `node::upsert` preserves an existing node's pin state, so assigning a pin on the input row did not pin it. The fixture now uses `node::set_pinned` and checks the stored position and pin. Its corrected regression passes in the workspace run.

The workspace failures were in unchanged `runtime_status::models::tests::process_tests` fixtures: `spawn_failure_timeout_and_cache_write_failure_are_contained` observed no catalog instead of its fake `memory` catalog at `tests.rs:578`, and `concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded` hit its 30-second deadline at `tests.rs:183`. Both passed in one isolated retry with no code changes. The workspace command remains recorded as failed; no full-suite green result is claimed. No model-query implementation or test is changed by this diff. CLI help is unchanged, so `make guide-cli-check` is not applicable.

### Problems 1–4 review verdict

The reviewer reported **NO REMAINING MUST-FIX ISSUES** for the whole feature-branch working-tree diff in Runner message `01M5M7YMZRWF2D54EGGYXVGYAQ`. Its independent shared-sidebar, remote-client, companion and overview tests passed (116 tests, exit 0, `target/836-reviewer-shared-sidebar.log`); whitespace validation passed (exit 0, `target/836-reviewer-shared-sidebar-diff-check.log`). It inspected the approved Pencil A and confirmed that it has no counts. The full workspace failures and successful isolated retries above remain preserved. Live checks were skipped under the brief; Jason's selected rechecks remain above. The subsequent behavior-neutral reuse work requested in human message `01M5M77X1ZT1Q0WV1WFXB0AJGD` is separate and requires its own review.

### PR CI history before Problems 3 and 4

PR #1010 initially contained Problems 1 and 2 at `31993ce7`. macOS CI passed (2,740 tests passed, 6 skipped). Windows attempt 1 exited 1 with 2,624 passed / 1 failed / 4 skipped: only the brief’s known `terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` flake failed. Unchanged attempt 2 exited 1 with the same totals: the redraw test passed, but `shell_echo_two_clients_and_identical_reattach_snapshot` hit its 10-second terminal-output deadline at `daemon_process.rs:419`; that test passed in attempt 1. All four picker regressions passed in both Windows attempts. Logs: `target/836-ci-windows-failed.log`, `target/836-ci-windows-retry-failed.log`, `target/836-ci-macos.log`. A third unchanged Windows run was already running when Jason questioned repeated runs. It exceeded the earlier runtimes and was superseded by Problems 3 and 4; cancellation was requested with `gh run cancel 39933610530` (exit 0). The cancelled attempt’s tests had completed successfully (2,625 passed, 4 skipped); cancellation occurred in the remaining job steps, so it is not a green CI claim (`target/836-ci-windows-cancelled.log`). No further automatic retry of that old candidate will be started. New Problems 3 and 4 remain uncommitted pending whole-diff review.

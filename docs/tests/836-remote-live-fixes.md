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

PR #1010 initially contained Problems 1 and 2 at `31993ce7`. macOS CI passed (2,740 tests passed, 6 skipped). Windows attempt 1 exited 1 with 2,624 passed / 1 failed / 4 skipped: only the brief’s known `terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` flake failed. Unchanged attempt 2 exited 1 with the same totals: the redraw test passed, but `shell_echo_two_clients_and_identical_reattach_snapshot` hit its 10-second terminal-output deadline at `daemon_process.rs:419`; that test passed in attempt 1. All four picker regressions passed in both Windows attempts. Logs: `target/836-ci-windows-failed.log`, `target/836-ci-windows-retry-failed.log`, `target/836-ci-macos.log`. A third unchanged Windows run was already running when Jason questioned repeated runs. It exceeded the earlier runtimes and was superseded by Problems 3 and 4; cancellation was requested with `gh run cancel 39933610530` (exit 0). The cancelled attempt’s tests had completed successfully (2,625 passed, 4 skipped); cancellation occurred in the remaining job steps, so it is not a green CI claim (`target/836-ci-windows-cancelled.log`). No further automatic retry of that old candidate will be started. Problems 3 and 4 subsequently received the clean whole-diff review above and were committed separately as `c186f81b` and `44a8ab6b`; they have not been pushed while the authorized follow-ups are in progress.


## Behavior-neutral helper extraction

Human Runner message `01M5M77X1ZT1Q0WV1WFXB0AJGD` requested further reuse after the Problems 3/4 handoff. The completed first step extracts the shared relative-time formatter used by the mission rail and remote last-seen text, plus one model helper for the synthesized local computer used by Settings and the picker. Callers retain their previous parsing, signed-out naming and clock behavior. New tests cover future timestamps and minute/hour/day boundaries in both languages, and named/generic/missing local-computer metadata. Existing tests are unchanged for this refactor. The focused helper run passed (2 tests, exit 0, `target/836-helper-refactor-focused.log`). The remaining sidebar-decoration, terminal-input/surface and pane-renderer refactors are paused for the security findings below. The duplicated revoked/unreachable error-string classification remains a follow-up: replacing it with a structured reason would touch the client protocol and needs a separate decision.

## Security findings from code review

Source: human Runner message `01M5M85WWJEG5W01KB5KRE4GCD`, verified against umbrella `b5869c0d`. These are code-review findings; none was live-exploited. The fixes use fakes, an in-memory secret store and the in-process relay; the crew operates neither live app and reads no actual secret store. No wire format, request, service contract, crate or migration changes.

### Problem 5: weak PINs need an informed choice

Cause: setting a PIN checked only the six-digit ASCII format and gave no warning about obvious repeated digits, sequences or common choices that could fit inside five online guesses. CPace prevents offline guessing; it does not prevent a correct online guess. The initial rejection implementation was superseded by Jason's explicit warn-and-allow decision in Runner message `01M5MD17NHSYJ2XB6FERGRZJG5`.

Fix: all six ASCII digits remain valid at the host and on the wire. Settings and `runner remote pin` share weak-pattern detection, warn that the PIN could be guessed within five tries, and allow the person to keep it only after a separate explicit confirmation. Settings shows the English or Simplified Chinese warning after both entries match, without displaying the PIN. Choose another clears both fields; Escape/outside click dismisses without saving. The safe secondary button receives focus, Tab cycles the two actions, and held Enter/Space cannot approve the choice. The CLI warning is bilingual and requires `y` or `yes`; empty input/EOF/default No leave the PIN unchanged. Strong PINs keep the existing flow. Existing pairings remain valid. No pairing-code redesign or follow-up is planned; Jason keeps the current user-chosen PIN design.

The original rejection attempt's fake-store regression failed before that initial implementation (exit 100, `target/836-weak-pin-before.log`) and its eight focused checks passed (exit 0, `target/836-weak-pin-focused.log`). Those historical results do not validate the revised warning policy. The revised fake-store and GUI regressions both fail against the rejection candidate (2 failures, exit 100, `target/836-weak-pin-warning-before.log`). The corrected focused run passes all 10 selected checks (exit 0, `target/836-weak-pin-warning-focused.log`): core detection/format, host acceptance without unpairing, Settings warning/private errors/two-step flow, CLI confirmation/default cancellation and trust persistence/lock behavior. Core cases retain every repeated digit, forward/reverse/wrapped runs, common patterns, leading zeroes and malformed inputs. The warning designs are `awP6z` / `Xs2de` in `runner/desktop/specs/836-web-remote-control.pen`, saved and pushed as private design commit `d25a03d`. Required gates and re-review for this revision are recorded below. Problems 6 and 7 are unchanged.

### Problem 6: forged rejection deletes a saved pairing

Cause: a plaintext `unknownDevice` rejection was converted to an error before the host handshake was authenticated, then the connect path deleted the saved client key based on that string. The relay can forge this frame.

Fix: remove that trust mutation. Rejections remain diagnostics; the client keeps its key and reports that this computer was not recognized, with fingerprint-verification guidance. The app's copy no longer claims that this unauthenticated response proves host revocation. Only an explicit Forget or submitting Pair again removes the saved key; the CLI retains its refusal to pair over an existing key. Inspection of the other pre-authentication paths found no other stored-pairing mutation: successful pairing saves keys after authenticated completion, while host device/last-seen updates follow verified handshakes; wrong-PIN lock accounting remains intentional.

The fake relay sends each plaintext reject kind without authenticating a host. `unauthenticated_rejects_preserve_saved_pairing_keys_until_explicit_forget` fails before the fix (exit 100, `target/836-reject-trust-before.log`), then verifies unchanged saved keys and paired directory state until explicit Forget. The existing real-host-revoke relay case now verifies key retention followed by explicit Forget. English/Simplified Chinese copy, guide and spec match this behavior.

### Problem 7: pending pairing survives a PIN change

Cause: the pending host handshake captured the old PIN and could finish inside its deadline after Set PIN. Trust persistence is asynchronous, so checking only the final handshake would also leave a completion pending across a later change or remote-task restart.

Fix: capture a pairing generation at handshake start and verify it before finishing and again after saving trust. Set PIN, turning off Allow remote access, signing out and Reset remote access invalidate it; restoring the remote task allocates a fresh process-wide generation. Before saving other changes or reporting success, each invalidating command durably removes unconfirmed devices through the ordered trust writer. This also orders older rollback snapshots, so they cannot restore removed trust. A failed save reports an error and restarts the remote task rather than acknowledging the changed in-memory state. Authenticated channels and existing trusted devices remain usable after Set PIN. Pause/reset retain their existing channel-closing behavior.

The in-process relay regressions for a pending final handshake and a delayed trust-save completion both fail before the fix (2 failures, exit 100, `target/836-pending-pair-before.log`). They cover Set PIN, Allow remote access off, Reset remote access and a delayed completion after a simulated remote-task restart, including the resulting persisted trust. A separate regression confirms that an authenticated channel still answers requests after changing the PIN and that the new PIN can pair a new device. No sleeps or network are added.

The first combined security run passed 48/49 tests (exit 100, `target/836-security-focused.log`): the existing socket-loop test still tried to set a now-rejected sequential fixture PIN. Its setup and peer input now use the same nontrivial test PIN. The corrected combined security run passes all 50 tests (exit 0, `target/836-security-focused-final.log`), including all host-channel/client relay cases and the new regressions.

Jason's selected live rechecks, after code review: show the weak-PIN warning in both app languages and from `runner remote pin`, confirm that declining leaves the PIN unchanged and explicitly accepting keeps it; accept a PIN with leading zeroes; keep current pairings and authenticated terminals working after a PIN change; verify Pair again/Forget explicitly removes retained trust after fingerprint confirmation. Pending-handshake and forged-reject cases are covered by the fake relay and require no live attack. The larger pairing-code redesign and the remaining refactors are not implemented by this security diff.


### Security and helper validation results

| Check | Exit code | Evidence |
| --- | --- | --- |
| Final focused security regressions | 0 | `target/836-security-focused-final.log`: 50 passed |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `target/836-security-nextest.log`: 2,755 passed, 6 skipped, in 299.998 seconds; no retries |
| Workspace Clippy, warnings denied | 0 | `target/836-security-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-security-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-security-fmt-check.log` |
| `git diff --check` | 0 | `target/836-security-diff-check.log` |

This complete workspace run covered the initial security/helper candidate, including the previously failing unchanged model-query fixtures; it does not erase the older failed runs recorded above. The CLI help golden took 67.245 seconds and passed. CLI help is unchanged, so `make guide-cli-check` is not applicable. The review below found a persistence gap that these green tests did not cover. The security and helper changes remain uncommitted pending clean review; the remaining terminal/sidebar reuse work is paused until the security fixes are clean. No live checks ran.

### Problem 7 review correction: durable invalidation before success

Reviewer message `01M5M9FKX9GDX3G5DH7G9H5VVM` found one P1 must-fix: a pairing could already have saved its provisional device while its completion remained unconsumed. The initial fix invalidated only process memory, then rolled trust back later. A process exit after Set PIN reported success could leave that device in the saved trust; a fresh daemon with a new writer could accept it. The reviewer found no additional must-fix in Problems 5/6 or the two helper extractions. Its independent 54 tests and whitespace check passed (exit 0, `target/836-reviewer-security.log`, `target/836-reviewer-security-diff-check.log`), but the verdict was not clean.

The correction awaits removal of every unconfirmed device through the production command path and the existing ordered trust writer before other effects, including the new PIN save, execute. It snapshots trust before the transition so Reset identity can still preserve confirmed devices if saving its new identity fails. Pending bookkeeping is removed only after the filtered trust save succeeds. A write failure returns a command error and exits the remote task; a later completion cannot recreate successfully purged trust.

Three additional regressions exercise the production command and effect path with an in-memory secret store and temporary files. They hold a saved pairing completion, run Set PIN, Allow remote access off, sign-out or Reset identity, then reload saved state into a fresh machine and new trust writer before consuming the old completion. A real Noise IK reconnect from the provisional device is refused; confirmed trust is retained except for Reset identity. A held writer proves the command and PIN save wait for durable removal; a failed writer proves the command reports failure and the stored PIN is unchanged, then retries through the same production path and verifies that a late completion cannot restore access. No sleeps, network or manually corrected trust save are used.

| Check | Exit code | Evidence |
| --- | --- | --- |
| Fresh-load, held-write and failed-write regressions before durable invalidation | 100 (expected failure) | `target/836-pairing-durable-before.log`: all 3 failed |
| Host/channel regressions after durable invalidation | 0 | `target/836-pairing-durable-focused.log`: 42 passed |
| Workspace Clippy, warnings denied | 0 | `target/836-security-durable-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-security-durable-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-security-durable-fmt-check.log` |
| First workspace run after durable invalidation | 100 | `target/836-security-durable-nextest.log`: 2,757 passed, 1 failed, 6 skipped; correction below |
| Whitespace validation after durable invalidation | 0 | `target/836-security-durable-diff-check.log` |

The first complete workspace run after this correction exposed a real regression in `daemon::server::tests::client_accept_errors_back_off_and_preserve_live_sessions`: setting the first PIN in an isolated daemon created `remote-trust.json`, despite there being no pending pairing or older trust writer. The correction uses the durable barrier only when the pairing writer already exists; without one, no pending pairing or detached trust write can exist. The existing test is unchanged. All 43 selected host/channel and server regressions then passed (exit 0, `target/836-pairing-durable-final-focused.log`). An intermediate attempt used let-chain syntax unsupported by this crate's Rust edition; nextest exited 101 and formatting exited 1 before running tests (`target/836-pairing-durable-parse.log`, `target/836-security-durable-fmt-parse.log`), then the syntax was corrected. Final formatting passes (exit 0, `target/836-security-durable-final-fmt-check.log`).

| Final correction check | Exit code | Evidence |
| --- | --- | --- |
| Host/channel and isolated-server regressions | 0 | `target/836-pairing-durable-final-focused.log`: 43 passed |
| Full workspace nextest after the first-PIN correction | 0 | `target/836-security-durable-final-nextest.log`: 2,758 passed, 6 skipped, 294.162 seconds; no retries |
| Workspace Clippy, warnings denied | 0 | `target/836-security-durable-final-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-security-durable-final-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-security-durable-final-fmt-check.log` |
| `git diff --check` | 0 | `target/836-security-durable-final-diff-check.log` |

The final full workspace run passes, including the unchanged isolated-server regression; the CLI help golden took 65.026 seconds and passed. All prior failures remain recorded. The corrected candidate received the clean whole-diff review below. Jason's live rechecks above remain unchanged; the persistence race is covered in process by fakes.

### Security and helper review verdict

The reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete feature-branch working-tree diff in Runner message `01M5MAZZX6N9TPCP48CP38DKRR`, including Problems 5–7 and both helper extractions. It confirmed the durable command barrier, first-PIN guard, preserved confirmed trust and fresh-load regressions resolve the P1. Its independent security/helper checks passed (85 tests, exit 0, `target/836-reviewer-security-durable.log`), as did an explicit run of the isolated-server regression (1 test, exit 0, `target/836-reviewer-security-first-pin.log`) and whitespace validation (exit 0, `target/836-reviewer-security-durable-diff-check.log`). It inspected the final full workspace and both Clippy passes; earlier failures remain preserved. Live checks were skipped under the brief and remain Jason's selected checks above. Remaining terminal/sidebar/pane reuse work is outside this verdict and requires a separate working-tree review. No new-head CI result is claimed.

## Shared terminal input, copy and paste

This behavior-neutral step implements item 1 of human Runner message `01M5M77X1ZT1Q0WV1WFXB0AJGD`. Chat, mission, agent update and remote terminals now call one terminal-module handler for Option-as-Meta, Option-copy swallowing, IME/shortcut routing, raw key encoding and scroll-to-bottom on a successful send. The handler returns the existing send result; each surface retains its own error, notification and propagation behavior. Copy uses one selection-to-clipboard helper, retaining the surfaces' Windows propagation behavior when no selection exists. Local chat and mission paste use one text/image dispatch helper after their existing file/image resolution; agent update and remote keep text-only clipboard paste through a shared helper. Interactive/control guards are unchanged. Existing tests are untouched.

New fake-transport tests cover composing/printable/shortcut/Option key routes, raw Meta encoding, scroll behavior on success and failure, unchanged clipboard without a selection, copying the selected text, bracketed text paste, runtime/platform image shortcuts and surfaced write errors. The two new tests and eight existing terminal-control tests pass (10 tests, exit 0, `target/836-shared-input-focused.log`); this focused command selected the app binary, so it does not claim library IME-test coverage. A now-unused `ClipboardItem` import reported in that run was removed. All required full gates pass below; the working-tree review is clean. No live app, daemon, agent or service check ran.

The remaining pane-renderer and terminal-surface consolidation is outside this step. The current remote split normalizes/clamps its ratio and draws a one-pixel separator; the local card renderer uses percent sizes, resize gutters and bottom-corner padding. Reusing its card geometry changes the remote terminal viewport and resulting host size, beyond a strictly behavior-neutral extraction. The sidebar status renderers likewise differ in glyphs, dot size and tooltips. These presentation differences were flagged through Runner before implementing the remaining items, as the human request requires. Jason approved adopting local presentation here, including its viewport/host PTY size changes, in Runner message `01M5MCYXH6HFG2C9RK5MHDKB97`. That work remains separate from the PIN warning revision.

| Shared input check | Exit code | Evidence |
| --- | --- | --- |
| New shared-input and existing terminal-control regressions | 0 | `target/836-shared-input-focused.log`: 10 passed |
| Full workspace nextest | 0 | `target/836-shared-input-nextest.log`: 2,760 passed, 6 skipped, 291.920 seconds; no retries |
| Workspace Clippy, warnings denied | 0 | `target/836-shared-input-clippy.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-shared-input-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `target/836-shared-input-fmt-check.log` |
| `git diff --check` | 0 | `target/836-shared-input-diff-check.log` |

The full run includes the library's five IME and five clipboard/paste tests. The CLI help golden took 65.032 seconds and passed. CLI help is unchanged, so `make guide-cli-check` is not applicable. This is local validation; PR #1010 still has the older pushed candidate and has no new-head CI result for these changes.

### Shared input review verdict

The reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete feature-branch working-tree diff in Runner message `01M5MBXB5Z3AANPVKZCTPP7RN7`, including the shared terminal input/copy/paste extraction. It independently passed 25 shared-input, control, updater, library IME and library paste tests (exit 0, `target/836-reviewer-shared-input.log`) and whitespace validation (exit 0, `target/836-reviewer-shared-input-diff-check.log`), and inspected the full workspace and both Clippy passes. It confirmed existing tests and all surface-specific guards, error handling, notifications and Windows propagation are preserved. No additional live matrix is needed for this behavior-neutral extraction; no crew live checks ran. Earlier UI/security rechecks remain Jason's. Sidebar-decoration, pane-renderer and terminal-surface presentation work remains outside this verdict; Jason subsequently authorized local presentation in Runner message `01M5MCYXH6HFG2C9RK5MHDKB97`.

## Current-head CI before the PIN-policy revision

The reviewed candidate at `40419794` was pushed to PR #1010 against `nightly-remote`. `gh pr checks 1010 --watch` exited 0 with no retries: macOS passed in 11m50s (2,760 tests passed, 6 skipped) and Windows passed in 16m33s (2,645 passed, 4 skipped), run `39954124901`. Evidence: `target/836-ci-current-watch.log`, `target/836-ci-current-macos-final.log`, `target/836-ci-current-windows-final.log`. The first attempt to download the completed macOS job's logs exited 1 only because GitHub's CLI waits for the entire workflow to finish; both final downloads exited 0. The tree was clean and synchronized with origin. These CI results cover the previous rejection policy and do not claim CI for the revised warning behavior.

## Problem 5 revision: warn and allow

Human Runner message `01M5MD17NHSYJ2XB6FERGRZJG5` explicitly replaces rejection with a private warning and affirmative confirmation while preserving all six-digit PINs on the wire. English/Simplified Chinese Pencil states `awP6z` / `Xs2de` were drawn and visually checked before implementation; layout warnings were corrected, the file was saved with Cmd+S and design commit `d25a03d` was pushed. The new fake-driven regressions and focused pass are detailed under Problem 5 above. No redesign, protocol change, live app/service check or actual secret-store read is involved. This revision received the clean working-tree review below and was folded into Problem 5's commit as Jason requested; the remaining presentation reuse work is separate.

| Warning revision check | Exit code | Evidence |
| --- | --- | --- |
| New GUI/fake-store regressions against the rejection candidate | 100 (expected failure) | `target/836-weak-pin-warning-before.log`: both failed |
| Initial focused warning checks | 0 | `target/836-weak-pin-warning-focused.log`: 10 passed |
| First compile after adding held-key coverage | 101 | `target/836-weak-pin-warning-held-compile.log`: the test event lacked GPUI's required `prefer_character_input` field |
| Final focused checks, including held Enter/Space at both warning buttons | 0 | `target/836-weak-pin-warning-focused-final.log`: 10 passed |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `target/836-weak-pin-warning-workspace-final.log`: 2,762 passed, 6 skipped, 299.580 seconds |
| Workspace Clippy, warnings denied | 0 | `target/836-weak-pin-warning-clippy-final.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-weak-pin-warning-clippy-updater-final.log` |
| `cargo fmt --all --check` | 0 | `target/836-weak-pin-warning-fmt-final.log` |
| `git diff --check` | 0 | `target/836-weak-pin-warning-diff-final.log` |

The held-key test compile error was corrected by explicitly setting `prefer_character_input: false`; the final focused and full runs include that correction. The full run had no failures or retries. Its only slow test, the CLI help golden, passed in 64.936 seconds. CLI help is unchanged, so `make guide-cli-check` is not applicable. The GUI regression uses a fake lifecycle store and verifies the submission boundary without a live RPC; the daemon regression uses an in-memory secret store. Problems 6 and 7 and their trust-invalidation behavior are unchanged. These local gates cover the warning revision; the earlier CI result at `40419794` does not.

### Warning revision review verdict

The reviewer reported **NO REMAINING MUST-FIX ISSUES** for the handed-off whole working-tree diff in Runner message `01M5MEKT93NDXV8ND103XGZHKB`, including the app actions/focus/held-key handling, CLI interactive and save boundaries, both host validators, tests, locales and documentation. It independently passed 11 focused checks (exit 0, `target/836-reviewer-weak-pin-warning.log`) and whitespace validation (exit 0, `target/836-reviewer-weak-pin-warning-diff-check.log`), inspected the full workspace and both Clippy passes, and checked Pencil frames `awP6z` / `Xs2de` read only. No native UI, real CLI, pairing or service check ran under the mission restriction. Jason should recheck warning/cancel/accept in both UI languages and interactive CLI, a leading-zero PIN and preservation of existing pairing/terminal access. The previous Problems 6/7 review stands; the remaining presentation reuse and Problems 8–11 are outside this verdict. The warning revision folds into the original Problem 5 commit; its CI must be checked on the new pushed head.


## Shared sidebar status presentation

Human Runner messages `01M5M77X1ZT1Q0WV1WFXB0AJGD` and `01M5MCYXH6HFG2C9RK5MHDKB97` request reuse of local sidebar status presentation and explicitly authorize the resulting visual changes. The remote sidebar still reduced status to four booleans and drew its own hand, spinner and five-pixel unread dot, omitting reason/error priority and the local tooltips. This step maps the existing companion status/reason/unread fields to `AgentStatus`, then uses the same `StatusRollup::render` as local rows. No protocol or service change is involved.

The shared renderer now supplies approval, answer, unknown-input, working, unread and error presentation and priority. Empty/stopped rows dim like local rows; shells do not produce agent attention. Running missions without live members suppress stale unread flags as local missions do; pending mission questions remain visible through the shared answer-needed indicator. The overview contains no status times: equal internal markers retain host pane/member order for ties and are used only as flags by sidebar rollups, never as elapsed times.

Pencil was updated before implementation: `pzyfX` is the shared sidebar tree used by all eight existing remote workspace/reconnection frames, retaining PINNED/PROJECTS/RECENTS and showing the local status marks; the stale picker badges in those frames were removed. Visual verification showed no overflow. Replacement warnings referenced the removed fragments; subsequent inspection confirmed the old fragments no longer exist and the final call reports no warnings. The file was explicitly saved and design commit `227fbe3` pushed. The existing local sidebar component remains intact. Pane geometry, terminal surfaces and the Problem 11 header are separate pending work.

Selected checks: a fake-driven GPUI regression switches tab and mission rows through all companion states, future-value fallbacks and each input reason via overview and session-status pushes, verifies shared status indicators and unread geometry, retains pending-question attention, and confirms no enrollment, input or explicit Take control request is sent by the status updates. A data-adapter test checks round-trip status/reason/unread preservation and the local multi-session priority, tooltip and stable tie order. Existing remote sidebar/tree tests and shared status tests guard grouping and the unchanged local renderer; the brief's full workspace, both Clippy, formatting and whitespace gates remain required. Jason's later live recheck is to compare local/remote working, unread, approval/answer/input/error and dimmed stopped rows, including pushed changes and tooltips; the crew runs no live checks.

### Shared sidebar status validation results

| Check | Exit code | Evidence |
| --- | --- | --- |
| Initial regression compile | 101 | `target/836-sidebar-status-before.log`: the GPUI selector requires a static string; corrected to static selector cases |
| Regression before the presentation change | 100 (expected failure) | `target/836-sidebar-status-before-final.log`: remote rows do not render the shared approval indicator |
| Initial adapter/renderer focused compile | 101 | `target/836-sidebar-status-focused.log`: the test reused non-Copy companion enums; corrected fixture cloning |
| Corrected focused run before isolating row fixtures | 100 | `target/836-sidebar-status-focused-corrected.log`: 11 passed, 1 failed; the geometry assertion selected the unrelated mission row in the tab fixture |
| Focused checks with isolated tab/mission fixtures | 0 | `target/836-sidebar-status-focused-final.log`: 39 passed |
| Final focused checks, including both push types and future status/reason fallback | 0 | `target/836-sidebar-status-focused-compatible.log`: 39 passed |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `target/836-sidebar-status-workspace.log`: 2,764 passed, 6 skipped, 299.088 seconds |
| Workspace Clippy, warnings denied | 0 | `target/836-sidebar-status-clippy-final.log` |
| Workspace Clippy with `--features updater`, warnings denied | 0 | `target/836-sidebar-status-clippy-updater-final.log` |
| `cargo fmt --all --check` | 0 | `target/836-sidebar-status-fmt-check.log` |
| `git diff --check` | 0 | `target/836-sidebar-status-diff-check.log` |

The final fixture renders one tested row kind at a time, preserving the containment assertion and adding the exact shared unread-dot geometry assertion. The new adapter uses `effective()` so a future status remains idle and a future needs-you reason remains generic input, as the existing protocol requires. No existing tests were rewritten.

The full workspace run passed with no failures or retries. Its one slow test, the CLI help golden, passed in 67.456 seconds. All sidebar, remote model/control and shared status tests passed. CLI help is unchanged, so `make guide-cli-check` is not applicable. The source is unchanged since the final focused run. This step received the clean working-tree review below and is kept in its own presentation commit.

## PIN warning CI after the authorized fold

After clean review, the warning revision was folded into Problem 5 as `daa7fe8e`, and the later commits replayed on unchanged `origin/nightly-remote` (`b5869c0d`). Conflicts were confined to documentation. The rebased final tree exactly matches the saved reviewed candidate plus its recorded verdict (`564c3519853af82205c922cd6482bad7dad7f4f4`); Problems 6/7 remain separate and intact. The clean branch at `aede834f` was pushed to PR #1010 with the brief's authorized force-with-lease.

`gh pr checks 1010 --watch` exited 0 with no retries for run `39960549218`: macOS passed in 9m49s (2,762 passed, 6 skipped; test summary 225.238 seconds), Windows passed in 18m31s (2,647 passed, 4 skipped; test summary 504.672 seconds). Evidence: `target/836-warning-ci-watch.log`, `target/836-warning-ci-macos.log`, `target/836-warning-ci-windows.log`; both log downloads exited 0. This CI covers the reviewed PIN warning revision at `aede834f`, not the uncommitted sidebar status work above. No merge, live setup operation, branch deletion or release action was performed.

### Shared sidebar status review verdict

The reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete feature-branch working-tree candidate in Runner message `01M5MG4MS66XCPE3B8QWDJQSEA`. It verified the handed-off diff hash, independently confirmed that the P5 fold preserved the saved final tree, traced local/remote callers and status pushes, and checked the timestamp-less flags against the shared count-only rollup tooltip. It independently passed 78 remote/shared-status/local-sidebar checks (exit 0, `target/836-reviewer-sidebar-status.log`) and whitespace validation (exit 0, `target/836-reviewer-sidebar-status-diff-check.log`), inspected the full workspace and both Clippy passes, and read-only inspected Pencil `pzyfX`. No live checks ran under the brief.

Jason should compare local/remote working, unread, approval/answer/generic-input/error glyphs and hover tooltips, mixed-pane dominant priority and pushed changes, stopped/empty dimming, shell rows without agent attention, and pending-question attention. The adapter's equal marker times remain limited to sidebar count tooltips; future header work must not feed them to elapsed-time formatting. Pane/surface reuse, the Problem 11 header and Problems 8–10 remain outside this verdict.

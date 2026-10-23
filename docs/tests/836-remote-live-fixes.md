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

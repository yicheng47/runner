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

On 2026-10-23 the reviewer reported **NO REMAINING MUST-FIX ISSUES** for Problem 1 in Runner message `01M5M09FPTVZ5RYYHA04DX41H1`. Its independent corrected regression and whitespace check both exited 0 (`target/836-reviewer-picker-corrected.log`, `target/836-reviewer-diff-check-corrected.log`). Live checks were skipped under the mission brief and remain Jason’s selected follow-up above. This verdict covers only the transparent-picker fix; the signed-out picker design is pending Jason’s choice, and PR action remains on hold.

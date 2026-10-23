# 836 Remote access switch validation

Date: 2026-10-23–24. Branch: `feat/836-remote-access-switch`, initially based on `864bc82b` of `nightly-remote`, then rebased onto `a7750819` before the PR. Scope: the four app changes in `docs/impls/briefs/836-remote-access-switch.md`. All app checks use in-process fixtures and fake transports; no app, live daemon, remote service or credential store is operated. Live checks belong to Jason on macOS and Windows.

## Design

Pulled `yeelinks_design` before editing. Design commits `c78be67`, `53fa86f` and `a99aeb3` are pushed. File: `runner/desktop/specs/836-web-remote-control.pen`. Frames: `zjsgx` (Account status and Sign out only), `fm3mr` (relay sign-in without route choice), `Leypq` (switch off), `Z4ltjp` (switch on, signed out), `Iwu1z` (local sidebar and Settings gear), `TKakw` (step 2.1 picker with status before Not paired). Inspected screenshots for layout, clipping, contrast and spacing. Local review PNGs: `target/design-ref/836-remote-access-switch/`.

The human approved the switch frames in Runner message `01M5MQ4W8VJEQ4SP6Q5DG9RM16`. Change 4 was added in `01M5MQ3Q8R1BDCFM5XK974E83B`; preserving and including existing saved design edits was authorized in `01M5MQ8W18EGE53F5X63K5V122`. The prior Account usage and first-visit route-choice frames are named Superseded; Tailscale is marked Future #954.

## Changed behavior and selected checks

| Change | Check and reason | Result |
| --- | --- | --- |
| 1. Account loses its usage section and footer | `every_status_renders_in_both_languages_and_themes` seeds fake remote statuses in the test store. Signed-in states must put Sign out directly after the status card and end the pane there. Both locales and dark/light themes are covered. | Passed, exit 0. |
| 2. One relay route; first visit and Reset return to relay sign-in | The status rendering test asserts the relay entry in signed-out/stopped states, no Tailscale card, and retained approval/connected states. `account_actions_reuse_requests_and_sign_out_and_reset_require_confirmation` uses `RemoteRequests` to confirm Reset returns to the relay entry and Sign in issues the enrollment request directly. | Passed, exit 0. |
| 3. Remote access switch | Fake-driven `remote_switch_defaults_off_and_persists_without_sign_in_across_windows` checks off UI, the direct gear, Account navigation, two windows, saved settings and no enrollment in both locales. `remote_switch_signed_in_requires_confirmation_and_failed_logout_stays_visible` checks legacy/CLI signed-in override, cancel, confirm, failed sign-out and retained identity/PIN/pairings. `remote_status_keeps_signed_in_surfaces_visible_and_wakes_every_window` covers each signed-in state and stale metadata after sign-out. `disabling_remote_access_returns_every_window_to_this_computer` proves remote views release follows and return to local. `remote_switch_cancels_pending_enrollment_when_turned_off` covers leaving approval. | Passed in full app suite, exit 0. |
| 4. Unpaired picker status | `picker_status_precedes_unpaired_detail_from_directory` refreshes a fake directory with online and offline unpaired computers and checks status-first text, including last seen, in both locales. The renderer uses that detail without changing row controls. | Passed, exit 0. |

No maintained live case covers the new remote switch yet. The selected live checks below are feature-specific. Existing agent runtimes, IME and session lifecycle are outside the feature's live scope. There is no separate guide page listing Settings panes; `docs/guide/guides/remote-access.md` owns these surfaces.

## Commands and gates

Commands are logged under this worktree's `target/`; exit codes are captured from the commands themselves. Tests run with `CLAUDE_CONFIG_DIR` unset.

| Command | Log | Exit / result |
| --- | --- | --- |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --package runner-app --cargo-profile ci -E 'test(every_status_renders_in_both_languages_and_themes)'` | `target/836-change1-test.log` | 0; 1 passed. |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --package runner-app --cargo-profile ci -E 'test(surfaces::settings::remote::tests)'` | `target/836-changes1-2-test.log` | 0; 7 passed. |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/836-workspace-tests.log` | 100; 2765 passed, 1 failed, 6 skipped. See failure below. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/836-clippy.log` | 0. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/836-clippy-updater.log` | 0. |
| `cargo fmt --all --check` | `target/836-fmt.log` | 0. |
| `git diff --check` | `target/836-diff-check.log` | 0. |

Initial change-1 formatting check returned 1 for a trailing blank line after removing the obsolete copy test (`target/836-change1-fmt.log`); `cargo fmt --all` corrected it, and the gate above passed. The known terminal-redraw flake will be recorded if encountered.

The workspace run failed only in unchanged `runner-daemon::claude_accounts::tests::add_and_login_keeps_each_request_bound_to_its_created_account`, at `claude_accounts.rs:968`: the fixture’s `login-started` file read as empty instead of the temporary account folder. The fixture waits for file existence before reading its contents, so creation-before-write is a possible race; this is an inference, not an established cause. The daemon is outside this mission’s code scope and was not changed. Isolated retry passed (1 test, command exit 0) in `target/836-claude-account-retry.log`; no source change was made. The known terminal-redraw flake did not occur. The pre-review complete-candidate workspace gate passed with 2772 tests; the reviewer then found the two cases below.

## Complete candidate gates

| Command | Log | Exit / result |
| --- | --- | --- |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --package runner-app --cargo-profile ci -E 'test(picker_status_precedes_unpaired_detail_from_directory)'` | `target/836-change4-test.log` | 0; 1 passed. |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --package runner-app --cargo-profile ci -E 'test(remote_switch) \| test(remote_status_keeps_signed_in) \| test(disabling_remote_access)'` | `target/836-change3-tests-v4.log` | 0; 4 passed. |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --package runner-app --no-fail-fast --cargo-profile ci` | `target/836-final-app-tests-v2.log` | 0; 860 passed, 3 skipped. Includes the later pending-enrollment check. |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/836-final-workspace-tests.log` | 0; 2772 passed, 6 skipped. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/836-final-clippy.log` | 0. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/836-final-clippy-updater.log` | 0. |
| `cargo fmt --all --check` | `target/836-final-fmt.log` | 0. |
| `git diff --check` | `target/836-final-diff-check.log` | 0. |

Switch test development retained three unsuccessful attempts: `836-change3-tests.log` exited 101 for two missing mutable test handles; `836-change3-tests-v2.log` exited 100 because the request-only fake was installed before window creation and its deliberately unsupported subscription was reached; `836-change3-tests-v3.log` exited 100 because the test seeded signed-in status before draining the initial fake store snapshot. These fixture issues were corrected, and v4 passed without a product workaround. The initial full app run (`target/836-final-app-tests.log`, exit 100) found two existing mission/remote-view fixtures that had never enabled Remote access; their shared fixture now explicitly opts in, and the full rerun passed. No installed or development app or live daemon was operated.

## Review

Reviewer verdict for the entire working-tree diff of changes 1 and 2: **NO REMAINING MUST-FIX ISSUES** (Runner message `01M5MK06T6KD5RA1J1BKB6WNXM`). Reviewer independently reran the 7 remote Settings tests and whitespace check, both exit 0; evidence: `target/836-review-changes1-2-tests.log` and patch SHA-256 `44e92c46486d5fcde4065f76d6638145c8d92b5bd50b64613afbb6e0aca6e7fc`. This was partial clearance; the complete four-change review later found the two P2 issues resolved below. This review preceded source commits and PR creation. The brief folds into the first change, and each change retains its own commit with its spec/guide updates.

## Whole-diff review fixes

Reviewer message `01M5MRD5PK24JBESSBEBPV4W4W` found two P2 issues. Explicit Reset on a legacy/CLI-signed-in computer with an off stored preference fell back to the off explanation; Reset now saves the on preference before resetting so the result remains relay sign-in. Effective off also left remote Settings PIN and Reset modals open in other windows; these dialogs and their flags are now dismissed, and focus returns to the root. Both fixes stay in change 3. The reviewer found no other must-fix issue and independently passed 13 related tests (`target/836-review-all4-v1-tests.log`, exit 0).

`reset_keeps_remote_access_on_for_legacy_or_cli_signed_in_settings` covers false preference plus signed-in status through confirmed Reset. `turning_remote_access_off_dismisses_other_windows_pin_and_reset_dialogs` covers Set PIN, Change PIN and Reset in a second window, then confirmed switch-off in the first; the old dialog disappears, its flags/focus clear, and Enter cannot issue SetPin or Reset. The first two-test attempt (`target/836-review-fix-tests.log`, exit 100) placed the first window on Chat instead of Remote access, so Sign out confirmation could not render; the fixture was corrected to open Settings in both windows. Subsequent results follow.

| Check | Log | Exit / result |
| --- | --- | --- |
| Two fake-driven review-fix regressions | `target/836-review-fix-tests-v2.log` | 0; 2 passed. |
| Workspace nextest after review fixes | `target/836-review-fix-workspace-tests.log` | 0; 2774 passed, 6 skipped. |
| Workspace Clippy after review fixes | `target/836-review-fix-clippy.log` | 0. |
| Workspace Clippy with updater after review fixes | `target/836-review-fix-clippy-updater.log` | 0. |
| `cargo fmt --all --check` | `target/836-review-fix-fmt.log` | 0. |
| `git diff --check` | `target/836-review-fix-diff-check.log` | 0. |

## Final working-tree review

Reviewer verdict: **NO REMAINING MUST-FIX ISSUES** for all four changes, Runner message `01M5MRPEGKAN165MDX1G8XFK7D`. Independent rerun: 15 remote Settings/store/picker tests passed, command exit 0, `target/836-review-all4-v2-tests.log`; whitespace check exit 0. Reviewed candidate patch: `target/836-review-all4-v2.patch`, SHA-256 `242a0c3dffe01757d1826cff9985cbf9065c2264ffc0e2006ab402799b787be9`. The post-fix workspace gate completed with command exit 0: 2774 passed, 6 skipped. Live checks remain with Jason.

## Commit checks

After clean review, each change was restored on its own preceding commit state and its regression check rerun before committing. The initial brief commit was folded into change 1. All 18 final tracked file hashes matched the reviewed candidate exactly after reconstruction.

| Commit change | Log | Exit / result |
| --- | --- | --- |
| 1. Account; status rendering in both locales and themes | `target/836-commit-change1-test.log` | 0; 1 passed. |
| 2. Relay entry; remote Settings tests | `target/836-commit-change2-test.log` | 0; 7 passed. |
| 4. Unpaired picker status; fake directory regression | `target/836-commit-change4-test.log` | 0; 1 passed. |
| 3. Switch; remote Settings/store/picker regression group | `target/836-commit-change3-tests.log` | 0; 15 passed. |

## Current umbrella integration

`origin/nightly-remote` advanced to `a7750819` with m8 while review was in progress. The four authorized commits rebased without conflicts. `git range-diff` reported every patch unchanged, and all 15 app source file hashes still match the reviewed candidate. The shared spec and guide retain both missions' changes. Required gates were rerun on this base; no new implementation change was needed.

| Gate | Log | Exit / result |
| --- | --- | --- |
| Workspace nextest | `target/836-rebased-workspace-tests.log` | 0; 2789 passed, 6 skipped. |
| Workspace Clippy | `target/836-rebased-clippy.log` | 0. |
| Workspace Clippy with updater | `target/836-rebased-clippy-updater.log` | 0. |
| `cargo fmt --all --check` | `target/836-rebased-fmt.log` | 0. |
| `git diff --check origin/nightly-remote` | `target/836-rebased-diff-check.log` | 0. |

## PR review: plain Sign out on a legacy preference

Claude Code found a P2 in PR #1014 (Runner human message `01M5MTMBC64XF7GGNWCYVXM34D`): Account/avatar Sign out left an absent/off saved preference unchanged, so a legacy or CLI-signed-in install unexpectedly hid remote surfaces when its signed-in override ended. Confirm now saves `!disable_access`: plain Sign out and Reset preserve the on preference; the switch's confirmed disable saves off.

The existing signed-in switch regression now seeds a false preference with a fake signed-in status, confirms Sign out from Account, then checks saved settings on disk, signed-out Account, Account navigation, the other window's picker/avatar and the on Remote access pane. Its red run failed at the saved-preference assertion before the fix (`target/836-pr-review-signout-red.log`, exit 100). After the fix the 15 related remote Settings/store/picker tests passed (`target/836-pr-review-signout-tests.log`, exit 0). A disk-persistence assertion was then added before the final workspace gate. The spec and guide describe the same behavior. No live checks ran.

| Gate after the PR review fix | Log | Exit / result |
| --- | --- | --- |
| Workspace nextest, including the disk assertion | `target/836-pr-review-signout-workspace.log` | 0; 2789 passed, 6 skipped. |
| Workspace Clippy | `target/836-pr-review-signout-clippy.log` | 0. |
| Workspace Clippy with updater | `target/836-pr-review-signout-clippy-updater.log` | 0. |
| `cargo fmt --all --check` | `target/836-pr-review-signout-fmt.log` | 0. |
| `git diff --check` | `target/836-pr-review-signout-diff-check.log` | 0. |

Original-head CI run `39987060189` passed macOS and Windows while this fix was reviewed. The old watch was intentionally stopped (exit 143) to work on the fix. Updated-head CI will be reported in PR #1014 and the Runner handoff after review and push.

The reviewer cleared this four-file working-tree fix with **NO REMAINING MUST-FIX ISSUES** (Runner message `01M5MV58Q5W8KN9REKJNPY1T32`). Independent Sign out/Reset/dialog regression run: 3 passed, exit 0, `target/836-review-signout-final-tests.log`; whitespace check exit 0. The reviewed patch SHA-256 is `4afe4a47084ade89dc7b313a64bfa4dd9dc7dc3d75a09f9c993308680db7c6b5`, verified unchanged before adding this review result and folding into the switch commit. Live checks remain skipped under the brief.

## Jason's live rechecks

On both macOS and Windows:

1. Signed-in Account contains the account/service/computer/fingerprint status card and Sign out only. Check English and Chinese layout.
2. With Remote access enabled, first visit and Reset show relay sign-in directly, with no route chooser or Tailscale card. Reset still requires confirmation and clears the existing identity, PIN and pairings as before.
3. On a fresh settings file, Remote access defaults off and remains in Settings. Only its switch row and explanation appear; Account is absent, every window omits the computer picker, and the footer gear opens Settings directly.
4. Turning it on restores Account, the picker and avatar/person footer without starting sign-in. Confirm the choice survives restart and propagates across windows.
5. An already-signed-in install, including one signed in with `runner remote login`, keeps its remote surfaces visible. Turning off while signed in shows Sign out confirmation; cancel keeps them, confirm signs out and hides them while retaining identity, PIN and paired computers. A failed sign-out keeps remote surfaces available and shows its error. On a legacy/CLI sign-in with a false saved preference, plain Sign out from Account or the avatar menu keeps the switch on, Account listed and the picker visible.

6. With Remote access on, check online and offline unpaired rows in the picker: status comes first, offline rows keep last-seen information, and Not paired follows it. The window button and ⋯ remain in place.

7. With an existing/CLI sign-in and no saved on preference, confirm Reset returns to relay sign-in with the switch on. With Set PIN, Change PIN or Reset open in a second window, confirm switching off and signing out in the first dismisses the second window’s dialog; Enter must have no PIN/Reset effect.

These are pending human checks, not a claimed live or full regression pass.

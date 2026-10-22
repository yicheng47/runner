# #836 m6 — host desktop validation

Candidate: `feat/836-m6-host-desktop`, based on `origin/nightly-remote` at `053a2907`. macOS automated checks only. The app was not launched; neither production nor development sessions were operated. No new crate, database migration, daemon or client-protocol change.

## Changed behaviors and selected checks

| Behavior | Check and reason | Result |
| --- | --- | --- |
| Ordered terminal control | `ordered_controller_gates_every_shared_size_and_input_path` and `resync_keeps_the_last_name_while_a_fresh_unowned_snapshot_has_none`: snapshot, ordered and stale frames, metadata, resync and the unknown-name fallback | Pass |
| Desktop size pushes | Recording transport verifies consecutive ordinary immediate pushes and no shared resize call while remote-controlled or unowned; split-tab rendering checks pushes resume as soon as Desktop is acknowledged | Pass |
| Covered input | Shared mirror gate exercises keys, text, raw mouse bytes, paste/file-drop text, composer submission, IME composition/commit and input reset; no terminal bytes or input observations leave. The mission integration check also verifies feed-composer submission preserves its draft without posting to covered recipients | Pass |
| IME handover cleanup | `composition_started_before_takeover_clears_authoritative_drafting_on_desktop_ack` starts real InputTracker composition before takeover, cancels locally while Browser/None own the terminal, rejects stale Desktop frames, and verifies ordered resize and resync acknowledgments clear authoritative composing/Drafting without a local commit; new local preedit survives later Desktop resizes | Pass |
| Pane and group cards | Controller state tests, actual split-tab and mission surface transitions, group-to-pane fallback, unknown name, below-tab mission overlay and card layout in English/Chinese and dark/light themes | Pass |
| Take back / Continue here | Pointer and Enter actions both send each terminal’s measured pane size; hidden mission slots use slot geometry, closed drawers retain their current grid, and Desktop acknowledgment restores ordinary pushes | Pass |
| Pairing notification | `pairing_notification_and_toast_open_remote_access_once_per_event` checks the system notification, repeat pairing, click destination and denied-permission toast action; `pairing_copy_is_localized_in_both_languages` checks English and Chinese | Pass |
| Connected now | `trust_rows_dialogs_and_lock_toast_render_in_both_languages_and_themes` renders the connected row, toggles connected off and checks the dot disappears, across both languages/themes | Pass |

## Automated gates

Each final command writes a separate log under the ignored `target/m6-validation/`; exit codes are captured from that command directly.

| Command | Exit | Log |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 — 2,651 passed, 6 skipped | `target/m6-validation/review-ime-nextest.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `target/m6-validation/review-ime-clippy-2.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | `target/m6-validation/review-ime-clippy-updater-2.log` |
| `cargo fmt --all --check` | 0 | `target/m6-validation/review-ime-fmt.log` |
| `git diff --check` | 0 | `target/m6-validation/review-ime-diff-final.log` |

Development iterations exposed incorrect overlay placement/types and unused imports (Clippy exit 101), keyboard and notification test setup omissions (nextest exit 100), and an incorrect test notification-response field (compile exit 101). The layout test initially omitted the card border and assumed every Chinese sentence wraps; the mission fixture initially omitted the terminal-wake event. Corrected the setup and layout expectations, retaining the failures in `focused-failed-1.log`, `focused-2.log`, `focused-4.log`, `focused-6.log` and `nextest-2.log` under `target/m6-validation/`. The first full run passed 2,645 tests (exit 0); the next run caught the layout-test assumption (2,647 passed, 1 failed; exit 100). The last focused run passed all 8 control/split/mission checks (exit 0, `focused-7.log`). The integration fixtures use GPUI’s simulated windows and an in-memory recording transport; no new test sleeps or uses a network. A later full run passed all feature checks but failed `runner-cli::golden built_cli_goldens::project_missing` while starting its isolated daemon (`runnerd is not running`; 2,649 passed, 1 failed; exit 100, `nextest-final.log`). That unchanged CLI test passed alone (exit 0, `golden-retry.log`), then the complete final run passed all 2,650 tests with 6 existing skips (exit 0, `nextest-final-2.log`).

## Commit-boundary validation

The first commit, `feat(ui): add host terminal control cards`, folds in the existing mission brief and plan. It was tested with the pairing-notification and Connected now source changes temporarily absent, so the check covers the actual first commit rather than the combined tree: workspace nextest passes 2,649 tests with 6 existing skips (exit 0, `target/m6-validation/commit1-nextest.log`), workspace Clippy passes (exit 0, `commit1-clippy.log`), formatting passes (exit 0, `commit1-fmt.log`), and whitespace passes (exit 0, `commit1-diff.log`).

The second commit, `feat(ui): notify pairing and show connected computers`, restores the full source candidate byte-for-byte against the saved clean-review hashes. Its combined source is the candidate covered by the 2,651-test run and both Clippy variants above. The third commit documents the behavior, spec choices, validation and pending live checks; it changes no executable source.

## Spec details selected

- Controller and size remain ordered with terminal output; metadata cannot advance them. The mirror remembers the last remote name through resync, with Another computer as the fresh unowned fallback.
- One card groups all live terminals only when their remote name and controller state match. Mixed or partially controlled groups keep individual pane cards; a one-terminal mission still receives a mission card.
- Hidden mission slots use the slot pane’s measured geometry. Shells in an open drawer use that drawer’s geometry. A closed drawer has no pane, so Take back retains its grid until the drawer opens and ordinary fitting resumes. Mission-composer submissions to covered recipients, including broadcasts, keep their draft and do not post.
- A preedit cancelled while covered queues its authoritative cleanup until Desktop acknowledgment, then clears the daemon’s composing hold without sending input while covered.
- Unknown notification permission uses the toast fallback. Only a live pairing event raises a notification; baseline trust loads and reconnections do not replay one, while a new pairing after unknownDevice does.

## Live checks pending for Jason

These checks are pending, not blocked. None ran in this mission, as authorized in the brief.

- TERM-RESIZE-01 and TERM-RESIZE-02 on macOS and Windows: the pane’s size push changed, so verify unchanged desktop geometry and first paint.
- #838’s two-window steps on macOS and Windows: confirm desktop window ownership and terminal fragments after resizing, switching and taking back from either window.
- Once m7 can connect another Runner: pane card, Take back, Continue here, whole split-tab and mission card, per-terminal size restoration, system notification, toast fallback, and Connected now after pairing, reconnection and disconnect.

## Review

The initial working-tree review found one must-fix: local IME cancellation was suppressed along with covered input, leaving the daemon’s composing flag and Drafting delivery hold set after Take back. The regression reproduces the bug before the fix (exit 100, `target/m6-validation/review-ime-before.log`). The mirror now retains the pending cancellation without sending it while covered, then flushes it only after a snapshot or ordered resize acknowledges Desktop. The focused control suite passes after the fix (8 tests, exit 0, `target/m6-validation/review-ime-focused.log`). The regression checks both explicit cancellation and a blocked marked-text update during remote control, authoritative Idle after resize/resync Desktop acknowledgment, and preservation of a new local preedit on later Desktop resizes. The full workspace rerun passes after the fix (2,651 tests passed, 6 existing skips; exit 0, `target/m6-validation/review-ime-nextest.log`); both Clippy variants, formatting and whitespace gates also exit 0. The reviewer rechecked the complete working-tree diff on 2026-10-22 and returned `NO REMAINING MUST-FIX ISSUES`. All 11 selected control, split-tab, mission and pairing tests passed in the reviewer’s independent run (exit 0, `target/m6-validation/reviewer-recheck.log`); the reviewer also verified authoritative IME cleanup and whitespace (exit 0). This is a clean headless feature review; the live checks above remain pending for Jason.

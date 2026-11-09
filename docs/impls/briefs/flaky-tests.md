# Fix the flaky tests and retry build-time downloads

Jason requested this mission on 2026-11-09. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-flaky-tests` on `fix/flaky-tests`, based on `origin/main` at `709a39e24c4ca3eb01574a0f4c1d3d171eda085a`. The root checkout stays on `main`.

## Problem

CI keeps failing on tests unrelated to the change under test, and people re-run until it passes. A survey of 300 CI runs since 2026-10-30, including first attempts later hidden by re-runs, found the tests below. Separately, 2 nightlies on 2026-11-09 failed because a GitHub release download returned 504 and the download does not retry.

Outcome: each open flake below has its cause fixed, or a written reason it could not be established, and build-time downloads survive a transient 5xx.

## Open flakes: fix these

**`copy_on_select_copies_a_finished_drag_only_when_on`** in `crates/runner-app/src/terminal/element.rs`. 4 failures on macOS and Windows, latest 2026-11-09 (runs 42449379593, 42363602713 attempt 1, 42557247687 attempt 1), 3 of them after #1197. It panics on `left_terminal.selection_text().unwrap()` straight after the first drag, so the selection is `None`. #1197 waited for the clipboard text and did not address this step. Cause unknown. One lead to test, not a conclusion: the pane's size push in prepaint (`session.resize`) may reach the terminal after the mouse-down, and a column change can drop a selection.

**`a_chat_in_a_project_opens_its_checkout_with_git_status`** in `crates/runner-app/src/surfaces/files/tests.rs`. 1 failure on Linux, 2026-11-08 (run 42471355639 attempt 1), at `assert!(h.has("FILES_LETTER src/new.rs"))`. The test writes `a.txt` then `src/new.rs`, but waits only for `FILES_LETTER a.txt`, so a status taken between the writes appears to satisfy the wait. Check the other tests in this file for the same shape of wait.

**`an_authenticated_hello_records_the_kind_its_client_names_and_saves_it`** in `crates/runner-daemon/src/remote/channels/tests.rs`. 5 failures, Windows only, latest 2026-11-04 (run 41771761302; 41768114079 attempts 1 and 2; 41619546150 and 41598444694 attempt 1). `saved_kind` panics with `the saved trust never recorded <kind>`, with Phone, Computer and Unknown all seen, at 3.5 to 3.95s. Unconfirmed hypothesis: `saved_kind` polls `Trust::load` every 10ms, the open read blocks the save's file replace on Windows, and the failed save in `remote/channels.rs` is only logged as `could not save last seen`, so the newest value is never written. `TrustWrites::persist` keeps generation order, so an out-of-order save is not the explanation. Establish the cause first. If saves really can be lost to a concurrent reader, that is a product bug: fix it minimally and say so in the PR, rather than only quieting the test.

**`heartbeat_write_failures_release_local_access_removal_commands`** in `crates/runner-daemon/src/remote/tests.rs`. 2 failures, Windows only, latest 2026-11-04 (run 41837647321; 41316854021 attempt 1), `pause: Elapsed(())` from the 3s `tokio::time::timeout` that follows the `tokio::time::pause` / `advance` / `resume` sequence.

## One-off failures: investigate, fix only with a concrete cause

- `mismatch_restart_waits_for_lock_and_installs_new_sidecar`, `crates/runner-cli/tests/daemon_process.rs`: Windows on `main`, run 42245189205, `the old runnerd was moved aside instead of awaited`.
- `built_cli_goldens::mission_remove_role_lead`: Windows on `main`, run 41837647321, `golden daemon: runnerd is not running`, 28s, same run as the heartbeat failure.
- `persistent_hook_reporter_delivers_to_real_isolated_daemon`: Windows, run 42239613387 attempt 1.
- `companion::tests::exit_archive_and_status_recovery_drain_a_held_frame_before_closing` and `cat_echo_and_da1_reply_once_with_zero_and_two_mirrors`: macOS, run 41089832620.
- `hook::tests::total_deadline_bounds_handshake_write_and_ack_without_retry` and `native_command_reporters_preserve_utf8_and_provider_neutral_results`: macOS, run 41882905496.
- `early_codex_resume_exit_keeps_the_conversation_for_the_next_resume` (Windows, 41208419616), `folder_chip_offers_projects_and_folders_on_the_default_page_and_folders_in_a_split` (Linux, 41181457187), `oversized_request_fails_without_disconnecting_or_queueing` (macOS, 41081159978).

Fetch logs with `gh run view <run> --log-failed`, adding `--attempt <n>` for an earlier attempt. These failed once each. Read the test and the failure; if there is no concrete race to point at, leave the test alone and list it in the PR as not reproduced.

Believed already fixed, no action unless evidence says otherwise: `managed_app_pauses_instead_of_replacing_another_apps_daemon` (#1162), `a_status_that_arrives_clears_the_error_of_one_that_failed` (#1218), `a_nested_repository_is_no_changed_file` (`95eff7ff`), `synthetic_wake_does_not_overwrite_a_newer_forwarder_transition` (#1180).

## Build-time download retries

Add bounded retries covering transient 5xx to these downloads, which currently fail on the first error: the Sparkle downloads in `script/bundle-mac`, `.github/workflows/nightly.yml` and `.github/workflows/release.yml`; and `Invoke-WebRequest` in `script/windows/inno-setup.ps1`, `script/windows/conpty.ps1` and `script/windows/simplysign.ps1`. Match the nightly verify step's existing `public_curl` retry (`--retry 5 --retry-delay 10`). The pinned checksums stay. Keep this a separate commit from the test fixes.

## Boundaries

Do not mark a test ignored, add nextest retries, or raise a timeout unless the cause really is that budget, and then say why in the commit. Prefer waiting on the condition the test needs over sleeping. Assertions keep their strength: a fix makes the test wait correctly, it does not loosen what it checks. No unrelated refactors.

## Validation

For each test changed, name the race and show how the fix closes it. Repeat every changed test many times with the CI profile, for example with nextest's stress options or a shell loop, under parallel load where the race needs it, and report the counts. Run the affected crates' tests and workspace clippy. macOS is the local platform; for the Windows-only failures, the PR's Windows CI job is the proof. A temporary stress commit on this branch is allowed for that, and must be gone before the final review. Do not launch the desktop app. No live checks are expected, since this changes tests and build scripts; if a product fix changes what a person sees, the reviewer says so.

## Authorization and handoff

Jason approved the claude duo and authorized commits, pushing this branch and opening one PR against `main`. The PR description lists each test with its cause and fix, or the reason it was left alone, plus stress counts.

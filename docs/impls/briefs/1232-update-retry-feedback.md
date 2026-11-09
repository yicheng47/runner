# #1232 — Retry on a failed update check shows the check, and the cause is logged

Jason asked for a mission to fix [#1232](https://github.com/yicheng47/runner/issues/1232) on 2026-11-09; this is the default claude trio. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1232-update-retry-feedback` on `fix/1232-update-retry-feedback`, based on `origin/main` at `b2c15e38`.

**The worktree holds an uncommitted, unvalidated draft of deliverables 1 to 3**, written by the operator before this became a mission. Its test run was stopped, so it may not compile or pass. It is your starting point and yours to change or discard. `git diff` shows it.

## Problem

Three separate problems; the issue has the full trace.

1. **Retry gives no feedback.** `SparkleEvent::CheckStarted` moves the state to `UpToDate { checking: true }` only from `UpToDate`. From `Failed` it changes nothing, so the red error stays for the whole check, and a repeat failure with the same message leaves the state identical, so `apply_event` returns false and nothing repaints. The Windows and Linux flows have the same `matches!(state, UpToDate)` condition in `check_for_updates`.
2. **The error is discarded.** `show_updater_error` forwards only `localizedDescription`, and nothing is logged, so `runner.log` has no record of why a check failed.
3. **One 504 fails a nightly after it has published.** The public verification step's `curl --fail` calls do not retry, so a transient 504 skips pruning and the completion report.

## Outcome

- Pressing Retry after a failed check shows the existing checking state until the check finishes, then the found update, up to date, or the error again, on macOS, Windows and Linux.
- A failed check logs Sparkle's error domain, code, description and underlying errors, without logging signed release-asset URLs.
- A transient 5xx on a nightly's public verification is retried before the run fails.

## Read first

- `crates/runner-app/src/updater.rs`: `UpdateState`, `request_retry`, `manual_check`, `preview_check`, `parse_dev_preview`.
- `crates/runner-app/src/updater/sparkle_flow.rs`: `apply_event`, `event_for_abort`, and its tests.
- `crates/runner-app/src/updater/sparkle.rs`: `show_updater_error`, `did_abort_with_error`.
- `crates/runner-app/src/updater/windows.rs`, `linux.rs`: `check_for_updates`, `finish_*_check`.
- `crates/runner-app/src/surfaces/settings/updates.rs`: `hero_content`; the checking state already exists, so no new UI is needed.
- `.github/workflows/nightly.yml`: `Verify the public nightly release and downloads before pruning`; `script/test-nightly.py` parses this workflow.

## Deliverables

1. **Shared start of a check.** One `UpdateState` method moves `UpToDate` and `Failed { step: Check }` to `UpToDate { checking: true }`. `CheckStarted`, Windows, Linux and `preview_check` all use it, so it is not dead code on a macOS build without the `updater` feature. A failed download, verify or install stays on screen. A failed Windows or Linux check must keep its `info` through the candidate, as it does today.
2. **Log the failure.** Log once per failed check, in `did_abort_with_error`, at `warn`. Sparkle 2.9.5 calls that for every abort, background checks included, after `showUpdaterError`'s acknowledgement. It also calls it for `SUNoUpdateError` (1001) and a cancel, which must not log, nor must Runner's own declined background check. Walk `NSUnderlyingErrorKey` for the underlying errors. Never log `userInfo`, because `NSErrorFailingURLKey` holds the redirect's signed URL, and cut any URL in a description to its host.
3. **Retry the verification.** The step's six `curl` calls retry transient failures (`--retry 5 --retry-delay 10`), through one shell function.
4. **A previewable failed check.** Add a debug-only `RUNNER_DEV_UPDATE_AVAILABLE=<version>:failed-check` state to `parse_dev_preview`, `Failed { step: Check }`, so qa can exercise Retry in the development app. Today's `failed` preview is a download failure, whose Retry takes the download path. Not in the draft.

**Non-goals:** no automatic in-app retries of a failed feed fetch, no change to where the feed is hosted, no new UI, and no change to other `curl` calls or to `release.yml`.

No guide change is expected; check `docs/guide/` for the Updates page and update it only if it describes this differently.

## Tests

- `apply_event`: from a failed check, `CheckStarted` shows checking and reports a change; the same error again restores it and reports a change; found and not found settle. A failed download stays through `CheckStarted`. A retained found update survives a retry that ends without a result.
- Which aborts log, and a logged entry that keeps the host and drops a signed URL.
- The preview: `failed-check` parses, Retry shows checking and returns to the error. Extend `dev_preview_reaches_every_state_and_ignores_unknown_ones`.
- The regression tests for deliverables 1 and 2 must fail without the fix; say which in the handoff.
- Gates: `runner-app` tests, workspace Clippy with warnings denied, **Clippy with `--features updater` on `runner-app`, since `sparkle.rs` compiles only with it**, `cargo fmt --all --check`, and `python3 script/test-nightly.py`. Drive macOS, Windows and Linux CI green. Windows and Linux are CI only.

## Live checks (qa)

Use the macOS development app from this worktree. No real agents are needed. Take the QA lock as AGENTS.md describes. Launch with `RUNNER_DEV_UPDATE_AVAILABLE` set only in the child's environment.

- **L1, Retry shows the check:** with `0.19.5:failed-check`, open Settings → Updates: the red error shows with **Retry**. Press Retry: the status becomes "Checking for updates…" with a busy button, then the error returns. Press it again and it repeats. Reason: the reported symptom, through the real view and repaint.
- **L2, other failures keep their path:** with `0.19.5:failed`, Retry goes to Downloading as it does today, and shows no checking state. Reason: `preview_check` and the shared method changed. Relates to `SET-UPDATE-06`.

Preview does not exercise Sparkle, whose `CheckStarted` path the `apply_event` tests cover.

**P1, packaged build (not for qa):** real Sparkle flows need an authorized install, so qa does not run this. On a nightly containing this change, Jason turns the network off, presses Retry on the failed check, sees Checking and then the error, and finds the `Sparkle update failed` line in `runner.log`. It is not a merge gate.

The verdict gives pass or fail per check with screenshot evidence, and the PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1 and L2. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR uses `Fixes #1232`, names P1 for Jason, and carries `risk-medium`, `frontend-only` and `needs-live-test` until qa's checks pass. No co-author trailers, per AGENTS.md.

## Authorization

The claude trio may commit, push this branch and open one PR against `main`. It does not merge.

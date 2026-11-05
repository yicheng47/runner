# #1167 — Flaky synthetic wake vs. newer forwarder transition test

Jason approved a claude duo mission on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1167-wake-transition-flake` on `fix/1167-wake-transition-flake`, based on `origin/main` at `b7be3b5a`. Issue: [#1167](https://github.com/yicheng47/runner/issues/1167), P3.

## Problem

`runner-daemon session::manager::tests::wake::synthetic_wake_does_not_overwrite_a_newer_forwarder_transition` failed once on Linux aarch64 ([run 41802250411 attempt 1](https://github.com/yicheng47/runner/actions/runs/41802250411/attempts/1), #1162's head `bb9709cc`, which touches no session code): final activity was `Busy`, expected `Idle` (`wake.rs:534`).

The test orders its threads with `thread::sleep(1 ms)` before dropping the session guard and `thread::sleep(2 ms)` before starting the forwarder's Busy→Idle. It assumes the wake thread has by then read its baseline revision under the session lock and parked in `try_append_with_retry`. Lead, unconfirmed: on a slow runner the wake has not taken the lock yet, so the transitions land first, the wake reads the new Idle revision as its baseline, and its `Delivered { revision }` then legitimately sets Busy. `SessionModel::apply` already ignores a `Delivered` whose revision is stale (`session/state/mod.rs`, `SessionEvent::Delivered`), so the product guard looks sound; confirm rather than assume, including whether any window exists between reading the baseline and the append.

## Intended outcome

- If the race is only in the test, make the test wait on the state it needs (the wake has read its baseline and is parked on the event-log append) instead of sleeping, so it is deterministic on every platform. A test-only hook is acceptable if it stays small and `#[cfg(test)]`.
- If there is a real window in `synthesize_wake_busy`'s newer-transition guard, fix the product and land a deterministic test that fails without the fix, as #1160 did.
- Say in the PR which it was and the evidence. Check the sibling tests in `wake.rs` that use the same sleep-based ordering (e.g. the `record_output` test above it) and fix them the same way only if they share the flaw.

## Read first

- `AGENTS.md`, including Test Scope and Crew Missions.
- `crates/runner-daemon/src/session/manager/tests/wake.rs`: the test, `hold_event_log_lock`, `manager_with_contended_wake_sink`, `assert_wake_completed_or_gave_up`.
- `crates/runner-daemon/src/session/manager/status.rs`: `synthesize_wake_busy`, `note_forwarder_transition`.
- `crates/runner-daemon/src/session/state/mod.rs`: revision tracking and `Delivered`.
- The event-log sink's `try_append_with_retry` and its retry budget.

## Deliverables and boundaries

- Reproduce or narrow the window first (loop the test under load, or widen the window in a scratch change) to confirm the cause before fixing.
- Do not paper over it with longer sleeps, retries or `#[ignore]`.
- Non-goals: other flaky tests, wake behaviour changes beyond a real guard fix, nightly or release changes.

## Validation and handoffs

Name the changed behaviours and the check covering each before running anything. Show the test looping clean many times locally under load with this worktree's own target directory; run `runner-daemon` tests with `--locked`, workspace Clippy with warnings denied, `cargo fmt --all --check` and `git diff --check`. Drive required macOS and Windows CI green, and Linux too since it reproduced there.

@coder investigates, implements and hands the diff to @reviewer. @reviewer reviews code only; a test-only fix needs no live checks, and a product guard fix in the daemon is covered by in-process tests, so live checks are not expected unless the review finds otherwise.

## Authorization

Jason approved this claude duo mission: commit, push this branch and open one PR against `main`, labelled per AGENTS.md. Stop at the open PR with green CI and report through Runner with the PR URL, final SHA, cause, changes and validation.

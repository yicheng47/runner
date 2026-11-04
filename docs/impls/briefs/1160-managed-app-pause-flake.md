# #1160 — An app meeting another build's daemon always pauses

Jason approved a claude duo mission on 2026-11-04. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1160-managed-app-pause-flake` on `fix/1160-managed-app-pause-flake`, based on `origin/main` at `ef7c2950`. Issue: [#1160](https://github.com/yicheng47/runner/issues/1160), P2. The 0.19.3 release is being cut from `main` in parallel; this mission does not touch it.

## Problem

`runner-cli::daemon_process managed_app_pauses_instead_of_replacing_another_apps_daemon` fails intermittently: 3 of 63 completed CI runs since it landed on 2026-11-02 (`f4615cef`), two of them on `main` today after #1153 (`a1221ecb`) touched the test. The same code passes on its pull request.

- Linux aarch64, [run 41773839709](https://github.com/yicheng47/runner/actions/runs/41773839709): the app emitted `daemon/reconnected` where it should have paused (`assert_ne!` at `daemon_process.rs:1050`).
- Windows, [run 41777740537](https://github.com/yicheng47/runner/actions/runs/41777740537): no in-use pause within the test's 20 s (`Elapsed` at `daemon_process.rs:1057`).
- Linux aarch64, [run 41395370541](https://github.com/yicheng47/runner/actions/runs/41395370541) on `fix/1116-daemon-takeover-guard`: same test.

The Linux failure suggests a real race, not just a slow runner. One lead, unconfirmed: `LaunchConfig::connect_or_spawn_with_restart` (`runner-core/src/daemon_process.rs`) takes over a mismatched daemon when its welcome reports `app_clients == 0`. If the other build's app has connected but the daemon has not yet counted it when this app's hello arrives, this app replaces the other app's daemon and reconnects. Check where `app_clients` is counted relative to the welcome in `runner-daemon/src/daemon/server.rs`, and what the test's lock hand-off (`startup_lock`, `wait_unlocked`, `drop(starter)`) does or does not order.

## Intended outcome

Find the cause and fix it where it lives:

- If the product can replace a daemon another app is using, fix that race so an app meeting another build's daemon always pauses with the in-use notice and never reconnects to or replaces it. The fix and a deterministic test that would have caught it land together.
- If the race is only in the test, make the test wait on the state it needs instead of timing, so it passes deterministically on every platform.

Either way, say in the PR which it was and the evidence. Look at `remote::channels::tests::an_authenticated_hello_records_the_kind_its_client_names_and_saves_it` (one Windows failure, [run 41771761302](https://github.com/yicheng47/runner/actions/runs/41771761302)) only if the cause is shared or obvious; otherwise leave it in #1160.

## Read first

- `AGENTS.md`, including Test Scope and Crew Missions.
- `crates/runner-cli/tests/daemon_process.rs`: `managed_app_pauses_instead_of_replacing_another_apps_daemon` and its helpers (`other_build`, `connect_app`).
- `crates/runner-core/src/daemon_process.rs`: `connect_or_spawn_with_restart`, `take_over`, `IN_USE_GRACE`, `startup_lock`.
- `crates/runner-core/src/protocol/managed.rs`: the reconnect loop that emits `daemon/reconnected` or `daemon/disconnected` with `in_use`.
- `crates/runner-daemon/src/daemon/server.rs`: app client counting, the welcome's `app_clients`, `TAKEOVER_REFUSED`.
- `git show f4615cef` for the original design, and `docs/arch/process-model.md`.

## Deliverables and boundaries

- The root cause, the fix, and a test that fails without it. Reproduce first: loop the test locally (`cargo nextest run --locked -p runner-cli --test daemon_process managed_app_pauses_instead_of_replacing_another_apps_daemon` repeatedly, under load) and, if it helps, narrow the window deliberately in a scratch change to confirm the race before fixing.
- Do not paper over it with longer timeouts, retries or `#[ignore]`.
- Update `docs/arch/process-model.md` only if the takeover or pause contract changes.
- Non-goals: other flaky tests, the takeover feature's UI copy, nightly or release changes.

## Validation and handoffs

Name the changed behaviours and the check that covers each before running anything. Show the test looping clean many times locally with this worktree's own target directory; run the affected `runner-core`, `runner-daemon` and `runner-cli` tests with `--locked`, workspace Clippy with warnings denied, `cargo fmt --all --check` and `git diff --check`. Drive the required macOS and Windows CI checks green; Linux too, since it reproduced there.

@coder investigates, implements and hands the diff to @reviewer. @reviewer reviews code only and decides from the diff whether live checks are needed: a test-only fix needs none; a fix to the takeover or pause path in the app may need one with two development builds. If the clean review names checks, the mission's operator adds the codex `qa` role once the mission settles.

## Authorization

Jason approved this claude duo mission: commit, push this branch and open one PR against `main`, labelled per AGENTS.md. Stop at the open PR with green CI and report through Runner with the PR URL, final SHA, cause, changes and validation.

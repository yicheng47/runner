# 473 — Quit tears sessions down concurrently

Tracking issue: [#473](https://github.com/yicheng47/runner/issues/473). Bug, P2, universal. Baseline `main` at `f654ca9` (2026-09-06), with [#472](https://github.com/yicheng47/runner/issues/472) landed. Companion: #472 was the launch abort that a slow quit sets up; it is fixed and out of scope here.

## What ships

Quit takes about as long as the slowest single session instead of the sum. `stop_running_sessions_on_quit` signals and reaps every running session at the same time. Each session keeps its per-session log lines; one new summary line reports how many sessions were stopped and the total wall time. The same fan-out serves `kill_all_for_mission` and `kill_all_for_runner`, which today run the identical sequential loop. No grace period, signal order, or per-session semantics changes.

## Why it is slow today

- `crates/runner-app/src/bootstrap.rs:251` — `stop_running_sessions_on_quit`: stamps the rows for resume-on-launch, then `for id in ids { core.sessions.kill(&id) }`, one after another.
- `crates/runner-backend/src/session/manager/lifecycle.rs:21` — `SessionManager::kill` blocks on `runtime.stop` and then joins the forwarder thread. Its only manager-wide lock is the membership map, taken briefly in `session_state`; everything else is per-session (`Arc<Mutex<SessionState>>`, the handle's `killer` and `child` mutexes, `PtyRuntime::lookup` at `pty_runtime.rs:971` takes the runtime map lock only to clone the handle). `SessionManager` is already `Sync` and shared with every forwarder thread; nothing forbids several kills at once.
- `crates/runner-backend/src/session/pty_runtime.rs:288` — `stop` → `stop_and_reap_child` (`:442`): SIGHUP, poll up to `HUP_GRACE` (6 s), SIGKILL, `KILL_GRACE`; then `reap_descendants` (`session/process/unix.rs:149`) SIGTERM, `DESCENDANT_TERM_GRACE`, SIGKILL. A claude-code TUI leaves about 2.5 s after SIGHUP, codex and shells in about 20 ms; a session that ignores SIGHUP costs the full 6 s. With ten sessions the sum was about 16 s on 2026-09-03.
- `lifecycle.rs:162` `kill_all_for_mission` and `:248` `kill_all_for_runner` — the same sequential loop and the same failure aggregation, so a mission stop with two claude slots also pays twice.
- The forwarder threads write their exit rows through the r2d2 pool (`db.rs:21`, eight connections, `busy_timeout = 5000`); concurrent exits already happen today when sessions die on their own, so the DB side needs nothing.

## Fix shape

- Add `SessionManager::kill_many(&self, ids: &[String]) -> Result<()>` in `lifecycle.rs`: `std::thread::scope`, one thread per id calling `self.kill(id)`, join them all, collect failures in the ids' order, and return one `Error::msg` in today's shape (`"<id>: <error>; <id>: <error>"`). Empty `ids` returns `Ok(())` without spawning. No thread pool, no channel, no new crate.
- `kill_all_for_mission` and `kill_all_for_runner` keep their id filtering and error prefixes and call `kill_many` for the loop. `stop_running_sessions_on_quit` calls `kill_many` after the resume stamp and keeps its `"failed to stop sessions on quit: …"` message. `kill` itself is untouched.
- Summary line in `stop_running_sessions_on_quit` after the fan-out returns, whatever the outcome: `tracing::info!("quit teardown: stopped {n} sessions in {elapsed:?}")` (the app crate logs through `tracing`; see `crates/runner-app/src/logging.rs:145`). Emit it only when `n > 0` so a quit with nothing running stays quiet. The per-session `exited … after SIGHUP` lines in `pty_runtime.rs` stay exactly as they are.
- Regression test in `crates/runner-backend/src/session/manager/tests.rs`: give `FakeRuntime` a stop barrier (a `std::sync::Barrier` set for the number of sessions, waited on inside `stop`; the existing single-use `stop_gate` at `tests.rs:107` is not enough). Spawn three fake sessions, call `kill_many` from a helper thread, and wait on its completion with `recv_timeout` of a few seconds. Sequential kills deadlock on the barrier — the first `stop` waits forever for the other two — so the test fails by timeout without the fan-out and passes with it. Assert `Ok(())` and that all three ids reached `stops`. Keep `kill_all_for_mission_attempts_every_session_and_aggregates_failures` (`tests.rs:2627`) green; it proves the aggregation still works through `kill_many`. The `stop_running_sessions_on_quit` test in `bootstrap.rs:411` stays green as well.

## Rules of the road

- Do not change `HUP_GRACE`, `KILL_GRACE`, `DESCENDANT_TERM_GRACE`, the signal order, `stop_and_reap_child`, or `reap_descendants`. The fix is overlap, not shorter waits.
- Do not add rayon, tokio, a thread pool, or any new dependency. `std::thread::scope` is enough.
- `kill_many` is platform-agnostic; nothing behind a `cfg`. The Windows `stop` path gets the same fan-out for free.
- Do not launch the Runner app (`make run`); the human smoke-tests. Verify with `cargo test -p runner-backend -p runner-app`, `make clippy`, `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on the feature branch, push, open the PR, and drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: the human merges after their own check.

## Verification

- `cargo test -p runner-backend -p runner-app` green, including the new barrier test. `make clippy` and `make fmt` clean.
- Handoff lists the diff, the checks run, and the confirmation that the barrier test times out against the sequential loop.
- Reviewer checks: `kill` unchanged; failure aggregation identical in shape; the three callers all go through `kill_many`; the summary line is logged on the error path too; no grace constant moved.

## Jason's smoke test (after landing)

1. Open about eight chats (mostly claude-code) plus a running mission, then ⌘Q. Expect: the Dock icon is gone within roughly 3 s. `~/Library/Logs/com.wycstudios.runner/runner.log` shows the `exited … after SIGHUP` lines with overlapping timestamps and one `quit teardown: stopped N sessions in …` line.
2. Relaunch. Expect: the same sessions resume as before; the resume-on-launch stamp still precedes the kills.
3. Stop a running two-slot mission from its header. Expect: it stops in one claude-code teardown, not two.

## Non-goals

Shorter grace periods, changing what SIGHUP or SIGKILL is sent to, the #491 quit-confirmation gate, the reopen path fixed in #472, and any change to how sessions are marked for resume.

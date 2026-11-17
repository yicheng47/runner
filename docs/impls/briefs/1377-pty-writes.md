# #1377 — PTY writes off session locks

Jason approved this mission for the claude duo on 2026-11-17. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1377-remote-host-stall` on `fix/1377-remote-host-stall`, cut from `origin/main` at `4923afd7`. The worktree uses its own `target/`.

This is the first of several fixes for [#1377](https://github.com/yicheng47/runner/issues/1377). Read the issue and the [investigation comment](https://github.com/yicheng47/runner/issues/1377#issuecomment-6946555850) first: it has the incident timeline, the lock inventory with file and line references, and the ranked fixes. This mission is fix 1 of that comment.

## Problem

`PtyRuntime`'s `write_to` (`crates/runner-daemon/src/session/pty_runtime.rs`) does a blocking `write_all` with no deadline. Its callers hold shared locks across it:

- `inject_direct_stdin_with_wait_timeout` (terminal typing, local and remote, via the `native-term-input-*` worker) holds the session's `DeliveryGate.state` and `Mutex<SessionState>`;
- `inject_reserved` (router and report delivery) holds `DeliveryGate.state`;
- `resubmit` (`session/manager/bookkeeping.rs`) holds `DeliveryGate.state` and the terminal's input tracker through `TerminalModel::with_unchanged_input`;
- `inject_paste` holds `Mutex<SessionState>` across the Enter key;
- `inject_stdin` and terminal query replies (`write_reply` on the `native-term-events-*` thread) hold the handle's writer lock.

When an agent stops draining its tty input and about 1 KB queues, a macOS PTY master write blocks indefinitely. Every status snapshot, companion `initialize()`, awaited remote request and app request that touches that session then waits forever, and the stall spreads from there. `forget_runtime_handle` (`session/manager/lifecycle.rs`) and `forget_session_state` (`output.rs`) also join the input worker while holding the global `terminal_control`.

## What to build

1. All PTY input for a session goes through one per-session writer that performs the blocking write itself. No caller holds `Mutex<SessionState>`, `DeliveryGate.state`, the terminal input tracker or `terminal_control` while bytes are being written. Enqueueing under those locks is fine; waiting on the write is not.
2. The writer is bounded. Give its queue a byte limit and the in-flight write a deadline. If the agent does not drain its input in time, the pending and new input for that session fails promptly with a visible error (the existing `input_error` path, or the delivery path's failure handling), instead of blocking. Other sessions and the daemon keep working. Pick the limits and explain them in the PR.
3. Preserve today's semantics:
   - one ordered stream per session, covering typing, paste, delivery, resubmit Enter and terminal replies;
   - generation checks, so bytes for a superseded or stopped session are dropped rather than written to its replacement;
   - the delivery gate's tickets, `in_flight`, `InputQueueDrained` and `ReadinessChanged` events;
   - the status model's input checkpoint and `InputWriteFailed` rollback when a write fails;
   - `capture_codex_session_key` after a submitted Enter;
   - `resubmit`'s rule that Enter is sent only if the input is unchanged.

   Where a caller needs the write's outcome, make the failure reach the same handling asynchronously. Say in the PR which callers changed from synchronous results.
4. Release `terminal_control` before joining the input worker in `forget_runtime_handle` and `forget_session_state`.
5. Keep Windows working. ConPTY uses the same `write_to`, and CI runs both platforms.

Read first: the investigation comment; `session/manager/output.rs` (the inject functions and `write_stdin_bytes`); `session/manager/bookkeeping.rs` (`resubmit`); `session/manager/terminal.rs` (`TerminalHost` for the input and reply workers); `session/pty_runtime.rs` (`write_to`, `SessionHandle`); and `TerminalModel::with_unchanged_input` in `crates/runner-terminal/src/terminal/model.rs`.

Starting material: the investigation left an uncommitted reproduction in this worktree. It is `blocked_pty_stdin_write_reproduces_1377_route_silence_reconnect_hang_and_core_amplification` in `crates/runner-daemon/src/remote/channels/tests.rs`, plus a `#[cfg(test)] is_terminal_control_locked` helper in `session/manager/terminal.rs`. Its fake runtime blocks `send_bytes` for one session. Today it shows the remote route going silent, a reconnect never getting `Welcome`, status snapshots blocking and input to a healthy session blocking. Turn it into the regression test: while one session's write is blocked, `session_status_snapshot`, companion `Ping`, a reconnect's `Welcome`, `mark_direct_sessions_viewed` and input to another session all complete, and the blocked session's input fails within the deadline. Keep or discard the rest as you judge.

Non-goals (later PRs on #1377): a deadline on companion `initialize()` and awaited requests; non-blocking `Lease`, `Stream` and `ConnectionCleanup` drops; narrowing `release_browser_channel`; DB connection and transaction ordering; moving app daemon calls off the main thread; watchdogs and stall logging.

## Validation

Unit tests for the writer: per-session ordering across the caller paths, a stale-generation drop, deadline expiry and queue overflow surfacing an error, and other sessions staying responsive. Add the regression test above. Follow AGENTS.md's rule for tests that cannot race their background work: assert through waits on expected state, with no fixed sleeps. Run each new test in a loop of at least 20 and state the count. Run `cargo test -p runner-daemon` and `cargo test -p runner-terminal` if `model.rs` changes, then `make clippy`, `cargo fmt --all --check` and `git diff --check`.

Live checks: the reviewer decides from the diff. Expected candidates are typing and pasting into a local agent chat, a mission message delivered to an idle member and to a working member, and typing into a host terminal from a paired remote computer. If the clean review names checks, the operator adds the codex `qa` role afterwards. qa records them in `docs/tests/runs/1377-pty-writes-mac.md`. A check that needs a second paired computer may be recorded as Blocked for Jason to run.

## Authorization

Approved crew: claude duo (coder, reviewer). The crew may commit, push this branch and open a PR against `main` that refers to #1377 without closing it. It carries `core-review` and `risk-high`, plus `needs-live-test` if the review names live checks. Fold this brief into the implementation commit.

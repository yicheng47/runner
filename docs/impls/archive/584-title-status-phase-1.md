# 584 — Phase 1: session status from the title spinner

Tracking issue: [#584](https://github.com/yicheng47/runner/issues/584). Spec: [584](../../features/archive/584-title-status-detection.md). Implemented in [#585](https://github.com/yicheng47/runner/pull/585), included in v0.8.9. This is the original mission brief; its commands and proposed later phases are historical. Current follow-ups are [hooks (#347)](../../features/347-hook-based-session-status.md), [shell process detection (#586)](../../features/586-shell-status-detection.md), and [title display (#587)](../../features/587-terminal-provided-titles.md).

**Phase 1 only.** Spinner detection for agent runtimes. Baseline divergence for shells is phase 2 and is explicitly out of scope — do not attempt it, do not add a hook for it.

## What ships

A session whose runtime prefixes its window title with an animation glyph is Busy; when the prefix goes, it is Idle. That classification, not byte arrival, drives status for any session that has ever shown one. Sessions that never do are unaffected and keep the byte detector.

Fixes the measured failure: in the #583 capture Codex drops its spinner at t≈2 s and Runner currently reports Busy for a further 17 s while the composer animation runs at 6.6 Hz.

## Where the code is

The whole path exists; nothing here is new architecture.

- `crates/runner-terminal/src/terminal.rs:381` — `Event::Title(new_title)` in the `native-term-events-{session_id}` thread, which today only stores into the `Arc<Mutex<String>>` behind `title()` (`:407`, currently **zero callers**). Classification goes here.
- `crates/runner-terminal/src/terminal.rs:329` and `:514` — `core.sessions.report_input_state(&session_id, …)` is the precedent for a terminal→backend signal. Copy its shape.
- `crates/runner-backend/src/session/manager/mod.rs:946` `report_input_state` — the backend end of that precedent.
- `crates/runner-backend/src/session/manager/mod.rs:1102` `note_forwarder_transition(session_id, state, source)` — the sink. It already dedupes (`:1119`) and already special-cases one source (`:1110`). `source` values in use: `forwarder`, `agent`, `input-submit`.
- `crates/runner-backend/src/session/manager/output.rs:159` — the consumer that turns a `RuntimeOutput::StatusTransition` into a mission `runner_status` append or, for direct chats, `publish_direct_activity`. **Reuse this; do not duplicate its bookkeeping.**
- `crates/runner-backend/src/session/pty_runtime.rs:674` `IdleDetector`, `:785` where its `Arc<Mutex<…>>` and the transition `tx` are created, `:823` the reader loop, `:92` `SessionHandle` (the runtime's per-session record, reached by `lookup`).
- `crates/runner-backend/src/session/runtime.rs:234` the `SessionRuntime` trait. `has_foreground_process` (`:266`) has a default body — that is the accepted way to extend it without touching every impl.

## Shape

The sender half of the status channel lives only inside `pty_runtime::spawn`; `OutputStream` holds the receiver. So the title signal cannot push a transition directly and must reach the runtime, which owns the sender.

1. **Runtime seam.** `SessionHandle` keeps a clone of the transition `tx` and of the existing `Arc<Mutex<IdleDetector>>`. Add a defaulted trait method — `note_declared_status(&self, session, RunnerStatus) -> RuntimeResult<()>` — implemented by the PTY runtime as: look the handle up, set the detector's `current` so it does not immediately contradict, and send `StatusTransition { state, source: "title" }`. Everything downstream then works unchanged.
2. **Manager.** `SessionManager::report_declared_status(&self, session_id, RunnerStatus)`: call the runtime method, then set `title_status_armed` on `SessionState` only on success. Hold the state lock across both steps so the output consumer cannot race arming. The default runtime method rejects unsupported reports, preserving byte detection.
3. **Suppression.** In `note_forwarder_transition`, ignore `source == "forwarder"` once that session is armed — a branch parallel to the existing `suppress_local_input_busy` one. Byte transitions keep flowing for unarmed sessions.
4. **Terminal.** In the `Event::Title` arm: classify, compare to the last classification for this session, and only on a change call `core.sessions.report_declared_status(…)`. Keep storing the title as today.

**Classification for phase 1.** Busy iff the title's first grapheme is a braille animation glyph (`U+2800`–`U+28FF`). `✳` is a resting title prefix in the Claude recording, so it classifies Idle and cannot arm a session. Blank titles are not a signal. A session arms the first time it observes a title *with* a braille spinner — never on a bare title, or a shell's prompt title would arm it and then wrongly pin it Idle.

## Rules of the road

- No behavior change for sessions with no spinner in any title. That is the regression risk and the reviewer's first check.
- Keep `IdleDetector` and its tests intact. This adds a source, it does not replace one.
- No new config, no settings surface, no UI change. The status vocabulary and every renderer stay as they are.
- No phase 2 groundwork: no baseline learning, no `InputTracker` precedence work, no OSC 9;4.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`) — Jason smoke-tests. Verify with `make verify`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit, push, open the PR titled `feat(session): derive status from the title spinner, byte detector as fallback (584)`, drive CI green on both required checks. Do not merge. No worktrees, no extra checkouts, no extra agents.

## Tests

Fixture-driven against real recordings, not synthetic input. Replay these fixtures in `crates/runner-terminal/fixtures/` and name them in the handoff:

- `crates/runner-terminal/fixtures/codex-title-working.ndjson` — Codex, 100×30, 10.893 s, copied from `/tmp/snow.01M2D3VQ7B140BQYCX1H46E56N.ndjson`. Busy at 177 ms, Idle at 7428 ms, then no further transition despite 46 output events over 3465 ms; the largest gap is 153 ms and none reaches the byte detector's two-second threshold. This single recording proves both working detection and the continuous-output regression behind #583; it does not replay the original 19.7-second capture.
- `crates/runner-terminal/fixtures/claude-session.ndjson` — already present. The initial `✳` title does not arm; braille starts Busy at 5.038 s and the resting `✳` title returns Idle at 7.891 s.

Plus, as pure unit tests on the classifier: a spinner-prefixed title is Busy; the same title without the prefix is Idle; a blank title is not a signal; a shell prompt title (`jason@Jasons-Mac-Studio:~/repos/runner`) never arms a session. And on the debounce: ten identical classifications in a second produce one transition.

`make verify` green, plus `cargo test -p runner-terminal` and `-p runner-backend`. Read cargo's exit status directly or set `pipefail`; a piped cargo returns the pipe's status.

## Handoff must report

The classification predicate as written, the arming rule, every call site added, and the fixture assertions with their measured transition times. State explicitly that a session with no spinner produces byte-derived transitions exactly as before, and how you proved it.

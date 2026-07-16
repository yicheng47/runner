# Mission status local-input suppression

## Status

Planned. Tracks issue [#302](https://github.com/yicheng47/runner/issues/302). No design gate: no UI change — the sidebar mission row and rail badge just stop lying.

## Problem

Chats and missions share the PTY `IdleDetector`, but its transitions exit through two doors in the forwarder consumer (`session/manager/output.rs:71-104`). Direct chats go through `publish_direct_activity` (`mod.rs:664-698`), which tracks `session.activity`, drops forwarder-sourced busy while `suppress_local_input_busy` is set, and dedupes unchanged states. Mission sessions go through `ForwarderEmitCtx::try_append_runner_status` (`mod.rs:200-217`), which appends the raw transition to the mission event log.

Mission sessions are also outside the activity bookkeeping entirely: spawn/resume busy seeding is gated on `mission_id.is_none()` (`spawn.rs:1199`), and `publish_direct_activity` never runs for them, so `session.activity` stays `None`. That makes `inject_direct_stdin`'s suppression arming (`output.rs:216-218`, requires `activity == Some(Idle)`) and its synthetic `input-submit` busy transition dead code for mission sessions — the whole mechanism silently no-ops.

Net effect: typing into an idle mission runner's terminal echoes bytes, the detector flips busy, the raw row lands in the log, and the sidebar shows "Mission working" (`Sidebar.tsx:2982` via `mission_activity_from_log`, `commands/mission.rs:1705`) plus the rail badge (`MissionWorkspace.tsx:832`) for at least the 750ms threshold. The identical interaction in a chat tab shows nothing. Only the SIGWINCH resize grace (`9c18453`) reached missions, because it lives at the detector level in `pty_runtime.rs`.

## Key Decisions

1. **One transition gate for both surfaces.** Extract the surface/suppress decision out of `publish_direct_activity` into a manager-level helper — `note_forwarder_transition(session_id, state, source) -> bool` — that updates `session.activity`, applies the `suppress_local_input_busy` rules (swallow forwarder-sourced busy while set; clear on idle), and returns whether the transition should be surfaced. The forwarder consumer calls it on every `StatusTransition` regardless of path; only the sink differs (Tauri `session/status` event vs mission-log append). Semantics can't drift again because there is only one place they live.
2. **Suppressed busy must be repaired at submit, or real work goes invisible.** Once an echo-busy is swallowed, the detector is already in Busy and will not re-emit when the agent starts genuinely working — the log would read idle through an entire real turn. Chats already solve this: `inject_direct_stdin` computes a synthetic busy transition on submit from `Idle` (`output.rs:203-214`, source `input-submit`). With decision 3 tracking activity for missions, that same computed transition applies; it just needs to reach the mission log instead of the Tauri event. This is the load-bearing pair: suppression without submit-repair is a worse bug than the one being fixed.
3. **Track `session.activity` for mission sessions.** The gate in decision 1 writes it on every forwarder transition, which also makes `inject_direct_stdin`'s existing idle-check arming work for missions verbatim. No seeding changes: mission "busy until proven idle" stays a projection-side default (`mission_activity_from_latest` already treats a live handle with no status row as not-idle).
4. **Sessions carry a status sink.** Store the mission `ForwarderEmitCtx` (already `Clone`, holds the cached `Arc<EventLog>`) on `SessionState` at spawn/resume, beside `suppress_local_input_busy`. `inject_direct_stdin` routes its computed transition to the sink: `events.status(...)` for direct chats, blocking `append_runner_status(state, "input-submit")` for missions. Forwarder appends remain best-effort and non-blocking because their consumer shares the terminal-output channel; submit repair is load-bearing, runs on the command thread after the keystroke reaches the PTY, and waits through transient log-lock contention instead of dropping the busy row. Permanent append failures are logged.
5. **The router change is accepted, and it is small.** The router projects busy/idle from the same `runner_status` rows (`router/handlers.rs:212`). After this fix it no longer sees echo-busy — "user is typing in a runner's terminal" stops deferring anything that keys off idle. That is the correct reading: typing was never evidence the runner is working. The submit-repair in decision 2 means genuinely-started turns are still visible immediately. Any future "hold delivery while the human is typing" behavior should be its own explicit signal, not a side effect of echo bytes.
6. **Dedupe falls out of activity tracking.** The gate only surfaces a transition when `session.activity` actually changes, so the suppress-then-idle sequence (idle → suppressed echo-busy → idle again) appends nothing instead of a duplicate idle row. Consumers already take latest-per-handle, so this is hygiene, not a behavior fix.

## Non-Goals

- Changing `IdleDetector` thresholds, the resize grace, or anything in `pty_runtime.rs`.
- An unread/attention model for mission rows (spec 39 explicitly scoped mission-row status out; still out).
- Backfilling or rewriting historical `runner_status` rows in existing mission logs.
- Any change to the CLI's own `runner status` signal path (`cli/src/signal.rs`) — agent-reported statuses stay untouched.
- Deferring router signal delivery while the human types (see decision 5 — separate feature if ever wanted).

## Implementation Phases

### Phase 1 — shared transition gate

- Add `note_forwarder_transition(&self, session_id, state: SessionActivityState, source: &str) -> bool` to `SessionManager`: lock the session state, apply the suppression + dedupe rules currently inlined in `publish_direct_activity`, update `session.activity`, return surface/skip.
- Rewrite `publish_direct_activity` on top of it (behavior identical; existing tests hold).
- In the forwarder consumer (`output.rs:71-104`), call the gate before both sinks; skip `try_append_runner_status` when it returns false.
- Tests (`session/manager/tests.rs`): mission echo-busy (suppression set, forwarder busy) appends nothing; idle transition clears suppression and appends; unchanged-state transitions append nothing; direct-chat paths unchanged.

### Phase 2 — status sink on the session

- Add the optional emit ctx to `SessionState`; populate it in mission `spawn` / `resume` where the `ForwarderEmitCtx` is already constructed.
- In `inject_direct_stdin`, route the computed submit transition to the sink instead of assuming `events.status`; source stays `input-submit`.
- Clear the sink with the rest of the handle state on exit (`lifecycle.rs` already resets `activity` / `suppress_local_input_busy` — same sweep).
- Tests: type-then-submit on an idle mission session appends exactly one busy row with source `input-submit`; suppressed episode without submit appends nothing; direct-chat submit still emits the Tauri event and no log row.

### Phase 3 — validation

- `cargo fmt` + `cargo clippy` + `cargo test --workspace` (CI enforces the lint gates), `pnpm exec tsc --noEmit`, `pnpm run lint`.
- Manual pass over the verification list in a dev build.

## Verification

- [ ] Idle mission, click a runner terminal, type without submitting: no "Mission working" spinner, no rail-badge flip, no new `runner_status` rows in the mission log.
- [ ] Submit the prompt: busy row (source `input-submit`) lands immediately; spinner lights; settles idle after the turn.
- [ ] Type, wait for the echo to settle, then submit: same as above — the suppressed episode leaves no idle/busy row pair behind.
- [ ] Direct-chat behavior byte-for-byte unchanged: typing no spinner, submit spinner, unread dots still armed-gated (#296 tests green).
- [ ] Router: signal delivery to an idle runner is not deferred by typing in its terminal; a mid-turn runner still reads busy.
- [ ] Resize while idle still doesn't flip busy (detector-level grace untouched).
- [ ] `cargo test --workspace`, `cargo clippy`, `pnpm exec tsc --noEmit`, `pnpm run lint` clean.

## Relevant Code

- `src-tauri/src/session/manager/output.rs:71-104` — forwarder consumer fork (gate call site), `:190-241` — `inject_direct_stdin` (suppression arming, submit transition, new sink routing).
- `src-tauri/src/session/manager/mod.rs:200-217` — `try_append_runner_status`, `:453-460` — `SessionState` (new sink field), `:664-698` — `publish_direct_activity` (rebase onto the gate).
- `src-tauri/src/session/manager/spawn.rs:1199` — the `mission_id.is_none()` seeding gate (unchanged, cited for context); emit-ctx construction sites for the sink.
- `src-tauri/src/session/manager/lifecycle.rs:67,217` — state resets to extend.
- `src-tauri/src/commands/mission.rs:1652-1714` — log→activity projection (unchanged consumer).
- `src-tauri/src/router/handlers.rs:212` — router status projection (behavioral note, decision 5).

## References

- Issue #302 — bug: mission status ignores local-input suppression.
- Issue #296 / impl `0029-armed-tab-completions-and-pill-removal.md` — the chat-side attention tightening this brings missions in line with.
- Archived spec `docs/features/archive/13-pty-silence-idle-detection.md` — origin of the byte-silence heuristic and the mission/direct fork.
- Issue #124 — origin of the forwarder-emitted `runner_status` rows.

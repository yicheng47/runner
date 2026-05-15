# 13 — PTY-silence idle detection (replace runner-CLI status reports)

> Tracking issue: [#124](https://github.com/yicheng47/runner/issues/124)

## Motivation

Today the busy/idle state for a runner session is reported by the agent
itself: the system prompt is supposed to call `runner status busy` /
`runner status idle` via the bundled CLI after each turn. The CLI appends
a `runner_status` event to the mission's NDJSON log; the router consumes
it (handlers.rs:212), updates its per-handle status map, and nudges the
lead when a worker reports idle. The frontend's RunnersRail busy/idle dot
projects from those events.

This works when the agent cooperates. It's brittle whenever it doesn't:

- **Agents forget.** A claude-code or codex turn that finishes via tool
  call doesn't always reliably terminate with a `runner status idle`
  call. The lead never gets nudged, the user sees a stale "busy" dot.
- **Off-the-shelf TUIs can't cooperate at all.** Gemini CLI, raw shell,
  any agent without runner-CLI integration → permanently looks busy.
- **System-prompt fragility.** "Always call `runner status idle` at end
  of turn" is one of those rules every system prompt has to remember;
  bad runner templates omit it and the entire mission's observability
  rots.
- **Tool-call confirmation prompts.** When claude-code stops mid-turn
  to ask the user "Allow this edit?", the agent is *waiting on stdin*
  (the human-on-the-keyboard scenario, not the lead). It should look
  idle to the lead so the lead can step in; today it stays busy until
  the agent thinks to emit a status update, which it usually doesn't
  from inside a confirmation prompt.

The terminal already has the information needed. The session forwarder
thread reads every byte the agent writes to its PTY via `pipe-pane`.
If no bytes arrive for ~1s, the agent is idle by any reasonable
definition: it's not writing output, it's not running tool calls, it's
parked at a prompt. The moment bytes start flowing again, it's busy.

This spec moves authoritative busy/idle from "agent reports it" to
"session forwarder infers it from PTY silence." Works for every TUI,
no agent cooperation required, no system-prompt rule to remember.

## Scope

### In scope (v1)

- **Per-session idle detector** in the session forwarder. Tracks
  `last_output_byte_at: Option<Instant>` plus current `RunnerStatus`
  (`Busy` | `Idle`). Updated on every byte read from `pipe-pane`.
- **State transitions** emit synthetic `runner_status` events to the
  mission's NDJSON log, using the same shape the CLI emits today
  (`{ kind: signal, type: runner_status, from: <handle>, payload:
  { state, source: "forwarder" } }`). The `source` field is new and
  lets the router / future debugging tools distinguish forwarder-
  inferred state from agent-reported state.
- **Idle threshold + hysteresis.**
  - Default threshold: `750ms` of byte-silence → flip to `Idle`.
  - Wake threshold: any byte after `Idle` → flip immediately to
    `Busy` (no hysteresis on the wake direction).
  - Configurable via a settings DB row keyed by runner template (e.g.,
    a slow TUI might want 2s); v1 ships a single global default.
- **Direct-chat sessions** participate too. They don't have a router
  / lead nudge, but the workspace UI's RunnersRail dot is the same
  surface; PTY-silence drives it identically.
- **Router-side dedupe / latest-wins** stays as it is. The router's
  existing logic that keeps only the most-recent `runner_status` per
  handle absorbs the increased emission rate fine.
- **Agent-side `runner status` CLI verb stays for one release as a
  back-compat alias**, deprecated. Calls still append a
  `runner_status` event (now with `source: "agent"`); router still
  consumes it. After one release we delete the verb.

### Out of scope (deferred)

- **Per-template idle threshold UI.** The settings affordance to tune
  the threshold per runner template. The infrastructure should accept
  a template-level value but v1 ships the global default; the UI
  control comes later if real templates need different values.
- **Notes / annotations.** Today `runner status idle --note
  "compiling"` lets the agent attach a free-form note to the status
  event. The forwarder can't infer that. Out of scope to preserve;
  if real users miss it, revisit with a separate `runner annotate`
  verb that doesn't conflate status and note.
- **OSC / semantic-prompt parsing.** iTerm2's OSC 133 lets a shell
  semantically mark "I'm at a prompt." Parsing those would give
  precise idle detection without timing heuristics, but support is
  uneven across TUIs (claude-code / codex don't emit them) and not
  worth the complexity in v1.
- **Cursor-line pattern matching.** Sniffing the TUI's prompt line
  to know when the agent is parked. Brittle, agent-specific, big
  surface area. No.
- **Tool-call vs. prompt-wait distinction.** Both look like "no bytes
  flowing" to the forwarder; both surface as `Idle`. For v1 that's
  fine — the lead nudge is the same in both cases ("worker is
  waiting"). Future work could disambiguate by inspecting whether
  `pane_in_mode` or the cursor is on a known prompt glyph.

### Key decisions

1. **Forwarder is the new authoritative source.** The agent-reported
   path was conceptually elegant ("the agent knows best when it's
   done") but empirically unreliable. PTY silence is observable
   without agent cooperation and works for every TUI, including ones
   that have no runner-CLI integration. The "agent knows best"
   premise was wrong: the agent doesn't reliably emit at turn end,
   and it can't emit while waiting on a confirmation prompt at all.
2. **Synthetic events go through the event log, not a side channel.**
   The forwarder appends a real `runner_status` event to the NDJSON
   log (with `source: "forwarder"`). Same wire shape as today; same
   router consumption path; same projections in the UI. No new IPC
   topology, no new subscriber tree. The audit trail tells the truth.
3. **Carry a `source` field but don't branch on it.** The router
   treats forwarder-emitted and agent-emitted `runner_status` rows
   identically (latest wins). `source` exists for debugging / future
   diagnostics, not for runtime decisions. This avoids the trap of
   "agent says busy, forwarder says idle, who wins?" — we just trust
   the most recent, and since the forwarder fires more often by design,
   it dominates in practice.
4. **No hysteresis on busy→idle, hysteresis-of-zero on idle→busy.**
   Going busy on the *first* byte after silence is fine — a streaming
   TUI's first byte is reliable. Going idle requires a full
   `750ms` of silence, which gives breathing room past punctuation
   pauses in agent output streams. Spec freezes the constant in code
   with a comment.
5. **Drop the agent-CLI verb in a deprecated phase, not immediately.**
   Templates and docs in the wild still call `runner status`; making
   it an error would break existing missions on upgrade. Phase 4
   ships the forwarder; one release later we delete the verb. Same
   pattern any DB migration would follow.

## Implementation phases

### Phase 1 — idle detector in the session runtime

- Add `IdleDetector` to `src-tauri/src/session/tmux_runtime.rs` (or a
  sibling module). State:
  ```rust
  struct IdleDetector {
      last_byte: Option<Instant>,
      current: RunnerStatus,
      threshold: Duration, // 750ms default
  }
  ```
- Methods:
  - `on_bytes(&mut self, n: usize) -> Option<RunnerStatus>` — called
    by the forwarder on every successful `read`. Updates `last_byte`,
    and if currently `Idle`, returns `Some(Busy)` so the caller can
    emit a transition.
  - `tick(&mut self) -> Option<RunnerStatus>` — called periodically
    by a timer. If `last_byte.elapsed() > threshold` and current is
    `Busy`, returns `Some(Idle)`.
- Inject into the forwarder loop:
  - The loop already uses `poll()` with a timeout for FIFO
    readability; bound it to `min(remaining_idle_window, current_timeout)`
    so it wakes up exactly when the threshold expires even with no I/O.

### Phase 2 — emit synthetic events

- Per session, when a transition fires, call a new
  `SessionManager::emit_runner_status(handle, state, source:
  "forwarder")` that appends to the mission's NDJSON log via the same
  `EventLog::append` path the CLI uses today.
- Direct-chat sessions go through the same path but write to the
  direct-chat session's event log (not a mission log). Confirm the
  RunnersRail subscriber for direct chats also reads `runner_status`
  from there.
- Bump the synthetic event's `payload` to include `source: "forwarder"`.
  Agent-emitted events from the CLI get `source: "agent"` (modify
  `cli/src/main.rs::Status` and `signal::run_status` to inject this).

### Phase 3 — wire to existing consumers

- Router's `handlers::runner_status` (handlers.rs:212) is unchanged in
  logic but the increased event volume needs verification: the
  lead-nudge fires only on worker→idle transitions, which is
  rate-limited by physical PTY behavior (≤1 transition per
  threshold-window) — no new flood risk.
- RunnersRail UI is unchanged. Status projection already filters to
  latest-per-handle.
- Router's "synthetic busy on first stdin injection" (router/mod.rs:
  390) becomes redundant once the forwarder fires busy reliably on
  the next output byte. Keep it for now (it's defensive against an
  agent that takes >threshold to respond to injection) and remove it
  in a later cleanup pass.
- **Hard dependency on [#125](https://github.com/yicheng47/runner/issues/125).** The current
  worker-idle handler (`handlers.rs:230-245`) injects `[runner_status]
  @worker is idle` into the lead's stdin as a fake user message. With
  forwarder-derived idle firing ~10× more often than agent-reported
  idle, leaving that branch in place would flood the lead's TUI on
  every mid-stream pause. Spec 13 cannot ship until #125 lands — the
  injection branch must be deleted, leaving only the in-memory
  status-map update for observability consumers (RunnersRail, future
  `runner roster` pulls).

### Phase 4 — deprecate the agent-CLI verb

- Update `cli/src/main.rs::Status` to print a one-line stderr
  deprecation notice on use (`runner status is deprecated; busy/idle
  is now inferred from PTY activity`). Keep the event-emit behavior
  so existing templates don't regress.
- Update the runner-CLI help / `runner help` and any references in
  `docs/arch/v0-arch.md` §6.3 to call out the new inference path and
  the deprecation.
- Schedule removal for the release after this lands.

### Phase 5 — verification

- Backend tests:
  - `IdleDetector` unit tests covering: silent stretch → idle
    transition; byte arrival flips to busy immediately; no
    transition when state doesn't change.
  - Integration test (`#[ignore]` gated): real tmux pane running a
    stub agent that emits 100ms of output, then silence, then more
    output. Assert two synthetic `runner_status` events land in the
    log with the expected timing.
- Manual smoke:
  1. Start a mission with a claude-code lead. Send a prompt. Observe
     RunnersRail dot transitions: green (busy) during stream,
     gray (idle) within ~1s after stream ends. No `runner status idle`
     call needed in the system prompt.
  2. Configure a runner template that runs `bash` (no runner-CLI
     integration). Send a command. Confirm the dot reflects busy/
     idle correctly purely from PTY activity.
  3. claude-code asks a confirmation prompt mid-tool-call (e.g. an
     edit prompt). Confirm RunnersRail flips to idle once the prompt
     is rendered and bytes stop flowing; lead gets the idle nudge.
  4. Existing template that *does* call `runner status idle` keeps
     working — the event lands, the router consumes it, no
     double-nudge (latest-wins absorbs both sources).
  5. `runner status idle` on the CLI prints the deprecation stderr
     but still succeeds.

## Verification

- [ ] Forwarder emits `runner_status` events on PTY busy↔idle
      transitions for every active session (mission slot or direct
      chat).
- [ ] RunnersRail busy/idle dot tracks actual PTY activity within
      ~1s of the last byte.
- [ ] Agents with no runner-CLI integration (raw shell, gemini-cli,
      etc.) get correct busy/idle behavior with no template changes.
- [ ] Lead receives the idle-nudge on worker→idle, identical to
      today's behavior, without the worker calling `runner status
      idle`.
- [ ] `runner status` CLI verb still works and prints a deprecation
      notice on stderr; emitted events carry `source: "agent"`.
- [ ] Router's "synthetic busy on first inject" path still wins on
      the edge case of injection landing before any PTY output.
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit` clean.
- [ ] No new IPC commands; no schema changes; no agent-protocol
      changes beyond the deprecation stderr line.

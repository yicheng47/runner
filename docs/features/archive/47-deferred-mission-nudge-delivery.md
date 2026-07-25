# 47 — Deferred mission nudge delivery

Tracking: [#328](https://github.com/yicheng47/runner/issues/328)

## Motivation

Every mission notification funnels into `Router::inject_and_submit` (`src-tauri/src/router/mod.rs:426`): the text is written into the recipient's PTY stdin and a spawned thread fires `\r` 80ms later. To the agent's TUI this is indistinguishable from a human typing a message and pressing Enter — which is the point, but it means the delivery shares the TUI input box with the actual human. When the user is mid-typing in that pane, the nudge appends to their half-typed draft and the trailing Enter submits the concatenation: corrupted draft, garbled message to the agent, and a lost train of thought. The fixed 80ms Enter can also race the user's own Enter.

Affected senders (all route through the same primitive): directed and broadcast `message_nudge` inbox lines, `human_said` relays, `ask_lead` relays to the lead, and `human_response` answers.

There is no PTY mechanism to deliver "around" the input box, and TUI-side tricks (clear line, re-type the draft after delivery) are runtime-specific and fragile. Deferral is the correct shape: nudges are wake-only — the message body lives in the inbox projection (arch §5.5.0) — so delaying delivery until the pane's input is clean costs nothing and can never corrupt input.

## Scope

- Per-session **pending-local-input** tracking in the session manager, fed by the single keystroke path all panes share (`session_inject_stdin` → `inject_direct_stdin`).
- A per-session **router outbox**: `inject_and_submit` parks deliveries while the recipient pane has pending input or very recent typing, and flushes when the input clears.
- Coalescing of queued inbox nudges on flush.
- Out of scope: TUI-side input-box manipulation (no clear-and-retype), spawn-time first-turn delivery (`first_turn_argv` / `inject_paste_with_verify` paths), direct-chat send paths, and any change to nudge wording or the inbox projection.

## Key Decisions

1. **Dirty heuristic lives on `SessionState`.** `local_input_pending: bool` plus `last_local_input_at`. Printable/text bytes set pending; submit (`\r`) and Ctrl+C (0x03) clear it; escape sequences (arrows, etc.) refresh `last_local_input_at` without setting pending. Conservative false-positives are acceptable — a wrongly-held nudge is late, a wrongly-delivered one corrupts input.
2. **Defer on pending input OR recent typing.** Delivery is blocked while `local_input_pending` is set, and also within a short recently-typing window (~2s since last keystroke) to avoid racing a draft that hasn't produced its first byte-classification yet.
3. **Hold until input clears — no max-delay cap.** A capped flush would re-introduce the collision at cap expiry. The worst case (user walks away mid-draft) delays a wake, not a message: the body is already in the inbox, and the flush fires the moment the user submits or clears the draft.
4. **Coalesce inbox nudges, preserve relay bodies.** N parked `[inbox]` lines flush as one summary line; `human_said` / `ask_lead` / `human_response` bodies flush in arrival order, uncoalesced.
5. **Queue lifecycle follows the session.** Flush triggers: input-clear (submit or Ctrl+C observed in `inject_direct_stdin`), and respawn/resume of the recipient session (fresh TUI = empty input box). Session exit drops its queue — consistent with today's behavior, where a nudge to a dead session is a warn-and-drop.
6. **`synthesize_wake_busy` moves to flush time.** The recipient is marked busy when the injection actually lands, not when it parks — otherwise the rail shows a busy badge on an agent that hasn't been woken yet.

## Implementation Phases

### Phase 1 — pending-input tracking

- Add `local_input_pending` / `last_local_input_at` to `SessionState` (`src-tauri/src/session/manager/mod.rs`), updated in `inject_direct_stdin` (`src-tauri/src/session/manager/output.rs:197`) beside the existing submit/suppression bookkeeping.
- Expose a `SessionManager` query for the router (`input_quiescent(session_id) -> bool`) and a clear-notification hook.
- Unit tests over byte classes: printable sets pending, `\r` clears, Ctrl+C clears, escape sequences refresh the timestamp only.

### Phase 2 — router outbox

- Per-session queue in router state; `inject_and_submit` consults `input_quiescent` and parks instead of injecting when false.
- Flush on input-clear notification and on session respawn registration; coalesce parked inbox nudges; drop queue on session removal.
- Move `synthesize_wake_busy` to flush time.
- Router tests: park-then-flush ordering, coalescing, queue drop on exit, no re-delivery on bus replay (watermark still applies at enqueue time).

### Phase 3 — validation

- `cargo test --workspace`.
- Manual: type half a message in a mission pane, have another runner send mail — no injection until you submit or clear; then the nudge lands alone on a clean input line.

## Verification

- [ ] Half-typed draft in a mission pane + incoming inbox nudge: draft is untouched; nudge arrives after submit/clear, on its own line.
- [ ] Three nudges parked behind a draft flush as one coalesced line.
- [ ] `human_response` parked behind a draft flushes with its full body, in order, after the draft clears.
- [ ] Nudge to a pane with no pending input delivers immediately (unchanged fast path).
- [ ] Recipient respawn flushes its parked queue into the fresh TUI.
- [ ] Busy badge on the rail appears at actual delivery, not at park time.
- [ ] `cargo test --workspace` clean.

## Relevant Code

- `src-tauri/src/router/mod.rs:426-447` — `inject_and_submit` (park point, 80ms Enter thread), `:369` — `synthesize_wake_busy`.
- `src-tauri/src/router/handlers.rs:175-210` — `message_nudge` directed/broadcast; `human_said` / `ask_lead` / `human_response` handlers in the same file.
- `src-tauri/src/session/manager/output.rs:197-262` — `inject_direct_stdin`, the single keystroke path (submit detection, suppression bookkeeping to extend).
- `src-tauri/src/session/manager/mod.rs:472` — `SessionState` (new fields beside `suppress_local_input_busy`).
- `src-tauri/src/commands/session.rs:83` — `session_inject_stdin` command (frontend keystroke entry).
- `docs/impls/archive/0007-spawn-time-prompt-delivery.md` — prior art for removing an injection race at a different lifecycle point.

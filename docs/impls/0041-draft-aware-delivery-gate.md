# Draft-aware delivery gate: replace the one-way input latch with a line model

## Status

Planned. Tracking issue [#359](https://github.com/yicheng47/runner/issues/359), spec [54](../features/54-draft-aware-delivery-gate.md).

## Problem

`local_input_pending` (`session/manager/mod.rs`) is a boolean latch: any printable byte sets it (`classify_local_input`, `session/manager/output.rs:16`), and only `\r`, `\x03`, respawn, or lifecycle transitions clear it. Backspace, Ctrl-U, and Escape are `ActivityOnly` — they refresh the 2s recency window and leave the latch set. Type one stray character into a mission slot and backspace it away: the box is empty, the latch is true, and `reserve_delivery` returns `PendingInput` forever. The blocked outbox never schedules a retry (unlike `RecentlyTyping`, which does), so the mission silently stops coordinating.

The existing UI surface makes it worse, not better: `InboxBlockedPill` ("typing detected, delivery paused") offers exactly one action, gated on idle — a button that injects `\r`. That *submits a line to the agent*: the destructive clear the spec exists to eliminate, shipped as the remedy.

## Key Decisions

1. **The draft is a bounded byte buffer on `SessionState`, not a boolean and not a length counter.** Word-delete (`\x17`) cannot be modeled by a counter, and a counter fails open the moment it undercounts. Keeping the actual bytes (capped at 4 KB; at the cap the buffer saturates and only a clearing key empties it — fail closed) gives every editing key faithful semantics, and the gate reads exactly one bit: `draft.is_empty()`. The buffer replaces `local_input_pending`; `last_local_input_at` stays.

2. **`classify_local_input` grows from three classes to draft operations; `update_local_input_state` applies them under the existing lock-and-rollback contract.** The byte→op table, per input chunk: printables and bracketed paste (`\x1b[200~…`) and `\x16` append; `\x7f`/`\x08` pop one char; `\x17` pops a word; `\x15` clears; a lone `\x1b` clears (Esc empties the composer in claude-code and codex); `\r` clears (submitted); `\x03` clears (interrupted); `\x1b\r` — the Shift+Enter newline the frontend sends — **appends**, closing a pre-existing hole where a multi-line draft never set the latch at all; **Up/Down arrows (`\x1b[A`/`\x1b[B`) mark the draft non-empty** — in claude-code and codex, Up at an empty composer recalls a previous message into it, so treating arrows as neutral would fail open into a recalled draft; Left/Right/Home/End and every other CSI sequence cannot materialize content and stay activity-only. Unknown control bytes: activity-only, content untouched — when unsure the model stays non-empty, and the backstop and manual clear are the release valves. `inject_direct_stdin`'s write-failure rollback snapshots and restores the whole draft struct, as it does the latch today.

3. **`RECENT_LOCAL_INPUT_WINDOW` survives unchanged.** Emptiness and recency answer different questions: an empty buffer says the box is clean, the 2s window says fingers may be mid-keystroke. Both remain conditions in `reserve_delivery` and `input_quiescent` (resolving the spec's open question: keep it).

4. **The abandonment backstop is enforced lazily at reserve time and retried on schedule — the `RecentlyTyping` pattern, not a timer thread.** `DRAFT_ABANDON_WINDOW` = 10 minutes (const, with a test-tunable field mirroring `resize_settle_ms`). `reserve_delivery` with a non-empty draft whose `last_local_input_at` is older than the window proceeds as `Ready` — after ten untouched minutes the draft is abandoned by definition and unblocking the mission wins. Below the window it returns `PendingInput` carrying the remaining time, and the router schedules an outbox retry at that deadline exactly as it already does for `RecentlyTyping`. This also fixes the silent-stall property for free: a gated outbox always has a scheduled future.

5. **The manual clear injects `\x15` — no new command, no model-only lie.** A clear that only reset Runner's model would leave the human's characters sitting in the composer, and the next delivery would collide with them; the model must not diverge from the box. Injecting Ctrl-U through the normal `inject_direct_stdin` path actually empties the composer line (readline-universal; supported by the claude-code and codex composers), never submits, never interrupts, and flows through the same classifier — so model and reality clear together, `InputCleared` fires, and the blocked outbox retries through the existing listener. The pill's `\r`-injecting button is replaced by this, and the idle-only gating on the action drops (clearing a draft is safe regardless of agent state).

6. **Surfacing rides the existing `DeliveryBlockedEvent` machinery untouched.** `blocked_transition` already distinguishes draft-gated (`pending_input_blocked`) from in-flight, and the pill only renders for the draft case — the state model was the missing piece, not the event plumbing. The pill copy becomes explicit about whose input is blocking ("delivery paused — you have unsent input here") with the always-available "Clear draft" action. No new slot states, no sidebar changes.

7. **Every latch lifecycle site moves to the draft struct.** Respawn/`install_handle` reset, the `kill` and lifecycle clears (`lifecycle.rs`), `purge_session_buffers`, the prune-empty condition on `SessionState`, and `input_quiescent` all swap `local_input_pending` for `draft.is_empty()` mechanically. `reserve_delivery`'s contract is otherwise unchanged.

## Goals

- Type a character into a slot, backspace it away → the inbox delivers with no Enter, no Ctrl-C, no waiting.
- A real unsubmitted draft — typed or pasted, single- or multi-line — keeps the gate closed; submit or clear it and delivery resumes immediately.
- A forgotten draft stops blocking after `DRAFT_ABANDON_WINDOW`, and a gated outbox always has a scheduled retry — no silent stalls.
- The pill's action never submits to or interrupts the agent.

## Non-Goals

- Weakening the gate: delivering into a live draft remains the failure this design must never introduce.
- Hook-based agent status (spec 52/#347) — orthogonal axis, unchanged.
- Screen-scraping the TUI input line; changing the injection mechanism, inbox pull model, or outbox semantics beyond the PendingInput retry scheduling.
- Cursor modeling. The buffer ignores cursor position; only emptiness feeds the gate (Left/Right/Home/End are activity-only; Up/Down are content events per decision 2, not cursor tracking).

## Implementation Notes

- `src-tauri/src/session/manager/mod.rs` — `DraftState` struct (buffer + cap), replaces `local_input_pending` on `SessionState`; `DRAFT_ABANDON_WINDOW` + test-tunable field; `reserve_delivery` emptiness/abandonment logic; `input_quiescent`; prune-empty condition; respawn reset.
- `src-tauri/src/session/manager/output.rs` — classifier table per decision 2; `update_local_input_state` applies ops and reports the empty↔non-empty transition for `InputCleared`.
- `src-tauri/src/session/manager/lifecycle.rs` — clear sites move mechanically.
- `src-tauri/src/router/mod.rs` — `DeliveryReservation::PendingInput` carries the abandonment remaining-time; the reservation handler schedules the outbox retry (mirror of the `RecentlyTyping` arm).
- Instrumentation: every empty↔non-empty transition logs one `[draft-gate]` line to `runner.log` with the cause (`printable`, `paste`, `history-recall`, `backspace-emptied`, `esc`, `submit`, `interrupt`, `abandoned`, `manual-clear`, `saturated`) — the resize-gate pattern; "why is this inbox blocked" becomes answerable from one file.
- `src/components/InboxBlockedPill.tsx` — copy per decision 6; action injects `\x15` via the existing `api.session.injectStdin`; idle gating on the action removed.
- Reference for the editing state machine shape: Orca's `pendingShellCommandLine` model in `~/repos/orca/src/renderer/src/components/terminal-pane/pty-connection.ts` (~line 1246 as of this writing; the spec's line numbers have drifted) — borrow the transitions, not the design (spec 54 §prior art: Orca's own delivery behavior is the anti-pattern). Runner's port drops the cursor, adds `\x1b\r`, lone-Esc, and bracketed-paste handling.

## Validation

- Rust unit tests on byte sequences against the model: type→backspace-to-empty opens the gate; Ctrl-U opens; lone Esc opens; Up/Down close an empty draft (history recall); Left/Right/Home/End leave an empty draft open and a non-empty one closed; Shift+Enter (`\x1b\r`) keeps it closed; bracketed paste keeps it closed until `\r`; `\x03` opens; unknown control bytes leave content untouched; the 4 KB saturation stays non-empty until a clearing key.
- `reserve_delivery` tests: non-empty draft → `PendingInput` with remaining time; empty draft + recent activity → `RecentlyTyping`; abandoned draft (tunable window pinned low) → `Ready`; write-failure rollback restores the draft.
- Router test: a PendingInput reservation schedules an outbox retry that fires after abandonment and delivers.
- Frontend: pill renders the draft copy and the clear action calls `injectStdin` with `\x15`; no idle gating.
- Manual (spec 54's checklist): stray-keystroke recovery, real-draft gating, Ctrl-U, paste, abandonment, manual clear neither submits nor interrupts, gated-vs-busy visual distinction.
- `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

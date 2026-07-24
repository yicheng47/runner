# 50 — Inbox delivery blocked indicator

> Tracking issue: [#336](https://github.com/yicheng47/runner/issues/336)

## Motivation

Mission inbox bodies are durable, but their wake nudges share the runner's terminal input. Feature 47 correctly parks a nudge rather than corrupting a draft, and feature 48 correctly refuses to clock-nudge while input remains pending. This creates a safe but silent state: unread coordination mail is waiting, the agent is not being woken, and the human may not realize their draft is the blocker.

Ctrl+U makes the ambiguity visible. Runner sees the keystroke but cannot know whether it emptied a single-line draft or removed only one line from a multiline draft. Reconstructing the child TUI's editor state from xterm input would be brittle and runtime-specific, so Runner should keep the conservative delivery gate and tell the human when it is blocking inbox delivery.

## Scope

- Show a pane-local, non-modal indicator when a live mission session has unread inbox mail and pending local input prevents its wake nudge from being delivered safely.
- Use mechanism-based copy such as `Inbox waiting (2) — typing detected, delivery paused`, including the unread count when it is greater than one. The copy must not assert that a draft exists (backspace-cleared input is undetectable, so the box may already be empty) and must not claim any action notifies the worker: clearing input releases the parked nudge, and Runner delivers it. No @handle in the copy — the indicator is pane-local, so the affected runner is already unambiguous.
- Include a `Clear input (⌃C)` button on the indicator that emits a single Ctrl+C through the ordinary local-input write path — byte-identical to the user pressing the key. `ClearPending` fires organically and the existing 500ms flush grace plus fire-time re-check still apply, so typing during the grace re-parks delivery. Show the button only while the runner is idle: Ctrl+C during a busy turn would interrupt the agent. A lone Ctrl+C is safe on an already-empty box (worst case in claude-code it primes the exit hint; the pill's disappearance after delivery removes the second-press risk).
- Keep the indicator outside the terminal byte stream so it cannot alter, submit, or clear the user's draft.
- Clear the indicator when unread count reaches zero, input clears and delivery proceeds, the session exits, or the mission unmounts.
- Drive the UI from ephemeral in-process state transitions; do not persist blocked-delivery state or append coordination-log events for it.
- Preserve the push-not-pull protocol. This is a human-facing explanation of a blocked wake, not agent polling or a new delivery path.

## Out of scope

- Reconstructing the agent TUI's draft buffer from xterm keystrokes or screen output.
- Treating Ctrl+U as proof that all input is clear.
- A force-deliver action that bypasses delivery safety checks or splices a nudge into remaining draft text. The `Clear input` button is not this: it performs the same keystroke the indicator teaches, through the same write path, with every reservation/grace check left in place.
- Direct chats, which have no mission inbox.

## Implementation phases

1. **Blocked-state projection** — combine the router's unread projection with the session delivery reservation/outbox state and emit only blocked/unblocked transitions for live mission sessions.
2. **Pane indicator** — render the state beside the affected terminal with concise copy and no focus stealing.
3. **Lifecycle cleanup** — clear blocked state on successful delivery, watermark advance, session exit, router unmount, and pane/session replacement.
4. **Validation** — cover transition deduplication and concurrency in Rust, then cover pane rendering and cleanup in frontend tests.

## Verification

- [ ] Typing a draft while unread mail arrives parks the nudge and shows the indicator on the correct pane.
- [ ] Pressing Enter or Ctrl+C clears the input, permits delivery, and removes the indicator.
- [ ] Ctrl+U does not falsely declare multiline input clear; the indicator remains until an unambiguous clear.
- [ ] A watermark advance to zero unread removes the indicator without injecting anything.
- [ ] Empty inboxes, direct chats, busy sessions without blocked local input, and unrelated panes never show the indicator.
- [ ] Session exit and mission stop remove blocked state with no stale indicator after remount.
- [ ] The Clear input button sends exactly one Ctrl+C through the local-input path: a leftover draft is cancelled, an empty box is unchanged, ClearPending is observed, and delivery proceeds after the grace; the button is absent while the runner is busy.
- [ ] Repeated reconciliation ticks do not duplicate UI events or churn rendering while the blocked state is unchanged.

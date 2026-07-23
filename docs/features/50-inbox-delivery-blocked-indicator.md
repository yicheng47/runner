# 50 — Inbox delivery blocked indicator

> Tracking issue: [#336](https://github.com/yicheng47/runner/issues/336)

## Motivation

Mission inbox bodies are durable, but their wake nudges share the runner's terminal input. Feature 47 correctly parks a nudge rather than corrupting a draft, and feature 48 correctly refuses to clock-nudge while input remains pending. This creates a safe but silent state: unread coordination mail is waiting, the agent is not being woken, and the human may not realize their draft is the blocker.

Ctrl+U makes the ambiguity visible. Runner sees the keystroke but cannot know whether it emptied a single-line draft or removed only one line from a multiline draft. Reconstructing the child TUI's editor state from xterm input would be brittle and runtime-specific, so Runner should keep the conservative delivery gate and tell the human when it is blocking inbox delivery.

## Scope

- Show a pane-local, non-modal indicator when a live mission session has unread inbox mail and pending local input prevents its wake nudge from being delivered safely.
- Use direct copy such as `Inbox waiting — submit or cancel your draft to notify @handle`, including the unread count when it is greater than one.
- Keep the indicator outside the terminal byte stream so it cannot alter, submit, or clear the user's draft.
- Clear the indicator when unread count reaches zero, input clears and delivery proceeds, the session exits, or the mission unmounts.
- Drive the UI from ephemeral in-process state transitions; do not persist blocked-delivery state or append coordination-log events for it.
- Preserve the push-not-pull protocol. This is a human-facing explanation of a blocked wake, not agent polling or a new delivery path.

## Out of scope

- Reconstructing the agent TUI's draft buffer from xterm keystrokes or screen output.
- Treating Ctrl+U as proof that all input is clear.
- A force-deliver action that can splice a nudge into remaining draft text.
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
- [ ] Repeated reconciliation ticks do not duplicate UI events or churn rendering while the blocked state is unchanged.

# Direct Chat Response Status

> Implements the first narrowed slice of [#130](https://github.com/yicheng47/runner/issues/130). This does not implement macOS notifications.

## Context

Issue #130 originally scoped OS notifications for agent-to-human events. The first useful product slice is smaller: while a direct chat is open or listed in Runner, the app should show that the agent has produced a response and is now idle. This gives the human an in-app attention signal before adding notification permission, routing, and suppression complexity.

Direct chats are off-bus. Mission sessions already persist availability through `runner_status` events in the mission log, and the workspace rail projects that into busy/idle dots. Direct chats do not have a mission event log, so the PTY forwarder's busy/idle transition is currently dropped for them.

## Goal

When a direct chat has an unread agent response and the agent is idle, reflect that on the chat's status control.

For this plan, "has a response" means all of:

- The user has sent input into that direct chat since the last cleared response hint.
- The PTY emitted output for that same session after that input.
- The forwarder reported the session as idle after the output burst.

The status control should return to normal live/running state once the user sends the next input, switches to another session, stops/resumes the session, or the session exits.

## Non-goals

- No macOS notification plugin.
- No OS permission prompt.
- No notification click routing.
- No global notification setting.
- No persisted unread model.
- No sidebar badge count.
- No mission behavior changes.

## Decisions

1. Use the existing PTY idle detector as the source of truth for "agent idle".
2. Add a live Tauri event for direct-chat availability instead of appending direct-chat events to a mission log that does not exist.
3. Track response-readiness in the direct chat frontend, not in SQLite.
4. Keep this status visual-only for now. The status control remains the same surface; it just gains a response-ready state.
5. Use the active direct session id as the correlation key. Do not infer readiness from runner id because one runner can have multiple direct chats.

## Step 1: Emit live session status

Files: `src-tauri/src/session/manager.rs`, `src/lib/types.ts`

- Add a `SessionStatusEvent` payload with `session_id`, `mission_id`, `state`, and `source`.
- Add a `status` hook to `SessionEvents`.
- Have `TauriSessionEvents` emit `session/status`.
- In the forwarder consumer's `RuntimeOutput::StatusTransition` branch, always emit `session/status`.
- Keep the existing mission `runner_status` append path unchanged for mission sessions.
- Direct chats still skip event-log append; they only consume the live `session/status` event.

The important invariant: mission behavior remains exactly as-is. The new event is an additional live projection, not a replacement for persisted `runner_status`.

## Step 2: Track user-input and response output

Files: `src/components/RunnerTerminal.tsx`, `src/pages/RunnerChat.tsx`

- Add an optional `onUserInput` callback to `RunnerTerminal`.
- Fire it for all PTY input paths:
  - normal `term.onData` bytes,
  - Shift+Enter manual injection,
  - image paste's Ctrl-V injection.
- In `RunnerChat`, when the active terminal fires `onUserInput`, mark that session as awaiting a response and clear any existing response-ready hint.
- Listen to `session/output` for the active direct session. If output arrives while awaiting a response, mark that a response has been seen.

This intentionally keys off user input, not any output. Startup banners, resume replays, and initial TUI paints should not create a "response ready" state before the human has asked for anything.

## Step 3: Combine response output with idle

Files: `src/pages/RunnerChat.tsx`

- Listen to `session/status` for the active direct session.
- Track the latest `busy` / `idle` state.
- Derive a new UI state only when `hasAgentResponse && latestStatus === "idle"`.
- Clear both flags on:
  - active `sessionId` change,
  - user input,
  - `session/exit`,
  - `endChat`,
  - `resumeChat`,
  - archived/read-only route.

The idle transition should be the final gate. Agent output alone means "work is in progress"; response-ready should wait until the output burst has settled.

## Step 4: Reflect on the chat status control

Files: `src/pages/RunnerChat.tsx`

- Extend the existing chat state derivation with a response-ready visual state for running sessions.
- Recommended label: `response`.
- Recommended color: existing warning/amber token, because it is attention-worthy but not an error.
- Keep the Stop action available in this state; the session is still live.
- Do not show SessionEndedOverlay in this state.

If the label feels too verbose in the header, use `ready` instead. The behavioral contract matters more than the exact copy.

## Step 5: Tests

Rust:

- Unit-test that a direct session receiving `RuntimeOutput::StatusTransition { state: Idle }` emits `session/status` with `mission_id = null`.
- Keep existing mission `runner_status` tests unchanged.

Frontend:

- Typecheck is the main guard unless a component-test harness already exists for `RunnerChat`.
- Add a small pure helper for status derivation only if the inline state logic starts getting hard to reason about.

Manual:

1. Start a direct chat.
2. Send a prompt.
3. While the agent is streaming, the status remains live/running.
4. After the agent finishes and the forwarder idles, the status control shows `response`.
5. Type the next prompt; the status returns to live/running immediately.
6. Stop/resume the chat; no stale response state survives.
7. Switch between two chats; response state is scoped to the correct session id.

## Validation

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo fmt --check`
- `cargo test -p runner direct_forwarder_emits_live_session_status`
- `cargo test --workspace`

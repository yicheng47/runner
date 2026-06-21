# Direct Chat Activity Status

> Implements the first narrowed slice of [#130](https://github.com/yicheng47/runner/issues/130). This does not implement macOS notifications.

## Context

Issue #130 originally scoped OS notifications for agent-to-human events. The first useful product slice is smaller: while a direct chat is open, the app should show whether the live agent is actively working or idle. This gives the human an in-app attention signal before adding notification permission, routing, suppression, and mission-workspace complexity.

Direct chats are off-bus and do not have a mission event log, so the PTY forwarder's busy/idle transition is currently dropped for them. Stage one is direct chats only; mission sessions and mission workspace status are intentionally excluded.

Current code map:

- `SessionEvents`, `TauriSessionEvents`, `OutputEvent`, and `ExitEvent` live in `src-tauri/src/session/manager/mod.rs`.
- The forwarder consumes `RuntimeOutput::StatusTransition` in `src-tauri/src/session/manager/output.rs`.
- Session manager tests live in `src-tauri/src/session/manager/tests.rs`.
- `RunnerTerminal` owns the xterm output subscription and PTY input paths; `RunnerChat` owns the direct-chat header status pill and Stop/Resume lifecycle.

## Goal

The direct chat status control should use four display states:

- `busy` — the session process is live and the forwarder currently sees agent activity.
- `idle` — the session process is live and the forwarder has settled after the latest output burst.
- `stopped` — the session exited cleanly or was stopped by the user.
- `crashed` — the session exited unsuccessfully.

There is no separate `response`, `ready`, unread, or notification state in this pass. The first slice is only live activity status for direct chats.

## Non-goals

- No macOS notification plugin.
- No OS permission prompt.
- No notification click routing.
- No global notification setting.
- No persisted unread model.
- No sidebar badge count.
- No response-ready state.
- No live status changes for mission sessions.
- No mission behavior changes.

## Decisions

1. Use the existing PTY idle detector as the source of truth for direct-chat activity.
2. Add a live Tauri event for direct-chat activity instead of appending direct-chat events to a mission log that does not exist.
3. Keep SQLite session lifecycle unchanged: persisted `sessions.status` remains `running | stopped | crashed`. `busy | idle` are live UI projections for an alive direct chat.
4. Use direct-chat display labels `busy | idle | stopped | crashed`. The live direct-chat event should expose `busy | idle`, matching the forwarder's `RunnerStatus::Busy | Idle`.
5. Use the active direct session id as the correlation key. Do not infer activity from runner id because one runner can have multiple direct chats.
6. Mission behavior remains exactly as-is: mission forwarder status still appends `runner_status` with `busy | idle` to the event log.
7. Default alive direct chats to `idle` until a live activity event says otherwise. A DB `running` row only means the process is alive; it does not prove the agent is actively working.

## Step 1: Emit Live Direct-Chat Status

Files: `src-tauri/src/session/manager/mod.rs`, `src-tauri/src/session/manager/output.rs`, `src-tauri/src/session/manager/tests.rs`, `src/lib/types.ts`

- Add a direct-chat `SessionActivityEvent` payload with `session_id`, `state`, and `source`.
- Add `SessionActivityState = "busy" | "idle"` and `SessionActivityEvent` in `src/lib/types.ts`.
- Add a default no-op `status(&SessionActivityEvent)` hook to `SessionEvents`.
- Have `TauriSessionEvents::status` emit `session/status`.
- In `output.rs`, when the forwarder receives `RuntimeOutput::StatusTransition`, emit `session/status` only when `emit_ctx.is_none()` (direct chat). Map `RunnerStatus::Busy` to `"busy"` and `RunnerStatus::Idle` to `"idle"`.
- When `emit_ctx` is present, keep the existing mission event-log append path and do not emit the live direct-chat event.

The important invariant: this adds a direct-chat projection only. Mission behavior remains unchanged.

## Step 2: Track Live Status in RunnerChat

Files: `src/pages/RunnerChat.tsx`, `src/lib/types.ts`

- Listen to `session/status` for the active direct session.
- Store the latest live activity state per active `sessionId`.
- When a DB `running` row has no live activity event yet, display `idle` as the conservative default. The agent process is alive, but no current work has been observed.
- Clear the live activity state on:
  - active `sessionId` change,
  - `session/exit`,
  - `endChat`,
  - `resumeChat`,
  - `archiveChat`,
  - archived/read-only route.

No user-input or output correlation is needed in this simplified model. Output activity drives the forwarder's `busy`/`idle` projection directly.

## Step 3: Combine Activity with Lifecycle

Files: `src/pages/RunnerChat.tsx`

- Derive a direct-chat display status from persisted lifecycle plus live activity:
  - `status === "stopped"` -> `stopped`
  - `status === "crashed"` -> `crashed`
  - `status === "running" && latestActivity === "busy"` -> `busy`
  - `status === "running"` -> `idle`
- Keep the existing resuming overlay/button as a transitional control state, but do not add `resuming` to the steady direct-chat status model.
- Stop/Resume/session lifecycle behavior stays unchanged.

## Step 4: Reflect on the Chat Status Control

Files: `src/pages/RunnerChat.tsx`

- Update the existing header status pill to show `busy`, `idle`, `stopped`, or `crashed`.
- Recommended visual:
  - `busy`: existing accent/live treatment.
  - `idle`: muted accent or neutral treatment; the process is healthy, just settled.
  - `stopped`: existing neutral stopped treatment.
  - `crashed`: existing danger treatment.
- Keep Stop available for both `busy` and `idle`; the session is live in both states.
- Do not show `SessionEndedOverlay` for `idle`.
- Do not change the sidebar in this pass. Sidebar badges/counts remain out of scope.

## Step 5: Tests

Rust:

- Unit-test in `src-tauri/src/session/manager/tests.rs` that a direct session receiving `RuntimeOutput::StatusTransition { state: Busy }` emits `session/status` with `state: "busy"`.
- Unit-test that a direct session receiving `RuntimeOutput::StatusTransition { state: Idle }` emits `session/status` with `state: "idle"`.
- Unit-test that a mission session still appends `runner_status` and does not emit the live direct-chat event.

Frontend:

- Typecheck is the main guard unless a component-test harness already exists for `RunnerChat`.
- Prefer a small pure helper for status derivation if the inline state logic starts getting hard to reason about.

Manual:

1. Start a direct chat.
2. After the initial TUI settles, the status can show `idle`.
3. Send a prompt.
4. While the agent is streaming, the status shows `busy`.
5. After the agent finishes and the forwarder idles, the status shows `idle`.
6. Stop the chat; status shows `stopped`.
7. Resume the chat; no stale `idle` state survives the resume transition.
8. Switch between two chats; live activity state is scoped to the correct session id.

## Validation

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo fmt --check`
- `cargo test -p runner direct_chat_status_transition_emits_session_status`
- `cargo test --workspace`

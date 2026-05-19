# Direct chats — multi-chat-per-runner + agent-native session resume

> Post-MVP follow-up to PR #23. Reframes "direct chat" from "the live PTY a
> runner currently has" to "a named conversation with that runner that
> persists across app restarts and can be resumed later". Multi-chat per
> runner falls out for free.
>
> Companion to `docs/impls/0001-v0-mvp.md` (the umbrella plan) and
> `docs/arch/v0-arch.md`. Lives in its own file because (a) it changes the
> direct-chat lifecycle model, not just the UI, and (b) parts of it
> supersede `lookup_prior_agent_session_key` from the resume work that
> already landed on PR #23.

## Why

Two real problems that emerged after the agent-native resume work landed:

1. **One direct chat per runner is the wrong shape.** The UX feels like a
   chat app — multiple parallel conversations with the same agent are
   normal, the way DMs work. The current sidebar tray with a single
   `@handle direct` row per runner can't express this and forces users to
   end one chat to start another.

2. **The resume scope was wrong.** `lookup_prior_agent_session_key`
   scoped direct-chat resume by `runner_id` alone — under multi-chat
   that means clicking *any* stopped chat for runner X resumes whichever
   chat happened to have the newest `started_at`. The user clicked
   "chat A", they expected chat A.

Both problems collapse into one fix: stop treating "direct chat" as a
property of the runner. A direct chat *is* a session row, with its own
identity, its own `agent_session_key`, and a lifetime that survives
across PTY respawns.

## What we're not doing

- **No new tables.** `sessions` already carries everything we need —
  `id`, `runner_id`, `mission_id`, `agent_session_key`, `status`,
  `archived_at`. The earlier sketch with `parent_session_id` /
  `chat_id` / chains was solving a problem that doesn't exist if a
  session row's lifetime spans respawns.
- **No `lookup_prior_agent_session_key`.** Removed. The resume path
  reads `agent_session_key` off the row being respawned.
- **No reply threading, no cross-chat memory, no transcript persistence.**
  Direct chats remain off the event bus — the C6 scrollback ring + the
  agent's own session state is the only memory.
- **No rename UI in this chunk.** `title` ships as a column with a
  derived default; the rename affordance is a follow-up.

## Model

A direct chat **is** a `sessions` row with `mission_id IS NULL`. The row
is the chat's stable identity; its lifecycle is:

```
spawn ──► running ──► stopped ──► (resume click) ──► running ──► stopped ──► …
                                                                      │
                                                                  archive
                                                                      ▼
                                                                 hidden from
                                                                 SESSION tray
```

Same row throughout. Same `id`. Same `agent_session_key`. On every
resume the row is updated in place: `status = 'running'`, new `pid`,
new `started_at`. If the agent CLI hands us a refreshed key on resume,
we overwrite the column.

Mission-scoped sessions are unchanged — `mission_id` is set, the row is
created at `mission_start` and reaped at `mission_stop`, no resume
affordance (mission stop is final).

## Schema delta

Already shipped in PR #23 (or queued):
- 0002 — `sessions.agent_session_key TEXT NULL`.
- 0003 — `sessions.archived_at TEXT NULL`.

This chunk adds:
- 0004 — `sessions.title TEXT NULL`.

The UI defaults the label to `@<handle> · <relative-time>` when `title`
is NULL. A future chunk wires a rename affordance that sets `title`.

## Backend changes

### Remove

- `lookup_prior_agent_session_key` and the `ScopeKey` enum in
  `src-tauri/src/session/manager.rs`. Callers fetch the row directly.

### Add

- `SessionManager::resume(session_id)` — respawn a PTY for an existing
  `sessions` row. Reads the row, runs the runtime adapter with the
  stored `agent_session_key` (or generates a fresh one for claude-code
  if it's NULL), updates the row in place (`status='running'`, new
  `pid`, `started_at = now`, `stopped_at = NULL`, persists any new
  agent_session_key), starts the reader thread. Refuses if the row is
  already running, or if `mission_id IS NOT NULL` (mission sessions
  don't resume — `mission_start` owns their lifecycle).
- Tauri command `session_resume(session_id)`.
- Tauri command `session_rename(session_id, title)` *(deferred to
  follow-up; ships when the UI affordance lands)*.

### Refactor

- `SessionManager::spawn_direct` — keep its current shape. It still
  creates a new direct-chat row. The spawn path stops calling
  `lookup_prior_agent_session_key`; it just lets the runtime adapter
  generate a fresh `agent_session_key` (claude-code) or leave it NULL
  (codex pre-capture). New chat = new row, period.
- `SessionManager::spawn` (mission) — same change: drop the lookup, let
  the adapter self-assign. Mission resume across mission_stop/start is
  out of scope for this chunk; it's the next-level question and is
  deferred.

### Sidebar SESSION query

`session_list_recent_direct` becomes a flat list — one row per direct
session, not per runner:

```sql
SELECT s.id, s.runner_id, r.handle, s.status, s.title,
       s.started_at, s.stopped_at,
       (s.agent_session_key IS NOT NULL) AS resumable
  FROM sessions s
  JOIN runners r ON r.id = s.runner_id
 WHERE s.mission_id IS NULL
   AND s.archived_at IS NULL
 ORDER BY CASE WHEN s.status = 'running' THEN 0 ELSE 1 END,
          COALESCE(s.stopped_at, s.started_at) DESC
```

Running sessions sort first; within each band, most recently active
first.

## Frontend changes

### Sidebar (`src/components/Sidebar.tsx`)

- Replace `runner_activity`-driven `active` state with a list driven by
  `session_list_recent_direct`. Refetch on:
  - `session/exit` — live → stopped flip.
  - `runner/activity` — new spawn or kill.
- One row per chat (no per-runner collapsing). Label is
  `title ?? "@${handle} · ${relativeTime(started_at ?? stopped_at)}"`.
- Status dot:
  - `running` → `bg-accent` (current behavior).
  - `stopped` / `crashed` → `bg-fg-3` (muted).
- Click handler: always navigate to `/runners/<handle>/chat` with
  `state: { sessionId, sessionStatus }`. Auto-resume on click was
  considered and rejected — it conflated "I want to look at this
  chat" with "I want to relaunch the agent". The chat surface owns
  the running/stopped UI: a stopped row lands on a dimmed terminal
  with a Session ended overlay, and the user explicitly clicks
  **Resume** there. Passing `sessionStatus` through navigation state
  is what lets RunnerChat's attach path seed the pane with the row's
  real status without waiting for `chatMeta` to round-trip.

### Runner Detail "Chat now"

Always creates a new chat (new row). To resume an existing chat, use
the SESSION sidebar row. This matches the chat-app analogy and avoids
the "did I want a new conversation or to continue the last one?"
ambiguity.

### RunnerChat page

Three entry modes:
- `state.sessionId` set, `sessionStatus = "running"` → attach to the
  live PTY. Used by the sidebar (running rows) and the Runners page's
  chat pill.
- `state.sessionId` set, `sessionStatus = "stopped" | "crashed"` →
  attach with the pane seeded as stopped. The dim-terminal +
  Session ended overlay renders, and the user clicks **Resume** to
  drive `session_resume` from inside the chat page (with the cyan
  "resuming" transitional state and centered loader pill).
- `state.runnerId` set → spawn fresh (`session_start_direct`). New
  chat row.

The sidebar deliberately does not pre-resume — Resume is an explicit
gesture inside the chat surface, where the cyan transitional state
(`Pencil node GZhHO`) is wired to the user's intent.

## Migration steps (chunk order)

Each step lands as its own commit on the same PR; they're tightly
coupled and not worth splitting across PRs.

1. **0004 migration + `title` column.** No behavior change yet.
2. **`SessionManager::resume` + `session_resume` command.** Backend
   only; no UI consumer yet.
3. **Drop `lookup_prior_agent_session_key`.** Update `spawn` and
   `spawn_direct` to call the runtime adapter without a `prior_key`.
   Verify mission spawn still works (resume across mission stop/start
   was the only behavior we lose; mission resume is out of scope).
4. **`session_list_recent_direct` flat-list rewrite.**
5. **Sidebar SESSION tray rewrite.** Per-chat rows, status-aware dot.
   Click navigates to the chat with `state.sessionId` +
   `state.sessionStatus`; resume is an explicit gesture inside the
   chat surface, not auto-fired on click.
6. **RunnerChat resume affordance.** Stopped chats land on the
   dimmed-terminal + Session ended overlay. The Resume button drives
   `session_resume` in place and runs the cyan transitional state
   (canvas wipe → centered loader → agent repaints).
7. **Tests.** DB query test for the flat list, runtime adapter still
   covered by existing tests, integration test for spawn → stop →
   resume that asserts the same `id` and `agent_session_key` survive.

## Definition of done

- Two parallel chats with the same runner (claude-code) work — closing
  one doesn't affect the other.
- Stopping a chat keeps it visible in the SESSION tray with a muted
  dot.
- Clicking a stopped chat resumes the agent conversation, lands on the
  same `sessions.id`, and the agent CLI continues where it left off.
- Closing the app and reopening preserves both chats; resuming each
  one continues its respective conversation.
- Mission sessions are unaffected — `mission_start` / `mission_stop`
  semantics unchanged.
- Archived chats (a session with `archived_at IS NOT NULL`) don't
  appear in the SESSION tray.

## Open questions / deferred

- **Mission session resume across mission_stop/start.** The 0002 work
  laid the groundwork (every mission session has an
  `agent_session_key`), but this chunk doesn't expose a UI for it.
  Could route through the same `session_resume` command once we
  decide the mission-side UX (per-slot resume? whole-mission resume?).
- **Codex post-spawn key capture.** Implemented in
  `src-tauri/src/session/codex_capture.rs`. After a fresh codex
  spawn or resume that doesn't already have a key, a bounded
  background watcher polls `$HOME/.codex/sessions/<yyyy>/<mm>/<dd>/`
  for the rollout file whose first-line `session_meta` payload's
  `cwd` and `timestamp` match this spawn, and writes
  `payload.id` into `sessions.agent_session_key`. Falls back to
  `std::env::current_dir()` when the spawn didn't set an explicit
  cwd (the child inherits parent's cwd in that case). Best-effort:
  if the rollout never appears (codex crashed, rollouts disabled),
  the row keeps a NULL key and codex falls back to fresh spawn —
  the `resumable` flag tracks this honestly.
- **Rename UI.** Schema column ships now; affordance ships later.
- **Archived workspace surface.** The destination for archived
  sessions and (eventually) archived missions. See
  `docs/impls/0001-v0-mvp.md` "Out of scope for MVP".
- **Per-runner grouping in the sidebar.** Started flat; if the tray
  gets noisy with many chats per runner, can collapse later. Don't
  preempt.

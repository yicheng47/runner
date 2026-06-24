# Codex Session-Key Capture Hardening

## Status

Shipped for issue [#229](https://github.com/yicheng47/runner/issues/229).

## Problem

Codex resume can bind a Runner session row to the wrong native Codex conversation. The resume argv path is already correct: when `sessions.agent_session_key` is set, `router::runtime::resume_plan("codex", Some(key))` builds `codex resume <uuid>`. The weak point is the post-spawn capture of that key.

Codex does not currently expose a fresh-spawn flag for caller-assigned session ids. Runner therefore watches `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`, reads the first `session_meta` row, and persists `payload.id` into `sessions.agent_session_key`. Today that watcher matches by cwd and spawn timestamp, then picks the earliest unclaimed rollout. That is not strong enough when several Codex sessions start close together in the same cwd: if rollout files flush out of spawn order, a watcher can claim a sibling session's native Codex id. The next resume faithfully resumes the wrong conversation.

The safe invariant is: stale or missing resume is acceptable; wrong resume is not.

Claude Code is intentionally separate: Runner can assign the native Claude session id at fresh spawn with `--session-id <uuid>` and later resume it with `--resume <uuid>`. The filesystem capture and prompt-marker workaround in this plan are Codex-only because Codex lacks the equivalent fresh-spawn id flag.

## Goals

- Persist a Codex `agent_session_key` only when capture is unambiguous for that Runner session row.
- Keep `codex resume <uuid>` using the key from the exact row being resumed.
- Fail closed: leave `agent_session_key = NULL` when capture cannot prove ownership.
- Expose the captured key in the direct-chat side panel for debugging and operator confidence.

## Non-Goals

- Do not introduce per-session `CODEX_HOME` yet. It is cleaner in theory, but Codex home carries auth, config, plugins, skills, sqlite state, caches, and session indexes. A per-session home would require a fragile overlay/symlink strategy and is too large for this bug.
- Do not parse conversation content or user prompts to infer identity.

## Proposed Backend Design

### Capture Inputs

Extend Codex capture so the watcher gets more than cwd and timestamp:

- Runner `session_id`
- resolved spawn cwd
- spawn timestamp
- spawned process pid, when available from `SessionRuntime::status`
- optional Runner prompt marker derived from the Runner `session_id`
- database pool

`PtyRuntime::spawn` already stores the child pid in `SessionStatus.pid`. The manager can query `runtime.status(&rt_session)` after spawn and before launching the capture watcher. If pid is unavailable, capture must either use a conservative fallback or leave the key null.

Start a bounded capture attempt after every fresh Codex spawn/resume without an existing `agent_session_key`. Keep that spawn-time attempt because mission starts often deliver the first turn through argv and Codex may create the rollout immediately. Also keep the capture context on the live `SessionHandle` and retry capture after submitted input (`Enter`/paste-submit), which covers empty direct chats where Codex does not create a rollout file until the user sends a real prompt.

When Runner already has a real first-turn body for a fresh Codex spawn, append an inert marker such as `<!-- runner-codex-session-key-capture:<runner-session-id> -->`. Codex records that prompt text into the rollout. If pid-assisted ownership is unavailable and several rollout files share the same cwd/time window, scan matching rollout content for the exact marker and persist only the rollout containing this session's marker. Do not add a marker-only prompt when there is no real first turn, because that would make Codex respond to internal metadata.

### PID-Assisted Rollout Ownership

Prefer pid-assisted matching over earliest-unclaimed matching.

On macOS, the capture thread can inspect the spawned Codex process with `lsof -p <pid>` or an equivalent platform helper and look for an open rollout file under `$CODEX_HOME/sessions/**/rollout-*.jsonl`. Once found, parse the file's first line and validate:

- first JSON line is `type == "session_meta"`
- `payload.cwd == resolved_cwd`
- `payload.timestamp >= started_at`
- `payload.id` is a UUID
- path is not already claimed by another watcher

Only then write `payload.id` into `sessions.agent_session_key`.

If there are zero candidate rollout files, keep polling until timeout. If there is more than one candidate, do not guess; keep polling briefly for convergence, then leave the key null.

If no pid-owned file is observable but the spawn prompt carried a Runner marker, the content-marker match is the next ownership proof. This is the mission burst workaround: multiple Codex workers can start in the same cwd at the same millisecond, but each first-turn prompt contains a different Runner session marker.

### Fallback Behavior

The existing cwd/timestamp scan can remain only as a guarded fallback, but it must not silently choose among multiple plausible sibling rollouts. Accept a fallback match only when exactly one unclaimed rollout matches the cwd/time window and Runner's database proves the current row is live, unkeyed, still has the spawn's `started_at`, and no other live unkeyed Codex row could share the effective spawn cwd. Treat `NULL` stored cwd as inherited cwd, so a `NULL` sibling and an explicit sibling matching the effective cwd are ambiguous. If any of those checks fail, leave the key null.

This changes the failure mode from "may resume the wrong conversation" to "may start fresh on resume." That is the right tradeoff.

In multi-runner missions where several Codex sessions start in the same cwd at nearly the same time, fallback may leave every affected row as `NULL` if pid-assisted ownership is unavailable. Do not map by slot order or rollout timestamp order; that would reintroduce sibling cross-wiring. A future reliable mission capture path needs caller-assigned Codex session ids or isolated per-session Codex homes.

### Tests

Add focused tests around `codex_capture`:

- Multiple matching rollouts in the same cwd are ambiguous and do not produce a key.
- Claimed rollout paths are skipped.
- A single matching rollout still captures.
- Invalid/non-UUID `payload.id` is rejected.

Add a `SessionManager::resume` test for Codex:

- Insert a stopped Codex direct-chat row with `agent_session_key = <uuid>`.
- Resume it through the fake runtime.
- Assert the spawned `SpawnSpec.args` contain `["resume", "<uuid>", ...]` for that same row and do not use another row's key.

## Direct-Chat And Mission Session Visibility

Add the native session key to the direct-chat info panel and mission runner-session rail.

Backend/API shape:

- Add `agent_session_key: Option<String>` to `DirectSessionEntry`.
- Include `s.agent_session_key AS agent_session_key` in `session_get`.
- For `session_list_recent_direct`, either include the same field or return `NULL AS agent_session_key` and have `RunnerChat` fetch full metadata with `session_get` for the active chat. Prefer the latter if we want to avoid sending every recent chat's key to the sidebar list.
- Add `agent_session_key: Option<String>` to mission `SessionRow` and include it in `session_list`.

Frontend:

- Add `agent_session_key: string | null` to `src/lib/api.ts`.
- In `src/pages/RunnerChat.tsx`, render a `session_key` row in the Runtime card and show the captured key or `NULL`.
- In `src/components/RunnersRail.tsx`, render `session_key` for each mission runner session and show the captured key or `NULL`.
- In `src/pages/MissionWorkspace.tsx`, use a bounded `session_list` refresh while running Codex mission rows are missing keys, and listen for a backend `session/updated` event after successful async capture so late first-activity capture refreshes the rail too.
- In `src/pages/RunnerChat.tsx`, listen for the same `session/updated` event and refetch the active chat metadata when its key is captured.
- Use the same compact mono, break-all treatment as `cmd` and `cwd`.

Expected UI:

```text
Runtime
Codex  CODEX

cmd          codex
cwd          /Users/jason/go/src/github.com/yicheng47/runner
session_key 019eef80-62fd-7c50-b451-bffce763f3fc
```

Pending/failed capture:

```text
session_key NULL
```

## Relevant Code

- `src-tauri/src/session/codex_capture.rs`: rollout scanning and `agent_session_key` persistence.
- `src-tauri/src/session/manager/spawn.rs`: starts Codex capture after fresh spawn/resume without a prior assigned key.
- `src-tauri/src/session/pty_runtime.rs`: exposes child pid through `SessionRuntime::status`.
- `src-tauri/src/router/runtime.rs`: builds `codex resume <uuid>`.
- `src-tauri/src/commands/session.rs`: direct-chat DTO and SQL queries.
- `src/lib/api.ts`: `DirectSessionEntry` frontend type.
- `src/pages/RunnerChat.tsx`: direct-chat side panel.

## Validation

- `cargo test -p runner codex_capture`
- Focused `SessionManager::resume` test for Codex argv binding.
- `pnpm exec tsc --noEmit`
- `pnpm run lint`

Manual smoke:

1. Start two Codex direct chats in the same cwd close together.
2. Confirm each direct-chat side panel shows either its captured `session_key` or `NULL` while capture is pending/failed.
3. Stop and resume each chat.
4. Confirm a keyed chat resumes its own conversation; an unkeyed chat starts fresh rather than attaching to a sibling conversation.

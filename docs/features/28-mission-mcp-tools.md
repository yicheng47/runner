# 28 — Mission MCP tools

> Tracking issue: [#206](https://github.com/yicheng47/runner/issues/206)

## Motivation

Runner MCP currently covers workspace configuration: crews, runners, and slots. That is useful for setup, but the highest-value external-agent workflow is operating and monitoring live missions. A Codex or Claude Code session should be able to ask Runner what is running, inspect a mission feed, see runner health, and perform basic lifecycle actions without switching to the app UI.

This makes Runner MCP useful for the main loop: "start the Build squad on this goal", "check whether the reviewers are idle", "show me the latest mission events", "stop/archive this mission". Configuration CRUD remains the foundation; mission tools are the operational layer.

## Scope

### In scope

- **Mission read tools:**
  - `mission_list` — list non-archived missions, optionally filtered by crew.
  - `mission_get` — fetch one mission by ID.
  - `mission_list_summary` — return the sidebar-style mission summary, including crew name, pending ask count, and whether any session is live.
  - `mission_feed` — return mission events from the NDJSON log, newest-first or oldest-first, with a limit and optional cursor/since offset if the existing event model supports it cleanly.
  - `mission_status` — return one mission's operational status in one compact object: mission row, crew, sessions, latest runner status by handle, pending asks, live/stopped session counts, and recent warnings.
- **Mission lifecycle tools:**
  - `mission_start` — start a mission for a crew using the same `StartMissionInput` semantics as the app.
  - `mission_stop` — stop a running mission.
  - `mission_archive` — archive a mission.
  - `mission_pin` — pin/unpin a mission.
  - `mission_rename` — rename a mission.
  - `mission_reset` — reset/restart a mission using the same guarded behavior as the UI.
- **Human interaction tool:**
  - `mission_post_human_signal` — post a human message/signal into a mission, matching the existing Tauri command behavior.
- **Session monitoring support used by mission status:**
  - Expose session rows for a mission through the status response instead of adding a broad session-control surface first.
  - Preserve the current product rule that individual slot resume is not the primary workflow; mission-level lifecycle remains the MCP surface.
- **Events and UI sync:**
  - Mutating MCP calls emit the same Tauri events the UI relies on today.
  - Mission feed/status tools read from the same event log and router/session state used by the app; no parallel cache.

### Out of scope

- Direct PTY streaming over MCP. The first version returns event-log feed data and status snapshots, not terminal byte streams.
- Fine-grained per-session control such as resizing PTYs, injecting stdin into one slot, or killing one runner. Keep the first mission MCP surface mission-oriented.
- Subscribing to live feed updates. Pollable snapshot tools are enough for v1; streaming/subscription can follow once MCP client behavior is clearer.
- Remote/non-local access controls. This stays under the existing local MCP binding model.

## Implementation Phases

### Phase 1 — Mission read tools

- Add `src-tauri/src/mcp/tools/mission.rs` and merge its router in `src-tauri/src/mcp/tools/mod.rs` / `server.rs`.
- Wrap existing command-layer helpers first: `mission::list`, `mission::get`, `mission::read_events`, and `mission_list_summary`.
- Define compact MCP DTOs only where the UI command shape is too chatty for an external agent, especially `mission_status`.
- Add tests that the tool registry includes the mission read tools and that `mission_feed` returns the same ordered events as `mission::read_events`.

### Phase 2 — Lifecycle tools

- Wrap `mission_start`, `mission_stop`, `mission_archive`, `mission_pin`, `mission_rename`, and `mission_reset` through the same command-layer functions used by Tauri.
- Ensure all mutation tools emit existing UI events and update open mission workspaces without reload.
- Keep error behavior identical to the UI path: unlaunchable crew, already-stopped mission, reset/archive constraints, and spawn failures should surface as MCP errors with the existing messages.

### Phase 3 — Operational status snapshot

- Implement `mission_status` as the main monitoring tool for external agents.
- Include mission metadata, crew metadata, session rows, per-handle latest runner status, pending asks, last event cursor/offset if available, and recent mission warnings.
- Prefer reconstruction from durable state and event log so the tool works after app restart and for paused/stopped missions.

### Phase 4 — Smoke tests and docs

- Add a short MCP smoke-test section for mission tools: start a mission, read status, read feed, stop/archive.
- Verify the resilient `runner-mcp` proxy still advertises the new mission tools when Runner.app is closed.
- Update any MCP feature docs that enumerate the full tool list.

## MCP smoke flow

With Runner.app open and a launchable crew ID available:

1. `mission_start` with `{ "crew_id": "<crew-id>", "title": "MCP smoke", "goal_override": "verify mission MCP tools" }` creates a running mission and returns `{ mission, goal }`.
2. `mission_status` with `{ "id": "<mission-id>" }` returns the mission row, crew row, session rows, live/stopped counts, pending asks, latest runner status by handle, recent warnings, and the latest feed offset.
3. `mission_feed` with `{ "mission_id": "<mission-id>", "order": "oldest_first", "limit": 10 }` includes the opening `mission_start` and `mission_goal` events.
4. `mission_stop` with `{ "id": "<mission-id>" }` kills live mission sessions while leaving the mission row running and visible as paused.
5. `mission_archive` with `{ "id": "<mission-id>" }` appends `mission_stopped`, marks the mission completed/archived, and removes it from active mission lists.

With Runner.app closed, `runner-mcp` should still list all mission tools through its fallback registry, while mission tool calls return the existing retryable "Open Runner.app" MCP error.

## Verification

- [ ] With Runner open, `mission_list` returns the same non-archived missions as the app sidebar/Missions list.
- [ ] `mission_feed` returns the opening `mission_start` and `mission_goal` events for a newly started mission.
- [ ] `mission_status` shows live/stopped session counts and latest runner status per handle after agents emit activity.
- [ ] `mission_start` from MCP creates a mission, spawns sessions, and the open UI updates without refresh.
- [ ] `mission_stop` from MCP stops the mission and app UI reflects the paused/stopped state.
- [ ] `mission_archive` from MCP removes the mission from active lists and does not leave live sessions behind.
- [ ] When Runner.app is closed, the resilient MCP proxy still lists the mission tools and mission tool calls return a retryable "open Runner" MCP error.
- [ ] `cargo test -p runner commands::mcp` and mission MCP tests pass.
- [ ] `pnpm exec tsc --noEmit` and `pnpm run lint` pass if frontend docs/settings copy changes.

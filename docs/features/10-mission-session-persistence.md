# 10 — Persist mission sessions across app restart

> Tracking issue: [#115](https://github.com/yicheng47/runner/issues/115)

## Motivation

The tmux runtime migration (impl 0004) was sold on the same restart-survives
win for all sessions, but only direct chats actually get it. Mission sessions
follow an asymmetric path on app start:

| Pane state on app start | Direct chat | Mission session |
|---|---|---|
| Pane alive | Re-attach: rebuild SessionHandle + forwarder, replay scrollback via `capture-pane`, **same agent process keeps running**. | **Kill the pane** (`runtime.stop`), mark the row stopped. User must click resume from the workspace, which spawns a fresh agent. |
| Pane dead | Read `pane_dead_status`, mark stopped/crashed. | Same. |

The carve-out lives in `src-tauri/src/session/manager.rs:1675-1703`. The reason
documented inline is real: the mission bus + router only mount on
`mission_attach` from the workspace UI, and `router::mod` doesn't replay stdin
side effects on reconstruction. So reattaching the pane without the bus would
silently drop `ask_lead` / `human_said` / `runner_status` events the agent
appends to the NDJSON log between app restart and workspace mount.

The fix is to make the bus mount eager instead of lazy. The event log is the
source of truth; the bus is a tail + fanout. Nothing about its lifecycle has to
wait for the workspace UI — that coupling is a leftover from when the bus was
created as part of the workspace mount path.

User-facing cost today: quit Runner mid-mission, agents die. In-flight tool
calls evaporate. Lead/follower coordination state is lost. The "agents running
unattended for hours" product story doesn't survive a single app quit. Power
users learn to keep Runner open; everyone else loses work.

## Scope

### In scope (v1)

- **Keep mission panes alive on app restart.** Drop the `is_mission` carve-out
  in `reattach_running_sessions`. Mission sessions take the same alive-pane
  path as direct chats: rebuild SessionHandle, install fresh `pipe-pane`,
  replay scrollback.
- **Eager mission bus + router mount on app startup.** New
  `mission_reattach_all` (or fold into `reattach_running_sessions`) walks every
  `running` mission row and runs the same logic `mission_attach` runs today —
  rebuild Router from the slot roster, mount the bus, register stdin
  injectors. Runs before session reattach so the bus is ready when the
  forwarder threads start emitting.
- **`mission_attach` stays idempotent.** The workspace mount call becomes a
  no-op when the bus is already mounted (the current `if state.routers.get(…)`
  guard already handles this — verify nothing else in the function has a side
  effect we need to preserve, and split out any UI-only setup).
- **Session resume UX unchanged for the actually-dead case.** If a mission
  pane died while Runner was down (process crashed, OS reboot killed tmux,
  user wiped tmux state), the row still flips to stopped/crashed and the user
  clicks resume from the workspace as today. The eager mount path is only
  about the alive-pane case.

### Out of scope (deferred)

- **Transcript-based resume when the tmux server is gone.** OS reboot, OOM,
  `pkill tmux`, kernel update — anything that kills the tmux server wipes
  every pane. Most agents (claude, codex) expose their own `--resume` /
  session-file primitive that Runner could call to re-spawn an agent with
  conversation context intact. Worth a separate spec; out of scope here
  because resume semantics differ per agent and would balloon the surface
  area. Track as a follow-up once this lands.
- **UI persistence-status badge.** A small indicator on the mission card
  showing "this mission will survive restart" once the eager-mount path
  ships. Nice for transparency but not blocking — once persistence is
  uniform, the badge is closing a gap that no longer exists.
- **History limit bump.** `history-limit 50000` is fine for almost every
  mission; very long unattended runs could blow past it. Separate config
  change, not blocking.

### Key decisions

1. **Reuse `mission_attach`'s logic, don't fork it.** The function that
   rebuilds the Router/Bus on workspace mount is exactly the function the
   startup path needs to call. Extract the bus-mount body into a private
   helper that both `mission_attach` (frontend-triggered) and a new startup
   reconciler call. No new code path for "startup attach" — just a different
   caller.
2. **Eager mount runs BEFORE session reattach.** Order matters: forwarder
   threads start emitting `session_output` as soon as `pipe-pane` is
   installed. If the bus isn't mounted yet, those bytes go to a non-existent
   subscriber and events written by the agent between server-keepalive and
   bus-mount get fanout-dropped. The NDJSON log is fine (the agent writes
   straight to disk via the bundled `runner` CLI), but Tauri-side `mission_*`
   events would be missed. So: mount bus → then reattach panes.
3. **Don't change the resume UX for dead missions.** Workspace-driven
   `session_resume` keeps the same shape. We're only changing what happens
   when the agent process is still alive — the dead-process flow is
   unaffected. This keeps the diff scoped and avoids touching the
   agent-resume contract.
4. **No frontend changes required.** The workspace UI already calls
   `mission_attach` on mount and the function is idempotent. After this
   change it'll find the bus already mounted and return immediately. The user
   experience is "I reopened Runner and my agents are still where I left
   them" — no new affordances needed.

## Implementation phases

### Phase 1 — extract bus mount from `mission_attach`

- Pull the bus + router setup body from `commands/mission.rs:mission_attach`
  into a private function on `AppState` (or a new `mission::attach_internal`
  helper). Signature roughly:

  ```rust
  pub(crate) async fn ensure_mission_router_mounted(
      state: &AppState,
      app: &tauri::AppHandle,
      mission_id: &str,
  ) -> Result<()>;
  ```

- Keep `mission_attach`'s outer shape (Tauri command, returns `Mission`,
  idempotent guard). It calls the helper for the side effect, then returns
  the loaded mission row.
- All existing tests against `mission_attach` should pass unchanged.

### Phase 2 — eager mount on app startup

- New `mission::reattach_all_running_missions(state, app)` in
  `commands/mission.rs` (or a sibling module). Queries `SELECT id FROM
  missions WHERE status = 'running' AND archived_at IS NULL`, iterates,
  calls `ensure_mission_router_mounted` for each, logs errors per mission
  without aborting the loop.
- Call site: `src-tauri/src/lib.rs::run`, immediately before
  `sessions.reattach_running_sessions(...)`. The `AppState` and
  `AppHandle` are both already in scope there.
- Failure mode: if `ensure_mission_router_mounted` fails for one mission
  (corrupt log, missing slot, etc.), log + skip — that mission falls back to
  the "stop and let the user resume" path when session reattach hits its
  rows. Per-mission isolation, no startup hang.

### Phase 3 — drop the mission carve-out in session reattach

- `src-tauri/src/session/manager.rs:1673-1704`: delete the
  `if is_mission { stop; mark_stopped; return; }` block in the alive-pane
  arm. The direct-chat branch (`attach_existing` → on failure: orphan-stop)
  becomes the only branch for alive panes.
- Update the inline doc comment that explains why missions are killed — it
  should now read as historical context for the migration, or just be
  removed.
- Adjust the unit test
  `reattach_running_sessions_kills_mission_panes_to_avoid_routing_drift` —
  it currently asserts the kill behavior. Rename to
  `reattach_running_sessions_reattaches_live_mission_panes` and assert the
  inverse: pane stays alive, SessionHandle is rebuilt, row stays `running`.
- Verify the dead-pane and missing-pane paths still flip the row to
  stopped/crashed correctly for mission sessions (the carve-out only fires
  for the alive arm, but double-check the unit test coverage matches).

### Phase 4 — verification + smoke

- Backend tests: `cargo test -p runner-lib session::manager::tests` covers
  the reattach unit tests; new test for `mission::reattach_all_running_missions`
  using the same `FakeRuntime` test stand-in.
- Integration: gated `cargo test -- --ignored` test that spawns a real tmux
  session, runs a stub agent that appends an `ask_lead` event after a
  simulated restart window, asserts the event is fanout-emitted to a
  subscribed `BusEvents` mock.
- Manual smoke (packaged app):
  1. Start a mission with two slots, send a prompt to the lead.
  2. While the lead is mid-response, quit Runner from the menu bar.
  3. Reopen Runner. Mission should appear `running` in the sidebar. Click
     into the workspace.
  4. Lead's response should still be streaming (or completed) in the feed.
     No "session crashed" badge. No resume prompt.
  5. Quit again, kill the tmux server externally (`tmux -L runner
     kill-server`), reopen. Mission should now show stopped with each slot
     marked stopped; clicking resume on a slot spawns a fresh agent (today's
     behavior).

## Verification

- [ ] Quitting Runner mid-mission and reopening preserves agent processes;
      no resume prompt, no lost in-flight tool calls.
- [ ] Events the agent appends to the NDJSON log between restart and
      workspace mount are fanout-emitted to Tauri subscribers once the
      workspace opens (no silent drops).
- [ ] Killing the tmux server externally still surfaces stopped/crashed
      state per slot with the resume affordance, as today.
- [ ] `mission_attach` is a no-op when the bus is already mounted from the
      startup path — no double-mount, no duplicate router registration.
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit` clean.
- [ ] Manual smoke: two-slot mission, mid-response quit + reopen, lead
      stream continues, no user action required.

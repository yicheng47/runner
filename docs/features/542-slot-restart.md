# 542 — Stop, resume, and restart a single mission slot

> Tracking issue: [#542](https://github.com/yicheng47/runner/issues/542) (bug)
> Priority: P1.
> Design: `design/runner.pen`, MISSIONS row "mission — per-slot stop / resume / restart (no confirm), stop-all confirm (542)" (`m4FND`): `Mission workspace — slot actions (542)` (`S2Rwi0`), `slot restarted (542)` (`Te6QB`), `slot stopped · per-slot card (542)` (`ZcePX`), `stop all confirm (542)` (`njeOD`). Signed off 2026-09-10.

## Motivation

Inside a mission the only lifecycle controls are mission-wide: the header **Resume** respawns every stopped slot and **Stop** kills every PTY, and a stopped slot's pane shows the "Mission paused" card with mission-wide Resume and Archive. There is no way to stop, resume, or restart one slot. The recovery move for a slot that is parked, hung, or holding a wrong conversation is therefore Stop all, then Resume all, which cuts off the other slots' turns for nothing.

Resume also cannot give a slot its initial prompt back. `SessionManager::resume_with_fresh_fallback` passes `first_turn = None` by design, which is right when the agent CLI restores its own conversation, but every fresh-spawn case loses the body a cold start would compose: a worker whose claude-code conversation file is gone comes back with nothing (only the lead re-fires its launch prompt, via `ops::session::session_resume` → `Router::fire_lead_launch_prompt`), and a codex or trae slot with no captured `agent_session_key` gets `ResumePlan::fresh()` and boots as a bare agent. Observed on 2026-09-10 when a claude-code reviewer slot parked on the bypass consent dialog (#541): the human's only option was to archive the mission.

The structure already supports the fix. A slot's row and pane are keyed by Runner's session id, not the agent's key; the `/clear` rekey (#459) swaps `agent_session_key` on a live row today. The fresh-spawn path exists (`resume_with_fresh_fallback` degrades to it), the first-turn composers are shared (`router::prompt::compose_launch_prompt` and `compose_worker_first_turn`, the same ones `ops::mission::start` calls), argv delivery of a first turn is the cold-start mechanism (`apply_runtime_args`), and `SessionManager::kill` is synchronous: `runtime.stop` returns only once the child is dead and reaped, then the row is reconciled. The bus history in `events.ndjson` outlives any agent conversation.

## Vocabulary

- **Stop slot** — kill this slot's PTY; the row goes `stopped`, the mission stays running. Today's mission-wide Stop, scoped to one row.
- **Resume slot** — respawn the row and let the agent CLI restore its conversation (`--resume <key>`, `codex resume <key>`). Today's per-row `session_resume`, exposed per slot.
- **Restart slot** — kill if running, then respawn the same row as a fresh conversation with the first turn a cold start would compose for it. The prior conversation is discarded; the bus history is not.

## Scope

### In scope

- **Per-slot controls on the rail card** (`S2Rwi0`). The card header's terminal button goes away (it and the card body both call `select_mission_session`); in its place two 24px icon buttons in the mission header's tinted style (`SessionControl` Header variant: icon only, no border, accent or danger at 80% alpha, 10% tint on hover). A running slot shows **Stop** (red square) and **Restart** (muted `rotate-ccw`); a stopped slot shows **Resume** (green play) and **Restart**. Restart stays muted (`$text-mid`) on a neutral raised hover background (`theme::raised()`); danger tint appears only while pressed or restarting. All three act at once: a per-slot action is a deliberate click on a named card, and the slot's bus history survives a restart, so no confirm.
- **Restart semantics** (backend). One op, `ops::session::session_restart(session_id)`: if the row is running, `SessionManager::kill` first; then the resume path with a new `fresh: bool` that forces `effective_prior_key = None`, composes the slot's cold-start body, and passes it as `first_turn` so it rides argv exactly as at mission start. The body is the lead's launch prompt (`compose_launch_prompt` over the router's `LaunchInputs`, the same view `fire_lead_launch_prompt` reads) or the worker preamble plus brief (`compose_worker_first_turn(runner.system_prompt, crew_addendum)`). claude-code self-assigns a new `--session-id` and the row's `agent_session_key` is replaced through the existing rekey write; codex gets its prompt marker so the capture thread attributes the new rollout as on a cold start; the Windows batch path uses the existing `queue_windows_batch_first_turn` fallback. The lead's paste-based `fire_lead_launch_prompt` is no longer needed for restart and stays only for the launch-time fresh fallback until phase 2 folds it in.
- **The fresh-fallback fix** (the second half of #542). When a Resume degrades to a fresh spawn for any slot and any runtime (claude-code conversation file missing, codex or trae with no captured key), it takes the same fresh path with the same composed first turn instead of booting bare. This replaces the lead-only `fresh_fallback_lead` hook and the `log::warn!("first-turn argv not delivered …")` branch.
- **Tell the lead** (`Te6QB`). A restart appends two events to the mission log: a `signal` from `human` of type `slot_restarted` with `{handle, session_id, prior_agent_session_key}`, rendered in the feed as "you · signal · slot_restarted → @handle · fresh conversation, brief re-sent"; and a `message` from `runner` to the lead: "@handle was restarted by the human and starts over with only its brief. Nothing it was told in this mission survives; re-send the task, branch, and anything else it needs." The router's `message_nudge` already pushes any directed message into the target's PTY, so the lead acts on it. When the lead itself is restarted, the message goes to every other slot instead, and the lead's launch prompt already tells it to `runner msg read`. `SignalType` is an open string, so `slot_restarted` needs no enum change; `KnownSignalType` stays as is.
- **Per-slot stopped card** (`ZcePX`). The stopped slot's pane keeps the bottom-docked `SessionOverlay::ended` card that "Mission paused" uses today, scoped to the slot: pause icon, title "Slot stopped", one paragraph ("@handle's PTY is closed; N other slots are still running. Resume continues its conversation where it left off. Restart discards it and starts over with the brief, the same first turn a cold start gives the slot."), then **▶ Resume slot** (primary) and **↻ Restart slot** (secondary). The card's actions call the per-slot ops. For one running sibling, use "1 other slot is still running". The mission-wide "Mission paused" card remains for the all-slots-stopped case with its Resume and Archive.
- **Stop-all confirm** (`njeOD`). The header Stop button's tooltip becomes "Stop all slots" and the click opens the `ConfirmDialog`: "Stop all N running slots?" counting only live slots ("Stop the running slot?" when N is 1), body "Every slot's PTY is killed and whatever turn it is on is cut off. The mission stays open; each slot can be resumed with its conversation, or restarted with its brief.", Cancel / Stop all. This is the only confirm in the feature: it is the one action that reaches every slot at once. Per-slot Stop, Resume, and Restart never confirm.
- **Rail card state.** A stopped card shows a grey presence dot and "stopped · exit N"; a restarting card shows "starting · fresh conversation" until the first turn lands, then the normal status. The session key row updates to the new key.

### Out of scope

- Preserving the agent conversation across a restart. Restart means a new conversation; Resume is the action that keeps one.
- A new session row per restart. The row is reused, so the router's handle-to-session map, the sidebar node, the tab, and the terminal model are untouched.
- Restarting with a different runtime, model, or effort. Slot overrides stay where they are edited today.
- The fresh-cwd trust dialog and other first-run prompts (#541 and its phase 3 check).
- Lifecycle requests racing the original queued cold-start spawn, including MCP restart before that spawn completes; the existing resume claim does not cover the original spawn queue.
- Any change to the mission-wide Resume beyond it now producing correct first turns through the shared fresh path.

## Implementation Phases

### Phase 1 — backend

- `crates/runner-backend/src/session/manager/spawn.rs`: give `resume_with_fresh_fallback` a `fresh: bool`. When set, or when the plan degrades to fresh, compose the first turn for the slot from the mission context (`mission_ctx` already carries `lead` and `crew_id`; the router's `LaunchInputs` carry roster, goal, addendum, and allowed signals) and pass it to `apply_runtime_args`, `codex_capture_prompt_marker`, and the Windows batch queue as `spawn_mission_session` does. Remove the "first-turn argv not delivered" warning branch and the `fresh_fallback_lead` field on `SpawnedSession`.
- `crates/runner-backend/src/ops/session.rs`: add `session_restart(state, session_id)` = kill if running (`state.sessions.kill`), then resume with `fresh: true`; append the `slot_restarted` signal and the `runner → lead` message through the mission's bus; drop the `fresh_fallback_lead` → `fire_lead_launch_prompt` hook. `session_resume` keeps its signature.
- `crates/runner-backend/src/mcp/tools/session.rs`: expose `session_restart` beside `session_resume` so a crew or a script can restart a slot.
- Tests in `session/manager/tests.rs`: worker restart carries the preamble plus brief on argv; lead restart carries the launch prompt; codex restart carries a fresh prompt marker and no `resume` prefix; claude-code restart passes a new `--session-id` and replaces the stored key; a Resume whose conversation file is missing now delivers the first turn for a worker; a Windows batch runner queues the first turn instead of argv. Router test: the two events land in order and the nudge reaches the lead.

### Phase 2 — app

- `crates/runner-app/src/surfaces/mission_workspace.rs`: rail card actions (Stop / Resume / Restart) with the Header-variant `SessionControl` styling and a new `SessionControlKind::Restart`; the per-slot stopped card replacing "Mission paused" when other slots are live; the stop-all `ConfirmDialog` and "Stop all slots" tooltip; the `slot_restarted` signal row in the feed; "starting · fresh conversation" on the card while the restart is in flight.
- `crates/runner-app/src/ui/session_control.rs`: the Restart kind (muted icon at rest and on neutral raised hover, danger tint while pressed or restarting).
- Tests: the existing `concurrent_resume_errors_match_the_backend_contract` pattern for restart-while-resuming; rail card action set per status; the stop-all confirm's copy carries the slot count.

### Phase 3 — smoke

- Restart a worker mid-mission: the pane relaunches, the first turn lands once, the feed shows the signal and the note to the lead, the lead re-sends the task, the worker replies through the bus.
- Restart the lead: it comes back with the launch prompt and reads the log.
- Stop one slot, keep two running, Resume it: the conversation continues where it left off, no first turn.
- Stop all from the header: the confirm names the running slot count; Cancel leaves everything running.
- Windows on JASONPC: the worker restart with a batch-wrapped runner.

### Phase 4 — docs

- `docs/arch/arch.md`: the mission runtime section gains the three verbs and the rule that a fresh spawn always carries the cold-start first turn.
- 527's archived spec: one line noting that the stall it describes now has a per-slot recovery.

## Verification

- [ ] A running slot's rail card shows Stop and Restart; a stopped slot's shows Resume and Restart; the terminal button is gone and clicking the card still opens the slot's tab.
- [ ] Stop on a card kills only that slot; the other slots' `runner_status` stays busy or idle and the mission stays running.
- [ ] Restart on a worker delivers `compose_worker_first_turn` on argv, replaces `agent_session_key`, appends `slot_restarted` and the `runner → lead` message, and the lead's PTY receives the inbox nudge.
- [ ] Restart on the lead delivers the launch prompt; the note goes to the other slots.
- [ ] Resume on a slot whose conversation is intact delivers no first turn; Resume on a slot whose claude-code conversation file is missing delivers the first turn for a worker as well as the lead.
- [ ] The stopped slot's card offers Resume slot and Restart slot; when every slot is stopped the mission-wide card still offers Resume and Archive.
- [ ] Header Stop opens the confirm and names the count; per-card Stop, Resume, and Restart act immediately.
- [ ] `~/.claude` and `~/.codex` are not written by a restart beyond what a cold start writes.

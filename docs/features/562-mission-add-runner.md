# 562 — Add runners to a running mission

> Tracking issue: [#562](https://github.com/yicheng47/runner/issues/562)
> Priority: P2.
> Status: planned, design first.
> Design: `design/runner.pen`, MISSIONS row: frames to draw before phase 3 — `Mission workspace — add runner (562)`, `add runner modal (562)`, `runner joined · rail card + feed (562)`.

## Motivation

A mission's roster is fixed the moment it starts. `ops::mission::start` snapshots the crew's slots into the per-mission `roster.json` sidecar ("frozen here at mission_start"), `mission_start_impl_with_size` spawns exactly one session per crew slot, `Router::new` bakes the roster into `LaunchInputs`, and the bus's `BusState.handles` is documented as "stable for the mission's lifetime". Once the crew is running, the only way to get another pair of hands is to stop the mission, edit the crew, and start over, which throws away every slot's conversation and the bus history's continuity.

That is the wrong shape for how missions actually go. The lead discovers mid-flight that the task splits three ways instead of two, or that a reviewer is needed that the crew never had; the human watching the feed sees the same thing. Jason asked on 2026-09-11 for the crew to be addable during a mission when either the lead or the human decides more agents are needed. The crew template stays what it is; the *mission's* roster grows.

The machinery mostly exists. A slot spawn is already a two-phase `register_mission_session` → `complete_mission_session_spawn` with a per-mission cancel flag, the router already queues nudges for sessions registered as pending (`register_pending_sessions`), the CLI validates `--to` against the sidecar on every call (no cache), and #542 made every fresh spawn carry its cold-start first turn. What is missing is a mission-scoped slot, a mutable roster in the router and bus, and the two entry points.

## Vocabulary

- **Template slot** — a row in `slots` that belongs to the crew (`mission_id IS NULL`). What the crew page edits, what every new mission starts with.
- **Mission slot** — a row in `slots` that belongs to one mission (`mission_id` set). Created by this feature, never shown on the crew page, deleted with the mission.
- **Mission roster** — template slots at start plus the mission's own slots, in position order. The sidecar, the router, and the bus all hold this list.
- **Add runner** — pick a runner template, give it a slot handle, and spawn it into a running mission as a new worker.

## Scope

### In scope

- **Mission-scoped slots** (backend). `slots` gains a nullable `mission_id` column. `slot::list(crew_id)` returns template slots only, so crew pages and `mission_start` are unchanged. A new `slot::list_for_mission(mission_id)` returns the mission roster: the crew's template slots followed by the mission's own slots, in position order; a mission slot's `position` continues after the last template slot. Handles are unique within the mission roster: a mission slot may not reuse a template handle of its crew or another mission slot of the same mission, but two missions of one crew may each add `@reviewer-2` (the `UNIQUE (crew_id, slot_handle)` constraint becomes partial: one index over template slots, one over `(mission_id, slot_handle)`). Mission slots carry the same `runtime_override`, `model_override`, and `effort_override` columns; only the runtime is settable at add time. `mission_delete` removes them with the sessions. `ensure_mission_router_mounted` builds the router from `list_for_mission`, so a mission with added runners survives an app restart with its full roster.
- **The add op** (backend). `ops::mission::mission_add_runner(mission_id, AddRunnerInput { runner: RunnerRef (id or handle), slot_handle: Option<String>, runtime_override: Option<String>, note: Option<String>, requested_by: Requester })`, where `Requester` is `Human` or `Slot(handle)`. It refuses unless the mission is `running` and not archived (a paused mission, every slot stopped, is fine: the newcomer spawns alone). It resolves the runner, picks the handle (`slot_handle`, else the template handle, suffixed `-2`, `-3`… past taken handles the way the crew page's `suggest_slot_handle` does), inserts the mission slot in a transaction, then in this order: registers the session row (`register_mission_session` with the mission's cwd, the app-wide permission mode, the grid hint size, and `compose_worker_first_turn` as the first turn, checked by `ensure_first_turn_fits`), tells the router (`Router::add_member` + `register_pending_sessions`), tells the bus (`BusRegistry::add_handle`), appends the handle to `roster.json`, appends the events below, and finally dispatches the background spawn through the mission's existing pending-cancel flag so Stop and Archive still abort a newcomer that has not forked yet. A newcomer is always a worker; the lead is not changeable mid-mission.
- **Mutable roster in the router and bus.** `LaunchInputs.roster` moves behind the router's state lock so `add_member(handle, display_name)` can push a row; a lead Restart after an add composes a launch prompt whose `== Your crewmates ==` lists the newcomer. `BusState.handles` gains an add path on the consumer thread (a control message alongside the tick), so the newcomer's inbox projects every message from its join event onward; nothing before the join is projected for it.
- **Events.** The op appends a `signal` of type `runner_added` from the requester (`human`, or the lead's handle) with `{handle, display_name, runner_handle, runner_id, session_id, note}`, then a broadcast `message` from `runner`: "@reviewer-2 (Reviewer) joined the mission, added by @lead. @reviewer-2: the lead is @lead; your crewmates are @coder and @tester. Read your inbox with `runner msg read` and wait for the lead's instructions. Everyone else: reach it with `runner msg post --to reviewer-2`." Then, when `note` is set, a directed `message` from the requester to the newcomer carrying the note verbatim. `message_nudge` already fans the broadcast into every roster member's PTY and queues the newcomer's nudges until its PTY is live, so the team learns of the newcomer and the newcomer learns its lead, its crewmates, and its task without a new first-turn composer. `runner_added` is app-emitted like `human_question`, so it stays out of `KnownSignalType`.
- **The lead's path** (CLI + router). A new `add_runner` entry in `KnownSignalType` so `runner signal add_runner --payload '{"runner":"reviewer","as":"reviewer-2","note":"Review @coder's PR once it opens."}'` passes the CLI's type check; `runner` is a template handle, `as` and `note` are optional. The router routes `add_runner` like `ask_lead`: it validates that `event.from` is the lead and the payload has a `runner` string, then hands the request to the core through a new `RouterUiNotifier` method (the router owns no DB and no session manager); the core runs `mission_add_runner` with `requested_by: Slot(lead)`. Anything else bounces with a `mission_warning` the lead sees in its terminal: a worker sender ("only the lead adds runners; ask via `ask_lead`"), an unknown template handle (the warning lists the known template handles, since the lead has no other way to discover them), a taken `as` handle, a roster already at `MISSION_ROSTER_CAP` (8, a constant, not a setting; the human path has no cap) so a looping lead cannot spawn agents unbounded. The launch prompt's `== Coordination ==` section gains one line describing the verb; the worker preamble does not mention it.
- **The human's path** (app). Under the rail's **Runner sessions** list, after the last card, a **+ Add runner** row in the crew page's "+ Add slot" style, shown while the mission is running. It opens an **Add runner** modal: the runner picker from the crew page's add-slot form (search, runtime option, template handle), a slot-handle field pre-filled with the suggested unique handle and validated against the mission roster with the crew page's `slot_handle_error`, and an optional multi-line **Note to the newcomer** ("Review @coder's PR once it opens"), Cancel / Add. Add calls the op with `requested_by: Human`; the modal closes, the new card appears at the end of the rail with the existing starting pill and a "joined mid-mission" caption until its status arrives, and its tab opens like any slot's. The op's errors surface in the modal (taken handle, mission not running).
- **Feed rows.** `runner_added` renders as a signal row on the requester's avatar: "@lead · signal · runner_added → @reviewer-2 (Reviewer) joined". The lead's `add_runner` request and any `mission_warning` bounce render through the existing signal and warning rows. The broadcast and the note render as ordinary message rows.
- **MCP.** `mission_add_runner(mission_id, runner (id or handle), slot_handle?, runtime?, note?)` beside `mission_start`, so a session driving Runner over MCP (the way #554 and #556 are used) can grow a mission it launched. It maps to `requested_by: Human`.
- **Per-slot lifecycle** (#542) works on a mission slot unchanged: Stop, Resume, and Restart resolve the slot through `sessions.slot_id`, and Restart recomposes the worker first turn from the mission slot's runtime override and the runner template. The mission-wide Resume respawns added runners with the rest.

### Out of scope

- Removing a runner from a mission. The human already has per-slot Stop; the lead asks the human. A mission slot's row stays for the mission's life so the feed keeps its handle.
- Promoting a mission slot into the crew template ("keep this runner for next time"). Do it on the crew page; the crew stays a template that missions read at start.
- Changing the lead mid-mission, or adding a second lead.
- Model and effort overrides at add time. Runtime is enough; model and effort come from the template (or edit the template first).
- Adding a runner to a completed, aborted, or archived mission, and adding one while the original cold-start spawn queue is still running (the request waits for the human to retry, the same gap #542 left).
- Replacing the newcomer's first turn with a mission-aware composer. The worker first turn stays byte-identical to a cold start; the join broadcast and the note carry the mission context over the bus.
- A confirm gate on lead-initiated adds. The feed shows the request and the join, and per-slot Stop undoes it; the roster cap is the guard.
- Concurrent add requests racing each other on handle choice beyond what the unique index refuses.

## Implementation Phases

### Phase 1 — backend

- Migration `0021_slot_mission_id.sql`: `slots.mission_id TEXT NULL REFERENCES missions(id) ON DELETE CASCADE`, the partial unique indexes replacing `UNIQUE (crew_id, slot_handle)` (SQLite needs the table rebuilt), an index on `mission_id`. `repo::slot::list_for_crew` filters `mission_id IS NULL`; add `list_for_mission`, `insert_for_mission`, and the `mission_delete` cascade check.
- `crates/runner-backend/src/ops/slot.rs`: `list_for_mission`; `crates/runner-backend/src/ops/mission.rs`: `mission_add_runner` as above, `ensure_mission_router_mounted` and `mission_resume` paths switched to `list_for_mission`, `write_roster_sidecar` gains an append, the sidecar comment and `cli/src/roster.rs` header stop saying "frozen".
- `crates/runner-backend/src/router/mod.rs`: roster behind the state lock, `add_member`, the `add_runner` dispatch arm and `RouterUiNotifier::add_runner_requested`; `handlers.rs`: validation and the four `mission_warning` bounces; `prompt.rs`: the coordination line. `crates/runner-backend/src/event_bus/mod.rs`: `add_handle` control message. `crates/runner-core/src/model.rs`: `KnownSignalType::AddRunner`.
- `crates/runner-backend/src/mcp/tools/mission.rs`: `mission_add_runner`.
- Tests: mission slots are invisible to `slot::list(crew_id)` and visible to `list_for_mission`; a second mission of the same crew can reuse an added handle; the op refuses a template handle collision, a non-running mission, and the cap for a slot requester but not for the human; the newcomer's session row carries the worker first turn on argv; the three events land in order and the newcomer's inbox holds the broadcast and the note; the lead's `add_runner` reaches the notifier while a worker's bounces; `reconstruct_from_log` after a restart rebuilds a roster that includes the mission slot; a lead Restart's launch prompt lists the newcomer.

### Phase 2 — CLI + docs

- `cli/src/signal.rs` accepts `add_runner` through `KnownSignalType`; `cli/src/help.rs` documents the payload.
- `docs/arch/arch.md`: the roster section says the mission roster is append-only, the signal table gains `add_runner` and `runner_added`, and the CLI verb list gains the payload shape.

### Phase 3 — design, then app

- Draw the three frames in `design/runner.pen` and stop for sign-off: the rail with the **+ Add runner** row, the **Add runner** modal, and the joined state (rail card with the "joined mid-mission" caption, the `runner_added` feed row, the `runner` broadcast row).
- `crates/runner-app/src/surfaces/mission_workspace.rs`: the row, the modal (reusing the crew page's `AddSlotForm` pieces and `suggest_slot_handle`), the caption, the feed row for `runner_added`. `app_store.rs`: `runner_added` joins the `event/appended` refresh list.
- Tests: the modal's handle validation against the mission roster; the rail renders the added session after `session/spawned`; the feed row copy.

### Phase 4 — smoke

- Human adds a reviewer to a running two-slot mission: the card appears, the newcomer boots with its brief, reads the join broadcast and the note, and the lead's next `runner msg post --to reviewer-2` lands.
- Lead runs `runner signal add_runner` with a valid template: the feed shows the request and the join; with an unknown template: the warning in the lead's terminal lists the template handles.
- A worker runs the same verb: bounced, nothing spawned.
- Quit and relaunch the app with the mission running: the added slot is in the rail and resumes with the rest.
- Restart the lead after an add: the launch prompt's crewmates list includes the newcomer.
- Archive, then delete the mission: the mission slot rows are gone and the crew page never showed them.
- Windows on JASONPC: the human path with a batch-wrapped runner.

## Verification

- [ ] `slot::list(crew_id)` never returns a mission slot; the crew page and `mission_start` are unchanged.
- [ ] `mission_add_runner` inserts the mission slot, registers the session with the worker first turn, updates router, bus, and sidecar before appending events, and spawns through the mission's cancel flag.
- [ ] `runner msg post --to <new handle>` validates in the CLI immediately after the add.
- [ ] The three events appear in order with the documented `from` and payloads; the newcomer's inbox projects the broadcast and the note; earlier history is not projected for it.
- [ ] `add_runner` from the lead spawns; from a worker, with an unknown template, with a taken handle, or at the cap it appends a `mission_warning` and spawns nothing.
- [ ] After an app restart the mission's router roster and rail include the mission slot; a lead Restart lists it in the launch prompt.
- [ ] Stop, Resume, and Restart on the added slot behave as on any slot.
- [ ] The **+ Add runner** row is hidden on completed, aborted, and archived missions; the modal refuses a handle already in the mission roster.
- [ ] `mission_delete` removes the mission slots; a second mission of the same crew can add the same handle while the first is still running.

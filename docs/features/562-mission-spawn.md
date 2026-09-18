# 562 — Missions as containers: spawn roles into a running mission

> Tracking issue: [#562](https://github.com/yicheng47/runner/issues/562)
> Priority: P2.
> Status: planned, design first.
> Design: `design/specs/562-mission-spawn.pen`, frames to draw before Phase 4.
> Rewritten 2026-09-18 from "Add runners to a running mission" (2026-09-11). The first draft grew a crew mission's roster from the rail or a lead signal. This version makes the mission the container that owns its roster, keeps the crew as one way to seed it, lets a mission start from a single role, and gives the lead the verbs to spawn roles and follow their state.

## Motivation

A mission's roster is fixed the moment it starts. `ops::mission::start` snapshots the crew's slots into `roster.json`, one session is spawned per slot, and the router and bus hold the roster immutably. When the lead finds mid-flight that the task splits three ways, or needs a reviewer the crew never had, the only move is to stop, edit the crew, and start over, losing every conversation.

The crew also sits in front of every delegation. The pattern Jason uses daily (a Claude Code session plans, a Codex agent implements) runs today from an external session over MCP, and needed a one-slot crew ("codex solo") just so one agent could hand work to another. A crew of one is a spawn with setup in front of it.

Delegation across runtimes is the lane nobody else covers: Claude Code's subagents spawn Claude, Codex's spawn Codex. A lead that picks a role (runtime, model, effort and brief already set) for each piece of work is also the token-subscription positioning working without the human choosing each time.

Crews stay. A repeatable shape (coder plus reviewer, or #630's re-runnable jobs) needs a fixed roster so runs can be compared. The crew becomes one way to fill a mission, not the only way in.

## Model

- **Mission**: a container with one event log, a cwd, a goal, a roster of slots, and exactly one lead. The mission owns its roster.
- **Crew**: a template. Starting a mission from a crew copies the crew's slots into the mission. Editing the crew afterwards never touches a running mission.
- **Role-seeded mission**: a mission started from one role, which becomes the lead. Its roster grows by spawning.
- **Spawn**: add a slot to a running mission and start its session. The lead, the human, or an MCP client can spawn.
- **Direct chats** stay off the bus.

Vocabulary: **crew slot** (a row the crew page edits) and **mission slot** (a row one mission owns). No new nouns. The vision's "Mission: one live activation of a crew" becomes "one live run of a roster, seeded from a crew or a role".

## Scope

### In scope

- **Mission-owned roster** (backend). `slots` gains a nullable `mission_id` and `crew_id` becomes nullable, with a check that exactly one is set. Crew slots keep `mission_id IS NULL` and are what the crew page lists. `mission_start` copies the crew's slots into mission slots in the same transaction that creates the mission, and each session points at its mission slot. Handles are unique per crew among crew slots and per mission among mission slots (the `UNIQUE (crew_id, slot_handle)` and `UNIQUE (crew_id, position)` constraints become partial indexes). A mission slot keeps the `runtime_override`, `model_override`, and `effort_override` columns and gains `added_by TEXT NULL` (`NULL` for seeded slots, otherwise `human` or the lead's handle). `mission_delete` removes the mission's slots. The migration backfills every existing mission: it copies the slot each of its sessions points at into a mission slot and repoints `sessions.slot_id`, so every roster read has one path, `slot::list_for_mission`.
- **The spawn op.** `ops::mission::mission_spawn(mission_id, SpawnInput { role: RoleRef (id or handle), slot_handle: Option<String>, runtime: Option<String>, model: Option<String>, effort: Option<String>, task: Option<String>, requested_by: Requester })`, where `Requester` is `Human` or `Slot(handle)`. It refuses unless the mission is running and not archived; a paused mission is fine, and the new slot then spawns alone. It resolves the role and picks the handle: `slot_handle`, else the role's handle, suffixed `-2`, `-3`… past taken handles as `suggest_slot_handle` does. Then it inserts the mission slot and registers the session (`register_mission_session` with the mission cwd, the mission's permission mode, the grid hint size, and `compose_worker_first_turn` checked by `ensure_first_turn_fits`). Then it adds the member to the router (`Router::add_member` plus `register_pending_sessions`) and to the bus (`BusRegistry::add_handle`), appends the handle to `roster.json`, and appends the events below. Last, it dispatches the spawn through the mission's pending-cancel flag, so Stop and Archive still abort a slot that has not started yet. A spawned slot is always a worker.
- **Mutable roster in the router and bus.** `LaunchInputs.roster` moves behind the router's state lock so `add_member` can push a row. A lead Restart after a spawn composes a launch prompt whose `== Your crewmates ==` section lists the new slot. `BusState.handles` gets an add path on the consumer thread (a control message alongside the tick), so the new slot's inbox projects every message from its join event onward and nothing before it. `roster.json` stops being frozen: the CLI already rereads it on every call, so `runner msg post --to <new handle>` validates right after the join. Its entries, today only `{handle, lead}`, gain optional `role` and `runtime` fields for `runner ps`; older sidecars still parse without them.
- **Join events.** The op appends a `role_joined` signal from the requester (`human` or the lead's handle) with `{handle, display_name, role_handle, role_id, runtime, session_id}`, then a broadcast `message` from `runner`: "@coder-2 (Coder, codex) joined the mission, added by @lead. @coder-2: your lead is @lead; your crewmates are @reviewer. Report back with `runner msg post --to lead`." When `task` is set, a directed `message` from the requester to the new slot carries it verbatim. `message_nudge` already delivers the broadcast to every roster member and queues the new slot's nudges until its PTY is live. `role_joined` is emitted by the app, like `human_question`, so it stays out of `KnownSignalType`.
- **The lead's verbs** (CLI plus router). Every verb below is lead-only. A worker sender is bounced with a `mission_warning` that points it at `ask_lead`.
  - `runner spawn <role> [--as <handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>] ["<task>"]` emits a `spawn_role` signal. The router validates the sender and the payload, then hands the request to the core through a new `RouterUiNotifier` method (the router owns no DB and no session manager), and the core runs `mission_spawn` with `requested_by: Slot(lead)`. These cases bounce with a `mission_warning` that the lead sees in its terminal: an unknown role (the warning lists the known role handles, since the lead has no other way to discover them), a taken `--as` handle, an unknown runtime, and a roster already at `MISSION_SPAWN_CAP` (8, a constant, not a setting; the human's path has no cap). The CLI returns at once and prints that the result arrives as the join broadcast or a warning.
  - `runner ps` folds `roster.json` and the event log, with no IPC. It prints one line per member: handle, role, runtime, state (`starting`, `working`, `idle`, `stopped`, `crashed`), and the time and first line of the member's last message.
  - `runner stop <handle>` emits a `stop_slot` signal. The core stops that session with the #542 per-slot Stop. The lead cannot stop itself.
  - The launch prompt's `== Coordination ==` section gains three lines for these verbs. The worker preamble does not mention them.
- **Role-seeded missions.** `missions.crew_id` becomes nullable. `mission_start` takes exactly one of `crew_id` or `role` (id or handle); a role seeds one mission slot, the lead. A crewless mission keeps its event log in `<app data>/missions/<id>/`, the directory that already holds the mission's scratch files and is removed with the mission (`ops/mission.rs:1270`); `event_log::mission_dir` takes an optional crew. The event envelope's `crew_id` becomes optional: it is omitted for crewless missions, and old logs parse unchanged. The CLI's `RUNNER_CREW_ID` becomes optional, while the other three variables are still required together. The launch prompt uses the mission title where it used the crew name, and a crewless mission has no crew-conventions layer. Deleting a crew keeps its current rule (refused while the crew has non-archived missions).
- **The human's path** (app). Under the rail's sessions list, a **+ Add role** row shown while the mission is running. It opens an **Add role** modal: the role picker from the crew page's add-slot form, runtime, model and effort options, a handle field pre-filled with the suggested unique handle and validated with `slot_handle_error`, and an optional multi-line **Task**. Add calls the op with `requested_by: Human`. The new card appears at the end of the rail with the starting pill and an "added by @lead" or "added by you" caption. Starting a mission gains a choice between a crew and a role, and a role's detail page gains **Start mission**.
- **Feed rows.** `role_joined` renders as a signal row on the requester's avatar: "@lead · spawned @coder-2 (Coder, codex)". `slot_exited` renders as a warning row when the outcome is `crashed` and a muted row otherwise. `spawn_role` and `stop_slot` requests, and their bounces, render through the existing signal and warning rows.
- **MCP.** `mission_spawn(mission_id, role, as?, runtime?, model?, effort?, task?)` beside `mission_start`, which maps to `requested_by: Human`. `mission_start` accepts `role` as an alternative to `crew_id`, so an outside session can start a one-role mission and spawn into it.
- **Per-slot lifecycle.** #542's Stop, Resume, and Restart work on a spawned slot unchanged, resolved through `sessions.slot_id`. The mission-wide Resume respawns spawned slots with the rest, and an app restart rebuilds the router from `list_for_mission`.

### What the lead hears without asking

The router pushes state changes to the lead, and `runner ps` is the snapshot when the lead wants one. Nothing requires the lead to poll.

| Change | How it reaches the lead | Today |
|---|---|---|
| A result or report | the worker runs `runner msg post --to lead`; the lead is nudged | built |
| A worker needs a decision | `ask_lead` is injected | built |
| A worker joined | the join broadcast | this spec |
| A spawn failed | a `mission_warning` in the lead's terminal | this spec |
| A worker went idle without reporting | the router injects "@coder is idle (last message 4m ago)" on a non-lead Busy→Idle transition, unless that worker posted to the lead during the same busy stretch. arch §8.1 documents this handler but `router/handlers.rs::session_status` only records the state | this spec |
| A worker stopped or crashed | the core appends a `slot_exited` signal from `router` with `{handle, outcome: stopped \| crashed, exit_code, by}`; the router injects "@coder crashed (exit 1)" to the lead unless the lead asked for the stop or the whole mission is stopping. Today `SessionDeliveryEvent::Exited` only drops the outbox (`router/mod.rs:1360`) | this spec |

### Out of scope

- Spawning from a direct chat. A chat's `RUNNER_*` environment is fixed at spawn (`cli/src/env.rs`), so a running chat cannot join a bus without being relaunched as a mission lead. Revisit as "resume this chat as the lead of a new mission" if role-seeded missions prove the pattern.
- Workers spawning (more than one level). Workers ask the lead with `ask_lead`.
- The lead reading a worker's terminal. Messages are the contract; raw TUI output is noisy and costs tokens.
- Passing approval waits on to the lead. The human's attention indicator already covers approvals.
- Removing a slot from a mission, promoting a mission slot into a crew, changing the lead, or a second lead.
- A confirm gate on the lead's spawns. The feed shows every spawn, per-slot Stop undoes one, and the cap is the guard.
- Spawning into a completed, aborted, or archived mission, or while the mission's cold-start spawn queue is still running (the request waits for a retry, the same gap #542 left).

## Implementation Phases

### Phase 1 — mission-owned roster and the spawn op

- Migration `0024_mission_slots.sql`: `slots` rebuilt with a nullable `mission_id` (references `missions(id)` on delete cascade), a nullable `crew_id`, a check that exactly one is set, `added_by`, and partial unique indexes over crew slots and over mission slots. It backfills mission slots from each mission's sessions and repoints `sessions.slot_id`.
- `repo::slot`: `list_for_crew` filters `mission_id IS NULL`; add `list_for_mission` and `insert_for_mission`. `ops::mission::start` copies the crew's slots. `ensure_mission_router_mounted`, `mission_resume`, and Restart read `list_for_mission`.
- `ops::mission::mission_spawn`, the mutable roster in `router/mod.rs` (`add_member`), `BusRegistry::add_handle`, the `roster.json` append, and the join events. The comments in `ops/mission.rs` and `cli/src/roster.rs` stop saying the roster is frozen.
- `mcp/tools/mission.rs`: `mission_spawn`.
- Tests: the backfill gives every existing session a mission slot with the same handle, role, and overrides; the crew page never lists a mission slot; editing a crew slot does not change a running mission's slot; two missions of one crew can each spawn `@coder-2`; the op refuses a taken handle and a non-running mission; the new session's row carries the worker first turn; the join events land in order and the new slot's inbox holds the broadcast and the task; after an app restart, `reconstruct_from_log` rebuilds a roster that includes the spawned slot.

### Phase 2 — the lead's verbs and what the lead hears

- `crates/runner-core/src/model.rs`: `KnownSignalType::{SpawnRole, StopSlot}`. `cli/src/main.rs`: the `spawn`, `ps`, and `stop` commands; `cli/src/help.rs` documents them.
- `router/handlers.rs`: `spawn_role` and `stop_slot` validation with their bounces, the idle notice with its "posted to the lead during this busy stretch" suppression, and the `slot_exited` injection. `router/prompt.rs`: the three coordination lines.
- The core appends `slot_exited` when a mission session exits, with the outcome, exit code, and who asked for the stop.
- `docs/arch/arch.md`: the roster is append-only, §8.1 gains `spawn_role`, `stop_slot`, `role_joined`, and `slot_exited` and states the idle notice as built, and §9 lists the new commands.
- Tests: `spawn_role` from the lead reaches the notifier, and from a worker it bounces; an unknown role bounces with the role list; the cap bounces the lead but not the human; `runner ps` folds a log with a join, a busy stretch, a message, and a crash into the right five lines; an idle notice is suppressed after a report to the lead and sent without one; a crash injects to the lead, and a `runner stop` exit does not.

### Phase 3 — role-seeded missions

- Migration `0025_crewless_missions.sql`: `missions.crew_id` becomes nullable.
- `event_log::mission_dir` with an optional crew; the envelope's optional `crew_id`; `cli/src/env.rs` with an optional `RUNNER_CREW_ID`; `mission_start` taking a crew or a role; `LaunchInputs` without a crew name or addendum; `mission_start` over MCP taking `role`.
- `docs/product/vision.md`: §3's Mission and Crew definitions and §4.2 as in Model above.
- Tests: a role-seeded mission writes its log under `missions/<id>/`; its lead's `runner msg post` works without `RUNNER_CREW_ID`; an old log with `crew_id` on every line still replays; deleting a role-seeded mission removes its log directory.

### Phase 4 — design, then app

- Draw the frames in `design/specs/562-mission-spawn.pen` and stop for sign-off: the rail with **+ Add role**, the **Add role** modal, a spawned card with its caption, the `role_joined` and `slot_exited` feed rows, and the crew-or-role choice when starting a mission.
- `crates/runner-app/src/surfaces/mission_workspace.rs`: the row, the modal (reusing the crew page's `AddSlotForm` pieces and `suggest_slot_handle`), the caption, and the feed rows. `app_store.rs`: `role_joined` and `slot_exited` join the `event/appended` refresh list. The new-mission flow and the role detail page gain the role option.
- Tests: the modal validates handles against the mission roster; the rail renders a spawned session after `session/spawned`; the feed row copy.

### Phase 5 — smoke

- Start a role-seeded mission with a Claude Code lead. The lead runs `runner spawn coder --runtime codex "Implement X"`, the codex slot boots, reads the join broadcast and the task, does the work, and reports with `runner msg post --to lead`; the lead wakes on the nudge.
- The lead runs `runner ps` while the coder works, then after it finishes, and sees the states change.
- Kill the coder's process. The lead is told it crashed, and the feed shows the row.
- The lead runs `runner stop coder`. The lead gets no crash notice, and the rail shows the slot stopped.
- A worker runs `runner spawn` and is bounced; nothing is spawned.
- The human adds a reviewer from the rail with a task.
- Quit and relaunch the app with the mission running: spawned slots are in the rail and resume with the rest.
- A crew mission started before the migration still resumes with its roster intact.
- Windows on JASONPC: the human's path and `runner spawn` with a batch-wrapped role.

## Verification

- [ ] The migration gives every existing mission's sessions a mission slot, and pre-migration missions resume, restart, and replay unchanged.
- [ ] The crew page lists crew slots only, and editing a crew never changes a running mission.
- [ ] `mission_spawn` updates the router, bus, and `roster.json` before appending events, and spawns through the mission's cancel flag.
- [ ] `runner msg post --to <new handle>` validates immediately after the join.
- [ ] The join events land in order with the documented `from` and payloads; the new slot's inbox has the broadcast and the task and no earlier history.
- [ ] `runner spawn` from the lead spawns. From a worker, with an unknown role, with a taken handle, with an unknown runtime, or at the cap it appends a `mission_warning` and spawns nothing.
- [ ] `runner ps` reports the right state and last message for every member, including crashed and stopped ones.
- [ ] The lead is told when a worker goes idle without reporting and when a worker crashes, and is not told about a stop it asked for.
- [ ] A role-seeded mission runs end to end with its log under `missions/<id>/`.
- [ ] **+ Add role** is hidden on completed, aborted, and archived missions, and the modal refuses a handle already in the mission roster.
- [ ] `mission_delete` removes the mission slots and, for a crewless mission, its log directory.

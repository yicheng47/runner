# 562 — plan

The phases of [feature 562](../../../features/archive/562-mission-spawn.md) in detail: how each lands, what reading the code changed, and what binds its mission. The program's state and its missions are in [README.md](README.md), and the dated log is [impl_log.md](impl_log.md).

## Phase 1 — the mission roster (mission 1, shipped)

Each mission keeps its roster in `roster.json` beside its event log, written at start and read by every reader, with version 1 files converted once at `runnerd` start. Specified in the spec's Phase 1 and briefed in `562-m1-mission-roster.md` (in git history since the 0.15.2 docs sweep). Merged on 2026-10-11 as [#868](https://github.com/yicheng47/runner/pull/868), shipped in 0.15.2; its record is [`562-mission-roster.md`](../../../tests/archive/562-mission-roster.md).

## Sessions independent of each other (mission 2, shipped)

Phase 2's decision 9, split out and shipped first: one mission session's exit or failed spawn never stops another. Brief [`562-m2-independent-sessions.md`](https://github.com/yicheng47/runner/blob/v0.15.3/docs/impls/briefs/562-m2-independent-sessions.md); merged on 2026-10-12 as [#874](https://github.com/yicheng47/runner/pull/874); record [`562-independent-sessions.md`](../../../tests/archive/562-independent-sessions.md).

## Phase 2 — add and remove roles while a mission runs

### Design

`runner/desktop/specs/archive/562-mission-spawn.pen`, drawn on 2026-10-12 against the shipped mission workspace (rail cards, feed renderer, avatar seeding) and the shipped Start chat dialog:

| Frame | Id | Shows |
| --- | --- | --- |
| Mission — Add role and Remove in the rail | `vfnFl` | **+ Add role** under the session cards; a worker card's remove button with its tooltip, "Remove from mission". The lead card has no remove button. |
| Mission — Add role dialog | `V61gij` | The dialog over the running mission, from an instance of `cmp/AddRoleDialog` (`cDBYo`). |
| Add role dialog — handle refused | `E8wxa` | A handle that was removed earlier in the mission, with its inline error and Add role disabled. |
| Mission — role joined | `IPOAI` | The new card at the end of the rail in `starting…`, its tab at the end of the strip, and the feed's join divider, the system introduction and the task. |
| Mission — Remove from mission confirm | `E0HNqg` | The confirmation, `cmp/ConfirmDialog` as Restart's confirm uses it. |
| Mission — role removed | `zd1d4` | The card and tab gone, the leave divider and the system notice; the removed agent's history stays in the feed. |

The dialog follows Start chat's structure: title "Add role" with a subtitle naming the mission; a **Role** field holding the role picker card with Runtime, Model and Effort (and Codex's Speed on its own row), each with the override dot and "role: X ↺ Reset" as in Start chat, and the hint "Starts with the role's settings. Change any of them for this agent only." under the card; **Handle** with an `@` prefix, prefilled with the suggested handle, and the hint "How the team addresses it."; **Task**, optional and multiline, with "Sent to @impl-2 as a message from you. Left blank, it waits until someone messages it."; a footer with "Works in <mission cwd>" on the left and Cancel (`esc`) and Add role (`⌘↵`) on the right.

### Decisions that bind

1. **A mission adds configured roles only.** No Direct (runtime-only) mode: a roster slot names its role by id, and its command, prompt and resume all come from that role. A bare runtime needs role-less slots, a separate change: since #878 a session records its own launch definition, and the roster only names roles. The Runtime, Model, Effort and Speed overrides cover running a role on another agent.
2. **An added role is always a worker, and the lead is fixed** (spec decisions 2 and 3). The lead card has no remove button, and the daemon refuses removing the lead.
3. **A removed handle stays taken** (spec decision 1). Suggestion skips it (`impl` → `impl-2` when `impl` is present or was removed), and typing it is refused with "@reviewer was removed from this mission, and a handle isn't reused. Try @reviewer-2."
4. **The log tells the bus who is on the team, and a directed message is never dropped.** This refines the spec's add-handle and remove-handle control messages. The bus replays `events.ndjson` from offset 0 on every mount and learns membership from `role_joined` and `role_removed` in the same pass, in log order. Membership bounds broadcasts only: a broadcast lands in the inbox of every handle on the team at that point in the log, and a directed message always lands in its addressee's inbox, whenever it was appended. Handles are never reused (decision 3), so a directed message cannot reach the wrong agent, and no order of writes can lose one. At mount the team is the roster's handles minus those that joined mid-mission, which come back in at their `role_joined`. Live and replay are one code path, and after a `runnerd` restart a newcomer's unread count again starts at its join. The bus honours these signals only when `from` is `human`. The bus's projection drives unread counts only: an agent reads its inbox with `runner msg read`, which scans the log itself (`runner-cli/src/msg.rs`, `is_inbox`) and prints every broadcast since the mission started, so a newcomer also sees the team's earlier broadcasts. That is intended while mission logs stay short (Jason, 2026-10-12), and `msg.rs` is unchanged.
5. **The ops change the router directly.** `LaunchInputs.roster` moves behind the router's state lock with add and remove methods. The add and remove ops call them before appending their events, so the broadcast that announces a newcomer already nudges it. Broadcast nudges, `record_slot_restart` and the lead's Restart prompt read the current roster. A re-mount builds the router from the file as today.
6. **Remove stops and drops; it does not archive.** The session row stays as the mission's history. It leaves the rail, the session lists and Resume all because `ops::session::list_for_mission` keeps only sessions whose slot is in the roster. `archived_at` stays the mission archive's flag.
7. **One lock per mission.** Add and remove each hold a per-mission mutex for their whole run, so two adds cannot pick the same handle and roster rewrites never interleave. The roster file is still written atomically through a temporary file renamed over it, and on Windows a rename that fails because the CLI holds the file open is retried a few times over about 200 ms.
8. **Add and remove are the person's verbs** (spec decision 5). The protocol operations carry no `from`. The CLI refuses them from inside a mission session and does not accept `--as` for them, as with `mission answer`.
9. **Each session is independent.** One session's exit or failed spawn never stops another (Jason, 2026-10-12). Before #874, a slot that exited on its own killed every live sibling and cancelled queued spawns (`reap_live_mission_siblings`, added in #346 for #338, when a dead slot showed one paused card over the whole mission and its siblings ran on behind it). #542's per-slot Stop, Resume and Restart had replaced that card, so #874 removed the reap from the exit epilogue and from mission start's spawn-failure path. A crashed slot shows crashed on its own card with Resume and Restart, the others keep running, the mission stays `running`, and the mission-wide Resume and Archive card still appears once every slot is stopped. This holds for crew slots and added roles alike; without it, a newcomer that dies at boot would take a running team down with it. It landed ahead of Phase 2 as mission 2 ([#874](https://github.com/yicheng47/runner/pull/874)), and Phase 2 builds on it.
10. **The CLI says `--handle`, not `--as`.** The spec drafted `add-role … --as <handle>`, but `--as` is already the CLI's caller identity flag ([648](../../../features/archive/648-runner-cli.md) decision 7).
11. **The feed shows `runner` as `system`.** Events keep `from: "runner"`, the author #542's restart notices already use, and agents still see `@runner` in their inbox. The feed labels those message groups `system`, in the hue and pixel avatar the app's seeding gives the seed `system`, like any other author, with the body muted as an automatic notice. This also relabels #542's restart notices (Jason, 2026-10-12, after a Runner-mark draft).
12. **Errors name the mission by its title and slots by `@handle`**, never by id.
13. **Agents learn the team from their inbox and `runner mission show`; there is no new command.** Every join and leave sends a system broadcast naming the change and the whole team, and the bus nudges each member to read it. For a look at any time, `runner mission show` already works inside a mission (it defaults to `RUNNER_MISSION_ID`) and prints the roster table (handle, lead, runtime, status), read through `list_for_mission`, which follows the roster from this phase on. What is missing is the agents knowing this: the worker preamble says "Valid handles: any slot in this crew" without saying where to find them, and the lead's prompt lists its crewmates as fixed at launch. Both prompts gain a line that the team can change while the mission runs, that the inbox announces it, and that `runner mission show` lists it. A `--to` refusal also lists the current handles, so an agent that addresses a removed handle learns the team in the same reply. `runner ps` stays reserved for the lead's verbs (Follow-ups).
14. **Nothing new assumes a mission has a lead.** Making the lead optional is a follow-up ([#873](https://github.com/yicheng47/runner/issues/873)), so Phase 2 must leave it a local change. Code added in this phase reads the lead from a slot's `lead` flag where it needs one, and never through `Router::lead_handle()` or `LaunchInputs::lead()`. The add path, the remove path, the bus and the team lists in the system broadcasts do not touch the lead at all. The router's mutable roster rows stay separate from its `LeadRow`, so the follow-up can make that one field optional. Refusing to remove the lead is a check on the slot's flag, not something later code depends on. "(lead)" in a team list comes from the flag, so a mission without a lead lists only handles. `role_joined` carries `"lead": false`, so a later join as lead is an additive change. Copy that only holds with a lead is reworded: the Task hint says "Left blank, it waits until someone messages it."
15. **A roster change waits for pending spawns.** A mission's spawns run on a background thread after the op that queued them returns, outside the mission lock, and `SessionManager::kill` returns `Ok` without doing anything for a session that has no process yet (`session/manager/lifecycle.rs`, `kill`). Add and remove are therefore both refused while any spawn of the mission is pending, from the cold-start queue or an earlier add, checked with `has_pending_mission_spawns`. Without that refusal, removing a slot that is still queued would drop it from the roster, and its spawn would start it anyway: a live agent off the rail, whose posts are refused, until Stop all or Archive. Add also registers its cancel flag right after its checks, before it inserts the session row, so a Stop all or Archive that lands while the add runs cancels the spawn. Archive does not take the mission lock and can only flip a flag that already exists (`cancel_pending_mission_spawns`); a flag registered later, just before the spawn, would miss it and start an agent in an archived mission. Remove also takes the slot session's resume claim before stopping and holds it through roster publication and rollback, so it refuses while Resume or Restart of that slot is in flight and, in the other order, those operations are refused until the smaller roster is published.

### Event contract

Add appends `role_joined` before it writes `roster.json`, so the CLI's `--to` check cannot accept the new handle before the bus knows it, and the other two events after the router is updated:

1. `role_joined`: signal, `from: "human"`, `to: null`, payload `{ "handle", "slot_id", "session_id", "role_id", "lead", "runtime", "model", "effort", "speed" }`; `lead` is always `false` in this phase (decision 14), and the last four are the effective values, `null` when unset.
2. A broadcast `message` from `runner`: "@impl-2 joined the mission as a worker: Implementation, on Codex. The team is now @architect (lead), @impl, @reviewer and @impl-2. `runner mission show` lists the current team." The newcomer receives it too, since its inbox opened at step 1.
3. When a task is given, a `message` from `human` to the new handle, with the task as its text.

Remove appends, after the session is stopped, the file rewritten and the router updated:

1. `role_removed`: signal, `from: "human"`, `to: null`, payload `{ "handle", "slot_id", "session_id" }`.
2. A broadcast `message` from `runner`: "@reviewer left the mission. Messages to @reviewer are refused from now on. The team is now @architect (lead), @impl and @impl-2. `runner mission show` lists the current team." The removed handle is already out of the bus's projection, so it does not receive this.

Neither type joins `KnownSignalType`, as `slot_restarted` does not, so the in-mission `runner signal` and `mission_signal` with a handle keep refusing them as unknown types.

### Where Phase 2 lands

Line numbers are as of `3a628e50`, after #874, #881 and #885.

**`runner-core`**

- `crates/runner-core/src/mission_roster.rs`: unchanged format. Added slots are ordinary `RosterSlot`s with fresh ULIDs and `position` one past the highest. The add's Runtime, Model, Effort and Speed are stored as the slot's overrides, so Restart, which since #878 rebuilds a session from its current role and its roster slot, gives the newcomer the same ones.
- The handle rule moves here from `crates/runner-app/src/surfaces/crews/logic.rs` (`suggest_slot_handle` at 54, `validate_slot_handle`, `slot_handle_error` at 108), with the scope in the "already used" message a parameter (crew or mission) and the removed-handle error added. The crew page keeps its behaviour and copy.
- `crates/runner-core/src/protocol/api.rs` (pattern: `mission_roster` at 119): `mission_add_role(input: MissionAddRoleInput) -> MissionRoleChange` and `mission_remove_role(mission_id, handle) -> MissionRoleChange`. `MissionAddRoleInput { mission_id, role_id, handle: Option<String>, runtime: Option<Runtime>, model: Option<String>, effort: Option<String>, speed: Option<CodexSpeed>, task: Option<String> }`; `MissionRoleChange { mission_id, handle, session_id }`. Both emit `mission/changed` and `session/updated`, as `mission_resume` does.

**`runner-daemon`**

- `ops/mission/roles.rs` (new, beside `roster.rs` and `feed.rs`): `add_role` and `remove_role`, and `removed_handles(log)`, which reads `role_removed` signals from `human`.
- **Add**, under the mission lock:
  1. Refuse unless the mission is `Running` and not archived ("mission “…” is not running"), and while any spawn of the mission is pending, from the cold-start queue or an earlier add ("a session of mission “…” is still starting; try again once it is up"), through a new `SessionManager::has_pending_mission_spawns` over `pending_mission_cancels` (`session/manager/lifecycle.rs` 168–200). Then register the add's cancel flag with `register_pending_mission_cancel` (decision 15). Every failure below drops it, identity-checked with `drop_pending_mission_cancel`.
  2. Load the roster (`roster::load`) and the removed handles. Take the role by id. Take the handle as given, checked with the shared rule against the roster's handles and the removed ones, or suggest one from the role's handle.
  3. Validate the overrides with `ops::slot::validate_runtime_override` and `normalize_override_value` (`ops/slot.rs` 44–51), as a crew slot does.
  4. Compose the worker's prompt channels as `mission_start_impl_with_size` does for a worker (`ops/mission.rs` 586–633: the runtime adapter's `prompt_channels().split(Worker, compose_worker_first_turn(role.system_prompt, crew_addendum))`), and check `ensure_first_turn_fits` (150).
  5. `register_mission_session` (`session/manager/spawn.rs` 752) with the new slot, which inserts the session row and records its launch definition (#878), as for any slot.
  6. Append `role_joined`.
  7. Write the roster with the new slot (`roster::write`).
  8. Router: add the roster row and `register_pending_sessions(&[(handle, session_id)])` (`router/mod.rs` 384).
  9. Append the system broadcast and, when given, the task.
  10. Spawn with `complete_mission_session_spawn` (`spawn.rs` 956) on a background thread with the flag from step 1, dropped identity-checked when the spawn ends, so a Stop all or Archive at any point since step 1 cancels it. A cancelled spawn leaves the row stopped, and the slot stays on the team for Resume. On failure, mark the session crashed and emit `session/exit` (as at `ops/mission.rs` 836–860); per decision 9 nothing else stops.
  A failure in step 5 changes nothing but the flag. A failure in step 7 or 8 appends `role_removed`, restores the previous roster file, drops the router entry and deletes the new session row, so the log, the file and the router agree again. The step 1 refusal of an earlier add's pending spawn is required, not only polite: `pending_mission_cancels` holds one flag per mission, and registering a second would replace the first, leaving that spawn uncancellable.
- **Remove**, under the mission lock:
  1. Refuse unless the mission is running and not archived, and while any spawn of the mission is pending, with Add's message (decision 15). Refuse the lead ("@architect is the lead and cannot be removed"), a handle that is not in the roster ("@x is not in this mission"), and one already removed ("@reviewer was already removed from this mission").
  2. Find the slot's session (`repo::session::latest_unarchived_id_for_slot`, `repo/session.rs` 298) and stop it if it is live with `ops::session::session_kill` (`ops/session.rs` 77), #542's Stop. That honours the stop grace period, so the call can take a few seconds. If the stop fails, nothing changes.
  3. Rewrite the roster without the slot, remove the router's row and handle entry (dropping its pending outbox), and append the two events.
- `ops/session.rs` `list_for_mission` (33–70): drop rows whose `slot_id` is not in the roster, after the Phase 1 overlay. `session_list`, `mission_resume_with` (`ops/mission.rs` 1064), the CLI's `mission show` and the app all go through it.
- `session/manager/spawn.rs` resume, at its mission-context lookup (2426, which today says "slot … referenced by session … is not in the mission roster"): a slot that is not in the roster but has a `role_removed` event fails with "@reviewer was removed from this mission" instead. Resume and Restart both pass through it.
- `router/mod.rs`: `RosterRow`s move from `LaunchInputs` (165–175) into `RouterState` (282), or behind their own lock, with `add_roster_member` and `remove_roster_member`. Update the readers: `handlers.rs` `message_nudge` (162–197), `record_slot_restart` (1325–1360) and `prompt_channels` (1545–1575), the last also used by `ops/mission/roster.rs` 774.
- `event_bus/mod.rs`: `BusState` (255) keeps the team that broadcasts reach. It starts from the roster's handles minus those with a `role_joined` in the log, adds a handle at its `role_joined` and drops it at its `role_removed`, inside `tick` before the message projection (around 326). A directed message lands in its addressee's inbox whether or not that handle is on the team, except a `to` of `human`. Update the "not supported in MVP" doc comment at 136.
- `router/prompt.rs`: the line of decision 13 in `WORKER_COORDINATION_PREAMBLE` (109–116) and in the lead's Coordination section (180–200), for example "- The team can change while the mission runs; your inbox announces every join and leave, and `runner mission show` lists the current team." The prompt literal tests (`compose_launch_prompt_matches_established_literal_contract`, `established_runtimes_keep_composed_bodies_byte_identical`) take the new line.
- `ops/mission.rs` `post_message` (the `--to` refusal at 466): the refusal lists the current handles, "@reviewer is not in the mission roster; the team is @architect, @impl and @impl-2."
- `AppCore`: the per-mission lock map, for example `mission_roster_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>`.
- `ops/mission/roster.rs` `write`: the Windows rename retry.
- `docs/arch/arch.md`: §7 and §8 for `role_joined`, `role_removed` and a roster that changes while the mission runs, §3 for the bus learning membership from the log.

**`runner-cli`**

- `command.rs` `MissionCommand` (308): `AddRole { mission, role, handle, runtime, model, effort, speed, task, task_file }` and `RemoveRole { mission, handle }`. `role` takes a handle or an id, resolved like the other role references. `--task` and `--task-file <path|->` conflict, as `--goal` and `--goal-file` do. Both commands refuse a caller inside a mission ("runner mission add-role is for the person; ask the person to add a role") and `--as`. `-q` prints the handle. `client.rs` (dispatch at 299) gains `mission_add_role` and `mission_remove_role`.
- `help.rs` COMMON FLOW (78–88): one line each for `mission add-role "$mission" <role> --task-file - < task.md` and `mission remove-role "$mission" <handle>`.
- `roster.rs` already rereads `roster.json` on every call, so `--to` accepts a newcomer and refuses a removed handle as soon as the file is rewritten. Its in-mission refusal lists the current handles, as the daemon's does (decision 13).
- `msg.rs` is unchanged: `runner msg read` keeps printing every broadcast since the mission started, a newcomer's included (decision 4).

**`runner-app`**

- `mission_workspace/rail.rs` `render_roles_rail` (180): the **+ Add role** row after the cards, shown when `mission_slot_actions_available` (`mod.rs` 111) holds. On worker cards, the remove button after the slot controls (`log-out`, tooltip "Remove from mission"), disabled while the card's other actions are.
- `mission_workspace/add_role.rs` (new): the Add role modal. It reuses Start chat's role select, setup fields and override notes (`surfaces/start_chat.rs` `render_role_fields` 1825 and `render_role_setup` 1864) by moving them into a module both forms use, rather than copying the override logic. The handle field validates on every edit with the shared rule against `this.roster` and the removed handles from `this.events`. The Task field is multiline, and Add role is disabled until a role is picked and the handle is valid.
- Confirmation: `runner_app::ui::ConfirmDialog`, as Restart's confirmation uses it (`mission_workspace/actions.rs` 149), with the frame's copy and a danger Remove.
- `mission_workspace/feed.rs`: `group_feed_blocks` (`surfaces/mission_feed.rs` 140) gives `role_joined` and `role_removed` a divider block, as it does `mission_start` and, since #881, `mission_stopped`, whose label the renderer picks from the signal (about 400–425); the two new dividers are rendered as in the frames: a `log-in` or `log-out` icon and "@impl-2 joined · added by you · 14:02" or "@reviewer left · removed by you · 14:31". `render_mission_message_group` (439) renders `runner` as `system` per decision 11.
- `mission_workspace/events.rs`: an `event/appended` carrying `role_joined` or `role_removed` triggers `refresh_open_mission` (324), which also reloads `mission_roster` from now on. A removed session's tab closes, and the Feed tab takes focus when it was active.
- `app_store.rs`: add `role_joined` and `role_removed` to the `Missions` refresh signals (60–68).

**Docs**

- The spec's Phase 2 bullets and Decisions to confirm, revised to decisions 4, 6, 9, 10 and 15 above, on this branch.
- `docs/tests/archive/562-add-remove-roles.md`: the test record with Jason's live checks.

### Tests

Daemon tests use `runtime: "shell"` roles and temporary app data, as Phase 1's do; a `runnerd` restart is an unmount followed by `ensure_mission_router_mounted`.

- Add: the three events land in order with the documented payloads; the bus's unread count for the newcomer covers the introduction and the task and nothing earlier, while `runner msg read` as the newcomer also prints the earlier broadcasts (decision 4); `--to` the newcomer is accepted right after the add, from the daemon (`post_message`) and from the CLI's `roster.json` check; a later broadcast reaches its inbox and nudges its session; after a re-mount the router's roster and the bus's handles include it and its unread count still starts at the join.
- Handles: the suggestion skips current and removed handles; an explicit removed, taken or invalid handle is refused with the shared messages; two missions of one crew can each add `@coder-2`; two concurrent adds to one mission get different handles.
- Add refusals: a mission that is not running, an archived one, one whose cold-start queue is running, one with an earlier add's spawn still pending, an unknown role and invalid overrides.
- Cancellation: a Stop all or an Archive after an add's checks cancels its spawn, and no process starts; after Stop all the newcomer stays on the team, stopped, and Resume starts it (decision 15).
- A newcomer that fails to spawn, or exits after spawning, is marked crashed while the rest keep running (decision 9, landed by mission 2).
- A failed roster write after `role_joined` appends `role_removed` and leaves no slot, row or router entry behind.
- Launch definitions (#878): an added slot's session records its definition with the add's overrides; after the role is edited, Resume replays the recorded one and Restart rebuilds it from the current role and the roster slot's overrides.
- Remove: the session stops, the roster drops the slot, the two events land, `--to` the removed handle is refused, a broadcast skips its inbox and its session, `session_list` and Resume all leave it out, resuming it by id fails naming the handle, and all of this survives a re-mount. Removing the lead, an unknown handle and an already-removed handle is refused, and so is any removal while a spawn of the mission is pending, cold start or an add's; the queued slot then starts and stays on the team.
- Role deletion stays refused while the unarchived mission has the removed slot's session.
- Bus: a `role_joined` or `role_removed` whose `from` is not `human` changes nothing; a directed message appended before its addressee's `role_joined` still lands in its inbox, and a broadcast appended before it does not.
- CLI: parsing, help, `--task-file -`, the in-mission and `--as` refusals, `-q`, and golden output for both commands; `runner mission show` inside a mission lists an added handle and leaves a removed one out; a `--to` refusal lists the current handles.
- Prompts: the worker's first turn and the lead's launch prompt carry the team-changes line.
- App: the handle rule in the modal (suggestion, taken, removed, invalid); the rail shows a session added after `session/spawned` and hides a removed one; feed blocks and copy for both signals and the `system` label on `runner` messages; the refresh on the two signals.

### Live checks

Run on macOS by the trio's `qa` slot in the development app, following `docs/tests/full-smoke-test.md`, with bounded test missions on real agents; Windows is Jason's. Recorded in `docs/tests/archive/562-add-remove-roles.md`. Selected because Phase 2 changes the roster, routing and inboxes of a running mission:

- Add a role from the rail with a task. The newcomer boots, reads the introduction and the task, and posts to the lead. The lead reaches it with `--to`, and a broadcast reaches it.
- Restart `runnerd` (quit the app and run `make run` again). The newcomer is still on the team and resumes, and its unread count does not include messages from before its join. Its `runner msg read` shows the earlier broadcasts too, as intended (decision 4).
- Remove a worker from the rail. Its session stops, `--to` it is refused, broadcasts skip it, its history stays in the feed, Resume all does not bring it back, and all of this holds after another restart.
- The same through `runner mission add-role` and `remove-role`, plus the refusals from inside a mission session.
- The affected regression cases: MIS-CREW-01, MIS-LIFE-01, MIS-MSG-01, MIS-MSG-02, MIS-FEED-01, and MIS-ROSTER-01, because the roster now changes while the mission runs and a remount rebuilds it from the file. Two new cases for the suite, add and remove, are written into the test record and promoted at the release sweep.

Two lessons from #881's QA run (2026-10-13). Prove what an agent was given from its saved conversation (the runtime's transcript), not from a cheap model's recall, which missed twice there; a recall is supporting evidence only. And keep test roles from answering every inbox notice: every join and leave broadcasts to the whole team, and two agents told to reply ACK to each message kept answering each other after a Restart in that run.

### Mission shape

Mission 3, on the codex trio (coder, reviewer, qa), at Jason's call on 2026-10-12, so `qa` runs the mission regression checks. It starts from `main` after [#878](https://github.com/yicheng47/runner/issues/878) merged as [#881](https://github.com/yicheng47/runner/pull/881) on 2026-10-13, so an added role's session records its launch definition like any other, and ends in one pull request against `main`, in three commits that each build and pass their tests: the handle rule moved into `runner-core`; the daemon, protocol and CLI; the app. The brief goes to `docs/impls/briefs/562-m3-add-remove-roles.md` and is folded into a commit. The coder and reviewer never run the dev app.

### Risks

- **A crash no longer stops the mission, so it is easier to miss.** Before #874 a dead slot paused everyone and the person noticed at once; now the crashed card is the only sign, and the team is not told. A crash notice to the team is the spec's `slot_exited` follow-up.
- **Stop takes time.** `session_kill` honours the stop grace period (M6.16), so a removal can block for a few seconds. The card's actions stay disabled while it runs, as they do during a Stop (`slot_actions` in `rail.rs` about 235), and the CLI waits.
- **A removed agent can post during its stop.** Until the roster is rewritten, `--from` the removed handle is still accepted. Its messages then sit in the feed before the leave divider, which is the true order.
- **The worker prompt names the crew.** `WORKER_COORDINATION_PREAMBLE` says "You are a worker in a crew" and "any slot in this crew". It holds for an added worker; Phase 3, missions without a crew, changes the wording.
- **A failed add leaves a trace.** A roster write or router failure after `role_joined` appends `role_removed`, so the feed shows the handle joining and leaving, the handle is used up (decision 3), and `role_joined` names a session row that was deleted. Accepted: the log stays true, and a failure there is rare.
- **A removed agent's unanswered question stays pending.** If it had asked the person (`ask_human`), the question stays open in the app after the removal, and answering it reaches no session. Accepted for this phase; the person sees the leave divider beside it.
- **Moving Start chat's role and setup fields into a shared module** touches a 5,000-line file the Start chat tests cover. The commit runs those tests unchanged.

## Phase 3 — start empty

A mission can start with no crew and no roles; roles join one at a time through Phase 2's add-role. Settled by Jason on 2026-10-15, replacing start-from-roles; the spec's Phase 3 has the scope. An empty mission has no lead, so this phase builds [#873](https://github.com/yicheng47/runner/issues/873)'s no-lead paths for it. It also builds [#849](https://github.com/yicheng47/runner/issues/849)'s Start mission redesign, since the No crew entry lives in that dialog's picker.

### Design

`runner/desktop/specs/archive/562-mission-spawn.pen`, row at y 3400, signed off by Jason on 2026-10-15 (`12bef39f`):

| Frame | Id | Shows |
| --- | --- | --- |
| Mission — no roles yet | `PhtPw` | The workspace right after an empty start. The goal is posted with no target. The feed shows "No roles yet" with a primary **Add role**, in place of the Mission paused overlay; the rail shows "No roles yet." above its **+ Add role**; the composer is in place but disabled, "Add a role to start messaging". |
| Start mission — from a crew | `AVJ15` | 849's dialog: no mode switch; a **Crew** card whose top row is the picker (crew picture, name, `3 slots · lead @architect`, chevron) over read-only roster rows (avatar, handle, LEAD, role, runtime · model · effort with override dots); Mission title, Goal (optional) and Working directory with Browse; footer Cancel (`esc`) and Start mission (`⌘↵`). |
| Start mission — No crew | `m2Noaa` | No crew picked: the card's top row is a `user-plus` tile, "No crew", `start empty`, and the card body says roles join from the mission one at a time, each as a worker. The Goal hint reads "Shared with every role that joins. Give each one its own task when you add it." |
| Start mission — crew picker open | `iJuGG` | The picker's menu: **No crew** first ("start empty · add roles from the mission"), a divider, then the crews with picture, name and summary; the current crew is checked. |
| Mission — first role joined, no lead | `BsErp` | `@impl` joined: its card has no LEAD badge; the join divider, the system introduction ("The team is now @impl."), and the person's task to `@impl`; the composer reads "Message everyone — @handle to address one". |
| Mission info — no Crew row | `blE2u` | The info panel for every mission: Mission ID, Goal, Working dir, Permissions, Started. The Crew row is gone. |

### Decisions that bind

1. **A start names no roles** (spec decision 2). `mission_start` takes an optional crew. Without one, `start` writes a version 2 `roster.json` with no slots, an empty `crew_name` and no addendum, appends `mission_start` and `mission_goal` as today, and spawns nothing. The mission is `running` until it is stopped or archived, like any mission.
2. **An empty mission has no lead, and its members work together without one** (spec decision 3; Jason, 2026-10-15). Members address each other with `runner msg post --to`, broadcast, read their inbox, see the team in `runner mission show`, and ask the person with `ask_human`. None of these goes through a lead today: `message_nudge`, `ask_human` and `human_response` route by handle and by asker (`router/handlers.rs` 100–197). Add role keeps adding workers. The lead stays optional rather than fixed at none: setting or clearing a lead while a mission runs is [#873](https://github.com/yicheng47/runner/issues/873)'s, which may make the lead optional for every mission. A crew start still requires and keeps exactly one lead.
3. **A mission's directory is fixed when it starts and never moves.** A crew start keeps `crews/<crew_id>/missions/<mission_id>/`. An empty start uses `missions/<mission_id>/`, the directory that already holds its `runner` shims (`cli_install.rs` 53). One resolver, `event_log::mission_dir(app_data, crew_id: Option<&str>, mission_id)`, picks between them, and every caller passes the mission's `crew_id`. Recommended over the alternative of moving every mission's log into `missions/<id>/` at `runnerd` start: that would make paths crew-free, but it moves every existing mission's files, the cost Phase 1 declined, and gains nothing a fixed path does not already give.
4. **`missions.crew_id` becomes nullable provenance with no foreign key.** It names the crew a mission started from, or nothing for an empty start. It is not a foreign key, because decision 3 needs a deleted crew's missions to keep finding their logs under its id, and `ON DELETE SET NULL` would lose that. This refines the spec's rule 4: deleting a crew leaves its missions' `crew_id` pointing at a crew that no longer exists, which every reader must treat as "no crew": `mission_list_summary` already does, and `mission_status` is made to (`MissionStatusSnapshot.crew` becomes optional), while the Crew row is gone (decision 12). Deleting a crew deletes only the crew and its slots: it stops refusing while missions are unarchived (`ops/crew.rs` 175–181) and stops deleting archived missions (182).
5. **The rebuild of `missions` needs the migration runner to turn foreign keys off first** (`migrations/README.md`, DDL rules). `sessions.mission_id` references `missions` with `ON DELETE SET NULL`, so dropping the old table inside a foreign-keys-on transaction would detach every session. The runner gains a per-migration flag that applies it with `PRAGMA foreign_keys = OFF` set before its transaction opens, runs `PRAGMA foreign_key_check` before commit and fails the migration on any row, and turns foreign keys back on. That lands as its own commit, then `0028_optional_mission_crew_restrict_deletes.sql` rebuilds `missions` with `crew_id TEXT` (nullable, no reference), copying every row and recreating `idx_missions_project`.
6. **The database never deletes: every `ON DELETE CASCADE` becomes `RESTRICT`** (Jason, 2026-10-15). Since `da4333ac` (2026-09-24) every delete path removes or unbinds its dependent rows in code, and each has a test with foreign keys off, so the cascades never act; `RESTRICT` turns the backstop from deleting into refusing. The same migration rebuilds `slots` (`crew_id`, `role_id`), `sessions` (`role_id`) and `session_attention` (`session_id`) with `RESTRICT`, following SQLite's documented table-change procedure under decision 5's foreign-keys-off mode, and copying every column. The `SET NULL` keys (`missions.project_id`, `sessions.mission_id`, `sessions.project_id`) stay: they clear a link rather than delete a row, and the app already unbinds those rows itself. `migrations/README.md`'s DDL rules change with it: the app owns every delete, a foreign key is `RESTRICT` or `SET NULL` and never `CASCADE`, and a referenced table is rebuilt through the foreign-keys-off mode.
7. **An empty mission's envelopes and environment carry no crew.** `Event.crew_id` and `EventDraft.crew_id` become `Option<String>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`, so existing logs still parse and a crew-less event omits the field. Its sessions get no `RUNNER_CREW_ID`, in the PTY environment and in the shim. The CLI's environment check (`runner-cli/src/env.rs` 43–60) stops requiring `RUNNER_CREW_ID`.
8. **The no-lead branches** ([#873](https://github.com/yicheng47/runner/issues/873)'s table, for missions without a lead only):
   - The goal: a lead-less member's first turn carries it, in a `== Mission goal ==` section after the preamble, omitted when the goal is empty. A crew mission's workers get no goal section; their prompt changes only as decision 16 says.
   - The worker preamble: a lead-less variant that says "team", not "crew", and points a member at the person with the lead prompt's `ask_human` line for decisions it cannot make.
   - `human_said` with no target, which today falls back to the lead (`router/handlers.rs` 49–66), is delivered to every live member. The app's composer is not on this path: without a recipient it already posts a broadcast `message` (`ops/mission.rs` `post_message` 437), which nudges every member.
   - A slot's restart (`Router::record_slot_restart`, `router/mod.rs` 1353) tells every other member, as it already does when the lead restarts.
   - `LaunchInputs.lead` becomes `Option<LeadRow>` (`router/mod.rs` 167, built at 1545–1576), and `Router::lead_handle()` returns `Option<&str>`. The lead's launch prompt (`prompt_channels`, 1578) is only composed for a mission that has one.
9. **The composer is disabled while the roster is empty.** Its placeholder is "Add a role to start messaging" then. In a mission without a lead it reads "Message everyone — @handle to address one"; a crew mission keeps "Message the crew — @handle to address one session" (`mission_workspace/state.rs` 63).
10. **An empty roster is not a paused mission.** The Mission paused overlay (`view.rs` 879–884, `terminal_pane.rs` 314) requires at least one roster slot. An empty roster shows the feed's empty state and the rail's, as in `PhtPw`.
11. **#849 is built in this mission.** The redesign is the dialog the No crew entry goes into, and drawing them together settled both. It lands as its own commit, closing #849, before the commit that adds No crew. #849's spec's Design line points at `562-mission-spawn.pen` (`AVJ15`, `iJuGG`); its other states (five or more slots, a crew with no slots) follow the same card in code without separate frames.
12. **The mission info panel drops its Crew row for every mission** (spec decision 14). `mission_workspace/attach.rs` stops loading the crew (88, 130), and `rail.rs` loses the section (705–740) and its `open_crew_editor` click.
13. **No fallback for a crew-less roster.** `roster::load` falls back to the crew's current slots when `roster.json` is missing or not JSON (`ops/mission/roster.rs` 80, 145). A mission without a crew, or whose crew was deleted, has nothing to fall back to, so that case returns the load error. The version 1 conversion at `runnerd` start (`daemon/boot.rs` 98) never sees one, since no version 1 file exists without a crew.
14. **A mission an agent starts does not take focus** (spec decision 13). That is today's behaviour: only the app's Start mission dialog opens the mission it starts (`start_mission.rs` 345); one started over the client protocol refreshes the sidebar through `mission/changed`. Nothing changes; a live check pins it.
15. **The CLI starts empty without `--crew`.** `runner mission start` takes `--crew` as optional. Without it the title defaults from the goal's first line, and with neither `--title` nor `--goal` the start is refused: "an empty mission needs --title or --goal". `runner --help`'s COMMON FLOW and the embedded `runner` skill teach delegation as `mission=$(runner mission start --title … -q)` followed by `runner mission add-role "$mission" <role> --task-file - < task.md`.
16. **`ask_lead` is removed from every mission** (Jason, 2026-10-15). It predates the inbox: it pastes a worker's question into the lead's terminal, while `runner msg post --to <lead>` reaches the same agent through the path every other message uses, with a nudge and a feed entry. The lead still escalates to the person with `ask_human` on a worker's behalf (`on_behalf_of`). So: `KnownSignalType::AskLead` goes (`runner-core/src/model.rs` 53, 65, 77) and with it the router's handler; `runner ask "<question>"` and `runner signal ask_lead` are refused with "ask the lead with `runner msg post --to <lead>`, or the person with `runner ask --human`"; the crew worker preamble's `ask_lead` line becomes "Questions for the lead go to it with `runner msg post --to <lead>`; `runner mission show` names it."; the lead prompt's "Workers escalate to you via `ask_lead`." becomes "Workers message you with questions." Old `ask_lead` events stay in their logs and keep rendering in the feed (`mission_workspace/feed.rs` 575) and the CLI. This replaces the lead-less refusal the plan first had, since no mission has `ask_lead` any more.

### Where Phase 3 lands

Line numbers are as of `cf839e16`.

**`runner-core`**

- `event_log/path.rs` (17–25): `mission_dir` and `events_path` take `crew_id: Option<&str>` (decision 3).
- `model.rs` `Event` (106) and `EventDraft` (123): `crew_id: Option<String>` (decision 7); the `EventDraft::signal` and `message` constructors take `Option<&str>`.
- `model.rs` `KnownSignalType::AskLead` (53, 65, 77) removed (decision 16); old events keep their `type` string and still parse.
- `protocol/model.rs` `Mission` (133): `crew_id: Option<String>`. `protocol/mission.rs`: `StartMissionInput.crew_id` (7) and `MissionStart.crew_id` (28) become `Option<String>`; `MissionSummary.crew_name` (74) becomes `Option<String>`.
- `mission_roster.rs`: unchanged format; an empty start writes `slots: []`.

**`runner-daemon`**

- `db/migrations.rs` and `db/mod.rs` (69): the foreign-keys-off migration flag and its `foreign_key_check` (decision 5), in its own commit with a test that a rebuild of a referenced table keeps its children.
- `migrations/0028_optional_mission_crew_restrict_deletes.sql`, registered in `MIGRATIONS` (72), and `schema.sql` regenerated: `missions` without the `crew_id` reference (decision 4) and `slots`, `sessions` and `session_attention` with `RESTRICT` (decision 6). A test with `run_migrations_up_to(27)` and populated crews, roles, slots, missions, sessions and attention rows checks that every row, every column and every `sessions.mission_id` survives and that `foreign_key_check` is empty.
- `ops/tests.rs` `every_delete_leaves_foreign_key_check_empty_on_a_real_database` (17–45): the pinned foreign-key list loses `missions.crew_id` and reads `RESTRICT` for the four former cascades; every delete path still passes with the pragma on.
- `migrations/README.md`: the DDL rules of decision 6.
- `ops/mission.rs` `start` (165–277): the crew checks (183–200) only when a crew is given; `roster::snapshot` for no crew; the directory from decision 3 (237). `mission_start_impl_with_size` (532) spawns nothing for an empty roster and mounts the router and bus with no handles. `ensure_mission_router_mounted` (around 977–1010) mounts an empty or lead-less roster. Every other `mission_dir` and `events_path` caller (`stop` 314, `read_events` 346, 423, `post_message` 478, `delete_archived` 1234, `mission_list_summary` 1450 and 1465) passes `mission.crew_id.as_deref()`; `mission_list_summary` resolves `crew_name` only when the crew exists (1429).
- `ops/mission/roles.rs` (add and remove): unchanged logic; paths and envelopes through the new signatures, and a lead-less member's first turn per decision 8.
- `ops/mission/roster.rs` `load` (57): no fallback without a crew or with a deleted one (decision 13), at the `crew::get` calls (79, 144).
- `ops/mission/feed.rs` `mission_status` (213): the crew is looked up, not required; `MissionStatusSnapshot.crew` becomes `Option<Crew>`, so `runner mission show`, which lead-less members use to see their team (decision 2), works for an empty mission and for one whose crew was deleted.
- `ops/crew.rs` `delete` (173–190): delete the crew and its slots only (decision 4).
- `router/mod.rs`: `LaunchInputs` (164–188, 1545–1576) with an optional lead; `lead_handle` (529); `record_slot_restart` (1353); `prompt_channels` (1578) for a mission with a lead.
- `router/handlers.rs`: `human_said` (49) per decision 8; `ask_lead` (69) and its dispatch arm in `router/mod.rs` (around 557) removed (decision 16).
- `router/prompt.rs`: the lead-less worker preamble beside `WORKER_COORDINATION_PREAMBLE` (109) and the goal section in `compose_worker_first_turn` (66); the crew preamble's `ask_lead` line (115) and the lead's "Workers escalate to you via `ask_lead`" (183) per decision 16. The literal contract tests (`compose_launch_prompt_matches_established_literal_contract` with its 338 literal, `established_runtimes_keep_composed_bodies_byte_identical`) take exactly those line changes and nothing else.
- `session/manager/spawn.rs` (846, 2640) and `cli_install.rs` (45–80, 128): no `RUNNER_CREW_ID` without a crew.
- `docs/arch/arch.md`: §3 and §10 for an optional crew and the `missions/<id>/` directory, §7 for an event without `crew_id`, §8 for a mission without a lead.

**`runner-cli`**

- `command.rs` `MissionCommand::Start` (318–339) and its handler (1480–1526): `--crew` optional, the title rule (decision 15).
- `env.rs` (11–60): `crew_id: Option<String>`.
- `command.rs` `ask_signal` (2100–2115): `runner ask "<question>"` without `--human` is refused with decision 16's hint; `runner ask --human` is unchanged. `runner signal ask_lead` is refused as an unknown type once `AskLead` leaves `KnownSignalType`, with the same hint. `output.rs` keeps rendering old `ask_lead` events, as its test at 1137–1146 pins; the CLI goldens (`tests/goldens/cli.txt`) and `help.rs`'s `AGENT_GUIDE` follow.
- `output.rs` (313, 339, 361): an empty crew renders as `—`.
- `help.rs` `AGENT_GUIDE` COMMON FLOW (78–88), which the embedded `runner` skill carries: the start-then-add-role pair beside the crew start, and the `--to <lead_handle>` line (86) saying that a mission without a lead is addressed by any member's handle from `mission show`.

**`runner-app`**

- `surfaces/start_mission.rs`: #849's redesign in its own commit (crew card, picker menu, roster rows, fields, footer; the Advanced section and session count removed), reusing `crew_picture`, `RoleAvatar`, `lead_badge` and `slot_setup_line` from `surfaces/crews/` as `pub(crate)`. Then the **No crew** entry first in the picker, its card, the Goal hint, picking it when there are no crews, and `MissionStart { crew_id: None, … }`.
- `mission_workspace/view.rs` (878–884): the paused overlay only with a non-empty roster (decision 10), and the feed's empty state with **Add role** for an empty roster.
- `mission_workspace/rail.rs`: "No roles yet." above **+ Add role** for an empty roster; the Crew section removed (705–740, decision 12).
- `mission_workspace/composer.rs` and `state.rs` (63): disabled with an empty roster; the lead-less placeholder (decision 9).
- `mission_workspace/feed.rs` (524–533): the goal row shows no `→ @target` without a lead.
- `mission_workspace/attach.rs` (88, 130): no crew load.
- `command_palette.rs` (193): search text without a crew name.

**Docs**

- The spec's status line, rule 4 refined by decision 4, and Phase 3's scope where this plan sharpened it, on this branch.
- `docs/tests/archive/562-start-empty.md`: the test record with the live checks.
- `docs/tests/regression/missions.md`: MIS-ASK-01 rewritten without `ask_lead` (decision 16).

### Tests

Daemon tests use `runtime: "shell"` roles and temporary app data; a `runnerd` restart is an unmount followed by `ensure_mission_router_mounted`.

- Start empty: the mission row has no `crew_id`, the log is in `missions/<id>/` with `mission_start` and `mission_goal` events that omit `crew_id`, `roster.json` has no slots, no session is created, and the router and bus mount; a re-mount works.
- First add: the newcomer's first turn carries the lead-less preamble and the goal section, and no mention of `ask_lead`; its environment has no `RUNNER_CREW_ID`; it posts with `runner msg post` and a second added role receives it. A crew mission's worker and lead prompts differ from today's only in decision 16's lines.
- Members without a lead: two added roles exchange a direct message each way and a broadcast, and one's `ask_human` answer returns to it and not to the other. `human_said` without a target reaches every live member; a restart notice reaches every other member.
- `ask_lead` removed (decision 16): `runner ask "<question>"` and `runner signal ask_lead` are refused with the hint in every mission; a log holding old `ask_lead` events still renders in the app's feed and in `runner mission feed`, and replaying it injects nothing.
- Refusals: `mission_start` with an unknown crew, a crew with no slots or no lead, as today; the CLI without `--crew`, `--title` and `--goal`.
- Crew deletion: deleting a crew with a running mission and an archived one keeps both rows, their logs and their sessions; the running mission still posts, adds a role, and re-mounts.
- Migration: decision 5's runner test, and 0028 against a populated version 27 database. With `RESTRICT`, deleting a crew, role, mission or session that still has dependents straight through `repo` fails, while every `ops` delete succeeds.
- Events: a log written before this change parses; a crew-less event serializes without `crew_id`.
- `mission_status` and `runner mission show` for an empty mission, a lead-less one with members, and one whose crew was deleted: the team is listed and the crew renders as `—`.
- CLI: parsing, help and golden output for a start without `--crew`; `env.rs` without `RUNNER_CREW_ID`.
- App: the picker's entries and order, No crew's card and hint, picking No crew with no crews, the start input without a crew; #849's roster rows for a crew with overrides and the keyboard order without Advanced; the empty workspace instead of the paused overlay; the composer disabled and its placeholders; the info panel without a Crew row.

### Live checks

Run on macOS by the trio's `qa` slot in the development app, under the standing authorization in `AGENTS.md`; Windows is Jason's. Recorded in `docs/tests/archive/562-start-empty.md`. Selected because Phase 3 changes how a mission starts, its directory and envelope, and every lead fallback:

- Upgrade the development database: start the candidate on the existing development data and compare counts of crews, slots, roles, missions, sessions and attention rows, and sessions with a mission, before and after. Existing missions and chats resume.
- Start a mission from the dialog with **No crew**. It opens on the empty state with no paused overlay; the composer is disabled. Add a role with a task: it boots with the goal, reads the task and posts. Add a second; a composer message without a recipient reaches both, and the two coordinate with each other through `--to` with no lead in the mission.
- Restart `runnerd` with one empty mission and one with a single added role. Both come back; the second resumes.
- From a direct chat, delegate with `runner mission start --title … -q` and `runner mission add-role … --task-file -`. The mission appears in the sidebar without taking focus.
- Delete a crew while a mission started from it runs. The mission keeps running, posting and adding roles, and its info panel has no Crew row.
- In any mission, `runner ask "<question>"` is refused with its hint, and `runner ask --human` reaches the person's card. An existing mission whose log has `ask_lead` events still opens and renders them.
- The affected regression cases: MIS-CREW-01 (a crew start through the redesigned dialog), MIS-LIFE-01, MIS-MSG-02 (the composer's default target), MIS-ASK-01 (rewritten in `docs/tests/regression/missions.md`: the worker messages the lead with `--to`, the lead escalates with `runner ask --human` on its behalf, and the answer returns), MIS-ROSTER-01 and MIS-FEED-01. New cases for an empty start and crew deletion go into the test record and are promoted at the release sweep.

### Mission shape

Mission 4, on the codex trio (coder, reviewer, qa), on `feat/562-start-empty` in `.worktrees/feat-562-start-empty`, ending in one pull request against `main` in five commits that each build and pass their tests: the migration runner's foreign-keys-off flag; migration 0028 (`crew_id` optional, `RESTRICT` instead of `CASCADE`) with the README's rules; the crew-optional start, envelope, directory and no-lead paths across core, daemon and CLI; #849's Start mission redesign; the No crew entry, the empty workspace and the info panel. The brief goes to `docs/impls/briefs/562-m4-start-empty.md` once this plan is signed off, and is folded into a commit. The coder and reviewer never run the dev app.

### Risks

- **0028 rebuilds four tables, two of them referenced** (`missions` by `sessions`, `sessions` by `session_attention`), and `sessions` is the largest table. Decision 5 guards it with `foreign_key_check`, the version 27 test proves no row or column is lost, and the live upgrade check runs it on real data; a failure rolls the migration back and the next start retries it.
- **`RESTRICT` turns a missed cleanup into an error.** A delete path that forgets a dependent now fails instead of being saved by a cascade. Every path already has a foreign-keys-off test, and `ops/tests.rs` runs them all with the pragma on.
- **A dangling `crew_id`.** After a crew is deleted its missions name a crew that does not exist. Every reader must treat that as no crew; the Crew row's removal leaves `mission_list_summary`, `mission_status` and the CLI's `CREW` column as the readers; the first falls back already, and the plan makes the other two.
- **Envelope shape.** Events of an empty mission omit `crew_id`. The bundled CLI and the app read through `runner-core`'s type; any outside script that parses `events.ndjson` and expects the field breaks, which this accepts.
- **New prompt text.** The lead-less preamble and the goal section are new words agents act on, and decision 16 changes one line of every crew mission's worker and lead prompts; the live checks read them from the agent's saved conversation, as #881's QA learned to.
- **A lead now reads questions from its inbox.** Without `ask_lead` a worker's question reaches the lead as an `[inbox]` nudge rather than inline text, one `runner msg read` later. MIS-ASK-01 checks the lead still picks it up.
- **An empty mission never ends by itself.** It stays `running` in the sidebar until the person stops or archives it, as a mission whose sessions all exited does today.
- **#849 widens the app commit.** It is presentation only and its own commit, so review can take it apart from Phase 3's behaviour.

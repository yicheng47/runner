# 562 — Missions as containers: missions without a crew, roles added and removed while they run

> Tracking issue: [#562](https://github.com/yicheng47/runner/issues/562), "delegate work to other agents". Priority: P1, milestone 0.16 (moved from 0.17 on 2026-10-11, when #562 became the parent of [#863](https://github.com/yicheng47/runner/issues/863) and [#704](https://github.com/yicheng47/runner/issues/704) closed into it). [#748](https://github.com/yicheng47/runner/issues/748), typing a mission's updates into the chat that started it, was a sub-issue until 2026-10-13, when Jason detached it as unrelated.
> Status: Phase 1, the mission roster, merged on 2026-10-11 as [#868](https://github.com/yicheng47/runner/pull/868), fixing [#863](https://github.com/yicheng47/runner/issues/863), and shipped in 0.15.2. Phases 2 and 3 each ship with their app UI, designed first. Phase 2, adding and removing roles while a mission runs, merged on 2026-10-13 as [#889](https://github.com/yicheng47/runner/pull/889), after [#878](https://github.com/yicheng47/runner/issues/878) merged as #881. Phase 3, starting a mission empty, merged on 2026-10-15 as [#906](https://github.com/yicheng47/runner/pull/906), with [849](https://github.com/yicheng47/runner/blob/8462b5194e0f1945ef62b1af1a2758c6816fd70d/docs/features/archive/849-start-mission-modal.md)'s Start mission redesign; its plan is Phase 3 of `docs/impls/562-mission-spawn/plan.md`. The implementation record is `docs/impls/562-mission-spawn/`. #562 closed on 2026-10-16; its follow-ups are [#916](https://github.com/yicheng47/runner/issues/916), [#917](https://github.com/yicheng47/runner/issues/917), [#918](https://github.com/yicheng47/runner/issues/918) and [#915](https://github.com/yicheng47/runner/issues/915).
> Design: `runner/desktop/specs/archive/562-mission-spawn.pen`, drawn on `main` phase by phase: Phase 2's frames before Phase 2 starts, Phase 3's before Phase 3, the latter together with the Start mission dialog of [849](https://github.com/yicheng47/runner/blob/8462b5194e0f1945ef62b1af1a2758c6816fd70d/docs/features/archive/849-start-mission-modal.md).
> Identity: a caller is the person at the app or a roster handle, never a location ([648](./648-runner-cli.md) decision 7).
> Related: [704](https://github.com/yicheng47/runner/issues/704), `runner session send` into a chat that is already open, closed into this issue on 2026-10-11 and not scheduled. [#863](https://github.com/yicheng47/runner/issues/863), the crew-edit roster bug that Phase 1 fixes. [#878](https://github.com/yicheng47/runner/issues/878), sessions recording their launch definition at spawn, the prerequisite for Phase 2. [#873](https://github.com/yicheng47/runner/issues/873), making the mission lead optional; Phase 3 builds its no-lead paths, since an empty mission has no lead.
> Rewritten 2026-10-10 against the `runnerd` code, after Jason set the direction: a crew is not a prerequisite for a mission, a mission can start from a single role, and roles can be added to or removed from a running mission when needed. The 2026-09-18 draft's lead verbs (`runner spawn`, `ps`, `stop`) and outside seats move to Follow-ups; they build on this.
> Revised 2026-10-11: Jason replaced the 2026-10-10 choice of a crew row per mission (`crews.kind`, `template` or `mission`) with a roster file in the mission's directory. The database holds the definitions that are listed and searched (roles, crews and their slots); a mission's roster is the mission's own state, like its event log.
> Revised 2026-10-12: Jason folded the app phase into Phases 2 and 3, so everything a phase adds can be done from the GUI as well as the CLI, and reopened Phase 3's interface for discussion: the drafted one is not good enough. Later that day he settled what is copied and when: roles and crews are templates, a session records its whole launch definition at spawn (#878, before Phase 2), and a roster never copies a role's configuration.
> Revised 2026-10-15: Jason settled Phase 3 as an empty mission. A start names no roles, and roles join one at a time through Phase 2's add-role. This replaces the start-from-roles draft and the 2026-10-10 recommendation that a mission cannot start empty, and returns to his 2026-09-18 direction that a mission starts empty or from a crew.

## Motivation

A mission can only start from a crew today. `ops::mission::start` refuses a crew with no slots or no lead, every session is spawned from one of the crew's slots, and the roster cannot change until the mission ends. Three problems follow:

- **One agent handing work to another.** Jason's daily pattern (a Claude Code session plans, a Codex agent implements) needs a one-slot crew ("codex solo") just to start a mission. A crew of one is setup in front of a single role.
- **A mission that needs another pair of hands.** When the task splits three ways, or a reviewer is needed that the crew never had, the only move is to stop the mission, edit the crew and start over, losing every conversation. The reverse, letting an agent go once its part is done, is not possible either.
- **A running mission does not own its roster** ([#863](https://github.com/yicheng47/runner/issues/863)). The daemon and the app read the crew's slots as they are now, so editing the crew while a mission runs changes that mission the next time its router is rebuilt, which happens on every `runnerd` restart, including an app update. The router, the bus, the CLI's `roster.json` and the live sessions then disagree.

Crews stay. A repeatable shape, such as coder plus reviewer, needs a fixed template so runs can be compared. The crew becomes one way to fill a mission, not the only way in.

## Model

- **Crew**: a named, reusable set of slots, each filled by a role, with exactly one lead, edited on the crew page. Unchanged. Roles, crews and their slots are the definitions the database holds, lists and searches.
- **Mission roster**: the slots one mission runs with, kept in `roster.json` in the mission's directory beside `events.ndjson`. It is written when the mission starts, changed only by that mission's own add-role and remove-role operations, and read by everything that needs the mission's roster: the router and bus when they mount, the handle checks on posts and signals, the lead's prompt, session resume, the app's mission view and the CLI.
- **Mission**: a container with one event log, a cwd, an optional goal, and one roster with at most one lead: a mission started from a crew has the crew's lead, and one started empty has none.
- **Start from a crew**: copy the crew's name, prompt addendum and slots into the roster. Editing or deleting the crew afterwards never touches the mission. `missions.crew_id` keeps naming the crew the mission started from.
- **Start empty**: a mission with no crew behind it and no roles. Nothing spawns at start; roles join one at a time through Add role. With no lead, the goal is shared with every member, and the person's messages without a recipient go to every member (Phase 3).
- **Add a role**: add a slot to the roster of a running mission and start its session. An added role is always a worker.
- **Remove a role**: stop the slot's session and drop the slot from the roster. Its messages stay in the feed and its handle stays taken in that mission. The lead cannot be removed.

**Why a file, not rows.** Nothing lists, searches or joins mission rosters across missions. The crew page, the pickers and `runner crew list` show crews, and "which missions use this role" is answered by `sessions`, which carries `role_id` and `mission_id`. Every reader of a roster reads one mission's. The file also exists already: `mission_start` writes `roster.json` for the CLI's `--to` check, and #863 is the daemon and the app reading the crew's current slots instead. A roster file needs no change to the `crews` or `slots` tables and no filter keeping per-mission copies out of crew lists. The 2026-10-10 draft gave each mission its own crew row instead; that put the same data in the database, needed a `kind` filter in every crew query, the role page's "Crews using this role", first-run seeding and the pickers, and needed an upgrade that moved every mission's directory.

### What is copied, and when

Settled by Jason on 2026-10-12. Roles and crews are templates. Each layer below them is copied from the layer above when it is created, and owns its copy from then on:

| Layer | What it is | Holds | Changed by |
| --- | --- | --- | --- |
| Role | Template for one agent | Runtime, command, args, env, system prompt, model, effort, speed | The role page |
| Crew | Template for a team | Slots (role, handle, lead, overrides), name, addendum | The crew page |
| Mission roster | The mission's team | Slots copied from a crew or added one at a time, with their overrides, and copies of the crew's name and addendum | Add role and Remove role |
| Session | One run of one slot or chat | Its launch definition, recorded at spawn: runtime, model, effort, speed, command (as configured), args, env and the role's system prompt, plus the agent's native session key; the mission parts of a prompt (crew addendum, the lead's goal and team) still compose at launch from the roster and log | Spawn and Restart |

1. **Copy at creation, then own it.** A mission copies its crew into `roster.json` when it starts. A session records its launch definition when it spawns, from its roster slot and the slot's role. Nothing re-reads the layer above afterwards unless asked to.
2. **Resume replays a session exactly.** Restart is the one explicit way to take up later edits: it starts a fresh conversation rebuilt from the current role and, for a mission slot, its roster slot.
3. **The machine stays live.** Resolving the command to an executable and applying the app-wide permission mode happen at every launch, because they describe the machine, not the agent.
4. **Links upward record where a thing came from.** `missions.crew_id`, a roster slot's `role_id` and `sessions.role_id` are provenance and display, not configuration. Deleting a crew keeps the missions started from it: the stored crew id remains as directory provenance without a foreign key, while status readers report no crew, and the roster keeps its copy of the crew's name and addendum for the mission's prompts (Phase 3, which makes `missions.crew_id` nullable). Deleting a role stays refused while an unarchived mission uses it, since role deletion still deletes the role's sessions.

Before #878 a session recorded its runtime and resolved command at spawn, and model, effort and speed only when overridden (#870), while Resume read the remaining configuration from the current role. [#878](https://github.com/yicheng47/runner/issues/878), merged as #881, records the whole definition at spawn for direct chats and mission slots before Phase 2, so that Phases 2 and 3 build on rule 1 instead of adding code that reads the role at resume. A roster therefore never holds a role's own configuration: a slot carries the role's id and the mission's overrides, and the session carries what actually ran.

UI copy keeps "Crew" for crews, the only thing a person picks or edits by that name. In code and docs the mission's slots are *the mission roster*. The vision's "Mission: one live activation of a crew" becomes "one live run of its own roster, seeded from a crew or started empty".

### The roster file

Version 2, written by Phase 1:

```json
{
  "version": 2,
  "crew_name": "Codex pair",
  "crew_addendum": "Team conventions…",
  "slots": [
    {
      "id": "01K7…",
      "handle": "lead",
      "lead": true,
      "position": 0,
      "role_id": "01K6…",
      "runtime_override": null,
      "model_override": null,
      "effort_override": null,
      "codex_speed_override": null
    }
  ]
}
```

- A slot keeps the id it had in the crew, so `sessions.slot_id` names a roster slot. Slots added in Phase 2 get new ids that exist only in the file.
- `crew_name` and `crew_addendum` are the copies mission prompts read. Phase 1 display surfaces keep reading the crew's current name. A mission started empty has an empty name, no addendum and no slots until a role joins.
- Version 1 is the file written before Phase 1: a bare array of `{ "handle", "lead" }`. It is converted once when `runnerd` starts, as described under Existing missions, and nothing else reads it.
- The file is written atomically, through a temporary file renamed over it, as it is today. Its type lives in `runner-core`, so the daemon and the CLI parse one definition.

## How it works today

Most of the runtime machinery already works per session and per handle; Phase 1 changes where the roster is read from, and later phases make it change while the mission runs.

| Layer | Today | With the mission roster |
| --- | --- | --- |
| Database | `missions.crew_id` and `slots.crew_id` are `NOT NULL`; a mission uses its crew's slots directly, and `sessions.slot_id` points at one of them. | Phase 1: unchanged. `sessions.slot_id` names a roster slot, which keeps its id from the crew. Phase 3: `missions.crew_id` is nullable, and empty for a mission started empty. |
| Start | `ops::mission::start` requires a crew with a lead, writes `roster.json` with each slot's handle and lead flag, and `mission_start_impl_with_size` reads the crew's slots again after the commit to spawn. | Builds the roster once in the start transaction, writes it, and spawns from it. |
| Re-mount | `ensure_mission_router_mounted` builds the router and mounts the bus from `slot::list(mission.crew_id)`, and reads the crew's name and addendum from its row. | Reads the roster file. |
| Posts and signals | `validate_roster_handle` and `post_message` check `--from` and `--to` against `slot::list(mission.crew_id)`. | Check against the roster file. |
| Session resume | `session/manager/spawn.rs` reads the slot row by `slot_id` for the handle and lead flag, and fails with "slot … no longer exists" once the crew dropped it; a lead's resume reads the crew's slots, name and addendum for its prompt. | Reads the slot, the name and the addendum from the roster file. |
| App | `mission_workspace/attach.rs` loads the roster with `slot_list(mission.crew_id)`. | Loads it with `mission_roster(mission_id)`. |
| CLI | Rereads `roster.json` on every call to validate `--to`. | Reads version 2. |
| Event log, events, env | `crews/<crew_id>/missions/<mission_id>/events.ndjson`; `crew_id` on every envelope; `RUNNER_CREW_ID` on every session. | Unchanged for a mission started from a crew. Phase 3: a mission started empty logs to `missions/<mission_id>/events.ndjson`, its envelopes carry no `crew_id`, and its sessions get no `RUNNER_CREW_ID`. |
| Crew lists, missions of a crew | Every crew is listed; `runner mission list --crew` filters on `missions.crew_id`. | Unchanged. |
| Crew deletion | Refused while the crew has unarchived missions; deleting it deletes its archived missions' rows. | Phase 1: unchanged. Phase 3: deleting a crew keeps its missions, which run on from their rosters. |
| Session spawn | `register_mission_session` then `complete_mission_session_spawn`, once per slot; nudges queue until the PTY is live. | Ready for one new slot. |
| Router routing | `session_by_handle` is a map under the router's state lock; `register_sessions` adds entries at any time. | Ready. |
| Router roster | `LaunchInputs.roster` is fixed at mount; broadcast nudges walk it and the lead's Restart prompt lists it. | Needs add and remove (Phase 2). |
| Bus | `BusState.handles` is fixed at mount ("Adding handles after mount is not supported in MVP") and decides whose inbox each message lands in. | Needs add and remove (Phase 2). |
| Per-session lifecycle | #542's Stop, Resume and Restart work on one session. | Ready; Remove uses Stop. |

## Scope

### In scope

- **The roster file** (Phase 1), as above, owned by one module in `runner-daemon` that writes it at start and loads it for one mission.
- **Start from a crew** (Phase 1). `mission_start` reads the crew's slots once, inside its transaction, writes them as the roster with the crew's name and addendum, and the spawn loop, the prompts, the router and the bus use that roster. Today it reads the slots three times, the last after the commit, so an edit in between could spawn a different roster from the one the CLI checks.
- **Every reader uses the roster** (Phase 1): `ensure_mission_router_mounted` (the router, the bus's handles, the crew name and addendum), `validate_roster_handle`, `post_message`, the session resume in `session/manager/spawn.rs` (the slot's handle and lead flag, and the lead's prompt), and the app's mission view through a new `mission_roster(mission_id)` call on the client protocol. It returns the same `SlotWithRole` rows as `slot_list`, with the slots from the file and each role from the database, and skips a slot whose role has been deleted.
- **Existing missions** (Phase 1). Converted once when `runnerd` starts, before any router mounts or session resumes: every mission, running or archived, whose file is version 1 or missing gets a version 2 file, and a version 2 file is left alone, so later starts only scan. Sessions all restart with `runnerd`, so no agent runs the CLI before the conversion is done, and every reader, the CLI included, knows only version 2. The conversion is one function, the only code that reads version 1, and is deleted once two releases have shipped with it. A missing file or one that is not valid JSON at load falls back to the crew's current slots, today's behavior, written as version 2 with a warning in the daemon log. Other I/O errors, unsupported versions and valid JSON with an invalid roster shape return an error without rewriting the file. This also refuses a leftover version 1 file after the conversion is deleted. The conversion:
  - The crew's name and addendum come from the crew row.
  - Each handle in the file is matched to a slot the mission's sessions ran under: first the session slot whose current handle is the same; then, in the order the sessions first started, a session slot that no other handle matched. That covers a slot #863 renamed, and one it deleted, for which the session's own role, runtime, model, effort and speed stand in for the slot row.
  - A handle without a session (its spawn failed) takes the crew's slot of that handle; a handle with neither is dropped with a warning in the daemon log, since nothing ran under it.
  - The file's handles, order and lead flags always win over the crew's current ones.
  - A mission with no file at all takes the crew's current slots, which is what it runs with today.
- **Deleting a role** (Phase 1). Also refused while an unarchived mission has a session for the role: "role @x is used by unarchived missions; archive them before deleting this role: …", naming the missions. Today the crew check covers a mission's roles only while its crew still has them; once the slot is removed from the crew, deleting the role deletes the running mission's session with it, since role deletion deletes every session of the role (`ops/role.rs`, `delete`).
- **Crew edits and deletion** (Phase 1). The crew page's slot operations need no guard, since a crew edit no longer changes a mission's runtime roster or prompt copies. Display surfaces, including mission summaries and CLI list/status, keep reading the crew's current name as before Phase 1; the rail's Crew row still reads and opens the current crew. Deleting a crew is unchanged.
- **Start empty** (Phase 3). A mission starts with no crew behind it and an empty roster: no slots, no crew name, no addendum and no lead. Nothing spawns at start. Roles join one at a time through Add role, each as a worker, and the mission runs until it is stopped or archived, like any mission. `missions.crew_id` becomes nullable (a table rebuild). The mission's directory is `missions/<mission_id>/`, where its `runner` shims already live, instead of `crews/<crew_id>/missions/<mission_id>/`. Its envelopes carry no `crew_id`, and its sessions get no `RUNNER_CREW_ID`. Mission headers, summaries, the sidebar and the CLI leave out the crew name it does not have.
- **A mission without a lead** (Phase 3, [#873](https://github.com/yicheng47/runner/issues/873)'s no-lead paths). An empty mission has no lead, since an added role is always a worker; setting or clearing one is [#873](https://github.com/yicheng47/runner/issues/873)'s. Every path that falls back to the lead gets a no-lead branch. The goal is composed into each member's launch prompt as the mission's goal; each role's own work is its Add role task. The person's messages without a recipient go to every member. Members ask the person with `ask_human`. A slot's restart is announced to every member, as joins and leaves are. Worker prompts stop saying "crew" when the mission has none. Crews keep exactly one lead, and a mission started from a crew is unchanged.
- **`ask_lead` is removed** (Phase 3; Jason, 2026-10-15). It predates the inbox; a worker reaches the lead with `runner msg post --to <lead>` like any other message, and the lead escalates to the person with `ask_human`. `runner ask "<question>"` and `runner signal ask_lead` are refused with that hint in every mission, and old `ask_lead` events keep rendering.
- **Deleting a crew keeps its missions** (Phase 3, rule 4 of What is copied, and when). The missions retain `crew_id` as directory provenance without a foreign key; readers treat a missing crew as no crew. Their rosters keep the copied name and addendum for prompts.
- **Add a role** (Phase 2). `mission_add_role({ mission_id, role_id, handle?, runtime?, model?, effort?, speed?, task? })` adds a configured worker to a running, unarchived mission. It refuses pending spawns, registers its cancel flag immediately after the checks, validates the handle against current and removed handles, records the session's launch definition, appends `role_joined` before writing the roster, updates routing, then appends the introduction and optional task before spawning. Slot overrides persist for Restart. A failed roster publication records a leave and rolls back the slot, router entry and session row.
- **Remove a role** (Phase 2). `mission_remove_role(mission_id, handle)` refuses the lead, unknown or removed handles, and pending spawns. It stops the session, writes the smaller roster, drops routing and pending deliveries, then appends `role_removed` and a system notice. The session row remains as history; session lists and Resume all filter by roster membership. It does not archive the removed session. One session's exit or failed spawn never stops another.
- **Router and bus** (Phase 2). The operations change the router's mutable roster directly. The bus replays the log from zero and learns membership from human join/leave signals in log order; membership bounds broadcasts only, and directed messages always reach their addressee's projection. Newcomer unread counts start at the join, but `runner msg read` still includes broadcasts from before it. One mutex per mission serializes adds, removals and roster writes; atomic file replacement retries briefly on Windows.
- **Protocol and CLI.** Phase 2 adds the above protocol operations and `runner mission add-role <mission> <role> [--handle <handle>] [--runtime …] [--model …] [--effort …] [--speed …] [--task … | --task-file <path|->]`, plus `runner mission remove-role <mission> <handle>`. They act as the person and refuse mission-session callers and `--as`. Phase 3 makes the crew optional: `mission_start` takes an optional `crew_id`, and `runner mission start` without `--crew` starts the mission empty. The title then defaults from the goal, so one of `--title` or `--goal` is required. A chat delegates with two calls, `runner mission start` and `runner mission add-role <mission> <role> --task-file -`, and `runner --help` and the embedded `runner` skill teach that pair.
- **App** (Phases 2 and 3). Each phase ships its GUI with its daemon and CLI work, so the person can do from the app everything the phase adds to the CLI. Phase 2: the mission rail gains **+ Add role** under its sessions while the mission runs, opening a modal with Start chat's shared role picker and Runtime, Model, Effort and Codex Speed controls, a handle pre-filled with the suggested unique handle, and an optional task. A worker card gains the **Remove from mission** button, with a confirmation. The feed renders `role_joined` and `role_removed` dividers attributed to the person and shows `runner` notices as `system`. Phase 3: the Start mission dialog's crew picker (849) lists **No crew** first, above the crews. Picking it turns the crew card into an empty state, "No crew · start empty" with a note that roles join from the mission, and the Goal hint says the goal is shared with every role that joins. With no crews, No crew is the only entry and is picked. Starting an empty mission opens its workspace. There the feed and the rail say no role has joined yet, each with **+ Add role**, in place of the Mission paused overlay that a mission with no live session shows today, and the composer stays in place but disabled, reading "Add a role to start messaging", until the first role joins. The mission info panel drops its Crew row for every mission, since a crew is only the template the roster was copied from. A mission started by an agent or a script appears in the sidebar without taking focus, as a mission started from the CLI does today; only the app's own Start mission dialog opens the mission it starts.
- **Docs.** `AGENTS.md` and `docs/product/vision.md` core vocabulary as in Model; `docs/arch/arch.md` §3 and §10 for the roster file (Phase 1), and §7 and §8 for `role_joined`, `role_removed` and the roster changing while a mission runs (Phase 2).

### Decisions to confirm

Each has a recommendation; the spec assumes it until Jason decides otherwise. Phase 1 needs none of them.

1. **A removed role's handle stays taken in the mission.** Reusing it would merge two agents' history under one name in the feed and in inbox projection. Adding the same role again gets `coder-2`. The add operation checks the mission's `role_removed` events.
2. **A mission can start empty** (Jason, 2026-10-15, reversing the 2026-10-10 recommendation that it could not). It has no lead and no sessions until a role joins, and roles join one at a time with Add role. A start never names roles: starting from one or more roles is dropped in favor of starting empty and then adding them.
3. **The lead is fixed for the mission's life.** A mission started from a crew keeps the crew's lead. One started empty has none, and its members work with each other directly through `--to`, broadcasts, the inbox and `ask_human`. Setting, removing or changing a lead while a mission runs is [#873](https://github.com/yicheng47/runner/issues/873)'s, which may make the lead optional for every mission (Jason, 2026-10-15).
4. **No roster cap for the person.** A cap arrives with the lead's spawn verb, where an agent could loop.
5. **Add and remove are for the person only** until the lead's verbs land; a mission agent calling them and `--as` are refused. The slot flag is `--handle`.
6. **Membership bounds broadcasts only.** The bus learns joins and leaves from the log; directed messages are retained regardless of membership. `runner msg read` keeps earlier broadcasts.
7. **Remove stops and drops, without archiving.** The historical session row remains; session lists and Resume all follow the roster.
8. **Sessions are independent.** A failed spawn or exit never stops siblings.
9. **Roster changes refuse pending spawns.** Add registers cancellation immediately after its checks, so Stop all or Archive can cancel the new spawn.
10. **With no lead, the goal is shared** (Phase 3). The goal goes into every member's launch prompt as the mission's goal, and each role's own work is its Add role task.
11. **With no lead, the person's untargeted messages go to every member** (Phase 3). The composer is shown but disabled until a role has joined.
12. **A mission without a crew lives in `missions/<mission_id>/`** (Phase 3), with no `crew_id` on its envelopes and no `RUNNER_CREW_ID`.
13. **A mission an agent starts does not take focus** (Phase 3). It appears in the sidebar, as a mission started from the CLI does today; only the app's own Start mission dialog opens the mission it starts. This settles open question 1 of the [agent environment](../../product/agent-environment.md).
14. **The mission info panel drops its Crew row** (Jason, 2026-10-15). A crew is only the template a roster is copied from, so a running mission no longer names or links it there.

### Out of scope

- **The lead's verbs** (Follow-ups): `runner spawn`, `runner ps` and `runner stop` from the lead, idle and crash notices to the lead (`slot_exited`), and outside seats, where an agent outside Runner holds the lead seat. Filed as [#916](https://github.com/yicheng47/runner/issues/916).
- **A copy of each role's configuration in the roster.** Replaced on 2026-10-12 by sessions recording their own launch definition ([#878](https://github.com/yicheng47/runner/issues/878), see What is copied, and when). Letting a role be deleted while missions that used it are unarchived, by making the sessions' role link become empty instead of deleting them, is [#917](https://github.com/yicheng47/runner/issues/917).
- Saving a mission's roster as a crew.
- Starting a mission from one or more named roles in one call. Dropped on 2026-10-15: a mission starts empty and the roles are added.
- Giving an empty mission a lead, or promoting a member to lead.
- A direct chat joining a mission as a slot. A chat's `RUNNER_*` environment is fixed at spawn; it drives a mission from outside instead.
- Workers adding or removing roles.
- Adding to or removing from a completed, aborted or archived mission, or while the mission's cold-start spawn queue is still running.

## Implementation Phases

Phase 1 is daemon and CLI work with no visible change. Phases 2 and 3 each begin by drawing their frames in `runner/desktop/specs/archive/562-mission-spawn.pen` on `main` and stopping for sign-off, then ship their daemon, CLI and app work in one pull request, so the GUI never trails the CLI. Every commit builds and passes its tests.

### Phase 1 — the mission roster (fixes #863)

One mission and one pull request against `main`, shipped as a 0.15.x patch that closes #863. No schema change and nothing visible: the rail's Crew row still opens the crew the mission started from.

- `runner-core`: the roster file type, version 2.
- `runner-daemon`: the roster module (writing at start, loading, and the fallback for a missing file or one that is not valid JSON), the startup conversion of version 1 and missing files, `mission_start` building and spawning from one roster, the readers listed under Scope switched to it, `mission_roster` on the client protocol, and the role-deletion refusal.
- `runner-cli`: `roster.rs` parses version 2.
- `runner-app`: `mission_workspace/attach.rs` loads the roster with `mission_roster`.
- `docs/arch/arch.md`: the mission roster in §3 and the roster file in §10's directory layout.
- Tests:
  - Starting a mission writes a version 2 file with the crew's name, addendum and slots, overrides included, and spawns those slots.
  - With a mission running, each crew edit (remove a slot, add one, rename a handle, move the lead, change an override, edit the addendum) followed by a re-mount, as at a `runnerd` restart: the router's roster and handle map, the bus's handles, the `--from` and `--to` checks, the lead's prompt and a worker's resume all follow the file, and the CLI accepts exactly the file's handles.
  - A session whose slot the crew deleted resumes instead of failing with "slot … no longer exists".
  - The startup conversion of a version 1 file for an untouched mission, a renamed slot, a deleted slot, a deleted slot whose handle a new crew slot reused, and a handle whose spawn failed, for running and archived missions; a mission without a file takes the crew's current slots; the conversion finishes before any router mounts, and a second start changes nothing.
  - The CLI reads version 2. At daemon load, a missing file or one that is not valid JSON falls back to the crew's current slots; other read errors and unsupported versions preserve the file and return an error.
  - `mission_roster` returns the file's slots with their roles and skips a slot whose role is gone.
  - Deleting a role is refused while an unarchived mission has a session for it, including after its slot left the crew, and allowed once the mission is archived.
  - Crew deletion behaves as before.
- Live checks, macOS and Windows: #863's reproduction (edit the crew while a mission runs, restart `runnerd`, then post from a removed worker, broadcast from the lead and resume a slot), and the affected cases of the mission regression suite: MIS-CREW-01, MIS-LIFE-01, MIS-CONFIG-01 and MIS-MSG-01.

### Phase 2 — add and remove while running

Starts after [#878](https://github.com/yicheng47/runner/issues/878) merges, so an added role's session records its launch definition like any other.

- Design: **+ Add role** in the rail and its modal, a card added mid-mission, a worker card's **Remove from mission** and its confirmation, and the `role_joined` and `role_removed` feed rows, drawn and signed off before code.
- `runner-daemon`: `ops::mission::{mission_add_role, mission_remove_role}`, the shared handle rule in `runner-core`, the router's mutable roster, the bus's log-ordered membership projection, the serialized roster file rewrite, and the `role_joined` and `role_removed` events.
- Client protocol operations and the `runner mission add-role` and `remove-role` commands.
- `runner-app`: in `mission_workspace`, the rail row, the modal (reusing Start chat's role/setup fields and the shared handle rule), the worker card remove button and the feed rows; `app_store.rs` refreshes on `role_joined` and `role_removed`.
- Tests:
  - Join events land in order with the documented payloads; the newcomer's inbox holds the broadcast and the task and nothing earlier; `--to` the newcomer validates right after the join and broadcasts reach it; `--to` a removed handle is refused and broadcasts skip it; removing the lead is refused; adding a removed role's handle again is suffixed or refused; a re-mount after a daemon restart rebuilds both changes from the file; two missions of one crew can each add `@coder-2`.
  - The modal validates handles against the mission roster, including removed ones; the rail shows a slot added after `session/spawned` and hides a removed one; feed row copy.

### Phase 3 — start empty

Settled by Jason on 2026-10-15. A mission can start with no roles, and roles join one at a time through Phase 2's add-role. This replaces the start-from-roles draft he reopened on 2026-10-12. A start never names roles, so the draft's questions fall away: per-role options at start, which entry is the lead, and a dialog that edits a crew it cannot save. Phase 3 builds [#873](https://github.com/yicheng47/runner/issues/873)'s no-lead paths, since an empty mission has no lead. Its dialog was drawn with [849](https://github.com/yicheng47/runner/blob/8462b5194e0f1945ef62b1af1a2758c6816fd70d/docs/features/archive/849-start-mission-modal.md)'s Start mission redesign, and mission 4 builds 849 too, as its own commit.

- Design: in `runner/desktop/specs/archive/562-mission-spawn.pen`, the crew picker's No crew entry and the empty crew card in the Start mission dialog (with 849's crew card and picker), the workspace of a mission no role has joined yet, the same mission after its first role joins, and the mission info panel without its Crew row, drawn and signed off before code.
- `runner-daemon`:
  - `mission_start` without a crew, the nullable `missions.crew_id` (a table rebuild), the `missions/<mission_id>/` directory, and envelopes and environment without a crew.
  - The same migration turns every `ON DELETE CASCADE` into `RESTRICT` (Jason, 2026-10-15): the app already removes dependent rows in code, so the database only refuses and never deletes.
  - The no-lead branches: the goal in every member's prompt, untargeted messages to every member, and restart notices to every member.
  - `ask_lead` removed from every mission: the signal type, its router handler, its prompt lines and the CLI's lead form of `runner ask`.
  - Worker prompt wording without a crew, and crew deletion keeping its missions.
- `runner-cli`: `--crew` optional on `mission start`, the title rule, and the start-then-add-role pair in `--help` and the embedded skill.
- `runner-app`: the crew picker's No crew entry and empty card (with 849), the empty feed and rail in place of the paused overlay, the composer disabled until the first role joins, the info panel without its Crew row, and display surfaces that leave out a missing crew name.
- Tests:
  - An empty mission starts with no sessions and no lead, and survives a `runnerd` restart.
  - A role added to it gets the goal and its task, and posts with `runner msg post`. A second role is reached by `--to` and by the person's untargeted message. `runner ask "<question>"` and `runner signal ask_lead` are refused, and no prompt mentions `ask_lead`.
  - `runner mission start` without `--crew`, `--title` and `--goal` is refused.
  - Deleting a crew keeps its missions, which run on from their rosters.
  - The dialog starts an empty mission with No crew picked, and picks No crew when there are no crews.
  - A mission with no roles shows the empty state, not the Mission paused overlay.

## Verification

- [ ] A mission started from a crew runs as before on its own roster file, and missions started before Phase 1 have their files converted at the first `runnerd` start, then resume, restart and replay unchanged.
- [ ] Editing a crew (slots, handles, lead, overrides, addendum) never changes a running mission, including after a `runnerd` restart.
- [ ] A role used by an unarchived mission cannot be deleted.
- [ ] An empty mission starts with no sessions and no lead. The first role added to it gets the goal and its task, and posts with `runner msg post`.
- [ ] With no lead, the person's untargeted messages reach every member and members work with each other directly. `ask_lead` is gone from every mission, and old `ask_lead` events still render.
- [ ] Everything Phases 2 and 3 add to the CLI also works from the app: adding and removing a role from the mission rail, and starting an empty mission from the Start mission dialog.
- [ ] An added role boots with the join broadcast and its task, is reachable with `--to` at once, receives later broadcasts, and survives a daemon restart.
- [ ] A removed role's session stops, `--to` it is refused, broadcasts skip it, its history stays in the feed, its handle is not reused, and the removal survives a daemon restart.
- [ ] The lead cannot be removed, and add and remove are refused on a mission that is not running.
- [ ] Every join and leave on the feed is attributed to the person.
- [ ] Live checks: #863's reproduction and the affected `MIS-*` cases in [`docs/tests/regression/missions.md`](../../tests/regression/missions.md), plus new cases for an empty start, an add and a remove, on macOS and Windows.

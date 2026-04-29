# Crew slots — runner-as-template + per-slot identity

> Decouples Runner (config) from Slot (in-crew identity). One crew can hold
> multiple slots that all reference the same Runner template, each with its
> own slot_handle and (in a follow-up) optional system-prompt override.
>
> Companion to `docs/impls/v0-mvp.md` (umbrella plan), `docs/arch/v0-arch.md`,
> and `docs/impls/direct-chats.md`. Lives in its own file because the
> change reaches schema, mission lifecycle, router prompt composition, the
> bundled CLI's identity env, and most of the crew-facing UI.
>
> **Update during implementation:** dropped the per-slot `role` field
> entirely from v0. `slot_handle` is the only per-slot identity. The
> launch prompt no longer surfaces a "Your role:" line; the lead infers
> structure from the runner template's `system_prompt`. Sections below
> still reference role — kept for historical context. The shipped
> migration (0006), `Slot` model, `slot_create`/`slot_update`,
> `CrewMembership`, and the launch prompt all omit role.

## Why

The current model treats `runners.handle` and `runners.role` as the
runner's identity. That works for v0 but breaks down when you have a
real coordination story:

1. **Same agent, two responsibilities in one crew.** A user wants two
   instances of `claude-pro` in the *same* crew — one as `@architect`,
   one as `@reviewer`. Today the schema has `PRIMARY KEY (crew_id,
   runner_id)`, so a Runner can be in a crew at most once.
2. **Role is a coordination concept, not a config field.** Same
   `claude-pro` template could be the architect in crew A and the
   reviewer in crew B. Storing role on the Runner forces the user to
   create a duplicate template per role.
3. **Handle collisions in event envelopes.** Two slots that both reference
   the same Runner currently both broadcast as `@claude-pro` in mission
   events — the router and the lead can't tell them apart.

Slots fix all three: the Runner becomes a *template* (runtime, command,
args, env, system_prompt), and each slot in a crew owns its own
in-crew handle + role.

## What we're not doing

- **Cross-crew slot reuse.** A slot belongs to exactly one crew. The
  template ref is what's reused.
- **Mission-level role override.** Role is set on the slot. Per-mission
  overrides would be a v0.x concern.
- **Per-slot system-prompt override.** Same shape as role, but
  intentionally deferred to a follow-up — see "Open questions" below.
  The schema delta here leaves room.
- **Renaming the Runner's `handle` field to `name`.** Calling it `handle`
  reads fine if it's "the template's display handle". We'll relax the
  uniqueness story in code (still globally unique) but keep the column
  name.

## Model

A **Runner** is a config template:

```
runners
  id, handle (template name, globally unique), display_name,
  runtime, command, args_json, working_dir, system_prompt,
  env_json, created_at, updated_at
```

A **Slot** is a position in a crew that references a Runner template
and carries an in-crew identity. The table is renamed from
`crew_runners` to `slots` to match the new vocabulary:

```
slots
  id (new PK, ULID)
  crew_id  → crews(id) ON DELETE CASCADE
  runner_id → runners(id) ON DELETE CASCADE   -- template
  slot_handle TEXT NOT NULL       -- in-crew identity, e.g. "architect"
  role TEXT NOT NULL              -- in-crew role, e.g. "implementer"
  position INTEGER NOT NULL
  lead INTEGER NOT NULL DEFAULT 0
  added_at TEXT NOT NULL
  UNIQUE (crew_id, slot_handle)   -- slot handle unique within a crew
  UNIQUE (crew_id, position)      -- existing invariant retained
  -- "at most one lead per crew" is enforced in the slot commands
  -- (transactional clear-others-then-set), not via a partial unique
  -- index. Keeps the SQL portable and the invariant visible in code.
```

A **Session** is one PTY run. Mission sessions hook to a slot;
direct-chat sessions hook to a Runner template directly:

```
sessions
  id, mission_id (nullable), runner_id, slot_id (nullable, new),
  cwd, status, pid, started_at, stopped_at,
  agent_session_key, archived_at, title, pinned_at
  -- mission session: slot_id IS NOT NULL, mission_id IS NOT NULL.
  -- direct chat:   slot_id IS NULL,    mission_id IS NULL.
```

`runner_id` stays on `sessions` for direct chats (which have no slot)
and as a redundant denormalization for mission sessions (lookup
performance — most session-side joins want the runner template too).

Identity rules:

- **In a mission**, the agent identifies as `slot_handle`. `RUNNER_HANDLE`
  env var, event envelope `from` fields, the bundled CLI's `runner msg
  post --to <handle>`, the router's pending-ask map keys — all use
  slot_handle.
- **In a direct chat**, the agent identifies as `runner.handle` (the
  template name). The `/runners/:handle/chat` URL keeps using the
  template handle.
- **Cross-context lookups.** The lead's launch prompt + roster sidecar
  list slots; the router resolves "ask_lead" / "human_said" by slot
  handle inside the mission's namespace.

## Schema delta

This chunk lands as a new migration. Existing data migrates in place
to a single slot per current `crew_runners` row.

### 0006 — `slots` replaces `crew_runners`, drop `runners.role`

Pre-release: no data preservation. The migration drops
`crew_runners` outright, creates a fresh empty `slots` table, drops
`runners.role`, and adds `sessions.slot_id`. Pure DDL; no Rust
backfill loop.

Existing dev databases will keep their `runners` and `crews` rows but
lose their slot/membership rows. Users re-add slots via the new Add
Slot affordance. We don't claim a clean upgrade path for v0.

```sql
DROP TABLE crew_runners;

CREATE TABLE slots (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    slot_handle TEXT NOT NULL,
    role TEXT NOT NULL,
    position INTEGER NOT NULL,
    lead INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    UNIQUE (crew_id, slot_handle),
    UNIQUE (crew_id, position)
);

ALTER TABLE runners DROP COLUMN role;

ALTER TABLE sessions ADD COLUMN slot_id TEXT;
```

Notes:

- **No partial unique index for `lead`.** The previous schema had
  `one_lead_per_crew` enforced as a partial unique index on
  `crew_runners`. We drop that constraint at the schema level and
  enforce "at most one lead per crew" inside `slot_set_lead` /
  `slot_create` instead — both wrap the lead change in a transaction
  that clears other slots' `lead` flag in the same crew before
  setting the new one. Same invariant; less SQLite-specific syntax;
  invariant lives next to the code that violates or upholds it.
- **`runners.role` drop** uses SQLite 3.35+ `ALTER TABLE … DROP
  COLUMN`; bundled rusqlite supports it.
- **`sessions.slot_id`** is nullable. Existing mission session rows
  stay NULL (they're historical and not addressable as slots
  anyway). New mission spawns set it; queries that need a slot ref
  tolerate NULL gracefully.

## Backend changes

### Rust model

- `Runner`: drop `role`. Keep `handle` (still globally unique;
  still used for direct-chat URLs).
- New `Slot` struct in `commands::slot` (renamed from
  `commands::crew_runner` — file moves accordingly). Fields: `id`,
  `crew_id`, `runner_id`, `slot_handle`, `role`, `position`, `lead`,
  `added_at`.
- `Session`: add `slot_id: Option<String>`.

### Commands

- `commands::runner`: drop `role` from `CreateRunnerInput` /
  `UpdateRunnerInput` / SELECT cols / tests.
- `commands::slot` (was `commands::crew_runner`):
  - `slot_create` (was `crew_add_runner`). Required params:
    `crew_id`, `runner_id`, `slot_handle`, `role`. Optional:
    `position` (defaults to MAX+1), `lead` (defaults to false).
  - New `slot_update(slot_id, { slot_handle?, role? })`.
  - `slot_delete` (was `crew_remove_runner`). Takes `slot_id`. Same
    crew-side cascade semantics.
  - `slot_set_lead(crew_id, slot_id)` — operates by slot id now.
  - `slot_reorder(crew_id, slot_ids[])` — reorders by slot id.
  - `slot_list(crew_id)` (was `crew_list_runners`). Returns slots
    joined with runner template fields needed for display.

### Mission spawn

- `mission_start`: iterate slots (not runners). For each slot, call
  `SessionManager::spawn` with the slot's identity:
  - `RUNNER_HANDLE = slot.slot_handle`
  - The session row gets `slot_id = slot.id`, `runner_id =
    slot.runner_id`.
  - The roster sidecar lists slots: `[{ handle: slot_handle, role,
    runtime, command, lead }, …]`.
- `mission_stop`: unchanged shape; reaps all sessions for the mission.

### Router

- `router::prompt::compose_launch_prompt`: roster comes from slots, not
  runners. Lead's "Your role: …" pulls from `slot.role`.
- `router::mod::Member`: `handle` field stores `slot_handle`. The
  registry keys by `(mission_id, handle)` (handle = slot_handle), same
  shape as today.
- `runner.command` / `args` / `env` / `system_prompt` still come from
  the runner *template* — the slot only owns identity + role.

### Direct chats

Unaffected. `spawn_direct` still inserts a session with `mission_id =
NULL` and `slot_id = NULL`. The chat header / tray label still uses
`runner.handle` as the template handle.

### Tests

Touches a lot of fixtures (`role: "test".into()`, `INSERT INTO runners
(... role ...)`, etc.). Mostly mechanical search-and-replace, plus
adding `slot_handle` / `role` to slot inserts and replacing
`crew_runners` table refs in fixture SQL with `slots`.

## Frontend changes

### Runner forms (drop role)

- `CreateRunnerModal`, `RunnerEditDrawer`: remove the role input.
- `Runner` interface in `lib/types.ts`: drop `role`.
- `RunnerDetail`, `RunnerChat` side panel, `Runners` list: drop role
  display (or surface it as "Roles in: <crew> · <role>" pulled from
  membership, follow-up).

### Add Slot

- `AddSlotModal` rewrites to take three inputs:
  1. **Runner template picker** (same UI as today).
  2. **Slot handle** — text input. Default value: the runner's handle;
     editable. Validation: non-empty, not already used in this crew.
  3. **Role** — text input. Required.

### Crew Editor

- Slot rows display `{slot_handle}` (mono) + role (small chip) + runner
  template handle (subtitle).
- Inline editing of slot_handle + role on each slot row.
- Lead toggle, reorder, remove still work — keyed by slot id.

### URLs / routing

- `/runners/:handle/chat` keeps `runner.handle` (template). No change.
- `/missions/:id` is unchanged; the workspace just uses slot_handle in
  the runners rail / event feed.

## Migration steps (chunk order)

Each step lands as its own commit on the same PR.

1. **Migration 0006** — drop `crew_runners`, create empty `slots`
   (no partial unique index; lead-uniqueness is enforced in app
   code), drop `runners.role`, add `sessions.slot_id`. Pre-release,
   so no data preservation. The runtime still references
   `crew_runners` after this step; chunk 2 switches it over.
2. **Backend Rust — slot model + slot commands.** `Slot` struct, new
   `commands::slot` module (renamed from `commands::crew_runner`),
   the six command renames listed above. Drop role from Runner. All
   SQL queries that referenced `crew_runners` switch to `slots`.
   Update tests + fixtures.
3. **Mission spawn + router.** `mission_start` reads slots, sets
   `slot_id`, env-vars from `slot_handle`. Router's `Member.handle` =
   `slot_handle`. Roster sidecar uses slot fields.
4. **Frontend forms.** Drop role from runner Create/Edit. Update
   AddSlotModal (slot_handle + role). Update CrewEditor display +
   edit. Rename `api.crew.{addRunner,…}` to `api.slot.{create,…}` to
   match.
5. **Frontend display cleanup.** Drop role from RunnerDetail /
   RunnerChat / Runners list.
6. **Tests + lint/typecheck.**

## Definition of done

- A user can add the same Runner template to a crew twice with two
  different slot_handles + roles, and start a mission where both slots
  spawn distinct PTYs identifying as `@<slot_handle_1>` and
  `@<slot_handle_2>`.
- The lead's launch prompt lists each slot's `slot_handle` + `role`,
  even when two slots share a runner template.
- Stopping and resuming a slot keeps the same `agent_session_key`
  (resume is keyed by session row, not by template).
- Direct chats still work at `/runners/:handle/chat`; they don't get a
  role and don't appear in any slot listing.
- `crew_remove_slot` removes one slot without affecting other slots in
  the same crew that share the runner template.
- Pre-existing crews keep their `crews` row but show zero slots; the
  user re-adds slots via the new Add Slot affordance and starts a
  mission successfully. No clean upgrade path for v0; pre-release
  data loss for memberships is accepted.

## Open questions / deferred

- **Per-slot `system_prompt_override`.** Schema-shaped to receive a
  `slots.system_prompt_override TEXT NULL` column in a follow-up.
  The runtime adapter would prefer that override when set, fall back
  to `runners.system_prompt` otherwise. Out of scope here so this
  chunk stays focused on identity + role.
- **Per-slot env / args overrides.** Same story as system_prompt;
  defer until there's real demand.
- **Mission-time role overrides.** A user might want to spawn the same
  crew with the lead reassigned. Mission-level overrides would live on
  `missions.role_overrides_json` or similar. Out of scope.
- **Slot rename uniqueness windows.** If a mission is running and the
  user renames a slot's `slot_handle`, the new handle takes effect on
  the next mission_start (the running mission's roster sidecar is
  frozen at start time). Document; don't rewrite live state.
- **Display affordance for "this Runner is used in N slots across M
  crews".** Useful info on the Runner detail page. Drop-in once the
  slot list query exists.
- **Mission-session resume.** Listed in `direct-chats.md` as deferred.
  Slot identity makes this cleaner — `agent_session_key` is per slot
  (not per runner) — so when we ship mission resume, the lookup is
  unambiguous.

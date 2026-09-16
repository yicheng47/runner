# 604 — Rename the runner entity to role

> Tracking issue: [#604](https://github.com/yicheng47/runner/issues/604)
> Status: shipped 2026-09-16 in [#618](https://github.com/yicheng47/runner/pull/618) (merge `8fefe12`; record [`604-role-rename/`](../../impls/archive/604-role-rename/README.md); missions `01M2MJ8WE488AJNN0NAHHF3QZ8` and `01M2MT7QCKQBC6EPKEXBPXCPGW` on codex-crew).
> Priority: P1 — the overload is visible in shipped UI and every new user pays for it.
> Platforms: macOS and Windows.
> Decision, 2026-09-15: the entity becomes `role`; the product stays **Runner**. The CLI keeps `runner signal` / `runner msg`, and "worker" keeps its existing mission meaning.

## Motivation

"Runner" names three things at once: the product, the reusable agent configuration, and the bundled CLI. The **Start a chat** modal puts all three on one screen, plus a fourth word for the entity:

| On screen | Actually means |
| --- | --- |
| `Direct \| Runner` toggle | the mode — ad-hoc runtime vs. saved configuration |
| **Runner** field → `@housekeeper` | the entity |
| `Runner default (Claude Code)` | the *entity's* engine, but it reads as *the app's* default |
| "…runs this **persona** on another agent…" | the entity again |

A new user has to be taught that "a Runner" is not "Runner", and the README is forced to prove it — "**Runner** — a reusable agent configuration" sits directly under a logo that says Runner.

The entity is already a role everywhere except the code. `README.md` defines a runner as "runtime, **role**, system prompt, working directory"; `AGENTS.md` says "a configured CLI agent runtime, **role**, and system prompt"; `migrations/0002_persona_only_seeds.sql` says these seeds "boot with just the **role identity**". The rename makes the code say what the docs already say.

The resulting vocabulary is all plain English with no learning curve: **role, crew, slot, mission, session, lead.**

## Decisions

**1. The product name does not change.** The collision with CI runners is real but out-of-category — nobody looking for a macOS app that orchestrates coding agents lands on GitLab Runner docs and gets confused. Discovery here is GitHub stars, word of mouth and the WeChat group, not search. Renaming the product would cost the bundle id, signing chain, updater feed continuity, repo redirect, landing page and both READMEs to arrive somewhere no better. Revisit only if Runner ever gets a real marketing push, where being unfindable by name starts costing money.

**2. `role`, not `worker`.** "Worker" already means a non-lead participant in a live mission — 91 references in `runner-backend`, `WORKER_COORDINATION_PREAMBLE`, `compose_worker_first_turn`, and the vision doc's flow ("if a worker needs human input, it emits `ask_lead`"). It also implies a running process, which is wrong for a stored configuration row. Using it for the entity would recreate the exact overload this issue removes.

**3. `slots` stays.** A role is the reusable definition; a slot is one position in one crew that a role fills, which is why two crews can use the same role under different handles. The overlap between a slot's handle (`@reviewer`) and a role's identity is cosmetic, not structural. Mitigate in copy by leading with the handle so the two words never sit adjacent. If it still reads muddy, `slot` → `seat` is a cheap follow-up, not part of this change.

**4. The CLI keeps its verbs.** `runner signal` and `runner msg` stay. The CLI speaks *as* a crew member, and those verbs appear in every brief already in the wild.

## Scope

### Data

- `runners` → `roles`; `slots.runner_id` → `role_id`; `sessions.runner_id` → `role_id`. One migration. The sessions column has carried the nullable role reference since migration 0007 and is corrected in the same rename.
- No field collision: `runners` has no `role` column, and `role` appears three times total in `runner-backend`.
- The row keeps its current shape. Role identity (`handle`, `display_name`, `system_prompt`) and launch config (`runtime`, `command`, `args_json`, `env_json`, `model`, `effort`, `working_dir`) continue to share it — a role that names its own engine is coherent, and splitting them is a separate question.

### Backend

- ~2222 `runner` references in `crates/runner-backend/src/`.
- `ops/runner.rs` → `ops/role.rs`.
- `WORKER_COORDINATION_PREAMBLE`, `compose_worker_first_turn` and the worker/lead distinction are untouched.

### MCP

- `runner_create`, `runner_get`, `runner_get_by_handle`, `runner_list`, `runner_update`, `runner_delete` → `role_*`.
- `slot_create` / `slot_update` parameters carrying `runner_id` → `role_id`.
- This is a breaking change for external callers, including our own Claude Code and Codex sessions. Acceptable at alpha; it is a cutover, not a pure refactor.
- Each renamed tool's description says "a crew role" explicitly, so an agent does not read `role_create` as RBAC or IAM.

### UI copy

| Now | After | Where |
| --- | --- | --- |
| `Direct \| Runner` toggle | `Direct \| Role` | `surfaces/start_chat.rs:1471,1479` |
| **Runner** field label | **Role** | `surfaces/start_chat.rs:1337` |
| `Runner default (Claude Code)` | `Role default (Claude Code)` | `ui/select.rs:731`, `surfaces/start_chat.rs:1708`, `surfaces/crews/logic.rs:150,151`, `surfaces/runners/logic.rs:148,184,185` |
| "Runner default or home directory" | "Role default or home directory" | `surfaces/start_mission.rs:76` |
| "slot override · Runner default follows the template…" | rewrite; a literal substitution reads circular | `surfaces/runners/edit.rs:384` |
| "Overriding runs this **persona** on another agent…" | "Overriding runs this **role** on another agent…" | `surfaces/start_chat.rs:1363` |
| **Agent**, **Chat name**, **Working directory** | unchanged | — |

`Agent` stays — it means *which CLI runtime*, a different axis from the role. **"Role default" is the line that earns the rename**: today "Runner default" is ambiguous between the app's default and the entity's, and afterwards it can only mean one thing.

Directory: `surfaces/runners/` → `surfaces/roles/`.

Two copy fixes to take while in this modal:

- The subtitle "Spawns a direct PTY in the selected directory" sits above a toggle where **Direct** is one of the two options, implying both modes are direct.
- **Chat name** is pre-filled with `@housekeeper`, duplicating the selection directly above it. The helper already says it is optional, so leaving it genuinely blank reads cleaner.

### Docs

`docs/product/vision.md`, `AGENTS.md`, and `README.md` + `README.zh-CN.md` in the same PR.

## Out of scope

- Product name, bundle id, repo, updater feed, icon.
- The CLI's `runner signal` / `runner msg` verbs.
- Retiring or renaming `slots`.
- Splitting role identity from launch config.

## Implementation phases

1. [x] Table rename plus migration, `slots.role_id` and `sessions.role_id`.
2. [x] Backend rename, `ops/runner.rs` → `ops/role.rs`.
3. [x] MCP tools and their descriptions.
4. [x] UI copy, and `surfaces/runners/` → `surfaces/roles/`.
5. [x] Docs, both READMEs together.

## Verification

- `make verify` clean; workspace test count unchanged.
- Fresh install and an upgrade from a v0.9.x database both open crews, roles, missions and chats.
- `role_list` over MCP returns what `runner_list` returned.
- Start a chat, Start a mission, crew detail and role edit each read with exactly one meaning of "role".

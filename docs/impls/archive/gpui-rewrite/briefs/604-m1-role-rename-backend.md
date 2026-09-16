# 604 — Mission 1: rename the runner entity to role — data, backend, MCP

Jason requested a Codex crew mission on 2026-09-16. Work in `/Users/jason/repos/yicheng47/runner` on the existing branch `feat/604-role-rename`, created from `main` at `47296e3` (the 0.9.4 bump) with this brief as its only commit. Build on it; do not create another branch, rebase or squash. Mission 2 (UI copy, the `surfaces/runners/` move, docs) follows on the same branch, so this mission ends with the workspace green but the words users see unchanged.

Read first: this brief; `docs/features/604-rename-runner-to-role.md` (binding: decisions 1–4, the Scope tables, Out of scope); `AGENTS.md` §Product Context for the vocabulary that stays; then the code: `crates/runner-backend/migrations/0001_init.sql` and `0007_direct_runtime_sessions.sql` (every table that references `runners`), `crates/runner-backend/src/db.rs` (the migration list), `model.rs` (`Runner`), `repo/runner.rs`, `ops/runner.rs`, `repo/slot.rs`, `repo/session.rs`, `ops/crew.rs`, `ops/mission.rs` (writes `roster.json`), `event_bus/mod.rs` (`runner_handle`), `mcp/tools/runner.rs`, `mcp/tools/slot.rs`, `mcp/tools/crew.rs`, `mcp/server.rs`.

## Ownership and authorization

The coder owns implementation and checks; the reviewer waits for an explicit Runner handoff, then audits the whole working-tree diff against `47296e3`. Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout, or worktree. Stop at a clean working-tree review. Commits, push, PR, merge and restarting the development app are not authorized; leave the work uncommitted on top of `47296e3`. Do not touch `design/runner.pen`, the READMEs, or `docs/` other than ticking this mission's items in the 604 spec.

## Deliverable

**Data.** One migration, `0023_roles.sql`, registered in `db.rs`: `runners` → `roles`, `slots.runner_id` → `role_id`, and `sessions.runner_id` → `role_id` (the spec lists only `slots`; `sessions` has carried a nullable `runner_id REFERENCES runners(id)` since 0007, so it renames in the same migration; record this as a spec correction). Use `ALTER TABLE … RENAME TO` and `RENAME COLUMN` with `legacy_alter_table` off so SQLite rewrites the foreign-key references itself; do not copy tables. Tests: a fresh database opens with `roles`; a database built through 0022 and seeded with a role, a crew with two slots, and a direct session with a role survives 0023 with every row intact, `PRAGMA foreign_key_check` empty, and cascade delete still working from `roles`.

**Backend.** The stored entity is a role everywhere in Rust: `model::Runner` → `Role`, `repo/runner.rs` → `repo/role.rs` with `RunnerRow` → `RoleRow`, `ops/runner.rs` → `ops/role.rs`, `runner_id` → `role_id`, `runner_handle` → `role_handle`, `runner_count` → `role_count`, and the functions, variables, SQL and test names that go with them, across `runner-backend` and the places in `runner-app` that name those identifiers, so the whole workspace builds. Persisted mission artifacts keep their keys: `roster.json` still writes `runner_handle` and the feed still carries the `runner_status` signal, because old missions must load and `mission-watch` reads them; use serde renames where a field name changes. `WORKER_COORDINATION_PREAMBLE`, `compose_worker_first_turn`, and the lead/worker vocabulary are untouched.

**What keeps the name.** These mean the product, the process, or the CLI, not the entity, and must not change: the crate names, `runner` the bundled CLI and its `signal` / `msg` verbs, the `runner-mcp` sidecar, the MCP client registration named `runner` in every agent's config, `com.wycstudios.runner`, the `RUNNER_*` environment variables, the `runner_status` signal, and prose in comments or strings that says "Runner" for the app. Before the handoff, run `git grep -n -i "runner" -- crates cli` and check every surviving occurrence is one of those; put the count before and after in the handoff.

**MCP.** `runner_create`, `runner_get`, `runner_get_by_handle`, `runner_list`, `runner_update`, `runner_delete` → `role_*`, in `mcp/tools/role.rs`; `slot_create` and `slot_update` take `role_id`; response fields that name the entity (`runner_id`, `runner_handle`, `runner_count`, `runners`) become their role forms in every tool's output, crew and mission tools included. Each renamed tool's description says "a crew role" in its first sentence so an agent never reads `role_create` as RBAC. This is a deliberate breaking cutover for external callers; no aliases for the old names.

**User-facing words stay for now.** Strings rendered in the app (labels, captions, error text shown in the UI) and the `surfaces/runners/` directory are mission 2. If an error string lives in the backend and is shown verbatim, leave its wording and rename only the identifiers around it.

## Verification

`make verify`; `cargo test --locked --workspace --no-fail-fast --profile ci` with the total test count unchanged from `47296e3` (the spec's gate; report both numbers); `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`. Plus the migration tests above and a test that `role_list` over the MCP server returns what `runner_list` returned for the same rows.

## Handoff

Final Runner handoff: branch and base, the rename map (old → new for types, modules, columns, tools, fields), the grep counts and the list of surviving `runner` meanings, the spec correction for `sessions.role_id`, checks with results, and the reviewer's explicit no-remaining-must-fix verdict. Leave the work uncommitted.

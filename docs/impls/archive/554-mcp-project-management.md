# 554 — Manage projects over MCP

Tracking issue: [#554](https://github.com/yicheng47/runner/issues/554). Spec: [554](../../features/archive/554-mcp-project-management.md). Feature, P1. Shipped 2026-09-10 in [#556](https://github.com/yicheng47/runner/pull/556) (mission `01M25TDXRPDZTQ30HZ319V72BQ`, codex-crew, 23 min launch→PR). Branch: **`feat/554-mcp-project-management` already exists and is checked out** — it carries the spec and this brief; work on it, do not create another.

## What ships

Four MCP tools in `runner-backend`: `project_create`, `project_rename`, `project_delete`, `mission_set_project`. Thin wrappers over the ops the sidebar already calls, plus a live-member guard on delete (an MCP caller sees no confirm dialog) and an order helper for the move. No UI change, no new event types, no CLI command.

## Where the code is

- `crates/runner-backend/src/mcp/tools/project.rs`: `ProjectIdArgs` `:12`, `command_error` `:17` (`Error::Msg` → `invalid_request`), the router with `project_list`/`project_get`, one test.
- `crates/runner-backend/src/mcp/tools/mission.rs`: arg structs from `:21`, `mcp_error` `:246`, `emit_mission_changed` `:324`, `mission_rename` `:498` is the template for a tool that takes `&AppCore`.
- `crates/runner-backend/src/ops/project.rs`: `project_create` `:44`, `project_rename` `:59`, `project_delete_impl` `:74` (returns archived chat ids), `project_delete` `:123` (returns `()`, owns the event fanout). Its one caller, `crates/runner-app/src/surfaces/sidebar.rs:1769`, ignores the value.
- `crates/runner-backend/src/ops/node.rs`: `node_move` `:138`, `ContainerChildren` / `container_children` `:229` (tab session ids plus `(mission_id, MissionStatus)`), `archive_child_missions` `:320`, `kill_running_children` `:359`. The tests' `test_core()` `:573` builds an in-memory `AppCore`.
- `crates/runner-backend/src/repo/node.rs`: `find_by_ref` `:213`, `ensure_project_node` `:229`, `ensure_mission_node` `:238`, `move_and_reorder` `:367` — it rejects any `ordered_ids` that is not exactly the destination's `pinned_position IS NULL` children. `delete_container_tabs_and_archive` `:806` fails the transaction on a session row still `running`.
- `crates/runner-backend/src/repo/session.rs:158` `get_row` → `SessionRowDb.status`.
- `crates/runner-backend/src/mcp/server.rs`: `tool_router_registers_workspace_and_mission_tools` `:67`, `every_tool_input_property_declares_a_type` `:125`.
- `docs/arch/arch.md:569`, the MCP paragraph.
- Reference only, do not touch: `crates/runner-app/src/surfaces/sidebar_logic.rs:321` `complete_unpinned_scope_order` is what a sidebar drag feeds `node_move`.

## Fix shape

1. **`project_create(name, cwd)`** in `tools/project.rs`: `ProjectCreateArgs { name, cwd }`. Trim; `cwd` must be absolute and `std::fs::metadata(..).is_dir()`, else `invalid_request` "cwd must be an absolute path to an existing directory". The check lives in the tool, not the op. Then `ops::project::project_create`; return the `ProjectRow`.
2. **`project_rename(id, name)`**: `ProjectRenameArgs`; `ops::project::project_rename`; return the row.
3. **`project_delete(id, force = false)`**: `ProjectDeleteArgs { id, #[serde(default)] force: bool }`. New `ops::project::live_members(conn, id) -> LiveMembers { session_ids, mission_ids }` over `container_children`: tab session ids whose `get_row` status is `Running`, and missions whose status is `Running`. When `force` is false and either list is non-empty, `invalid_request` "project has running members" carrying both lists (JSON in the message is fine) and nothing else happens. Otherwise call `ops::project::project_delete`, which changes to return `ProjectDeleteOutcome { archived_mission_ids, archived_session_ids }` — mission ids from the children snapshot `_impl` already takes, session ids from what `_impl` already returns; the sidebar keeps compiling by dropping the value. Unknown id → the op's "project not found" as `invalid_request`.
4. **`mission_set_project(mission_id, project_id)`** in `tools/mission.rs`: `MissionSetProjectArgs { mission_id, project_id: Option<String> }`. Resolve `find_by_ref(NodeType::Mission, …)` (missing mission or node → `invalid_request`); when `project_id` is `Some`, `ops::project::get` then `ensure_project_node`. If the node's `parent_id` already equals the target, return the mission unchanged. New `ops::node::append_order(conn, parent_id: Option<&str>, moved_id) -> Vec<String>`: the destination's `pinned_position IS NULL` children ordered by `position`, minus `moved_id`, plus `moved_id` at the end only when the moved node itself is unpinned. Then `ops::node::node_move(state, node.id, target_node_id, order)`; return `repo::mission::get`. `cwd` is untouched.
5. **Registry and docs**: add the four names to the server test list; `arch.md:569` names the four beside `project_*`.

## Rules of the road

- No new events. The ops already emit `project/changed`, `mission/changed`, `chat/layout-changed`, `session/updated`; the tools add nothing beyond what `mission_rename` does.
- Sidebar behaviour unchanged. `ops/project.rs` signatures may grow; the delete semantics may not.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-backend -p runner-app`, `make clippy`, `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

In `tools/project.rs` and `tools/mission.rs` against an in-memory `AppCore`. Lift `test_core` out of `ops/node.rs` into one `pub(crate)` `#[cfg(test)]` helper (for example `crate::test_support::test_core`) and point the node tests at it.

- create: a relative cwd and a non-existent absolute cwd are rejected; a `tempfile::tempdir()` path is accepted, `position` is the previous max plus one, and the project node exists.
- rename: unknown id → not found; a known id returns the new name.
- delete: a member tab whose session row is `Running` → refused with the id in the error and the project still present; a `Running` member mission → refused likewise; stopped-only members → archived and both id lists in the outcome. With `force: true` and a `Running` row that has no live PTY, `kill` is a no-op and `delete_container_tabs_and_archive` fails with "still running" — assert that error, which proves the guard was bypassed; the live force path is Jason's smoke test.
- set_project: unfiled → project → back to unfiled, with the node's parent and `missions.project_id` following; the mission lands last in the destination order; a pinned mission keeps its `pinned_position`; `cwd` unchanged; same container is a no-op.
- The two registry tests pass with the four names.

## Jason's smoke test (after landing)

1. From a Claude Code session: `project_create` with a relative path is refused; with a real directory the project appears at the bottom of the sidebar without a restart.
2. `project_rename` updates the sidebar row in place.
3. Start a chat in the project, `project_delete` without `force` → the error names the session; with `force: true` the chat closes and the project is gone.
4. `mission_set_project` on an unfiled mission → it moves under the project; `null` → back to unfiled; `mission_get` shows the same `cwd` both times.

## Non-goals

Reordering projects, moving tabs between projects, changing a project's cwd, a `runner` CLI command, bulk operations.

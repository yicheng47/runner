# 554 — Manage projects over MCP

> Tracking issue: [#554](https://github.com/yicheng47/runner/issues/554)
> Priority: P1.

## Motivation

The MCP server is how an outside agent drives Runner: a Claude Code session creates crews, starts missions, posts into the bus, and starts direct chats, all through `runner-mcp`. Projects are the one durable container those missions and chats live in, and the server exposes exactly two read-only tools for them, `project_list` and `project_get`. `mission_start` and `session_start_direct` accept a `project_id`, so an agent can file new work into a project that already exists, but it cannot create the project, rename it, file a mission that was started unfiled, or delete the project when the work is done. Each of those is a sidebar action today (`crates/runner-app/src/surfaces/sidebar.rs`: create from the project menu, rename in place, delete behind a confirm dialog, drag to file), which means every scripted or agent-driven workflow that needs a fresh project stops for a human click.

The backend already has the operations: `ops::project::project_create`, `project_rename`, and `project_delete`, and `ops::node::node_move` for filing. This spec is the MCP surface over them, plus the one guard an outside caller needs that the sidebar gets from its dialog.

## Scope

### In scope

- **`project_create(name, cwd)`.** Trims both values and rejects empty ones, as the op does. At the MCP boundary `cwd` must also be an absolute path to an existing directory, rejected as an invalid request otherwise; the sidebar gets this for free from its directory picker, and a typo here would otherwise surface much later as a spawn failure in every session of the project. Returns the new `ProjectRow`. The project appends to the sidebar order, as it does from the UI.
- **`project_rename(id, name)`.** Same validation as the op; returns the updated row.
- **`project_delete(id, force = false)`.** Follows the sidebar's semantics: member missions archive, member terminals close, member chats archive, and the project row and node go in one transaction. The sidebar shows a confirm dialog before it kills anything; an MCP caller sees no dialog, so the tool refuses while any member session is still running and returns the running session ids and mission ids in the error, unless `force` is true, in which case it takes the sidebar's path and kills them. Returns the archived mission ids and chat session ids.
- **`mission_set_project(mission_id, project_id | null)`.** Files a mission into a project or unfiles it (`null`). Resolves the mission's node through `repo::node::find_by_ref(NodeType::Mission, …)` and the target project's node, appends the mission at the end of the target container, and goes through `ops::node::node_move` so the mission's `project_id` pointer and the layout events are rewritten the way a sidebar drag rewrites them. Moving a mission does not change its `cwd`; that was copied at start.
- **Events.** Every write emits what the ops already emit (`project/changed`, `mission/changed`, `chat/layout-changed`, `session/updated`), so open windows refresh without a restart. No new event types.
- **Registry.** The four tools join the `tool_router_registers_workspace_and_mission_tools` list and pass `every_tool_input_property_declares_a_type`.

### Out of scope

- Reordering projects. `projects.position` and the project node's order both exist; which one the sidebar reads is not settled here, and an ordering tool would have to pick.
- Moving chat tabs between projects. A tab holds several panes and its node id is not something an MCP client knows; `session_start_direct` already takes a `project_id` for new chats.
- Changing a project's `cwd`. The sidebar cannot either; a project with the wrong directory is deleted and recreated.
- A `runner` CLI command for projects. The CLI is the mission bus for agents inside a mission, not a workspace admin tool.
- Bulk operations and dry runs. `project_get` plus `mission_list` filtered by project answers "what would this delete" well enough.

## Implementation Phases

### Phase 1 — tools

- `crates/runner-backend/src/mcp/tools/project.rs`: `ProjectCreateArgs { name, cwd }`, `ProjectRenameArgs { id, name }`, `ProjectDeleteArgs { id, force: bool }`, and the three tools, mapping `Error::Msg` to `invalid_request` as the file already does. The directory check lives in the tool, not the op, so the sidebar path is unchanged.
- `crates/runner-backend/src/mcp/tools/mission.rs`: `MissionSetProjectArgs { mission_id, project_id: Option<String> }` and `mission_set_project`, resolving nodes and delegating to `ops::node::node_move`.
- `ops::project`: a `running_member_sessions(conn, project_id) -> Vec<(session_id, Option<mission_id>)>` helper over `ops::node::container_children`, used by the delete guard and returned in its error.
- `crates/runner-backend/src/mcp/server.rs`: add the four names to the registry test.
- Tests in `mcp/tools/project.rs` and `mcp/tools/mission.rs` against `db::open_in_memory()`: create rejects a relative or missing cwd and appends position; rename returns the new name; delete refuses with the running ids while a member session row is `Running`, succeeds with `force`, and archives an unfiled-after-delete mission; `mission_set_project` moves a mission in and back out and leaves `cwd` alone.

### Phase 2 — docs

- `docs/arch/arch.md` §9 MCP paragraph (the "Outside agents and tools operate Runner itself" one): list `project_create`, `project_rename`, `project_delete`, and `mission_set_project` beside the existing `project_*` mention.

## Verification

- [ ] `project_create` with a relative or non-existent `cwd` returns an invalid-request error; with a real directory it returns a row whose `position` is one past the previous last project.
- [ ] `project_rename` on an unknown id returns `project not found`; on a known id the sidebar shows the new name without a restart.
- [ ] `project_delete` on a project with a running chat returns the running session id and does nothing; with `force: true` the chat is killed and archived and the project is gone; on a project with only stopped members it archives them and returns their ids.
- [ ] `mission_set_project` from unfiled into a project shows the mission under that project in the sidebar; back to `null` shows it unfiled; `mission_get` reports the same `cwd` before and after.
- [ ] `tool_router_registers_workspace_and_mission_tools` and `every_tool_input_property_declares_a_type` pass with the four new tools.

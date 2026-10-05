# Mission 1a: one request surface

Implementation record for [the brief](../briefs/645-m1-request-surface.md), dated 2026-10-05. Branch `refactor/645-m1-request-surface` lands on `feat/645-runnerd`, per Jason’s feed update. No app or real agent was launched for validation.

## Request surface

`runner_core::protocol::api` has **119 operations**, including **8 fast reads**: `mission_get`, `role_get`, `role_get_by_handle`, `session_activity_snapshot`, `window_snapshot`, `usage_snapshot`, `discovery_snapshot`, and `direct_chat_path`. The same macro table generates requests, responses, synchronous client methods, backend dispatch and the serde test. Every in-process call JSON-encodes and decodes its request and response, including backend errors. Subscription maps the existing event channel’s payload, lag and close semantics.

`runner-app` now depends directly on the existing `runner-core` workspace crate. No third-party dependency or workspace member was added. Old backend type paths re-export the moved definitions. Nested optional patches retain omit/clear/set semantics; existing catalog and MCP schema goldens pass without changing their expectations. Runtime capabilities are immutable registry metadata restored when decoding the existing public catalog JSON.

`AppStore` owns the client and rows. Jason’s 2026-10-05 correction preserves original call threads: synchronous handlers issue synchronous requests, and original background tasks remain background tasks. No new request queue, generation gate or stale-completion guard remains. Startup constructs windows synchronously as before. Runtime forms read the catalog and discovery directly through the client; window ownership handlers read a fast current snapshot.

Render paths use locally held usage, editor availability, ownership and session-detail values, refreshed by main-thread initialization, handlers and event callbacks. Baseline render-path core accesses were found and reported on the feed: mission grid-hint writes, focused chat-detail loading, usage snapshots, window ownership snapshots, and runtime environment/editor availability reads. The grid hint now updates from main-thread layout/resize/selection and store callbacks after layout measurement. These calls stay on the main thread, outside render/prepaint/paint. File-link opening reads the current daemon path through the fast `direct_chat_path` request.

## Logic moved into the backend

- Launch-resume claim consumption, drawer exclusions, resume iteration and staggering moved from bootstrap to `daemon::resume`, behind `consume_resume_on_launch`.
- Usage enablement and its refresh trigger moved behind `usage_set_enabled`; manual refresh uses `usage_refresh`.
- Windows command registry and macOS escalation adapters moved from the app defaults helper to `daemon::commands`, behind command status/default/action requests.
- Mission archive, pin and rename notification emission moved from UI callbacks into their dispatch handlers.
- Discovery/environment reads, editor executable checks and complete session-detail reads are backend snapshot requests. Crew list database access is behind `crew_list_all`.
- Skill and MCP catalog/read/write/default-registration effects use backend requests. Their UI policy and default selection remain in the app. Native clipboard file-path reads remain in their original backend implementation behind `session_clipboard_file_paths`; image file creation uses `session_paste_image`. Both requests keep their original calling thread.
- Window registration, focus, subject reports, mission ownership and teardown are requests. Bootstrap construction, MCP startup and quit remain for 1c; terminal and agent-update bridges remain for 1b.

## Guard allowlist

| Entry | Removing mission |
| --- | --- |
| `bootstrap.rs`: boot, NativeMcpServer, client host, wake installation, quit | 1c |
| `terminal/*`: TerminalSession and renderer | 1b |
| `terminal_ime.rs`: input error type | 1b |
| `surfaces/agent_update.rs`: UpdateTerminalEvents and host | 1b |
| `app_store.rs`: the `TerminalBridge::new(host.terminal_core()…)` construction only | 1b |

The guard ignores test modules, test-only fixture files, comments and string literals. It checks backend paths, `AppCore`, database access, direct core fields, `core(cx)` field access and raw update-host access. Its planted-violation test also verifies that a production violation after a test module is still caught.

## Verification

| Command | Exit | Counts / evidence |
| --- | ---: | --- |
| Before: `cargo test --locked --workspace --profile ci` with pipefail | 0 | 1,946 passed; 3 ignored |
| After: `cargo test --locked --workspace --profile ci` with pipefail | 0 | 1,952 passed; 3 ignored |
| `make verify` with pipefail | 0 | Workspace check, tests, Clippy, updater Clippy, formatting |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | All targets |
| `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater -- -D warnings` | 0 | Updater feature |
| `cargo fmt --all --check` | 0 | |
| `git diff --check` | 0 | |
| `cargo check --locked --workspace --all-targets --profile ci --target x86_64-pc-windows-msvc` | 101 | Local Windows SDK unavailable: `ring` C compilation cannot find `assert.h`; Runner compilation was not reached |
| `cargo check --locked -p runner-core --all-targets --profile ci --target x86_64-pc-windows-msvc` | 0 | Protocol and its tests type-check on Windows; all new cfg gates and moved imports manually audited |

All original test assertions and golden expectations remain. Test fixtures use the shared in-process client builder. Four title-parser tests moved with their module from backend to core. Six tests were added: two protocol tests, two in-process transport tests and two guard tests. The pane-focus workaround and its extra test were removed when original synchronous spawning was restored.

The first PR CI run passed macOS and failed Windows on two Unix-only test imports and the native clipboard reader’s missing app dependency. The fix gates the imports and restores the original clipboard reader to the backend, which already owns its platform dependencies, behind a synchronous request. The local Windows workspace check stopped in dependency compilation because this Mac has no Windows SDK C headers; the protocol crate type-checks for Windows. Every new cfg gate and moved import was reread, including the command adapters, test-only imports, runtime path helpers and clipboard platform blocks. No dependency was added.

## Main-thread requests

No call moved from the main thread to a background executor. Existing `background_spawn` jobs remain there. The following table lists all 89 non-`fast` request sites that can run on the main thread, including synchronous helpers and foreground task callbacks. Locations are relative to `crates/runner-app/src`. Requests inside listener callbacks declared by render functions execute only when the handler fires. MCP write closures and the user command-action helper execute only in their existing background jobs and are excluded. Mission 1c decides what to move when the socket transport arrives.

| App source | Request | Main-thread context |
| --- | --- | --- |
| `app_store.rs:442` | `mission_list_summary_impl` | `refresh` |
| `app_store.rs:469` | `node_list` | `refresh_nodes` |
| `app_store.rs:590` | `session_list_recent_direct` | `refresh_sessions_inner` |
| `app_store.rs:593` | `session_details` | `refresh_sessions_inner` |
| `app_store.rs:601` | `role_list` | `refresh_roles_inner` |
| `app_store.rs:611` | `crew_list_all` | `refresh_crews_inner` |
| `app_store.rs:622` | `node_list` | `refresh_nodes_inner` |
| `app_store.rs:634` | `project_list` | `refresh_projects_inner` |
| `app_store.rs:644` | `mission_list_summary_impl` | `refresh_missions_blocking_inner` |
| `app_store.rs:655` | `session_status_snapshot` | `refresh_activity_inner` |
| `app_store.rs:674` | `file_link_environment` | `refresh_render_snapshots` |
| `bootstrap.rs:220` | `app_woke` | macOS wake notification callback: synchronous on the posting thread; real wake notifications post on the main thread, but other posting threads are possible. |
| `main.rs:617` | `runtime_request_models` | `request_model_catalog` |
| `main.rs:785` | `session_get` | `new` |
| `main.rs:1080` | `usage_set_enabled` | `handle_app_store_update` |
| `main.rs:1674` | `window_register` | `open_runner_window` |
| `main.rs:1723` | `unregister` | `open_runner_window` |
| `app_store/mcp_removal.rs:158` | `mcp_client_status` | `remove_registration_at` |
| `app_store/mcp_removal.rs:178` | `mcp_remove_runner_entry` | `remove_registration_at` |
| `app_store/skill_defaults.rs:21` | `runtime_catalog` | `refresh_runner_skill_status` |
| `app_store/skill_defaults.rs:41` | `agent_skill_remove` | `initialize_skill_defaults` |
| `app_store/skill_defaults.rs:45` | `runtime_catalog` | `initialize_skill_defaults` |
| `app_store/skill_defaults.rs:77` | `agent_skill_status_root` | `skill_status` |
| `app_store/skill_defaults.rs:101` | `agent_skill_install_root` | `reconcile_skill_roots` |
| `app_store/command_default.rs:65` | `command_install_default` | `initialize_command_default` |
| `app_store/command_default.rs:134` | `command_directory_writable` | `command_install_inputs` |
| `app_store/command_default.rs:156` | `command_status` | `refresh_runner_command_status` |
| `surfaces/windowing.rs:23` | `report_subjects` | `report_current_subjects` |
| `surfaces/windowing.rs:42` | `mark_focused` | `sync_window_activation` |
| `surfaces/windowing.rs:49` | `mark_blurred` | `sync_window_activation` |
| `surfaces/windowing.rs:208` | `unregister` | `prepare_window_close` |
| `surfaces/app_shell.rs:90` | `runtime_status_list` | `agents_with_updates` |
| `surfaces/app_shell.rs:118` | `runtime_status_list` | `usage_installed` |
| `surfaces/app_shell.rs:558` | `usage_refresh` | `render_usage_popover` listener callback |
| `surfaces/app_shell.rs:1030` | `usage_refresh` | `render_app_sidebar` listener callback |
| `surfaces/app_shell.rs:1031` | `runtime_check_updates` | `render_app_sidebar` listener callback |
| `surfaces/settings_page.rs:899` | `runtime_check_updates` | `enter_settings_pane` |
| `surfaces/start_chat.rs:486` | `session_start_shell_in` | `new_terminal_tab` |
| `surfaces/start_chat.rs:519` | `session_close` | `new_terminal_tab` |
| `surfaces/start_chat.rs:646` | `session_start_shell_in` | `add_terminal_drawer_shell` |
| `surfaces/start_chat.rs:675` | `session_close` | `add_terminal_drawer_shell` |
| `surfaces/start_chat.rs:678` | `node_tab_upsert` | `add_terminal_drawer_shell` |
| `surfaces/start_chat.rs:765` | `session_start_shell_in` | `spawn_terminal_in_pane` |
| `surfaces/start_chat.rs:798` | `session_close` | `spawn_terminal_in_pane` |
| `surfaces/start_chat.rs:801` | `node_tab_upsert` | `spawn_terminal_in_pane` |
| `surfaces/start_chat.rs:902` | `role_list` | `open_start_chat_modal` |
| `surfaces/start_chat.rs:1127` | `runtime_refresh_models` | `refresh_start_chat_models` |
| `surfaces/start_chat.rs:1483` | `session_start_direct_with_speed` | `submit_start_chat` |
| `surfaces/start_chat.rs:1501` | `session_start_runtime_with_speed` | `submit_start_chat` |
| `surfaces/start_chat.rs:1517` | `session_rename` | `submit_start_chat` |
| `surfaces/start_chat.rs:2767` | `runtime_catalog` | `load_selectable_runtimes` |
| `surfaces/chat.rs:403` | `session_get` | `sync_active_chat_detail` |
| `surfaces/chat.rs:913` | `session_rename` | `submit_pane_rename` |
| `surfaces/chat.rs:1035` | `session_take_resume_on_launch` | `resume_visible_drawer_shell_on_launch` |
| `surfaces/chat.rs:1478` | `session_clipboard_file_paths` | `on_paste` |
| `surfaces/chat.rs:1865` | `node_tab_upsert` | `persist_active_tab` |
| `surfaces/chat.rs:2017` | `session_shell_has_foreground_process` | `request_close_drawer_shell` |
| `surfaces/chat.rs:2047` | `session_shell_has_foreground_process` | `request_close_terminal_pane` |
| `surfaces/chat.rs:2129` | `node_tab_delete` | `request_close_single_pane_tab` |
| `surfaces/chat.rs:2190` | `session_shell_has_foreground_process` | `request_close_terminal_tab` |
| `surfaces/sidebar/drag.rs:179` | `node_reorder_pinned` | `commit_sidebar_drop` |
| `surfaces/sidebar/drag.rs:187` | `node_move` | `commit_sidebar_drop` |
| `surfaces/sidebar/drag.rs:203` | `node_move` | `commit_sidebar_drop` |
| `surfaces/sidebar/project.rs:119` | `project_create` | `submit_project` |
| `surfaces/sidebar/project.rs:181` | `project_delete` | `confirm_delete_project` |
| `surfaces/sidebar/menus.rs:255` | `node_set_pinned` | `handle_sidebar_menu_action` |
| `surfaces/sidebar/rows.rs:412` | `session_rename` | `submit_sidebar_rename` |
| `surfaces/sidebar/rows.rs:414` | `node_rename` | `submit_sidebar_rename` |
| `surfaces/sidebar/rows.rs:424` | `project_rename` | `submit_sidebar_rename` |
| `surfaces/sidebar/rows.rs:434` | `mission_rename_impl` | `submit_sidebar_rename` |
| `surfaces/sidebar/archive.rs:292` | `node_tab_upsert` | `close_archived_pane` |
| `surfaces/sidebar/activation.rs:97` | `mark_blurred` | `mark_active_tab_viewed` |
| `surfaces/sidebar/activation.rs:100` | `node_mark_viewed` | `mark_active_tab_viewed` |
| `surfaces/settings/mcp.rs:754` | `mcp_validate_edit` | `validation` |
| `surfaces/crews/add_slot.rs:44` | `runtime_catalog` | `open_add_slot` |
| `surfaces/crews/add_slot.rs:287` | `runtime_catalog` | `refresh_add_slot_runtimes` |
| `surfaces/roles/edit.rs:235` | `role_list` | `submit_role_edit` |
| `surfaces/roles/logic.rs:75` | `runtime_catalog` | `ensure_runtime_present` |
| `surfaces/roles/create.rs:149` | `role_list` | `submit_create_role` |
| `surfaces/roles/delete.rs:91` | `role_list` | `confirm_role_delete` |
| `surfaces/mission_workspace/attach.rs:24` | `window_set_mission` | `open_mission` |
| `surfaces/mission_workspace/attach.rs:357` | `mission_grid_hint_set` | `sync_mission_grid_hint` |
| `surfaces/mission_workspace/attach.rs:427` | `session_take_resume_on_launch` | `resume_visible_drawer_shell_on_launch` |
| `surfaces/mission_workspace/drawer.rs:122` | `session_start_shell_in` | `add_terminal_drawer_shell` |
| `surfaces/mission_workspace/drawer.rs:154` | `session_close` | `add_terminal_drawer_shell` |
| `surfaces/mission_workspace/drawer.rs:304` | `session_shell_has_foreground_process` | `request_close_terminal_drawer_shell` |
| `surfaces/mission_workspace/state.rs:527` | `node_mission_layout_set` | `persist_mission_layout` |
| `surfaces/mission_workspace/input.rs:23` | `mark_direct_sessions_viewed` | `mark_active_session_viewed` |
| `surfaces/mission_workspace/input.rs:249` | `session_clipboard_file_paths` | `on_mission_paste` |

## Moved types

The following **93 named types/aliases** moved from backend definitions to protocol definitions. Each old backend path retains a `pub use`. Types already shared by core (`Runtime`, event types and command install status) continue to use their existing core definitions; their old backend aliases remain.

| Old path | New path |
| --- | --- |
| `runner_backend::agent_skill::InstallOutcome` | `runner_core::protocol::agent_skill::InstallOutcome` |
| `runner_backend::agent_skill::SkillRootState` | `runner_core::protocol::agent_skill::SkillRootState` |
| `runner_backend::agent_skill::SkillRootStatus` | `runner_core::protocol::agent_skill::SkillRootStatus` |
| `runner_backend::cli_install::CommandActionOutcome` | `runner_core::protocol::command::CommandActionOutcome` |
| `runner_backend::cli_install::CommandInstallInputs` | `runner_core::protocol::command::CommandInstallInputs` |
| `runner_backend::cli_install::CommandPlatform` | `runner_core::protocol::command::CommandPlatform` |
| `runner_backend::model::CodexSpeed` | `runner_core::protocol::model::CodexSpeed` |
| `runner_backend::model::Crew` | `runner_core::protocol::model::Crew` |
| `runner_backend::model::Mission` | `runner_core::protocol::model::Mission` |
| `runner_backend::model::MissionStatus` | `runner_core::protocol::model::MissionStatus` |
| `runner_backend::model::Role` | `runner_core::protocol::model::Role` |
| `runner_backend::model::Session` | `runner_core::protocol::model::Session` |
| `runner_backend::model::SessionStatus` | `runner_core::protocol::model::SessionStatus` |
| `runner_backend::model::Slot` | `runner_core::protocol::model::Slot` |
| `runner_backend::model::SlotWithRole` | `runner_core::protocol::model::SlotWithRole` |
| `runner_backend::model::Timestamp` | `runner_core::protocol::model::Timestamp` |
| `runner_backend::model::Ulid` | `runner_core::protocol::model::Ulid` |
| `runner_backend::ops::crew::CreateCrewInput` | `runner_core::protocol::crew::CreateCrewInput` |
| `runner_backend::ops::crew::CrewListItem` | `runner_core::protocol::crew::CrewListItem` |
| `runner_backend::ops::crew::CrewMemberPreview` | `runner_core::protocol::crew::CrewMemberPreview` |
| `runner_backend::ops::crew::UpdateCrewInput` | `runner_core::protocol::crew::UpdateCrewInput` |
| `runner_backend::ops::mcp::McpCatalog` | `runner_core::protocol::mcp::McpCatalog` |
| `runner_backend::ops::mcp::McpClientId` | `runner_core::protocol::mcp::McpClientId` |
| `runner_backend::ops::mcp::McpClientStatus` | `runner_core::protocol::mcp::McpClientStatus` |
| `runner_backend::ops::mcp::McpServerClientEntry` | `runner_core::protocol::mcp::McpServerClientEntry` |
| `runner_backend::ops::mcp::McpServerDefinition` | `runner_core::protocol::mcp::McpServerDefinition` |
| `runner_backend::ops::mcp::McpServerEntry` | `runner_core::protocol::mcp::McpServerEntry` |
| `runner_backend::ops::mission::MissionActivityState` | `runner_core::protocol::mission::MissionActivityState` |
| `runner_backend::ops::mission::MissionStart` | `runner_core::protocol::mission::MissionStart` |
| `runner_backend::ops::mission::MissionSummary` | `runner_core::protocol::mission::MissionSummary` |
| `runner_backend::ops::mission::PostMessageInput` | `runner_core::protocol::mission::PostMessageInput` |
| `runner_backend::ops::mission::PostSignalInput` | `runner_core::protocol::mission::PostSignalInput` |
| `runner_backend::ops::mission::ResumeMissionOutput` | `runner_core::protocol::mission::ResumeMissionOutput` |
| `runner_backend::ops::mission::StartMissionInput` | `runner_core::protocol::mission::StartMissionInput` |
| `runner_backend::ops::mission::StartMissionOutput` | `runner_core::protocol::mission::StartMissionOutput` |
| `runner_backend::ops::ListPage` | `runner_core::protocol::mod_types::ListPage` |
| `runner_backend::ops::node::NodeTabUpsertInput` | `runner_core::protocol::node::NodeTabUpsertInput` |
| `runner_backend::ops::project::ProjectDeleteOutcome` | `runner_core::protocol::project::ProjectDeleteOutcome` |
| `runner_backend::ops::project::ProjectScope` | `runner_core::protocol::project::ProjectScope` |
| `runner_backend::ops::role::CreateRoleInput` | `runner_core::protocol::role::CreateRoleInput` |
| `runner_backend::ops::role::RoleActivity` | `runner_core::protocol::role::RoleActivity` |
| `runner_backend::ops::role::RoleWithActivity` | `runner_core::protocol::role::RoleWithActivity` |
| `runner_backend::ops::role::UpdateRoleInput` | `runner_core::protocol::role::UpdateRoleInput` |
| `runner_backend::ops::runtime::RuntimeCatalogEntry` | `runner_core::protocol::runtime::RuntimeCatalogEntry` |
| `runner_backend::ops::runtime::RuntimeCatalogOption` | `runner_core::protocol::runtime::RuntimeCatalogOption` |
| `runner_backend::ops::runtime::RuntimeDefinition` | `runner_core::protocol::runtime::RuntimeDefinition` |
| `runner_backend::ops::session::DirectSessionEntry` | `runner_core::protocol::session::DirectSessionEntry` |
| `runner_backend::ops::session::SessionRow` | `runner_core::protocol::session::SessionRow` |
| `runner_backend::ops::session::StartDirectSessionOutput` | `runner_core::protocol::session::StartDirectSessionOutput` |
| `runner_backend::ops::slot::CreateSlotInput` | `runner_core::protocol::slot::CreateSlotInput` |
| `runner_backend::ops::slot::CrewMembership` | `runner_core::protocol::slot::CrewMembership` |
| `runner_backend::ops::slot::UpdateSlotInput` | `runner_core::protocol::slot::UpdateSlotInput` |
| `runner_backend::ops::window::SecondaryState` | `runner_core::protocol::window::SecondaryState` |
| `runner_backend::repo::node::NodeRow` | `runner_core::protocol::node::NodeRow` |
| `runner_backend::repo::node::NodeType` | `runner_core::protocol::node::NodeType` |
| `runner_backend::repo::project::ProjectRow` | `runner_core::protocol::project::ProjectRow` |
| `runner_backend::router::runtime::MissionPermissionMode` | `runner_core::protocol::permissions::MissionPermissionMode` |
| `runner_backend::router::runtime::PermissionMode` | `runner_core::protocol::permissions::PermissionMode` |
| `runner_backend::runtime_status::OverrideValidationError` | `runner_core::protocol::runtime::OverrideValidationError` |
| `runner_backend::runtime_status::RuntimeCommandSource` | `runner_core::protocol::runtime::RuntimeCommandSource` |
| `runner_backend::runtime_status::RuntimeExecutableStatus` | `runner_core::protocol::runtime::RuntimeExecutableStatus` |
| `runner_backend::runtime_status::RuntimeRowState` | `runner_core::protocol::runtime::RuntimeRowState` |
| `runner_backend::runtime_status::RuntimeStatusResponse` | `runner_core::protocol::runtime::RuntimeStatusResponse` |
| `runner_backend::runtime_status::ShellDiscoveryStatus` | `runner_core::protocol::runtime::ShellDiscoveryStatus` |
| `runner_backend::runtimes::helpers::Permissions` | `runner_core::protocol::runtime_metadata::Permissions` |
| `runner_backend::runtimes::RuntimeCapabilities` | `runner_core::protocol::runtime::RuntimeCapabilities` |
| `runner_backend::runtimes::RuntimeCatalog` | `runner_core::protocol::runtime::RuntimeCatalog` |
| `runner_backend::session::manager::SpawnedSession` | `runner_core::protocol::session::SpawnedSession` |
| `runner_backend::session::runtime::SessionActivityState` | `runner_core::protocol::session::SessionActivityState` |
| `runner_backend::session::status::Activity` | `runner_core::protocol::status::Activity` |
| `runner_backend::session::status::AgentObservation` | `runner_core::protocol::status::AgentObservation` |
| `runner_backend::session::status::AgentStatus` | `runner_core::protocol::status::AgentStatus` |
| `runner_backend::session::status::HumanInteraction` | `runner_core::protocol::status::HumanInteraction` |
| `runner_backend::session::status::Lifecycle` | `runner_core::protocol::status::Lifecycle` |
| `runner_backend::session::status::ObservationSource` | `runner_core::protocol::status::ObservationSource` |
| `runner_backend::session::status::TurnOutcome` | `runner_core::protocol::status::TurnOutcome` |
| `runner_backend::session::status::WaitReason` | `runner_core::protocol::status::WaitReason` |
| `runner_backend::session::status::WorkDetail` | `runner_core::protocol::status::WorkDetail` |
| `runner_backend::shell_path::DiscoveryOutcome` | `runner_core::protocol::discovery::DiscoveryOutcome` |
| `runner_backend::shell_path::DiscoveryResult` | `runner_core::protocol::discovery::DiscoveryResult` |
| `runner_backend::shell_path::LoginShellEnv` | `runner_core::protocol::discovery::LoginShellEnv` |
| `runner_backend::skills::GlobalState` | `runner_core::protocol::skills::GlobalState` |
| `runner_backend::skills::SkillCatalog` | `runner_core::protocol::skills::SkillCatalog` |
| `runner_backend::skills::SkillDocument` | `runner_core::protocol::skills::SkillDocument` |
| `runner_backend::skills::SkillEntry` | `runner_core::protocol::skills::SkillEntry` |
| `runner_backend::usage::AgentUsage` | `runner_core::protocol::usage::AgentUsage` |
| `runner_backend::usage::RefreshReason` | `runner_core::protocol::usage::RefreshReason` |
| `runner_backend::usage::RuntimeUsage` | `runner_core::protocol::usage::RuntimeUsage` |
| `runner_backend::usage::UnavailableReason` | `runner_core::protocol::usage::UnavailableReason` |
| `runner_backend::usage::UsageSnapshot` | `runner_core::protocol::usage::UsageSnapshot` |
| `runner_backend::usage::UsageWindow` | `runner_core::protocol::usage::UsageWindow` |
| `runner_backend::windows::Subject` | `runner_core::protocol::window::Subject` |
| `runner_backend::windows::WindowEntry` | `runner_core::protocol::window::WindowEntry` |

## Jason’s smoke list

A direct chat and a role chat (type, stop, resume, fork); a mission (start, stop and resume a slot, archive); creating, editing and deleting roles, crews and projects; every Settings pane (Agents refresh, usage pill, Skills, MCP, command install status); sidebar pin, rename, drag and a second window. This live smoke is reserved for Jason.

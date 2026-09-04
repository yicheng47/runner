# 474 — Mission terminal drawer: a shell beneath the mission

Tracking issue: [#474](https://github.com/yicheng47/runner/issues/474). Feature, P2. Spec: [`docs/features/474-mission-terminal-drawer.md`](../../features/archive/474-mission-terminal-drawer.md) — the spec wins on any detail this plan leaves out. Design: `design/runner.pen`, the `(474)` frames and `Spec — Mission terminal drawer (474) · v1`. Predecessor: [469](../../features/archive/469-terminal-drawer.md) (#471) — reuse its model, element, and lifecycle code.

## What ships

A primary, non-archived mission workspace gains one terminal drawer under its center column — beneath the Feed / runner-tab surface, beside (not under) the rail — toggled by the existing `toggle-terminal-drawer` action (⌥F12) or a `panel-bottom` glyph in the header's trailing group, ahead of the rail toggle. Same clamp, handle, strip, `+`, chevron as 469. Shells are `"shell"` direct sessions at the mission cwd; they never carry `mission_id` / `slot_id`, never reach the Feed, runner statuses, attention, sidebar, or direct tabs, and survive tab switches. **Stop** / **Resume** act on slots only; archive / delete closes drawer shells; state lives on the mission node and relaunches through the existing launch-claim path.

## Where it touches

- `crates/runner-app/src/pane_layout.rs` — **lift the drawer model out of `PaneLayout`.** `DrawerLayout` (`:180`), the `add` / `remove` / `activate` mutators (`:359`–`:405`), and `clamp_drawer_height` (`:600`) become one public `TerminalDrawer` type; `PaneLayout` embeds it behind its current accessors. Add `MissionLayout { drawer: TerminalDrawer }` (serde-defaulted, same wire shape) with `from_node_row` / layout JSON. `tests/pane_layout.rs` drawer tests pass unchanged; the mission round-trip and old-row defaults go beside them.
- `crates/runner-backend/src/repo/node.rs` — `ensure_active_sessions` builds `covered` from Tab rows only (`:733`); add `NodeType::Mission` rows through the same `session_ids` (`StoredLayout`, `:51`, already reads `drawer.shells` with `slots` defaulted empty). Leave `find_for_session`, pinning, attention, and `delete_container_tabs_and_archive` Tab-only — drawer shells must stay invisible to them.
- `crates/runner-backend/src/ops/node.rs` — new `node_mission_layout_set(state, node_id, layout)` beside `node_tab_upsert` (`:75`): require `NodeType::Mission`, `validate_layout` (`:26`), `UPDATE nodes SET layout`, emit `chat/layout-changed` so `AppStore::refresh_nodes` (`crates/runner-app/src/app_store.rs:382`) reloads. The workspace finds its row in `app_store.nodes` by `node_type == Mission && ref_id == mission_id`.
- `crates/runner-backend/src/ops/mission.rs` — `mission_archive_impl` and `mission_delete` (`:1279`) read the node layout and `session_close` (`ops/session.rs:518`) each drawer shell before `ensure_active_sessions` sweeps the row (`repo/node.rs:724`). `mission_stop_impl` is untouched — it iterates slot sessions, which drawer shells never are.
- `crates/runner-app/src/surfaces/mission_workspace.rs` — the workspace owns its terminals (`attached`, `:163`; `ensure_mission_terminal_attached`, `:1279`). Generalize attach to a session id pulled from `app_store.bridge.session(id)` so drawer shells — not in `self.sessions` — use the same `AttachedChat` bundle. `render_loaded_mission` (`:2831`) becomes a column: tabs + pane, handle, drawer, all inside the center `div` (`:2596`) so the rail (`:2614`) keeps full height. Header (`:2687`): the toggle goes in `trailing_actions` before `rail_action` (`:2727`), gated on `!self.archived()` (`:490`) `&& !self.secondary` (`:500`); Stop's title at `:2716` becomes "Kill every slot PTY; mission stays running so you can Resume". A drawer size estimate next to `estimated_mission_terminal_size` (`:1208`) subtracts `32 + 5 + 24` like `estimated_drawer_terminal_size` (`surfaces/chat.rs:1050`).
- `crates/runner-app/src/surfaces/panes.rs` — `render_terminal_drawer` (`:1452`) and `render_drawer_terminal` (`:1591`) are the reference. Extract the strip + chips into a free element fn taking shells, active id, labels, and the activate / close / add / hide callbacks so both surfaces call it; the terminal body and Shell exited card are small enough to mirror in the workspace with its own focus handle. `terminal_drawer_tooltip` (`:2289`) becomes `pub(crate)`, reused verbatim.
- `crates/runner-app/src/surfaces/start_chat.rs`, `main.rs` — `toggle_terminal_drawer` (`:250`) returns unless `route == Chat`; add a `Mission(_)` arm forwarding to `self.mission_workspace` (`main.rs:411`). Same for `new_terminal` (`:223`, the palette's path at `command_palette.rs:347`) and `close_window_or_pane` (`main.rs:197`): a focused mission drawer terminal hides on ⌘W. `add_terminal_drawer_shell` (`:299`) is the shape to copy — synchronous `session_start_shell` (`ops/session.rs:892`) with `project_id` and the resolved cwd, `spawned_id` rollback on error, then attach and focus.
- `crates/runner-app/src/surfaces/settings_page.rs` — `start_launch_auto_resume` (`:467`) sizes tab shells only (`:482`); mission drawer shells fall through `launch_dims_for` (`:138`) to the mission *pane* size. Add them to `direct_sizes` with the drawer estimate.
- `keymap.rs` — nothing new; `toggle-terminal-drawer` (`:123`) stays the single entry.

## Plan

1. **Model and persistence** — lift `TerminalDrawer` with no chat-tab behavior change; `MissionLayout` with defaults; `node_mission_layout_set`; mission rows join `covered`. Tests first: mission round-trip, old mission row reads hidden / 280 / empty, a drawer shell id on a mission node is never wrapped into a tab, `PaneLayout` tests untouched.
2. **Shell lifecycle** — start at mission cwd → project dir → `default_working_dir` → `$HOME`; attach by id; chip `×` through `session_shell_has_foreground_process` (`ops/session.rs:552`) + the existing confirm; Restart via `session_resume`; archive / delete close every drawer shell; relaunch dims cover them.
3. **Mission workspace UI** — the column with handle and drawer in the center only; shared strip element; terminal body + Shell exited card; clamp; resize finishes on `on_mouse_up` **and** `on_mouse_up_out` on the center column, not `on_drop` alone (469's review lesson, `panes.rs:503`); header toggle with honest tooltip; open / hidden / archived / secondary states.
4. **Entry points and exclusions** — route the action, the palette, and ⌘W; Stop copy; confirm no drawer shell reaches the Feed, runner statuses, attention, sidebar rows, direct-tab composition, `⌘1–9`, pane navigation, fork / split targets, or drag-drop.

Land phases in order; each leaves `cargo test -p runner-app` green, phases 1–2 also `cargo test -p runner-backend`, so review can happen per phase.

## Rules of the road

- Do not launch the Runner app (`make run`) — the human smoke-tests. Verify with tests, `make clippy`, `make fmt`.
- One drawer model. If phase 1 ends with two copies of the chip-selection rules or two clamps, it is not done.
- Follow the workspace's own patterns (`AttachedChat`, `SessionOverlay`, `IconButton`, `TerminalElement`, `MissionRailResizeDrag`); no new abstractions beyond the shared drawer type and the strip element.
- Keep the `"shell"` runtime out of attention and completion, as 64 and 469 decided; a mission drawer shell additionally never touches mission routing, messages, or events.
- No hardcoded shortcut text — the tooltip reads the effective binding.

## Non-goals

A drawer per runner or inside the rail, a shell that follows the selected runner, slot promotion, splits / reorder / rename / profiles inside the drawer, sharing or moving shells between missions and chat tabs, an interactive secondary workspace, and any change to the direct-shell recovery policy after an abrupt app exit (tracked separately).

## Verification

See the spec's Verification section for the test list; run `cargo test -p runner-app`, `cargo test -p runner-backend`, `make clippy`, `make fmt`, and report which ran. Manual smoke is the human's.

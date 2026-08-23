# Mission brief — M6.22: archiving the open mission leaves the mission route

Drafted 2026-08-23 after M6.21 landed (PR #430 → `09049d0`). Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), no `cwd`, title `M6.22 — Archive leaves the mission route`, the text below as `goal_override`. Jason's call: lands before the `v0.6.0` tag, beside M6.21.

---

Fix **M6.22** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.22; this brief also lives at `docs/impls/gpui-rewrite/briefs/m6-22-archive-leaves-mission.md`). **Branch off `main`** (`09049d0`, M6.21 merged); one feature branch in this checkout; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. Crate: `runner-app` (the binary is `Runner` since `7b4a864`; the package is still `runner-app`). Files: `surfaces/mission_workspace.rs`, `surfaces/sidebar.rs`, `surfaces/app_shell.rs` if the shared function lands there. Standing GPUI rules in `impl_log.md` apply — never `cx.new` a stateful child in render, notify scope is rebuild cost, `current_view()` only while rendering.

## The bug (Jason, 2026-08-23)

Archiving the mission that is open on the window leaves the window parked on that mission's feed page. Three paths archive a mission and each reacts differently; none does what the user expects:

1. **Feed header menu "Archive"** — `MissionWorkspace::archive_open_mission` (`mission_workspace.rs:2049`) → on success `open_runners` (`:2088`): jumps to the Runners page.
2. **Sidebar mission-row menu "Archive"** — `SidebarMenuAction::ArchiveMission` (`sidebar.rs:1013`) → `mission_archive_impl` → emits `mission/changed` with `&()`. The workspace's `mission/changed` handler (`mission_workspace.rs:1671`) calls `refresh_open_mission`, whose archived branch (`:1778`) clears the session tabs and keeps showing the feed of the now-archived mission. Parked.
3. **MCP `mission_archive`** (`runner-backend/src/mcp/tools/mission.rs:458`) → `emit_mission_changed` → identical to 2. Parked. A mission archived from another window arrives the same way.

## Expected behaviour (decided)

When the open mission becomes archived — by any path — the window leaves `AppRoute::Mission(id)` for the chat surface on its active tab, exactly as `leave_settings` does (`app_shell.rs:802-815`): `set_route(AppRoute::Chat)`, `ensure_active_tab_attached`, `mark_active_tab_viewed`, `focus_active_terminal`. The tabs model already remembers the last-active tab, so this is "jump back to the tab the sidebar highlighted before the mission was opened"; with no tabs, the chat surface's empty state. Never the Runners page. A mission archived while some other route is showing changes nothing about the route (its sidebar row disappears, as today).

## Implementation

- One function on the shell (e.g. `leave_archived_mission(mission_id, window, cx)`) that performs the route change above when `self.route == AppRoute::Mission(mission_id)`, else does nothing. `archive_open_mission` calls it instead of `open_runners`; the archived branch of `refresh_open_mission` calls it too — paths 2 and 3 (and another window) converge there. Keep the existing bookkeeping: `last_mission_terminal_ids` removal, `set_sidebar_archiving`, the `archiving` flag, `refresh_store`.
- Mind `set_route`'s `leaving_mission` deferred workspace reset (`mission_workspace.rs:815`): no double detach. The header path emits `mission/changed` after it navigates, so the refresh that follows must be a harmless no-op (route already left; generation guards) — not a second navigation and not an error toast.
- Secondary windows (`secondary_state`): same reaction. Verify `ensure_active_tab_attached` on a window with an empty tab list lands on the empty chat surface without an error.
- Event payloads: `sidebar.rs:1031` and the MCP tool emit `mission/changed` with `&()`, the workspace path sends `{ "mission_id" }`; the handler already treats a missing id as relevant. Leave them alone unless the fix needs the id — then add it on every emitter.
- No backend changes; no new events.

## Tests

Extract the decision into a pure function and unit-test it: route is `Mission(x)` and `x` is the archived id → leave to `Chat`; route is `Mission(y)`, `Runners`, `Settings`, `Chat` → no change. Existing mission-workspace tests are rewritten, not deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- `make run` on the dev profile: open a mission, archive it from (a) the feed header menu and (b) the sidebar row menu — each time the window lands on the active chat tab with its terminal focused, the sidebar highlights that tab, and the archived mission's row is gone. (c) MCP `mission_archive` is the same reaction as (b) — both arrive as `mission/changed` with no navigation of their own; exercise it through the dev app's MCP server if reachable, otherwise state in the handoff that (b) covers it. Archive a mission that is *not* open → route unchanged. ⇧⌘N: a second window showing the mission follows the same rule. Quit the dev app when done — one instance rule with the human's nightly.
- Handoff: the shared function, the three paths routed through it, the race note above, and which of (a)/(b)/(c) were exercised live.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) only the header path fixed while the sidebar/MCP paths still park on the archived feed; (2) navigation to Runners, or to the empty state while a tab exists; (3) a double route change, or a stale-generation early return that skips the navigation when the `mission/changed` refresh races the header path; (4) the route changing when the archived mission is not the open one; (5) `cx.new` in a render path, or a notify of the window root per event; (6) tests deleted instead of rewritten; (7) the dev app left running.

# 887 — Active sidebar and sidebar refresh

Historical design recovery (2026-11-12): superseded desktop spec canvases referenced in this record are preserved in `YeelinksCo/yeelinks_design` at revision `4605d1e43ab6835a555fdd3957469a6b55ddaec8`, under `runner/desktop/specs/` (including `archive/`). Their original frame ids resolve there; current product designs are in `runner/desktop/runner.pen`.

Tracking issue: [#887](https://github.com/yicheng47/runner/issues/887). Jason approved the final sidebar on 2026-10-19 and requested a Codex duo to implement it first. The decisions below supersede the issue's earlier alternatives.

## Motivation

Find work that is running or needs attention across projects without expanding every project tree. Preserve Projects as the default view and add Active as a remembered view of tabs and missions.

## Design

The approved Pencil source is `/Users/jason/repos/yicheng47/runner/design/specs/887-active-sidebar-view.pen` in the root checkout on main. Read it through Pencil; do not open or edit a `.pen` file in this worktree. `XzJHS` is the final populated screen; `YxSEl` is Projects, `sqqIk` is the empty Active state, `W5o1T` shows number shortcuts, `xuJyC` shows toggle states, and `osxwG` records decisions. The shared sidebar is `N4yXC`, instanced as `BSxqq` in the final screen. Discarded alternatives have been removed.

## Scope

- Use a list-filter icon beside Search to toggle Projects/Active. Highlight it in Active, with tooltips Show active only / Show all. Store the preference as an app setting and retain it across restarts. Workspace remains; Pinned, Projects and Recents are replaced by Active's groups.
- Order the groups **Chats, Missions, Terminals**. Each header shows its visible row count; hide empty groups. With no rows, show the designed empty message, Nothing is running or waiting.
- Following Jason’s native feedback and correction request on 2026-10-19, include all running ordinary chats, including idle/ready agents and agents without activity evidence, and all running missions with at least one live member. Include stopped chats/missions only when they have unread output or an error; selection alone does not retain them. Mission status without a live member does not count as running. Sort Error, Needs you, Working and Unread first, then other running rows, with recent activity within each status and a stable tie-break. A finished unread session remains until viewed; an error remains until it clears. No grace timer. Following Jason’s further native feedback on 2026-10-19, terminals follow the same rule: every running non-drawer shell is listed whether or not it is selected, a Busy shell sorts as Working, and an exited shell needs unread output or an error. This supersedes the original attention-only chat/mission filter, stopped selected-chat retention and the Busy-or-selected terminal filter.
- Following Jason’s 2026-10-19 correction, show one row per tab, as Projects does; this supersedes the earlier one-row-per-session design. A tab with any agent pane belongs to Chats; a tab of only shells belongs to Terminals. The row’s status rolls up its chat panes as Projects does (only a terminal-only tab uses its shells’ Busy status), and the tab is listed when any pane is running or the rollup has attention. Selecting the row or its number shortcut activates the tab and focuses the pane whose status the row shows, falling back to the tab’s focused pane when nothing needs attention. Missions remain one row each, using their existing member attention rollup; mission members are not duplicated as ordinary chat rows. Preserve existing window ownership and mission activation behavior.
- Each row has the provider/type mark, title, a muted location line and trailing attention glyph. Split tabs use the split icon and tab name (the first chat/terminal pane’s name unless the tab is manually named), matching Projects. Location is project, followed by its relative worktree/folder when different from the project root; without a project use the compact cwd. Truncate gracefully. Status detail and waiting duration remain in the pane header. Inline rename edits only the title line of the row that opened Rename; the location line and folder icon stay visible below the editor. Keep the editor on that row when attention reorders the list.
- Number shortcuts follow the visible Active list across Chats, Missions, Terminals, including attention-pane activation for split tabs. In the illustrated state, chats are 1–5, missions 6–7 and terminals 8–9. Respect configured bindings and existing macOS/Windows conventions.
- Keep the existing context-menu semantics. Active offers no drag, reorder or drop targets. Projects retains its existing organization, drag/drop, context menus and shortcut behavior. Changing view must not mutate the underlying organization.
- Plain terminal ordering uses existing Busy/activity status. #904 Phase 2 will later provide exact OSC 133 command boundaries; #861 will provide program status. Neither is implemented here. Drawer shells stay outside Active v1. Do not infer foreground processes or claim exact command activity from today's output heuristic.
- Match the approved shared sidebar chrome and footer in both views: conditional app-update button beside Runner's name, usage control at bottom, separate Settings gear, and the designed workspace/footer dividers. The usage control shows **Codex and Claude only**, left-aligned with a **10 px gap**, transparent at rest with a hover fill. Keep its hit area and existing usage-popover behavior. Use live weekly usage values and existing availability/loading semantics; illustration values are not constants. Antigravity was explicitly removed from this compact footer, not from supported runtimes or the usage popover.
- Use existing theme, platform, localization and accessibility conventions. New strings use the app's existing localization catalogs. Preserve macOS/Windows window chrome and sidebar resizing; this is not a window-chrome redesign.

## Implementation phases

1. Add persisted view selection and a deterministic Active projection over existing tabs, sessions and missions; keep data ownership in the existing layers.
2. Render the approved sections/rows, route row activation and number shortcuts through the same visible order, and apply the shared sidebar chrome/footer changes.
3. Add focused behavioral coverage, update the guide and complete the coder/reviewer working-tree loop. This first pass ends before native UI testing or publication.

Guide pages affected: `docs/guide/guides/chats-and-windows.md`, `docs/guide/reference/keyboard-shortcuts.md`, and any existing usage/update guidance that describes a moved control. Update `docs/guide/concepts/projects.md` only if its description needs to distinguish the two views.

## Verification

Before running checks, list changed behaviors and the selected checks with their reasons. Cover eligibility/attention order, recent-activity ties, counts and empty groups, mixed split tabs and attention-pane activation, mission aggregation without duplication, removal of stopped rows regardless of selection, shortcut order, excluded drawer shells, and persistence/default compatibility. Test meaningful behavior rather than layout constants.

Run `cargo test --locked -p runner-app --profile ci`, workspace all-target Clippy with warnings denied, updater-feature Clippy, formatting and diff checks. If another crate changes, run its relevant tests as well. Record commands/results and the clean review verdict in `docs/tests/887-active-sidebar.md`.

Native acceptance is a later step, not a claimed pass in this coding mission. Select the affected parts of `WS-SIDEBAR-01` (Projects organization remains intact), `WS-SIDEBAR-02` (per-pane unread and failure attention), `WS-SIDEBAR-03` (attention and activation), and `WS-MENU-01` (moved usage trigger/context menu) in `docs/tests/regression/workspace.md`. Add feature-specific visible-list number-shortcut, Active-state and light/dark/layout checks against the approved frames. Do not run a full regression or add unrelated runtime/IME checks.

# Mission brief — M6.20: sidebar drop indicator survives the release

Drafted 2026-08-23 after `v0.6.0`. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), no `cwd`, title `M6.20 — Sidebar drop indicator`, the text below as `goal_override`. First post-GA mission; lands as a nightly.

---

Fix **M6.20** (`docs/impls/gpui-rewrite/m6-remainder.md` §M6.20; this brief also lives at `docs/impls/archive/gpui-rewrite/briefs/m6-20-sidebar-drop-indicator.md`). **Branch off `main`**; one feature branch in this checkout; leave uncommitted docs alone. Crate: `runner-app` (binary `Runner`, package `runner-app`). Files: `surfaces/sidebar.rs`, `surfaces/app_shell.rs`. Standing GPUI rules in `docs/impls/gpui-rewrite/README.md` apply. **Crews do not launch the app**: Jason reproduces and smoke-tests; you instrument, reason, and fix.

## The bug (Jason, 2026-08-23, screenshot)

After dragging a sidebar row and releasing, the accent drop line (2 px, full row width — here between `@investing` and `@housekeeper` under PINNED) stays painted with no drag in progress. Jason also says "drag and move is somehow broken" — confirm with him whether the node actually moved in the failing case; the answer decides whether the missed path is before or after `commit_sidebar_drop`.

## What the code does today (traced)

State on `Sidebar`: `dragged_id`, `drop_target: Option<DropTarget>`, `drop_marker: Option<String>` (`sidebar.rs:209-211`). The line is rendered wherever `drop_marker` equals a row's `before`/`after` marker (`:2547-2586` rows, `:2377-2388` project headers, `:2442` containers, `:2487` end dividers).

Set by: `update_row_drop_target` (`:1373`) and `update_project_container_drop_target` (`:1398`), called from every `on_drag_move::<SidebarNodeDrag>` listener. Note GPUI's `on_drag_move` fires on the **capture phase of every `MouseMove` while `cx.active_drag` is set, regardless of hover** (`gpui-ce-0.3.3/src/elements/div.rs:282-306`); the app filters by `event.bounds.contains(position)`. `dragged_id` is also set in the `on_drag` constructor (`:2324`, `:2554`).

Cleared by: `clear_sidebar_drag` (`:1363`, notifies only if something was set) — reached from (a) `commit_sidebar_drop` (`:1419`, synchronous, clears at `:1495` on every branch including the two early returns) which every `on_drop` calls (`:2372`, `:2460`, `:2514`, `:2580`); (b) the window root's `on_mouse_up` (bubble, root hovered) and `on_mouse_up_out` (capture, root not hovered) in `app_shell.rs:172-185` → `NativeRoot::clear_sidebar_drag` (`sidebar.rs:483`); (c) `dismiss_transients` (`:551`).

GPUI on mouse-up (`window.rs:3778-3830`): capture listeners root-first, then bubble deepest-first; a `drop_listener` fires when its hitbox is hovered, takes `active_drag`, calls the listener, `stop_propagation()` (`div.rs:2102-2131`); afterwards any still-active drag is dropped unconditionally. So every `MouseUpEvent` dispatched to this window ends in a clear by (a) or (b). No listener between the rows and the root stops `MouseUp` propagation (checked: buttons stop `MouseDown` and key events only; `occlude()` is used only by overlays, the command palette, and the settings page). Therefore the line survives only if **the mouse-up never reaches this window's dispatch**, or **the marker is set again after the clear**, or **the sidebar is not repainted after the clear**.

Candidates to test, in order of likelihood: (1) the drag ends without a mouse-up in this window — the pointer crosses into the other Runner window (⇧⌘N) or the app deactivates mid-drag (⌘-Tab, a notification, the titlebar `window_control_area` starting a native window move at `app_shell.rs:596`); `cx.active_drag` is app-global, the sidebar state is per window; (2) a `MouseMove` with `active_drag` still set dispatched after the clear — check GPUI's synthesized hover-update move after a redraw, and the `on_drag` constructor re-running; (3) `clear_sidebar_drag` running on a different window's sidebar than the one that set the marker; (4) a repaint that never happens because the notify lands on an entity GPUI does not consider dirty for that window.

## Work

1. **Instrument first.** `tracing::debug!` (the app already has a file log; `target: "sidebar::drag"`) at every set and clear site, with the marker, `dragged_id`, the window id, and which path fired (`drop`, `root-up`, `root-up-out`, `dismiss`, `commit-early-return`). Hand Jason the branch with the exact reproduction steps to try (plain drag within PINNED and release on a row; release on a section header; release in the gap below the list; release outside the sidebar; release outside the window; drag across to a second window; ⌘-Tab mid-drag) and read the log he sends back. Name the missed path in the handoff.
2. **Fix the missed path**, and add the belt-and-braces rule so the line can never outlive the drag whatever path is missed: the sidebar's render derives indicator visibility from GPUI's truth — if `drop_marker.is_some() && !cx.has_active_drag()`, clear the three fields before rendering (no notify needed, it is already rendering). Keep the event-driven clears; the render check is the backstop, not the fix.
3. If the node did not move in Jason's failing case, trace that too: `commit_sidebar_drop`'s early returns (`dragged_id` mismatch, `drop_target` None) and the `node_reorder_pinned` / `node_move` error path, which today shows a toast only through `report_error`.

## Tests

`sidebar_logic.rs` has the pure drop-target tests; add a unit test for the render-time rule (a pure `indicator_visible(drop_marker, has_active_drag)` or equivalent) and, if the missed path is in the pure logic, a test that pins it. Existing tests rewritten, not deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- Jason's smoke test on the branch: every release case above leaves no line; a real move lands where the line pointed; the log shows the clear path each time.
- Handoff: the missed path named with the log evidence, the fix, the backstop, and whether the move itself was broken.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) a fix asserted from reasoning alone — the handoff must cite Jason's log for the failing path; (2) only the backstop added, the actual missed path left unexplained; (3) the render-time clear calling `cx.notify()` or mutating through `Entity::update` from inside render; (4) `on_drag_move` filtering weakened so a move outside a row's bounds sets a marker; (5) the instrumentation left at `info` level or logging on every mouse move; (6) tests deleted instead of rewritten.

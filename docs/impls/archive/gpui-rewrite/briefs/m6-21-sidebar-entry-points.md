# Mission brief — M6.21: sidebar entry points

Drafted 2026-08-23, after the tree swap. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), explicit `cwd` the `runner-gpui` worktree (now on `main`), title `M6.21 — Sidebar entry points`, the text below as `goal_override`. Jason's call: this lands before the `v0.6.0` tag — the one exception to plan decision 14 — because it is the first thing a new user sees.

---

Implement **M6.21** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.21 — read it first; it carries every measurement and token from the Pencil design; this brief also lives at `docs/impls/gpui-rewrite/briefs/m6-21-sidebar-entry-points.md`). **Branch off `main`** — after the 2026-08-23 tree swap `main` is the native app and `gpui-nightly` is retired; one feature branch in this worktree; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. One crate: `runner-app` (the binary; `runner-backend` is the core). Files: `surfaces/sidebar.rs`, `surfaces/sidebar_logic.rs` if the visible-row walk changes, the sidebar tests. Standing GPUI rules in `impl_log.md` apply — never `cx.new` a stateful child in render, notify scope is rebuild cost, `current_view()` only while rendering, `svg()` needs its own `.text_color()`.

## Design (settled on the canvas 2026-08-23 — `design/runner.pen` frame `Spec — Sidebar entry points (M6.21)`, component `cmp/SidebarNewChat`)

1. **New chat as the first WORKSPACE row**, above `runner` and `crew`. Same geometry as the nav rows (216 × 28, `[6,10]` padding, radius 4): `message-square-plus` 12 px in the accent colour, label "New chat" Inter 14/600 in the muted text ink (`$text-mid`, i.e. the colour of the unselected `crew` row), a `⌘N` hint 11/500 in the low ink at the right edge. No fill at rest, the hover fill of the nav rows on hover, **never the selected treatment** — it is an action, not a page. Click = the existing `NewChat(None)` action (`SidebarMenuAction::NewChat`, `sidebar.rs:158`), i.e. the Start Chat modal that ⌘N opens (`keymap.rs:98`). The RECENT header "+" (`:1854`) stays.
2. **"+" on project rows** (`render_project`, `sidebar.rs:2261`): a 12 px `plus` icon 8 px before the 14 px kebab, both with the kebab's hover/focus reveal, `$text-hi` on the hovered row. Click opens a two-item menu anchored under the row: **"New chat"** → `NewChat(Some(project.id))`, **"New mission"** → `NewMission(Some(project.id))` — the same actions the kebab builds at `:975-979`; the "+" menu uses the short labels, the kebab keeps "New chat in project" / "New mission in project". Collapsed and expanded projects behave the same.
3. **Trim the row menus**: remove "Open in New Window" (`:880`, `:947`) and "Remove from project" (`:891`, `:953`) from both the tab-row and mission-row menus, leaving Pin / Rename tab / Archive. Before deleting the `OpenInNewWindow` and remove-from-project `SidebarMenuAction` arms, check every other dispatcher (command palette, drag-to-RECENT, keymap) — the capabilities must survive by those paths: ⇧⌘N + a sidebar pick still opens a session in another window, dragging a row to RECENT still moves it out of a project. Keep the backend ops. Update the menu tests to the new item lists.

## Tests

The sidebar already has logic/layout tests (`sidebar_logic.rs`, the layout test from M6.10): add (a) the WORKSPACE row order with New chat first and its non-selectable state; (b) the project "+" menu's items and targets; (c) the two row menus' exact item lists. Existing tests that enumerate the old menus are rewritten, not deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- `make run` on the dev profile: New chat is the first WORKSPACE row, hover/click works, ⌘N still works; a project row shows "+" and "…" on hover and the "+" menu starts a chat and a mission inside that project; tab and mission row menus show exactly Pin / Rename tab / Archive; drag-to-RECENT and ⇧⌘N still work. Quit the dev app when done — one instance rule with the human's nightly.
- Handoff: the three changes as landed, the dispatchers checked before removing the action arms, and any divergence from the canvas measurements with the reason.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) the New chat row taking the selected treatment, being treated as a route, or stealing the nav's keyboard focus order; (2) the "+" visible at rest, misaligned with the kebab, or missing on collapsed projects; (3) a removed menu arm still dispatched somewhere, or drag-to-RECENT / ⇧⌘N broken by deleting an op; (4) "in project" left in the "+" menu labels, or the kebab's labels shortened; (5) `cx.new` in a render path, a per-hover notify of the window root, or an `svg()` without `.text_color()`; (6) the icon colour or the hint colour as literals instead of theme tokens; (7) tests deleted instead of rewritten.

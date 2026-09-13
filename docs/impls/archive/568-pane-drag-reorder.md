# 568 — Drag to reorder panes: a grip on the identity line, four drops by nearest edge

Tracking issue: [#568](https://github.com/yicheng47/runner/issues/568). Spec: [568](../../features/archive/568-pane-drag-reorder.md). Feature, P2. Shipped 2026-09-13 in [#573](https://github.com/yicheng47/runner/pull/573) (mission `01M2C50JAYDGEKHJDTPCJ1QZA4`, codex-crew; one review round — the grip wrapper needed a real min width so long titles yield to it, the pill took the sidebar pill's cap and shadow — then commit `c1d460b`, CI green in 5 min). Design: `design/runner.pen` frame `Spec — Drag to reorder panes (568) · v1` (`ITbDO`). Branch: **`feat/568-pane-drag-reorder` already exists and is checked out** — it carries this brief; work on it, do not create another.

## What ships

Every pane of a split tab, chat or terminal, carries a grip in its identity line: lucide `grip-horizontal`, centered in the free space between `⋯` and the split icon, `faint` at rest, `text` with an open-hand cursor on hover, closed hand while dragging. Only the grip drags; a pill with the pane's glyph and name follows the pointer. Every *other* pane in the tab takes four drops — left, right, up, down — by the nearest edge. The half the dragged pane will occupy is tinted `accent` at 12 % with a 1 px accent inset: the highlight is the preview. A drop is a move: the pane leaves its split as `×` leaves it and the target is split on that side with the pane as the new sibling at 50 / 50. The leaf keeps its id and session, so nothing remounts. The 240 × 160 floor applies per side. No center drop, swap, keyboard, pop-out, or undo; releasing anywhere else changes nothing.

## Where the code is

- `crates/runner-app/src/pane_layout.rs`: `SplitOrientation` `:34`, `PaneLeaf` `:40`, `PaneSplit` `:47`, `PaneNode` `:57`; `highest_node_number` `:82`; `split_leaf` `:91` (inserts an empty leaf as `b`); `PaneLayout::split` `:528`; `close_pane` `:567` collapses through the free fn `remove_pane` `:804`; `serialize` `:612` writes `tree` + `slots` + `drawer`; tests from `:866`.
- `crates/runner-app/src/surfaces/panes.rs`: `render_pane_node` `:1378` — the split container `:1394`–`:1470` is the house drag pattern (`SplitResizeDrag` on the gutter's `on_drag`, `on_drag_move` + `on_drop` on the container). Identity line `:1802`–`:1940`: glyph, name, status, `⋯`, spacer `div().min_w(px(0.)).flex_1()` `:1900`, `split_menu`, `×`; the empty-pane spacer is `:1924`. Each pane root `div` `pane-{id}` `:2280` measures itself into `pane_bounds` keyed by `PaneKey` `:2346`; `split_allowed` `:2414`, `split_decision` `:2436`. Tab body wrapper `:513`.
- `crates/runner-app/src/surfaces/chat.rs:1655` `split_pane`: the persist → reload → attach → focus sequence to reuse verbatim (`persist_active_tab`, `reload_tabs`, `ensure_active_tab_attached`, `remember_active_runner`, `mark_active_tab_viewed`, `focus_active_terminal`).
- `main.rs:136` `SplitResizeDrag` + its 1 × 1 `Render` `:150`; `crews.rs:2516` the slot drag handle for the hover/cursor idiom. `assets.rs` icons are `const` SVG bytes registered by name (`COLUMNS_2` `:72`); `theme.rs` `text()` `:271`, `faint()` `:279`, `accent()` `:283`. Sidebar titles read `layout.session_ids()` (`sidebar.rs:96`), so they re-compose on `reload_tabs` untouched.

## Fix shape

1. **Model** (`pane_layout.rs`). `pub enum DropSide { Left, Right, Up, Down }` with `orientation()` (Left/Right → Row) and `moved_first()` (Left/Up → the moved leaf is `a`). `PaneLayout::move_to(&mut self, pane_id, target_id, side) -> Result<()>`: error if the ids match or either is missing; `next = highest_node_number() + 1` on the current tree; clone the moved `PaneLeaf`; `remove_pane`; generalise `split_leaf` to insert a given node on a given side and split `target_id` with `s{next}`, `[50., 50.]`, the cloned leaf; `focused_pane_id = pane_id`. Pure fns beside `split_allowed`: `drop_side(pane: Bounds<Pixels>, pointer: Point<Pixels>) -> DropSide` — the smallest of the four normalised edge distances, ties to Left then Up; `drop_preview(pane, side) -> Bounds<Pixels>` — the half; `drop_allowed(pane: Option<Size<Pixels>>, side, zoom)` = `split_allowed(pane, side.orientation(), zoom)`.
2. **Grip and pill** (`panes.rs`, `assets.rs`). Add `GRIP_HORIZONTAL` (lucide: six `r=1` circles at x 5 / 12 / 19, y 9 / 15) as `grip-horizontal.svg`. Both spacers become a `flex_1` centering row holding the grip: `svg` 12 px, `theme::faint()`, hover `text()`, `CursorStyle::OpenHand`, `ClosedHand` while this pane is the one dragged, `.on_drag(PaneDrag { tab_id, pane_id, label, icon }, …)` on the grip only. `PaneDrag: Clone + Render` beside `SplitResizeDrag`, rendering the pill (panel bg, border, glyph + name at `text_ui`).
3. **Targets and preview** (`panes.rs`). Root state `pane_drop: Option<(PaneKey, DropSide)>`. On each pane root `:2280`: `on_drag_move::<PaneDrag>` — when the drag is another pane of this tab and `event.bounds` contains the pointer, set `(key, drop_side(bounds, pointer))` if `drop_allowed`, else clear; when this pane held the target and no longer contains the pointer, clear. Preview: an absolute overlay child of the pane root at `drop_preview`'s half, `theme::accent().opacity(0.12)` fill, 1 px accent border, no hitbox. `on_drop::<PaneDrag>` on the pane root applies the target; a fallback `on_drop` on the tab body wrapper `:513` applies it too (GPUI: `on_drag_move` reaches gaps and gutters `on_drop` does not; the deepest listener wins). Clear `pane_drop` when a drag ends without a drop (`cx.has_active_drag()` false at render).
4. **Apply** (`chat.rs`). `move_pane(pane_id, target_id, side, window, cx)`: re-check `drop_allowed` on `pane_bounds`, `layout.move_to`, then the `split_pane` sequence verbatim. Terminal panes take the same path — a move adds no pane, so `terminal_only` is not a gate.
5. **Docs.** `docs/arch/arch.md:201`: one sentence — a pane moves within the tree by drag, keeps its id, split ids continue the counter. `docs/tests/64-terminal-as-pane-option-smoke.md` §4: two checkboxes for the grip and a drop.

## Rules of the road

- No backend change; the layout JSON keeps `tree` + `slots` + `drawer`. No center drop, swap, keyboard, pop-out, undo, or settings.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (also `--features updater`), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `feat(ui): drag to reorder panes — grip on the identity line, four drops (568)`, drive CI green (`gh pr checks <n> --watch`; required check `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `move_to` on a three-pane tree, all four sides: leaf order, the vacated split gone, the new split `[50., 50.]` with the moved leaf on the right side of `a`/`b`, id and session kept, focus on it, split id one past the old counter. Same or unknown ids → error. Two-pane tree, left dropped right of its sibling → reversed.
- `drop_side` in each triangle and on both diagonals; `drop_preview` per side; `drop_allowed` at the floor on both axes and at zoom 1.5.
- Serialize round-trip after a move: `slots` in the new leaf order, `drawer` untouched. `cargo test -p runner-app` green.

## Jason's smoke test (after landing)

1. Three chats in a split: a faint grip on every identity line, brighter with the hand cursor on hover; name, `⋯`, split icon and `×` still do their own thing.
2. Drag a pane: the pill follows; over another pane the highlighted half follows the nearest edge. Drop on each side — the pane lands at 50 / 50, both agents keep running with their scrollback, focus stays on the moved pane, the vacated split collapses. Drag it back: the original order. Quit and relaunch: the order persists, the sidebar title matches.
3. Narrow the window until a side would break the floor: no highlight there, dropping does nothing.
4. Release over the dragged pane, the drawer, or outside the tab body: nothing changes.
5. A terminal-only tab with two terminals reorders the same way.

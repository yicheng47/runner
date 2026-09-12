# Drag to reorder panes

Tracking issue: [#568](https://github.com/yicheng47/runner/issues/568). Status: spec, 2026-09-12; Pencil-first, code waits for the frame sign-off. Priority P2.

## Motivation

Since [570](./archive/570-split-panes-redesign.md) a tab's layout is a free tree with any number of panes, but pane order is still whatever order the panes were opened in. Putting the reviewer on the left of the implementer means closing a pane and reopening the chat in the right place, and with [567](./archive/567-pane-close-archives-chat.md) even that workaround archives the chat first. Every split UI lets you move a pane.

Ghostty 1.3 is the reference. Hover near the top of a split and a grab handle appears, a small bar of dots; drag it and drop the split into any other split position, and its contents and running process move with it. The maintainers tried an always-visible bar first, users found it confusing, and it became a handle that shows on hover ([discussion 10553](https://github.com/ghostty-org/ghostty/discussions/10553)). Runner already has the place for the handle: the identity line that sits on every pane in a split, with free space in its middle.

## Behavior

### The handle

- **A grip in the identity line's free space.** A lucide `grip-horizontal` glyph, centered between `⋯` and the split icon, on every pane of a split tab, chat or terminal. At rest it is drawn in `faint`, the dimmest text tone, so it reads as texture rather than a control; on hover it brightens to `text` and the cursor becomes an open hand. Not hover-only: the identity line already carries resting chrome, and a handle that appears from nowhere is what confused Ghostty's users.
- **Only the grip drags.** The name still double-clicks to rename, the icons still press. Pressing the grip and moving starts the drag; the cursor becomes a closed hand and a small pill with the pane's icon and name follows the pointer.
- Single-pane tabs have no identity line and nothing to reorder, so no grip.

### Drops

While a pane is being dragged, every *other* pane in the tab is a target with four drops: left, right, up, down. Which one the pointer means is the nearest edge, so the pane is cut into four triangles by its diagonals. There is no center drop and no swap; a swap is two drags.

- **The highlight is the preview.** The half of the target pane the dragged pane will occupy is tinted with `accent` at 12 % and outlined with a 1 px accent inset, the way Zed and VS Code show a dock target. Nothing else changes until the drop.
- **A drop is a move.** The dragged pane leaves its split, its former sibling takes the space exactly as `×` leaves it, and the target pane is split on that side with the dragged pane as the new sibling at 50 / 50. The rest of the tree keeps its sizes. A pane's leaf moves in the tree with its id and the pane element is keyed by that id, so neither terminal remounts.
- **The size floor applies.** A side that would leave either pane under 240 × 160 px at 1× zoom shows no preview and does not accept the drop.
- **Over itself, the drawer, or outside the tab body**: no preview, and releasing there changes nothing.
- After a drop the dragged pane keeps focus, the layout persists through the tree writer, and the sidebar row's title re-composes from the new pane order.

### What does not change

- Resize by gutter, `⌘[` / `⌘]` focus cycling, the split icon and shortcuts, `×`, `⋯`.
- Sessions, drawers, the mission workspace.
- The sidebar's own drag-to-reorder of tabs.

## Non-goals

- Dragging a pane out to a new tab, or a sidebar chat into a pane. Both are the pop-out and pull-in that #568 also names; they need a drop target outside the tab body and are the next spec once this lands, on the same handle.
- Fork-to-split-pane. Spec 60 dropped it and this does not bring it back.
- Keyboard reordering, and an undo. Ghostty has both; Runner's undo is dragging it back.
- Multi-select or dragging a split subtree. One pane at a time.

## Design

`design/runner.pen`: one frame, `Spec — Drag to reorder panes (568) · v1`, built from the 570 frame's two-pane surface. It shows the identity line at rest and on hover with the grip, and two drags in flight with the pill under the pointer and the destination half of the target lit, one to the left and one to the top, and the rule as notes.

## Implementation Phases

1. **Design.** The frame above. Stop for sign-off.
2. **Model.** `PaneLayout::move_to(pane, target, side)` removes the leaf through the `×` collapse path and splits the target on that side with the moved leaf as `b` (or `a` for left and top); ids survive, so nothing remounts. `drop_side(pane_bounds, pointer) -> Side` (nearest edge by the diagonals), `drop_preview(pane_bounds, side) -> Bounds`, and the floor check as pure functions. Tests on a three-pane tree for every side, ids and sizes preserved, and the floor.
3. **Drag.** A `PaneDrag { tab_id, pane_id }` value on the grip's `on_drag`, the pill as the drag view; `on_drag_move` on each pane computes the side from the pointer and stores it on the root; the preview draws from that state; `on_drop` on the pane applies it, with a fallback `on_drop` on the tab body for the pixels a pane's drop hitbox does not cover (the GPUI gotcha the sidebar already handles). `grip-horizontal.svg` joins `assets.rs`; open and closed hand cursors on hover and drag.
4. **Docs.** The 64 smoke test's chrome section; `docs/arch` where it describes pane order.

## Verification

- Three chats in a split: the grip is faint on every identity line, brightens on hover with the hand cursor, and dragging shows the pill.
- Drag over another pane: the preview is the half where the pane will land and follows the nearest edge as the pointer moves. Drop on each side: the pane lands there at 50 / 50, both agents keep running, focus stays with the dragged pane, and the layout it left collapses. Drag it back: the original order. Quit and relaunch: the new order persists.
- Narrow the window until a side would break the floor: no preview on that side and the drop does nothing.
- Release over the dragged pane, over the drawer, or outside the tab: nothing changes.
- Name, `⋯`, split icon, and `×` on the identity line all still work; a terminal-only tab with two terminals reorders the same way.
- `cargo test -p runner-app` green.

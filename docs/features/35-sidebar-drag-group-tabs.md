# 35 — Sidebar drag to group and disband tabs

> Tracking issue: [#256](https://github.com/yicheng47/runner/issues/256).

## Motivation

Feature 38 replaced the impl 0023 accordion with a durable **Folder → Tab** hierarchy: every tab is one sidebar row, panes never appear in the sidebar, and both folders and tabs persist in SQLite. But you can only form or reshape a tab from the chat surface. There is no direct-manipulation way to say "put these two tabs together," move a tab into a folder, or pull one pane out as its own tab.

The natural gesture is drag-and-drop in the sidebar:

- **Group** — drag one tab row onto another to combine their sessions into one multi-pane tab.
- **Disband** — drag a pane from the chat surface back to the sidebar to create a single-pane tab.
- **Organize** — drag tab rows into and out of folders, replacing feature 38's context-menu-only move.

This is the sidebar-side complement to the chat-surface layout picker: same frontend tab model, reached by direct manipulation on the list you're already looking at. (Scope is the **CHAT list** — direct-chat tabs. Missions view runner sessions only and have no sidebar chat tabs; see feature 19.)

## Scope

### In scope (v1)
- **Tab onto tab** → combine their sessions into one persisted tab, up to the 3-pane cap.
- **Tab into/out of folder** → update the persisted `folder_id`; dropping outside folders produces an ungrouped tab.
- **Pane out to sidebar** → remove it from the source layout and create a persisted single-pane tab; the chat keeps running.
- **Drop feedback** — a "merge" highlight on the target row/group when the drop would group; reuse feature 23's drop-between accent line if reordering is combined.
- **DB write-through.** Every operation mutates `paneLayout` and writes the affected `tabs` row or folder membership through the feature 38 commands. Cross-window invalidation rehydrates from SQLite.

### Out of scope (deferred)
- **Row-order persistence via `sort_index`** — that's feature 23 (chat/mission list order). This spec reshapes *tabs*, not the backend sort; the two share a DnD library and the sidebar surface, not the persistence path.
- **Mission-workspace drag-to-split** — feature 19 / a mission-center follow-up.
- **Cross-window drag** — dragging a chat into another window's tab is multi-window territory (feature 12).
- **>3 panes per tab** — the preset model caps at 3; grouping honors it.

### Key decisions
1. **`@dnd-kit` over hand-rolled HTML5 DnD** — same rationale as feature 23 (keyboard, autoscroll, a11y). If 23 lands first, reuse its `DndContext`; otherwise this spec adds the dep. Tab rows and folder rows become the draggable and droppable units.
2. **Grouping vs reordering is disambiguated by drop target, not a mode** — dropping *between* rows reorders (feature 23); dropping *onto* a row/group merges (this spec). A center-vs-edge hit test on the target, like feature 19's original drop zones, keeps it one gesture.
3. **Use the pane-layout store and its DB write-through path** — grouping is still a `paneLayout` reshape via the existing pure helpers, but stable tab identity, order, and folder membership must remain durable.
4. **Respect the invariants the tab model already enforces** — move-not-copy (a chat lives in exactly one pane), ≤3 panes, and pin-as-a-unit (group pinning, #250). Disbanding or adding a member interacts with pin inheritance (`shouldInheritPinOnAdd`); resolve the exact rule at design time.

## Open design questions
- Dropping a loose chat onto a **collapsed** group — auto-expand it, or add silently and bump the count?
- Grouping two chats that live in **different existing tabs** — move the dragged one out of its old tab (lean: yes, move-not-copy), or refuse?
- Where does a newly formed tab land in sidebar order — at the dropped-on chat's slot, or float per pin rules?

## Design first
Per the design-first workflow, mock the drag affordances — merge-onto-row highlight, folder drop target, disband-drag ghost, cap-reject shake — in `design/runner-mvp-design.pen` against the feature 38 Folder → Tab frames before coding.

## Verification (sketch)
- [ ] Drag loose chat A onto loose chat B → a 2-pane tab appears with both as members; opening it shows the split on the chat surface.
- [ ] Drag a third chat onto that group → 3-pane tab; a fourth is rejected with feedback, not dropped.
- [ ] Drag a member out → it leaves the group and becomes a loose row; the group reflows, or dissolves to a leaf at 1 member; the chat keeps running.
- [ ] Move a member from tab X to tab Y → removed from X, added to Y, no duplicate anywhere.
- [ ] Every grouping or folder move survives restart and converges in another window through DB hydration.
- [ ] `pnpm exec tsc --noEmit` + `pnpm run lint` clean.

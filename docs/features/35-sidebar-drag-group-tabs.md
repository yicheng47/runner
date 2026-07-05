# 35 — Sidebar drag to group and disband tabs

> Tracking issue: [#256](https://github.com/yicheng47/runner/issues/256).

## Motivation

Impl 0023 shipped the **tab accordion**: a multi-pane chat tab now reads as one collapsible group in the sidebar CHAT list, and single chats stay flat leaf rows. But you can only *form* or *reshape* a tab from the chat surface — the layout picker splits the open chat into panes, and a sidebar pick fills an empty pane. There is no direct-manipulation way to say "put these two chats together" or "pull this one out" from the sidebar itself, which is the very list where you see all your chats.

The natural gesture is drag-and-drop in the sidebar:

- **Group** — drag a loose chat onto another loose chat (or onto a tab group) to bind them into one tab / add it as a pane.
- **Disband** — drag a member out of a group to pull it back into its own loose chat; drop the second-to-last member out and the group dissolves to a single leaf.
- **Reshuffle** — drag a member from one tab into another; move-not-copy.

This is the sidebar-side complement to the chat-surface layout picker: same frontend tab model, reached by direct manipulation on the list you're already looking at. (Scope is the **CHAT list** — direct-chat tabs. Missions view runner sessions only and have no sidebar chat tabs; see feature 19.)

## Scope

### In scope (v1)
- **Loose chat onto loose chat** → form a 2-pane tab (`cols-2` preset), both as members, the dropped-on chat in slot 0.
- **Loose chat onto a group** → add it as a new pane in that tab, up to the 3-pane cap. At the cap, reject with a subtle shake + tooltip — never a silent drop.
- **Member out of a group** onto the loose area → remove it from the tab (`closePane` semantics: the pane collapses, the chat keeps running and becomes a loose row). A 2-member group dropping to 1 dissolves to a leaf.
- **Member between groups** → move-not-copy across tabs (removed from the source tab, added to the target).
- **Drop feedback** — a "merge" highlight on the target row/group when the drop would group; reuse feature 23's drop-between accent line if reordering is combined.
- **Frontend-only.** Every operation mutates the `paneLayout` store (`applyPreset` / `assignSessionToPane` / `closePane` / `removeSessionFromLayout` / the tab set) — tabs are frontend display grouping (arch §3.6), so no backend, schema, or Rust changes.

### Out of scope (deferred)
- **Row-order persistence via `sort_index`** — that's feature 23 (chat/mission list order). This spec reshapes *tabs*, not the backend sort; the two share a DnD library and the sidebar surface, not the persistence path.
- **Mission-workspace drag-to-split** — feature 19 / a mission-center follow-up.
- **Cross-window drag** — dragging a chat into another window's tab is multi-window territory (feature 12).
- **>3 panes per tab** — the preset model caps at 3; grouping honors it.
- **Folder drop targets** — feature 17.

### Key decisions
1. **`@dnd-kit` over hand-rolled HTML5 DnD** — same rationale as feature 23 (keyboard, autoscroll, a11y). If 23 lands first, reuse its `DndContext`; otherwise this spec adds the dep. The accordion rows and group headers become the draggable + droppable units.
2. **Grouping vs reordering is disambiguated by drop target, not a mode** — dropping *between* rows reorders (feature 23); dropping *onto* a row/group merges (this spec). A center-vs-edge hit test on the target, like feature 19's original drop zones, keeps it one gesture.
3. **Operate on the pane-layout store, never the backend** — grouping is a `paneLayout` reshape via the existing pure helpers. Add one thin "form a tab from two loose chats" action if `applyPreset` doesn't cover the loose→loose case cleanly.
4. **Respect the invariants the tab model already enforces** — move-not-copy (a chat lives in exactly one pane), ≤3 panes, and pin-as-a-unit (group pinning, #250). Disbanding or adding a member interacts with pin inheritance (`shouldInheritPinOnAdd`); resolve the exact rule at design time.

## Open design questions
- Dropping a loose chat onto a **collapsed** group — auto-expand it, or add silently and bump the count?
- Grouping two chats that live in **different existing tabs** — move the dragged one out of its old tab (lean: yes, move-not-copy), or refuse?
- Where does a newly formed tab land in sidebar order — at the dropped-on chat's slot, or float per pin rules?

## Design first
Per the design-first workflow, mock the drag affordances — merge-onto-row highlight, disband-drag ghost, cap-reject shake — in `design/runners-design.pen` against the shipped accordion frames (`y6LaRZ`) before coding.

## Verification (sketch)
- [ ] Drag loose chat A onto loose chat B → a 2-pane tab appears with both as members; opening it shows the split on the chat surface.
- [ ] Drag a third chat onto that group → 3-pane tab; a fourth is rejected with feedback, not dropped.
- [ ] Drag a member out → it leaves the group and becomes a loose row; the group reflows, or dissolves to a leaf at 1 member; the chat keeps running.
- [ ] Move a member from tab X to tab Y → removed from X, added to Y, no duplicate anywhere.
- [ ] No backend/Tauri calls fire for any grouping op (frontend-only).
- [ ] `pnpm exec tsc --noEmit` + `pnpm run lint` clean.

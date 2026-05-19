# 19 — Mission split view

> Tracking issue: [#166](https://github.com/yicheng47/runner/issues/166)

## Motivation

The mission workspace today is a one-pane-at-a-time view: the user picks
between **feed** and one of the per-runner PTY tabs, and `activeTab`
holds a single subject (`"feed" | sessionId`). For an editor where the
job is "watch the lead and the worker coordinate," that's a constant
flip — Cmd-1 to the lead, Cmd-2 to the worker, back to the feed, repeat.
You can never *see* the handoff happen.

Multi-window (spec 12) doesn't fix this. Two windows on the same mission
trigger the duplicate-subject overlay and the secondary window won't
mount the PTY — by design, so stdin stays single-writer. The only path
to a true side-by-side view of two PTYs in the same mission is inside a
single window: split the center area, give each split its own active
leaf, render both terminals simultaneously.

Concretely the request is:

1. **Multi-pane support** — the center area can hold more than one
   tabbed pane, each with its own active leaf.
2. **Drag-tab-to-split** — drag a tab onto the edge of the existing
   pane to spawn a new pane and dock the dragged tab there.

This is the VSCode editor-group / iTerm split-pane model, scoped to the
mission center area.

## Scope

### In scope (v1)

- **Pane tree in the mission center.** Replace the single `activeTab`
  state with a layout tree: a binary tree whose leaves are *panes*
  and whose interior nodes are *splits* with an `orientation`
  (`"row" | "col"`) and a `sizes: [number, number]` (percentages). Each
  pane carries its own `openTabs: string[]` and `activeTab: "feed" |
  sessionId`.
- **One feed, one pane.** Exactly one pane in the tree owns the **feed**
  tab at any time. Dragging the feed tab to another pane moves it
  rather than duplicating. Two PTY tabs for the same session likewise
  cannot exist in two panes simultaneously — moving a tab is a move,
  not a copy. (See key decision 1 for why this matters for xterm.)
- **Drag-tab-to-edge.** Tab drag uses HTML5 drag-and-drop. While
  dragging, the target pane shows four drop zones (left / right / top /
  bottom edges, ~30% inset) plus a center "merge into this pane" zone.
  Dropping on an edge creates a new sibling pane in that direction,
  inheriting a 50/50 split. Dropping in the center moves the tab into
  that pane's tab strip.
- **Resize between panes.** A draggable gutter sits between siblings;
  drag updates `sizes` on the parent split node. Use
  `react-resizable-panels` (new dep) — it's the lightweight standard
  and matches the spec exactly.
- **Close pane → reflow.** Closing the last tab in a pane removes the
  pane and collapses the split: the sibling pane takes the full space
  by replacing the parent split node in the tree. The feed tab can't
  be closed (current behavior preserved) so the pane that owns the
  feed survives until you drag the feed elsewhere.
- **Per-pane focus ring.** One pane is "focused" at a time, indicated
  by a 1px accent border on the active tab strip and used for
  keyboard-shortcut targeting (e.g. `Cmd+W` closes the active tab in
  the focused pane).
- **Layout is per-mission, in-memory.** Lives next to `openTabs` in the
  `MissionWorkspace` component. Reset on mission switch. Default layout
  on mount: single pane, same as today — split view is opt-in via the
  drag interaction.

### Out of scope (deferred)

- **Persistence across app restart.** Save the layout tree alongside
  the mission session resume work (spec 10). Real follow-up, but layout
  persistence is meaningless until session persistence is solid; until
  then a restart-reset is the honest default.
- **3+ panes via the tab strip.** The binary tree supports arbitrary
  nesting (split a pane that's already a child of a split), so 3+
  panes work mechanically. The drop zones only expose the *first*
  split per pane in v1; deeper splits arrive naturally with no extra
  code. Don't ship a "split into 3" affordance.
- **Cross-mission split.** Showing mission A's lead next to mission B's
  worker. That's the multi-window job (spec 12). One mission per
  workspace; this spec doesn't change that.
- **Direct-chat split.** The request scopes to missions. RunnerChat
  stays single-pane in v1.
- **Synchronized input across panes.** No "type once, send to both."
  Each pane's terminal is its own stdin.
- **Saved layout presets.** "Lead + worker" / "feed + lead" templates.
  Out — let users drag.
- **Drag the *pane* itself.** Only tabs are draggable in v1. You can't
  pick up a whole pane and move it across the tree. (The same
  rearrangement is reachable by dragging the tabs.)

### Key decisions

1. **A subject lives in exactly one pane.** xterm holds DOM state and a
   live PTY stdin pipe. Mounting two `RunnerTerminal` instances against
   the same session would race on writes and double-feed the
   scrollback. Move-not-copy keeps `mission_attach` idempotent and the
   "one xterm per session" invariant intact.
2. **Layout state is local to `MissionWorkspace`, not Redux/context.**
   It's per-mission ephemeral UI state. Lifting it higher buys nothing
   and forces every consumer to reason about pane trees they don't
   care about.
3. **Add `react-resizable-panels`, not a hand-rolled splitter.** Drag
   resize + min/max + collapse semantics + keyboard support are easy to
   get wrong. The lib is ~5kb, no peer deps, and is the de-facto React
   choice. Avoid `allotment` (heavier, FlexLayout-style API) and
   `react-mosaic-component` (more than we need).
4. **HTML5 drag-and-drop, no `dnd-kit`.** Tab dragging is a single
   draggable type with a small set of drop targets; native DnD handles
   it. Adding a DnD library for one interaction isn't worth the bundle.
5. **The pane tree is the source of truth; `openTabs` / `activeTab`
   live on pane leaves.** A flat "list of tabs + which is active" model
   can't express "the same tab appears in two panes" — which is exactly
   the invariant we want to enforce, not work around (decision 1).
6. **No `display:none` swap across panes.** Today inactive *tabs* are
   `display:none` so xterm scrollback survives switches. With splits,
   a tab that *moved* from pane A to pane B must unmount in A and
   remount in B. xterm scrollback is held in the React component
   instance, so a remount means scrollback resets — but the backend
   terminal snapshot (`session_snapshot`) replays bytes on attach, so
   the visible scrollback is restored. Tradeoff is acceptable; the
   alternative (portal the xterm DOM node across panes) is brittle.

## Implementation phases

### Phase 1 — pane-tree data model

- New file `src/lib/paneTree.ts`:
  ```ts
  export type PaneId = string;
  export type LeafTab = "feed" | { sessionId: string };
  export interface PaneLeaf {
    kind: "leaf";
    id: PaneId;
    openTabs: string[];   // sessionIds; feed-ness tracked separately
    hasFeed: boolean;
    activeTab: "feed" | string;
  }
  export interface PaneSplit {
    kind: "split";
    orientation: "row" | "col";
    sizes: [number, number];
    a: PaneNode;
    b: PaneNode;
  }
  export type PaneNode = PaneLeaf | PaneSplit;

  export const singlePane = (
    openTabs: string[], activeTab: "feed" | string,
  ): PaneLeaf => ({ kind: "leaf", id: ulid(), openTabs, hasFeed: true, activeTab });
  ```
- Pure helpers (no React): `findPaneOfTab`, `moveTab(tree, tabId, target,
  edge | "center")`, `splitPane(tree, paneId, edge, newTab)`,
  `removeTabFromPane(tree, paneId, tabId)` (auto-collapses empty panes
  by promoting the sibling), `setActiveTab(tree, paneId, tabId)`,
  `setFocusedPane(tree, paneId)`.
- Unit tests for each: split + collapse round-trips, move-tab between
  panes, "can't have two feeds" invariant violation throws, sibling
  promotion preserves split orientation correctly.

### Phase 2 — render the tree

- `MissionWorkspace` swaps `activeTab`/`openTabs` state for a single
  `paneTree: PaneNode` (plus a `focusedPaneId: PaneId`).
- New `<PaneTreeView>` component recursively renders the tree:
  - `PaneSplit` → `<PanelGroup direction={orientation}>` from
    `react-resizable-panels`, two `<Panel>` children, an interior
    `<PanelResizeHandle>`.
  - `PaneLeaf` → existing tab strip + pane area, but driven by the
    leaf's own `openTabs` / `activeTab`. The tab strip's "close ×" and
    click handlers dispatch into pane-tree helpers via callbacks.
- All `RunnerTerminal` instances stay mounted for tabs that exist *in
  their pane's `openTabs`*. Inactive tabs within a leaf still use
  `display:none`. Tabs in *other* panes don't mount here at all.
- The feed pane finds the leaf with `hasFeed: true` and renders
  `EventFeed` + `MissionInput` there.

### Phase 3 — drag-and-drop

- `PtyTabButton` (and the feed tab) gain `draggable` + `onDragStart`
  carrying `{ paneId, tab }` on `dataTransfer` (custom MIME
  `application/x-runner-tab+json`).
- `<PaneLeaf>` body listens for `dragover` and, while a runner-tab
  drag is in flight, renders four edge zones and a center zone (light
  accent overlay, ~30% inset on edges, center is the remainder). Each
  zone is its own drop target.
- On `drop`:
  - Edge → `splitPane(tree, paneId, edge, draggedTab)` and (if
    dragged from another pane) `removeTabFromPane(...)` on the source.
  - Center → `moveTab(tree, draggedTab, paneId, "center")`.
- Drag-to-same-pane-edge is a no-op (already there); drag-to-same-pane-
  center is a no-op.
- Esc cancels (native).

### Phase 4 — focus + keyboard

- `focusedPaneId` tracks the last clicked pane. Click anywhere inside a
  `PaneLeaf` (tab strip or content) sets focus.
- Tab strip of the focused pane gets a `border-accent` underline; the
  others get `border-transparent`. (Mirrors the spec-12 overlay style.)
- `Cmd+W` (mac) / `Ctrl+W` closes the active tab in the focused pane,
  reusing `onCloseTab` semantics (snap to feed if it was active;
  collapse pane if its `openTabs` empties and it doesn't hold the
  feed).
- No new chord beyond `Cmd+W` in v1; everything else is mouse-driven
  via the drag affordance.

### Phase 5 — verification + smoke

- `paneTree.ts` unit tests (Phase 1) clean.
- Manual smoke:
  1. Open a mission with ≥2 slots. Drag the worker tab to the right
     edge of the pane. Verify: a new pane spawns on the right with
     just the worker tab active; the original pane keeps the feed +
     lead tab; resize gutter works.
  2. Drag the worker tab back to the center of the left pane. Verify:
     the right pane disappears, the left pane gets the worker tab
     back; both terminals still alive (no PTY crash, no double input).
  3. Type into the lead's terminal. Verify: input lands in the lead
     only — not echoed in the worker pane.
  4. Drag the **feed** tab to the bottom edge. Verify: feed moves to
     a new bottom pane; lead+worker stay in the top pane; the feed
     tab is no longer in the top tab strip.
  5. Close every PTY tab in the top pane. Verify: the top pane
     collapses, the feed pane takes the full mission area.
  6. Resume mission after a slot crashes. Verify: panes survive the
     resume; reopened tabs land back in their original panes.
  7. Switch to a different mission and back. Verify: layout resets to
     the single-pane default (no persistence in v1).
  8. Multi-window cross-check: open mission X in window A (split into
     two panes), then mission X in window B. Window B shows the
     duplicate-subject overlay (spec 12) — no PTY mounts there, and
     window A's panes keep working untouched.

## Verification

- [ ] Drag a tab to a pane edge → new split pane spawned with the tab
      docked.
- [ ] Drag a tab into the center of another pane → tab moved to that
      pane's tab strip; original pane no longer holds it.
- [ ] Resize gutter drag updates the split ratio; min/max prevent
      either pane from collapsing to 0.
- [ ] Two panes render two PTYs simultaneously; stdin in pane A does
      not leak into pane B's terminal.
- [ ] Closing the last tab in a non-feed pane collapses it; sibling
      pane reflows to fill.
- [ ] Feed tab is movable across panes but never duplicated.
- [ ] `Cmd+W` closes the active tab in the focused pane.
- [ ] Multi-window: opening the same mission in a second window still
      triggers the spec-12 overlay; no PTY mounts in the secondary
      regardless of pane layout in the primary.
- [ ] `pnpm exec tsc --noEmit`, `pnpm lint`, and `cargo test --workspace`
      clean.
- [ ] Layout persistence across app restart is *not* claimed (deferred).

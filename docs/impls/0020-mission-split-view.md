# Mission Split View (layout-picker)

## Status

In progress for issue [#166](https://github.com/yicheng47/runner/issues/166). Implements the intent of feature spec [docs/features/19-mission-split-view.md](../features/19-mission-split-view.md) but **pivots the interaction model** from that spec's drag-tab-to-edge to a TradingView-style layout picker (see "Pivot" below).

> **Update (design session):** direction narrowed to **direct-chat split first** — split the direct-chat surface (`RunnerChat`) into multiple chats side by side; the **mission workspace is unchanged** and mission split (#166) is deferred. The interaction (layout-picker popup, per-pane header + focus ring, resize gutter) was mocked in `design/runners-design.pen` (`Layout picker popup`, `Runner direct chat — 2-pane split`) along with a sidebar-alignment pass. This doc still describes the mission-split mechanics and will be rewritten for the direct-chat scope before implementation.

## Problem

The mission workspace is one-pane-at-a-time: `activeTab: "feed" | sessionId` holds a single subject, so watching a lead↔worker handoff means constantly flipping ⌘1/⌘2/feed — you can never *see* the handoff happen. Multi-window (impl 0018) doesn't help: a second window on the same mission hits the duplicate-subject overlay and won't mount the PTY (single-writer stdin, by design). The only way to see two of a mission's PTYs side by side is to split the center area inside one window.

## Pivot from spec 19 (drag-tab-to-edge → layout picker)

Spec 19 proposed VSCode/iTerm-style **drag-tab-to-edge** with four edge drop-zones per pane plus HTML5 DnD. We're replacing that with a **TradingView-style layout picker**: a small popup showing a grid of preset layouts; click one to apply it. Why:

- **Discoverable** — a visible "layout" button + a grid of options beats "somehow know to drag a tab to a pane edge."
- **Simpler to build** — no HTML5 DnD, no drop-zone hit-testing, no custom drag MIME. Phase 3 of spec 19 disappears.
- **Fits missions** — a mission has a handful of subjects (feed + lead + a worker or two). A few presets cover essentially every real arrangement; freeform drag-splitting is overkill for this surface.

**Cap at 2–3 panes.** Curated preset set: **1** (default) · **2** side-by-side · **2** stacked · **3** as 1-big+2-stacked · **3** columns · **3** rows. (The pane tree supports deeper nesting mechanically, but we only expose these presets.)

## Key Decisions

1. **Content model A — shared tab strip + focused pane (move-not-copy).** Keep today's single top tab strip (feed · lead · worker…). Add a **focus ring** on the active pane. **Clicking a subject loads it into the focused pane**, removing it from whatever pane previously held it. This reuses the existing tab strip with almost no new per-pane chrome, and keeps the xterm/stdin invariant (a subject is mounted in exactly one pane). *(Considered model B — a per-pane header dropdown, more literal to TradingView — but it's more per-pane UI and a bigger departure. Revisit only if the Pencil mockup argues for it.)*
2. **A subject lives in exactly one pane.** xterm holds DOM state + a live stdin pipe; two `RunnerTerminal`s on one session would race writes and double-feed scrollback. Move-not-copy keeps `mission_attach` idempotent and "one xterm per session" intact. (Same as spec 19 decision 1.)
3. **Each pane leaf holds ONE subject**, not an `openTabs` array. With the shared tab strip owning subject selection, per-pane multi-tab strips (spec 19's model) aren't needed — this materially simplifies the tree.
4. **`react-resizable-panels` for the gutters** (new dep, ~5kb, no peer deps). Presets construct fixed trees; the lib handles resize/min-max/keyboard. (Same as spec 19 decision 3.)
5. **Layout state is local to `MissionWorkspace`, in-memory, per mission**, reset on switch. Default on mount: single pane = today's behavior. (Same as spec 19 decision 2.)
6. **Moving a subject remounts its terminal.** Across panes there's no `display:none` swap — a subject that moves unmounts and remounts, so xterm scrollback resets, but the backend `session_output_snapshot` replays bytes on re-attach so visible scrollback is restored. (Same as spec 19 decision 6.) *Within* a leaf there's only ever one subject, so there's no intra-leaf hidden-tab case.

## Goals

- A "layout" affordance in the mission workspace opens a preset picker; choosing a multi-pane preset splits the center area with resizable gutters.
- Two (or three) of a mission's subjects render simultaneously — e.g. feed + lead, or lead + worker.
- Clicking a tab loads that subject into the focused pane (move-not-copy); the focus ring shows where it will land.
- `Cmd+W` closes the active subject in the focused pane (snap to feed / collapse pane per rules).
- stdin in one pane never leaks into another (single-writer preserved).
- Multi-window (impl 0018) still wins: a secondary window shows the overlay and mounts no PTY, regardless of the primary's pane layout.

## Non-Goals (v1)

- Drag-tab-to-edge / drag-the-pane (replaced by the picker).
- Per-pane multi-tab strips (each pane shows one subject).
- Layout persistence across restart (pairs with mission-session persistence).
- Cross-mission split (that's multi-window / impl 0018) and direct-chat split.
- Synchronized input across panes; saved layout presets/templates.

## Design (to mock in Pencil first)

Per the design-first workflow, mock these in `design/runners-design.pen` before coding:

- **Layout picker popup** — triggered by a toolbar button (near the tab strip / mission topbar). A compact grid of preset thumbnails (1 / 2-col / 2-row / 3 as 1+2 / 3-col / 3-row), current layout highlighted. Mirrors the TradingView picker but trimmed to the mission's needs.
- **2-pane layout** — e.g. feed (left) + lead terminal (right), resize gutter, focus ring on the active pane, shared tab strip on top.
- **3-pane layout** — e.g. feed + lead + worker (1-big+2-stacked), focus ring, gutters.

## Implementation Phases

### Phase 1 — pane-tree data model (`src/lib/paneTree.ts`)

- `PaneLeaf { kind:"leaf"; id; subject: "feed" | sessionId }`, `PaneSplit { kind:"split"; orientation:"row"|"col"; sizes:[number,number]; a; b }`, `PaneNode = PaneLeaf | PaneSplit`.
- Preset builders: `preset(kind, subjects)` → a fixed tree for each curated layout, filling leaves from the ordered subject list (feed first, then sessions).
- Pure helpers: `findPaneOfSubject`, `setPaneSubject(tree, paneId, subject)` (move-not-copy: clears the subject from any other leaf), `removeSubject`/`collapse` (promote sibling), `setSizes`, `focus`.
- Invariant: a subject appears in ≤1 leaf; feed appears in exactly ≤1 leaf. Unit tests: preset construction, move-not-copy clears source, collapse promotes sibling preserving orientation, feed-uniqueness.

### Phase 2 — layout picker popup

- `<LayoutPicker>` — the preset grid popover; `onPick(presetKind)` applies a preset to the current tree (mapping existing subjects into the new pane slots, feed-first).
- Toolbar button in the mission topbar/tab-strip row opens it; highlight the active preset.

### Phase 3 — render the tree

- `MissionWorkspace` swaps `activeTab`/`openTabs` for `paneTree: PaneNode` + `focusedPaneId`.
- `<PaneTreeView>` recursively renders: `PaneSplit` → `<PanelGroup direction>` + two `<Panel>` + `<PanelResizeHandle>`; `PaneLeaf` → feed subject renders `EventFeed`+`MissionInput`; session subject renders `SlotPtyPane`+`RunnerTerminal`. Reuse the existing `SlotPtyPane`/`EventFeed`/`MissionInput`.
- Shared top tab strip stays; a tab click calls `setPaneSubject(tree, focusedPaneId, subject)`.
- Keep the multi-window gate: while `isSecondary`, force a single feed pane and mount no PTYs (as today).

### Phase 4 — focus + keyboard

- `focusedPaneId` = last-clicked pane (click tab strip or content). Focused pane gets a 1px accent ring (mirrors the impl-0018 overlay accent).
- `Cmd+W` / `Ctrl+W` closes the focused pane's subject: snap to feed if it was feed-adjacent, else collapse the pane and reflow the sibling. Preserve existing meta-shortcut handling (the ⌘1–⌘9 tab shortcuts still select into the focused pane).

### Phase 5 — verification + smoke

- `paneTree.ts` unit tests clean.
- Manual smoke (adapted from spec 19): pick 2-pane → feed+lead side by side, resize gutter; load worker into the right pane via tab click (move-not-copy, worker leaves its old pane); type into lead → not echoed in worker; pick 3-pane; `Cmd+W` collapses focused pane; switch mission and back → resets to single pane; multi-window cross-check → secondary shows overlay, no PTY mounts regardless of primary layout.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.

## Relevant Code

- `src/pages/MissionWorkspace.tsx:113-170` — `activeTab`/`openTabs`/`terminalsRef` state + `isSecondary` gate (to be replaced by `paneTree`/`focusedPaneId`).
- `src/pages/MissionWorkspace.tsx:1075-1209` — the tab-strip row (`TabButton`/`PtyTabButton`) and the `<Pane active>` content stack (feed pane = `EventFeed`+`MissionInput`+`MissionPausedCard`; session panes = `SlotPtyPane`) — becomes `<PaneTreeView>`.
- `src/components/RunnerTerminal.tsx` — terminal mount/measure/resize; must refit on gutter resize and on remount-after-move (snapshot replay already handles scrollback).
- `src/components/EventFeed.tsx` — feed leaf.
- `src/lib/windowFocus.ts` (`isSecondaryFor`) — keep the multi-window gate.
- `package.json` — add `react-resizable-panels`.

## Open Questions

- Content model **A** (shared tab strip + focused pane) vs **B** (per-pane dropdown) — defaulting to A; confirm against the Pencil mock.
- Exact preset set + picker placement (topbar vs tab-strip row) — settle in the Pencil mock.
- Terminal refit strategy on gutter drag (debounced `fit()` per visible leaf).

## References

- Issue [#166](https://github.com/yicheng47/runner/issues/166); spec [docs/features/19-mission-split-view.md](../features/19-mission-split-view.md); TradingView layout picker (reference for the popup).

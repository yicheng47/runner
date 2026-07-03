# Direct-Chat Split View (layout picker)

## Status

In progress for issue [#245](https://github.com/yicheng47/runner/issues/245), spec [docs/features/34-direct-chat-split-view.md](../features/34-direct-chat-split-view.md). This doc was originally written for mission split ([#166](https://github.com/yicheng47/runner/issues/166), spec 19); that scope is deferred and #166 is closed. The layout-picker interaction survives; the surface is now the direct chat.

## Problem

Direct chats are one-at-a-time: `RunnerChat` (`/chats/:sessionId`) shows a single chat and switching means flipping through the sidebar. Driving two runners at once — an implementer next to a reviewer, two explorations — forces constant back-and-forth. Multi-window (impl 0018) helps only across missions/chats you want in separate windows; a quick side-by-side inside one window doesn't exist.

## What makes this cheap

`RunnerChat` already keeps every direct chat mounted simultaneously: `directSessions.map(...)` renders one absolutely-positioned `RunnerTerminal` pane per session and toggles `block`/`hidden` on the active one (`src/pages/RunnerChat.tsx:1175-1260`). All terminals are live; "switching chats" is a visibility flip. Split view is therefore not "mount more terminals" — it is "show 2–3 of the already-mounted panes at once". Move-not-copy and single-writer stdin hold by construction: each session has exactly one `RunnerTerminal` instance, ever.

## Interaction (TradingView-style picker)

- A **layout button** left of Stop in the chat topbar opens a **preset picker popup**: 1 (default) · 2 side-by-side · 2 stacked · 3 as 1-big+2-stacked · 3 columns · 3 rows. Active preset highlighted in accent.
- Each pane is a self-contained chat: **per-pane header** (terminal icon · chat name · CHAT chip · status dot) with the terminal below. The **focused pane** carries a 1px accent ring.
- **Resize gutters** between panes (`react-resizable-panels`).
- Picking a preset with more panes than open chats leaves the extras **empty**: focus moves to the first empty pane and the **`StartChatModal` opens with the focused chat's runner preselected** (same config is one Enter away; still changeable). Cancel leaves the empty state: "No chat in this pane" + New chat button + "or pick a chat from the sidebar".
- **Sidebar** reflects the layout: every pane-open chat row gets the selected-row fill (#33353D); the focused pane's row additionally shows a 2px accent bar on its left edge (mirrors the pane focus ring). Clicking a chat row while split loads that chat into the **focused pane** (move-not-copy: if it's visible in another pane, it moves there instead of duplicating).
- Topbar and right-hand runner panel follow the **focused pane's** chat, unchanged in shape.

## Key Decisions

1. **Layout picker, not drag-tab-to-edge.** Discoverable, no HTML5 DnD, and presets cover every real arrangement at ≤3 panes. (Carried over from the mission-split pivot.)
2. **Per-pane header + focus ring**, not a shared tab strip. Direct chats have no tab strip today, and the Pencil mock validated per-pane chrome as the way to show each pane's identity.
3. **A chat lives in exactly one pane.** One `RunnerTerminal` per session (already true); the layout maps sessionId → pane slot. Loading a chat into a pane clears it from any other slot.
4. **Terminals stay in the flat stack; visible ones geometry-sync to their pane rect.** Each visible session's absolutely-positioned wrapper is imperatively sized/positioned onto its pane's body rect (a `ResizeObserver` per pane body keeps them glued through gutter drags, window resizes, and panel toggles); hidden sessions keep today's `hidden` stack behavior. The terminals' React tree position never changes, so xterm never remounts — by construction, not by convention. *Rejected:* portal re-parenting (the original default). React's reconciler remounts a portal's children whenever the portal container changes — `updatePortal` in `ReactChildFiber` reuses the fiber only when `current.stateNode.containerInfo === portal.containerInfo` — so retargeting a terminal's portal to a different pane body destroys and recreates the xterm subtree, which is exactly the remount this feature exists to avoid (review finding on PR #244).
5. **`react-resizable-panels` for gutters** (new dep, ~5kb, no peer deps). Presets construct fixed 1–2 level trees; the lib handles resize/min-max/keyboard. `RunnerTerminal.fit()` refits on gutter drag (debounced) — same contract as the existing panel-collapse refit.
6. **Layout is in-memory, per window, chat-surface-scoped.** A module-level store (shared by `RunnerChat` + `Sidebar` via `useSyncExternalStore`) holds the pane tree + focused pane. Route param changes inside `/chats/*` do not reset it; navigating to a non-chat surface clears it. No persistence in v1.
7. **URL stays `/chats/:sessionId` = the focused pane's chat.** Focusing a pane or loading a chat into the focused pane navigates (replace) so refresh/deep-link keeps working; a deep link opens single-pane as today unless a layout is already live.
8. **Close pane ≠ stop session.** `Cmd+W` while split collapses the focused pane (sibling reflows); the chat keeps running in the hidden pool. Single-pane `Cmd+W` keeps its current behavior.
9. **Multi-window (impl 0018) still wins, via a multi-subject registry.** The one-subject-per-window registry couldn't arbitrate a split window: showing chats A+B while focused on A left B unreported, so another window could claim B and both would mount B's PTY (review finding on PR #244). The registry now stores `subjects: Vec<Subject>` per window (`window_report_subjects`); RunnerChat reports every visible pane's session, and the duplicate-subject gate runs per session: a pane whose session is owned by a later-focused window shows the overlay in that pane and mounts no terminal, same primary/secondary rules as today. MissionWorkspace keeps its single-subject wrapper.
10. **Sidebar shows pane state in place.** Every pane-open row gets the selected fill; the focused pane's row adds a 2px accent bar. Rows highlight wherever they sit in the CHAT list — no reordering. *Considered:* a marker icon instead of fill for non-focused panes (keeps "selected" unique but under-communicates what's on screen) and a VS Code-style "on screen" section at the top of the list (strongest overview, but reorders rows on split and is overkill at ≤3 panes). In-place highlight wins: zero new components and the accent bar ties directly to the pane focus ring.
11. **Right rail shows the focused session only.** *Considered:* stacking cards for all visible panes — but 320px split three ways leaves room for name + status, which the pane headers already show, and it stacks multiple Stop/Archive buttons (misclick surface for destructive actions on the wrong session). Inspector-follows-selection matches how the topbar and URL already behave. If real use shows a gap, the cheap future addition is a slim "also on screen" strip (other panes' names + status dots, click to jump focus) — not built in v1.

## Goals

- Layout button + picker in the chat topbar; choosing a preset splits the center area with resizable gutters.
- 2–3 direct chats visible and usable simultaneously, each with its own header and status.
- Sidebar shows which chats are open in panes and which is focused; clicking a row targets the focused pane.
- Empty panes funnel into the existing `StartChatModal` (runner preselected) or a sidebar pick.
- stdin in one pane never reaches another; no terminal remounts on layout changes.

## Non-Goals (v1)

- Mission-workspace split (#166, closed as deferred) and cross-window layouts.
- Synchronized input across panes; saved layout templates; layout persistence across restart.
- Per-pane runner panels (the right rail follows focus only).

## Design

Mocked in `design/runners-design.pen`:

- **Layout picker popup** (`Stq9b`) — preset grid, active preset stroked accent, hint "Layout resets when you leave chats".
- **Runner direct chat — 2-pane split** (`fxfRj`) — layout button left of Stop, focused pane ring, per-pane headers, gutter, sidebar showing open-pane fills + focused accent bar.
- **Runner direct chat — 3-pane split** (`WQmol`) — 1-big+2-stacked preset with the picker open in context.
- **Runner direct chat — 2-pane split, empty pane** (`kBqRL`) — post-split empty state: New chat button + sidebar hint, focus ring on the empty pane.

## Implementation Phases

### Phase 1 — pane-layout model (`src/lib/paneLayout.ts`)

- `PaneLeaf { id; sessionId: string | null }` (null = empty pane), `PaneSplit { orientation: "row" | "col"; sizes; a; b }`, plus `focusedPaneId`.
- Preset builders `applyPreset(kind, currentAssignments)` — fill slots from currently visible chats first (focused chat keeps the biggest slot), leave the rest empty.
- Pure helpers: `assignSession(tree, paneId, sessionId)` (move-not-copy), `closePane` (collapse, promote sibling), `visibleSessionIds`, `setSizes`.
- Module store + `useSyncExternalStore` hook; cleared on chat-surface unmount.
- Unit tests: preset construction, move-not-copy clears the old slot, collapse promotes sibling, empty-slot ordering.

### Phase 2 — layout picker popup

- `<LayoutPicker>` preset-grid popover per the mock; active preset highlighted; opens from the layout button placed left of Stop in the `RunnerChat` topbar.

### Phase 3 — render the pane tree

- `RunnerChat` center area: single-pane path renders exactly as today; multi-pane renders `<Group>`/`<Panel>`/`<Separator>` from the tree as a chrome layer (headers, focus ring, gutters, empty states) with the flat terminal stack geometry-synced onto the pane bodies (decision 4).
- Each pane: header (name/chip/status) + body; the body is the geometry target the session's already-mounted terminal wrapper is glued to. Empty panes render the empty state and auto-open `StartChatModal` (target-pane mode, runner preselected).
- `RunnerTerminal`'s own `ResizeObserver` refits on gutter drag (dims-deduped) — same contract as the existing panel-collapse refit.

### Phase 4 — sidebar + modal integration

- Sidebar chat rows read the layout store: open-pane fill, focused accent bar; row click → `assignSession(focusedPane)` + navigate.
- `StartChatModal`: optional `defaultRunnerId` + `onStarted` target-pane assignment instead of plain navigate.

### Phase 5 — focus + keyboard

- Click in a pane (header or terminal) focuses it; focus ring + topbar/right-rail/URL follow.
- `Cmd+W`/`Ctrl+W` while split closes the focused pane (collapse; session keeps running). Single-pane behavior unchanged.

### Phase 6 — verification + smoke

- `paneLayout.ts` tests clean; `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.
- Manual smoke: split 2-pane → both terminals live, typing isolated per pane; gutter resize refits; sidebar shows fills + focus bar and row click swaps the focused pane; 3-pane preset → empty pane auto-opens StartChatModal, cancel shows empty state; `Cmd+W` collapses; navigate to missions and back → single pane again; secondary window on a pane's session → overlay in that pane only.

## Relevant Code

- `src/App.tsx:90` — `/chats/:sessionId` route.
- `src/pages/RunnerChat.tsx` — `DirectSessionPane`, `directSessions`, the absolute pane stack (`active ? "block" : "hidden"`) this builds on; topbar Stop button for picker placement; gains the pane chrome renderers + geometry sync.
- `src/components/Sidebar.tsx` — `creatingChat` + `StartChatModal` wiring; chat rows gain pane-open fill + focused accent bar.
- `src/components/StartChatModal.tsx` — gains `defaultRunnerId` (target-pane `onStarted` stays caller-owned).
- `src/components/RunnerTerminal.tsx` — `fit()` on resize; gains `autoFocus` (a sibling pane's activation must not steal focus) and a `focus()` handle method.
- `src/lib/windowFocus.ts` + `src/lib/types.ts` + `src/lib/api.ts` — multi-subject reporting (`useReportSubjects`, `WindowEntry.subjects`).
- `src-tauri/src/windows.rs` + `src-tauri/src/commands/window.rs` — registry stores `Vec<Subject>` per window; `window_report_subjects`.
- `package.json` — add `react-resizable-panels` (v4 `Group`/`Panel`/`Separator` API) and `vitest` (dev; first frontend unit-test runner, wired into CI).

## Resolved Questions

- Portal re-parenting vs geometry-sync — resolved for geometry-sync without a runtime spike: React's reconciler keys portals by `containerInfo` identity, so retargeting remounts children (decision 4). Geometry-sync keeps the terminals' tree position fixed, which makes no-remount structural.
- Whether `applyPreset` should pull recently-active chats into empty slots instead of leaving them empty — v1 leaves them empty (explicit beats implicit; the modal opens immediately anyway).

## References

- Issue [#245](https://github.com/yicheng47/runner/issues/245); spec [docs/features/34-direct-chat-split-view.md](../features/34-direct-chat-split-view.md); deferred predecessor [#166](https://github.com/yicheng47/runner/issues/166) / spec 19; design+planning PR [#244](https://github.com/yicheng47/runner/pull/244); TradingView layout picker (interaction reference).

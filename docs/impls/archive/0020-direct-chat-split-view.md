# Direct-Chat Split View (layout picker)

## Status

In progress for issue [#245](https://github.com/yicheng47/runner/issues/245), spec [docs/features/34-direct-chat-split-view.md](../features/34-direct-chat-split-view.md). This doc was originally written for mission split ([#166](https://github.com/yicheng47/runner/issues/166), spec 19); that scope is deferred and #166 is closed. The layout-picker interaction survives; the surface is now the direct chat.

## Problem

Direct chats are one-at-a-time: `RunnerChat` (`/chats/:sessionId`) shows a single chat and switching means flipping through the sidebar. Driving two runners at once — an implementer next to a reviewer, two explorations — forces constant back-and-forth. Multi-window (impl 0018) helps only across missions/chats you want in separate windows; a quick side-by-side inside one window doesn't exist.

## What makes this cheap

`RunnerChat` already keeps every direct chat mounted simultaneously: `directSessions.map(...)` renders one absolutely-positioned `RunnerTerminal` pane per session and toggles `block`/`hidden` on the active one (`src/pages/RunnerChat.tsx:1175-1260`). All terminals are live; "switching chats" is a visibility flip. Split view is therefore not "mount more terminals" — it is "show 2–3 of the already-mounted panes at once". Move-not-copy and single-writer stdin hold by construction: each session has exactly one `RunnerTerminal` instance, ever.

## Interaction (TradingView-style picker)

- A **layout button** left of Stop in the chat topbar opens a **preset picker popup**: 1 (default) · 2 side-by-side · 2 stacked · 3 as 1-big+2-stacked · 3 columns · 3 rows. Active preset highlighted in accent.
- Each pane is a self-contained chat: **per-pane header** (terminal icon · chat name · CHAT chip · status dot · its own **Stop/Resume** control) with the terminal below. The **focused pane** carries a 1px accent ring. While split, the topbar's session control aggregates: **Stop all** when any visible pane runs, **Resume all** when none do; panes resume concurrently, each with its own settle tracking.
- Panes run **edge-to-edge as one connected surface** — no outer frame — separated by a single 1px **resize divider** (`react-resizable-panels`, widened pointer target).
- Picking a preset with more panes than open chats leaves the extras **empty**: focus moves to the first empty pane and the **`StartChatModal` opens with the focused chat's runner preselected** (same config is one Enter away; still changeable). Cancel leaves the empty state: "No chat in this pane" + New chat button + "or pick a chat from the sidebar".
- The split is a **tab** — a binding between specific sessions, not a viewport mode (arch §3.6, Window → Tab → Pane; this log's original wording called it the "chat group"). It renders while the open chat is a member; any other chat opens classic single-pane and pane tabs stay intact in the background until one of their members is opened again. Navigation never mutates a tab, and picking a pane preset from a non-member chat creates another pane tab instead of replacing the previous one.
- **Sidebar** reflects the on-screen tab: every pane-open chat row gets the selected-row fill (#33353D); the focused pane's row additionally shows a 2px accent bar on its left edge (mirrors the pane focus ring). Clicking a member's row focuses its pane; a non-member row is a plain navigation — unless the tab is on screen with an **empty focused pane**, which is the sidebar's explicit "fill this pane" gesture and the one row click that adds a member. *(Originally move-not-copy on every click — dogfooding showed navigation kept mutating the tab: moving a visible chat emptied the pane it came from, and opening an unrelated chat got sucked into the layout, evicting a member. Move-not-copy still holds for the assignment op itself: a chat lives in exactly one pane.)*
- Topbar and right-hand runner panel follow the **focused pane's** chat, unchanged in shape.

## Key Decisions

1. **Layout picker, not drag-tab-to-edge.** Discoverable, no HTML5 DnD, and presets cover every real arrangement at ≤3 panes. (Carried over from the mission-split pivot.)
2. **Per-pane header + focus ring**, not a shared tab strip. Direct chats have no tab strip today, and the Pencil mock validated per-pane chrome as the way to show each pane's identity.
3. **A chat lives in exactly one pane.** One `RunnerTerminal` per session (already true); the layout maps sessionId → pane slot. Loading a chat into a pane clears it from any other slot.
4. **Terminals stay in the flat stack; visible ones geometry-sync to their pane rect.** Each visible session's absolutely-positioned wrapper is imperatively sized/positioned onto its pane's body rect (a `ResizeObserver` per pane body keeps them glued through gutter drags, window resizes, and panel toggles); hidden sessions keep today's `hidden` stack behavior. The terminals' React tree position never changes, so xterm never remounts — by construction, not by convention. *Rejected:* portal re-parenting (the original default). React's reconciler remounts a portal's children whenever the portal container changes — `updatePortal` in `ReactChildFiber` reuses the fiber only when `current.stateNode.containerInfo === portal.containerInfo` — so retargeting a terminal's portal to a different pane body destroys and recreates the xterm subtree, which is exactly the remount this feature exists to avoid (review finding on PR #244).
5. **`react-resizable-panels` for gutters** (new dep, ~5kb, no peer deps). Presets construct fixed 1–2 level trees; the lib handles resize/min-max/keyboard. `RunnerTerminal.fit()` refits on gutter drag (debounced) — same contract as the existing panel-collapse refit.
6. **The layout is a per-window set of pane tabs, sticky, persisted by the main window.** A module-level store (shared by `RunnerChat` + `Sidebar` via `useSyncExternalStore`) holds pane trees + focused panes. A tab renders only while the open chat is a member — other chats render single-pane over the stored tabs, and neither route changes nor leaving the chat surface reset them. Picking a preset from a non-member chat creates a new pane tab; opening any member session re-activates that member's tab. The main window mirrors the set to localStorage (`runner.chat.layout`) so a relaunch restores pane tabs — chats come back stopped (the backend kills PTYs on quit) but in their panes, resumable. Persisted as preset + slot assignments + sizes and rebuilt through the preset builders, so a stale payload can't produce an unknown shape; sessions that vanished while the app was closed are swept to empty panes after the first chat-list fetch. Secondary windows stay in-memory — their labels don't survive a relaunch, and localStorage is shared, so persisting theirs would clobber the main window's tabs. Tabs are frontend-only state: the backend models sessions and coordination, never display grouping (arch §3.6). *(Originally in-memory-only, reset on leaving the surface; revised after dogfooding — losing the grouping on every restart read as a bug.)*
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
- Synchronized input across panes; saved layout templates.
- Per-pane runner panels (the right rail follows focus only).

## Design

Mocked in `design/runner-mvp-design.pen`:

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
- `Cmd+[` / `Cmd+]` cycle pane focus while split (iTerm2's pane keys); sidebar page navigation moves to `Cmd+Shift+[` / `Cmd+Shift+]` (the tab-switch idiom). OS-window cycling stays on macOS's native `` Cmd+` ``.
- `Cmd+W`/`Ctrl+W` while split closes the focused pane (collapse; session keeps running). Single-pane behavior unchanged.

### Phase 6 — verification + smoke

- `paneLayout.ts` tests clean; `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.
- Manual smoke: split 2-pane → both terminals live, typing isolated per pane; gutter resize refits; sidebar shows fills + focus bar and row click swaps the focused pane; 3-pane preset → empty pane auto-opens StartChatModal, cancel shows empty state; `Cmd+W` collapses; `Cmd+[`/`Cmd+]` cycle panes, `Cmd+Shift+[`/`]` still navigate pages; navigate to missions and back → layout retained; relaunch the app → same panes restored (sessions stopped, resumable); secondary window on a pane's session → overlay in that pane only.

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

## Follow-ups

- ~~Extract the split machinery out of `RunnerChat`~~ — shipped as `src/components/ChatPaneGroup.tsx`, and further than planned: ONE render path for every arrangement (a single chat is a tab of one pane; non-member chats get an ephemeral single-leaf tab). The earlier split/classic dual path restyled wrappers and re-parented overlays on every mode transition, and that seam is where a string of dogfooding bugs lived. `RunnerChat` keeps orchestration (store, navigation, session lifecycle, keyboard); the tab component owns chrome, the terminal stack, and geometry.
- ~~Keep terminals mounted across the `metaLoaded` navigation gap~~ — done: pane tabs stay mounted through navigation; archived-row PTY safety is enforced per session by the attach gate. Paired with a backend fix for the root of the "garbled scrollback" dogfooding reports: full-repaint runtimes (claude-code, codex) get their output buffer purged on resize (`SessionManager::resize`), since pre-resize bytes describe a stale grid width and mangle any later snapshot replay — the SIGWINCH repaint rebuilds the buffer at the new width.

## References

- Issue [#245](https://github.com/yicheng47/runner/issues/245); spec [docs/features/34-direct-chat-split-view.md](../features/34-direct-chat-split-view.md); deferred predecessor [#166](https://github.com/yicheng47/runner/issues/166) / spec 19; design+planning PR [#244](https://github.com/yicheng47/runner/pull/244); TradingView layout picker (interaction reference).

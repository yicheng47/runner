# Sidebar Tab Accordion (single- vs multi-pane tabs)

## Status

Planned. Design mocked in `design/runner-mvp-design.pen` — frame **"Tab accordion — sidebar spec"** (`y6LaRZ`). No tracking issue yet. Builds directly on the pane-layout model (impl 0020) and group pinning (#250); no backend changes.

## Problem

In the sidebar CHAT list, a multi-pane tab is invisible as a *unit*. Impl 0020 clusters the **active** on-screen tab's members into one contiguous block and gives each the selected-row fill (focused pane gets an accent bar), but they still render as flat sibling `SessionRow`s — nothing says "these three chats are one tab". Background tabs get even less: their member chats scatter into the CHAT list as ordinary rows, indistinguishable from loose single chats. The layout store already holds every tab (`layouts: PaneLayout[]`, persisted as `PersistedLayoutSet` v2), so the grouping exists in the model but never surfaces. You cannot tell a 3-pane tab from three unrelated chats, and you cannot tidy one away.

## What makes this cheap

The tab set is already there. `paneLayout.ts` keeps `layouts: PaneLayout[]` + `activeIndex`, each `PaneLayout` a preset tree with a user-settable `name: string | null`, all persisted to `runner.chat.layout` and restored on relaunch. `leaves()` / `visibleSessionIds()` already enumerate a tab's members in slot order, `leafForSession()` maps a chat back to its tab, and `getPaneLayoutsForTest()` proves the set is inspectable. Group pinning (`groupPinning.ts`) already makes a tab's members pin and cluster as a unit. So this is **not** new state or new coordination — it's a sidebar render pass over data the store already owns, plus one persisted `collapsed` bit per tab. The member rows stay `SidebarListRow`s (rename, pin, context menu unchanged); only the group header is new chrome.

## Interaction

- A tab with **≥2 member chats** renders as an **accordion group**: a header row (disclosure chevron ▸/▾ · split icon `columns-2`/`columns-3` · tab name · pane-count badge) with the member chat rows indented under a single left **rail**.
- A tab with **one chat** — or any un-tabbed chat — stays a plain **leaf row** (status dot + title), exactly as today. The absence of the disclosure triangle is the single-vs-multi tell; the split icon + badge is the reinforcement.
- **Chevron / header click toggles collapse** (persisted per tab). A collapsed group hides its members but keeps the count + split icon, so it still reads as a group.
- Clicking the **tab name** activates that tab (`activatePaneLayoutForSession`) and opens its focused pane's chat. Clicking a **member row** focuses that pane — the existing `openDirectChat` behavior, unchanged.
- The **active on-screen tab** keeps impl 0020's marks: its focused member shows the accent bar, and its rail renders in accent (`--color-accent`); non-active groups get a neutral rail (`--color-line`). The focused member's selected fill is the code's `--color-sidebar-selected` (#333640) / `-border` (#3b3e49) — matched in the mock (`c3SUAN`).
- **Group name** shows `PaneLayout.name`, defaulting to the focused (else first) member's title. Inline rename on the header writes `setGroupName`.
- **Pinning** a group pins every member as a unit (existing `groupPinning`); the whole accordion sits in the pinned region and the header carries a pin marker.

## Key Decisions

1. **Accordion at ≥2 *members*, not ≥2 *panes*.** An empty pane has no session, so it is not a sidebar row; a 2-pane tab holding one chat + one empty pane reads as a single chat in the list and renders as a leaf. Grouping keys on member count, so the accordion never shows a lone child. *Rejected:* rendering empty-pane placeholder rows inside the group — sidebar noise for a chat-surface concern.
2. **Surface *every* multi-member tab, not just the on-screen one.** The store already persists all tabs; making each an accordion (active or background) matches the mock's coexisting active-expanded + collapsed groups and lets you tidy background tabs. This **supersedes** today's active-only `clusterActiveGroupRows` treatment — that helper generalizes to "partition the whole CHAT list by tab membership".
3. **Collapse state lives in the tab, persisted with the set.** Add `collapsed?: boolean` to `PaneLayout` and `PersistedLayout` (optional field — old payloads default to expanded, no version bump). One `setTabCollapsed` action. Persisting per tab means a tidied group stays tidied across relaunch.
4. **Members stay `SidebarListRow`; only the header is new.** Rename, pin, status dot, context menu, accent bar all keep working by construction. The new `ChatTabGroup` owns the header + rail and delegates each member to the existing row.
5. **Distinction mechanism = disclosure triangle + split icon + count badge** (per mock), not color alone — legible for colorblind users and at a glance. A leaf never has a triangle.
6. **No backend touch.** Tabs remain frontend-only display grouping (arch §3.6). The sidebar reads the store; the store persists to localStorage as it already does.

## Goals

- Sidebar CHAT list renders each ≥2-member tab as a collapsible accordion (chevron · split icon · name · count · rail); single chats and un-tabbed chats stay leaf rows.
- Collapse state persists per tab across app restart.
- Active tab + focused pane stay visually marked (accent rail + accent bar + selected fill); member row/pin/rename/context-menu behavior unchanged.
- Tab name shows on the header and is inline-renamable.

## Non-Goals (v1)

- A first-class tab strip on the chat surface (separate, design-first feature).
- Empty-pane placeholder rows in the sidebar.
- Drag-to-reorder tabs, or dragging a chat between tabs.
- Any backend persistence or modeling of tabs.
- The `isGroupActiveFor` → `isTabActiveFor` naming sweep — optional cleanup that can ride along or stay parked.

## Design

`design/runner-mvp-design.pen`, frame **"Tab accordion — sidebar spec"** (`y6LaRZ`): a 240px sidebar slice showing the CHAT section with four tabs plus an annotation column.

- **Tab A** (`ur60w`) — multi, expanded + active: header ▾ + `columns-2` + green count badge; members under an accent rail; focused member in the selected fill (`c3SUAN`, #333640 / #3b3e49).
- **Tab B** (`o3lUh`) — single leaf, for contrast.
- **Tab C** (`InLbx`) — multi, collapsed: ▸ + `columns-3` + neutral badge, members hidden.
- **Tab D** (`W6GN5U`) — single leaf.

## Implementation Phases

### Phase 1 — model: per-tab collapse (`src/lib/paneLayout.ts`)

- Add `collapsed?: boolean` to `PaneLayout` and `PersistedLayout`; thread through `toPersistedLayout` / `fromPersistedLayout` (default `false`/absent). No `PersistedLayoutSet` version bump.
- `setTabCollapsed(sessionId | index, collapsed: boolean)` mutating the target tab and persisting; a `useCollapsed`-style read is unnecessary since `usePaneLayout`/the set hook already re-renders.
- Unit tests (vitest): collapse round-trips through serialize/deserialize; missing field defaults to expanded; toggling one tab leaves others untouched.

### Phase 2 — grouping helper (`src/lib/groupPinning.ts` or new `chatTabs.ts`)

- Pure `buildChatListItems(rows, layouts)` → ordered `(TabGroupItem | LooseChatItem)[]`, generalizing `clusterActiveGroupRows`: for each `PaneLayout` with ≥2 visible members, emit a `TabGroupItem { layout, members: rows-in-slot-order }` anchored where the group's best-sorted member sits; everything else stays a `LooseChatItem`. A chat appears exactly once.
- Unit tests: multi-member tab → group in slot order; single-member tab → loose; anchoring preserves the backend sort for loose rows; pinned group clusters in the pinned region; no session double-listed across two tabs.

### Phase 3 — `ChatTabGroup` component (`src/components/ChatTabGroup.tsx`)

- Header: chevron (▸/▾), split icon (`columns-2` at 2 members, `columns-3` at 3), name (`layout.name ?? focused/first member title`) with inline rename → `setGroupName`, count badge, pin marker when pinned.
- Rail wrapper (accent when this is the active tab, else neutral) around member `SessionRow`s; collapsed → render header only.
- Props: `layout`, `members`, `active`, `focusedSessionId`, plus the row callbacks (`onOpenChat`, `onContextMenu`, rename) forwarded to `SessionRow`. Chevron → `setTabCollapsed`; name click → `activatePaneLayoutForSession` + navigate.

### Phase 4 — wire into the CHAT section (`src/components/Sidebar.tsx`)

- Replace `chatRows.map(SessionRow)` (lines ~1013–1038) with `buildChatListItems(directSessions, getPaneLayoutsForTest-equivalent public getter).map(...)`, rendering `ChatTabGroup` for groups and `SessionRow` for loose chats.
- Expose the tab set to the sidebar via a small public accessor + subscription (reuse `subscribePaneLayout`); drop the active-only `activeGroupSessionIds` / `clusterActiveGroupRows` path in favor of the general one, keeping `focusedPaneSessionId` for the accent bar.
- Optional: give `CollapsibleSectionHeader` a `count` (number of top-level items) to match the mock's badge.

### Phase 5 — verify + smoke

- vitest for Phases 1–2 pure helpers; `pnpm exec tsc --noEmit`, `pnpm run lint` clean.
- Manual smoke: open a 2- and a 3-pane tab → each renders as an accordion with the right split icon + count; collapse one, relaunch → stays collapsed; single chats stay flat leaf rows; pin a group → all members pin and cluster, header shows the pin marker; the active tab shows the accent rail + focused member's accent bar + selected fill; rename a group header; clicking a member focuses its pane, clicking the name activates the tab.

## Relevant Code

- `src/lib/paneLayout.ts` — `PaneLayout` (`name`, add `collapsed`), `layouts[]`/`activeIndex`, `PersistedLayout` (`toPersistedLayout`/`fromPersistedLayout`), `leaves`/`visibleSessionIds`/`leafForSession`, `activatePaneLayoutForSession`, `setGroupName`; add `setTabCollapsed` + a public tab-set getter.
- `src/lib/groupPinning.ts` — `clusterActiveGroupRows` (generalize to `buildChatListItems`), `pinnedSessionIds`, `groupPinTargets`, `shouldInheritPinOnAdd`.
- `src/components/Sidebar.tsx` — CHAT section render (~1002–1041), pane-open/focused derivation (283–306, 289–294), `SessionRow` (1601), `SidebarListRow` (1338, `selected`/`accentBar`/`pinned`), `CollapsibleSectionHeader` (1293).
- `src/components/ChatTabGroup.tsx` — new: accordion header + rail.
- `design/runner-mvp-design.pen` — `y6LaRZ` (spec), member selected fill `#333640` / `#3b3e49`.

## Open Questions

- **Section count semantics** — top-level items (groups + loose chats) vs total chats. Leaning top-level items to match the grouped mental model; cheap to flip.
- **Active tab: force-expanded?** A collapsed active tab would hide the focused, on-screen chat's row. Options: respect the persisted collapse anyway (the chat is visible on the surface regardless), or auto-expand the active tab while it is on screen. Leaning auto-expand-active — a collapsed group whose chat is live in a pane reads as a lie.

## References

- Design: `design/runner-mvp-design.pen` frame `y6LaRZ`.
- arch §3.6 — Window → Tab → Pane (frontend-only display grouping).
- impl [0020](0020-direct-chat-split-view.md) — pane-layout model, active-group clustering, sidebar pane-state marks.
- impl [0022](0022-new-chat-pane-fill-window-ownership.md) — pane fill + ownership reporting.
- Group pinning (#250) — pin/cluster a tab's members as a unit.

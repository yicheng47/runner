# 43 — Sidebar pinned section

> Tracking issue: [#317](https://github.com/yicheng47/runner/issues/317)

## Motivation

Pinned things are scattered. A pinned chat tab sorts to the top of the CHAT section and a pinned mission sorts to the top of the MISSION section, so "the stuff I keep coming back to" lives in two places separated by whatever else the sidebar holds. The user-visible intent of pinning — "keep this at hand" — wants one location.

This is a presentation-layer change only. The discussion that produced this spec explicitly rejected unifying the data models (making missions rows in the `tabs` table): a mission's surface is tab-like (it reuses the pane machinery and the `SidebarTabRow` shell), but its semantics are not — slot-bound composition, reset/respawn lifecycle, feed, and live-activity attention would all need `kind` special-cases in every tab code path (dnd move-not-copy, `ensure_active_sessions`, folder archive, unread watermarks). Missions and tabs stay separate models; the sidebar just re-partitions the rows it already renders.

Builds on #315 (unified nav scroll surface).

## Scope

> **Sequencing update:** spec 44 (#318) lands first. PINNED then renders as the `pinned_position` derived view over the node tree, and pin/unpin writes `pinned_position` on the node — which also retires the `groupPinning.ts` fan-out (pin becomes tab-level naturally). Still no new backend work in this spec; the column ships with #318.

### In scope (frontend only)

- **PINNED section** at the top of the nav column (above PROJECT), holding every pinned chat tab and every pinned mission in one list. The pinnable population spans all of it: unfiled CHAT tabs, tabs nested under a project, and missions (project-bound or not).
- Pinned rows render **only** in PINNED — they leave their origin sections, including a project's nested list. The pinned-first sort inside CHAT (`Sidebar.tsx` tab ordering) and the pinned-first mission sort are retired; origin sections show only unpinned rows.
- Rows keep their type-specific rendering, click targets, and context menus (`MissionRow`, `ChatTabGroup`/`SessionRow` adapters unchanged). Pin/unpin via context menu is the only way rows move between sections.
- Section hidden entirely when nothing anywhere is pinned — no pinned tab, no pinned project-nested tab, no pinned mission.
- Default ordering: missions and chat tabs interleaved, using each row's existing sort key (missions by `pinned_at` recency, tabs by the existing recent-direct sort). Deterministic, no new state.

### Out of scope

- **Drag and drop, entirely** — both drag-into-PINNED-to-pin and drag-to-reorder. Doing them now would mean interim special-cases (a `MissionRow` draggable for one interaction; a `pin_position` column bolted onto the current section layout) that the section merge below would rewrite. Drag arrives with that rework, not before.
- **Merging the MISSION and CHAT sections generally.** The likely end-state is PINNED / PROJECT (holding both chats and missions — missions already carry `project_id`) / one recent list for unfiled items, dissolving the type-based sections. That's a follow-up spec once PINNED has proven out the interleaved rendering; mission dragging (into PINNED, into projects) and pinned reordering belong to it — as does manual ordering *inside* a project group, which doesn't exist today either (tab `position` is folder-scoped; project groups filter the global list). All three consume the same "persisted order for heterogeneous rows" mechanism and should be designed once, together.
- **Mission-as-tab data unification.** Rejected — see Motivation.

### To be decided

- Whether PINNED shows a small section header like the others or renders headerless above PROJECT.
- Interleave order tie-breaking (pin time vs. recency) once real usage shows which reads better.
- Whether pinning the active split group (which fans out to all members) should visually collapse to one PINNED row per tab (expected: yes — reuse the existing `ChatTabGroup` row).

## Implementation phases

Single phase: partition pinned rows out of MISSION/CHAT into a new top section in `Sidebar.tsx`; retire pinned-first sorts in the origin sections; hide-when-empty.

## Verification

- [ ] Pin a chat tab and a mission → both appear in PINNED, disappear from CHAT/MISSION; unpin returns them.
- [ ] Group pin fan-out still yields one PINNED row for a split tab.
- [ ] PINNED hidden when nothing is pinned.
- [ ] Attention indicators (working/unread) and status dots render unchanged on pinned rows.
- [ ] Existing chat drag-reorder inside folders is unaffected by the new section.
- [ ] `pnpm exec tsc --noEmit` and `pnpm run lint` clean.

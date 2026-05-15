# 16 — Sidebar mission detail

> Tracking issue: [#134](https://github.com/yicheng47/runner/issues/134)

## Motivation

A Mission carries three pieces of identity that the user needs in
working memory while inside the workspace: **what the mission is for
(goal/description)**, **where it runs (cwd)**, and **which crew it
came from**. All three are stored on the `missions` row
(`goal_override`, `cwd`, `crew_id`) and shipped in `Mission` to the
frontend (`src/lib/types.ts:133-149`). None of them surface in the UI
once the mission is created.

Today the sidebar shows missions as a single row with title +
status dot. The workspace topbar shows title + chip + status pill +
meta row (branch / runner count / age). Neither shows:

- The goal text — what this mission is supposed to accomplish. Users
  context-switching between missions read the title, can't remember
  the brief, have to scroll back in the feed to find the
  `mission_goal` event.
- The working directory — useful to confirm "yes, this mission is
  rooted in the repo I think it's rooted in" before pasting a
  command into the input. Today the only way to check is to open a
  PTY tab and run `pwd`.
- The crew that templated this mission — relevant when you want to
  start a new mission from the same template or jump to the crew
  editor to tweak its roster.

The sidebar's selected mission row is the natural home for this
detail: it's already the "this is the mission you're looking at"
anchor, and expanding it on selection adds detail exactly when the
user needs it, without crowding rest-state missions.

## Scope

### In scope (v1)

- **Auto-expand the selected mission row** in the sidebar to show:
  - **Goal text** — `mission.goal_override` if set, else the crew's
    `goal` (which is templated into the mission at start time).
    Truncated to ~3 lines with a "show more" toggle that expands
    inline to the full text. Plain text only in v1 (no markdown
    rendering inside the sidebar — too narrow).
  - **Working directory** — `mission.cwd` shown with middle-truncation
    (e.g. `~/go/src/.../runner`). Full path on tooltip hover. Click
    opens the folder in macOS Finder via `tauri-plugin-shell`'s
    `revealItemInDir` or a small wrapper command.
  - **Crew name** — links to `/crews/<crew_id>` on click.
  - **Started time** — relative ("started 14m ago"), updates every
    minute.
- **Inline edit** affordances:
  - **Goal**: click the text to enter edit mode (textarea grows to
    fit, autosaves on blur via `mission_update`). Esc cancels.
  - **cwd**: a small folder-pick button next to the path opens the
    Tauri folder picker; selecting a new folder calls
    `mission_update` to persist.
- **Non-selected missions stay collapsed** — the single-line row
  remains unchanged for the rest of the sidebar.
- **Manual collapse** via a chevron at the top-right of the expanded
  row. Stored in `localStorage` under
  `runner.sidebar.mission.detail.collapsed.<missionId>` so toggling
  away and back remembers the user's choice.
- **Light-theme parity** — the panel uses the same `bg-panel` /
  `border-line` / `text-fg-2` tokens as the rest of the sidebar.
  Spec 15's light theme picks it up via CSS variables; no
  per-theme branches.

### Out of scope (deferred)

- **Markdown rendering** of the goal. The feed's first
  `mission_goal` event already renders markdown via the existing
  `MessageBody` component; in the sidebar it would either bloat
  the column or wrap unreadably. Plain text.
- **Mission archive / pin / stop** from this panel. Those already
  live in the `mission_ctx_menu` (right-click) and on the workspace
  topbar's kebab menu. Duplicating them in the sidebar panel adds
  surface area without new capability.
- **Slot roster preview.** The runners rail on the right already
  shows live slot state. Echoing it here would split attention.
- **Rename** from this panel. Already in the workspace topbar via
  `window.prompt("Rename mission", …)` (MissionWorkspace.tsx:253).
- **History / event timeline** in the detail. The feed is the
  timeline.
- **A full mission-detail page route.** All useful detail fits in
  the expanded sidebar row; a route is overkill.

### Key decisions

1. **In the sidebar, not the topbar.** The topbar's meta row is
   one-line by design — it's a "you are here" anchor, not a detail
   surface. Multi-line goal text and a long cwd path don't fit. The
   sidebar already has vertical space and is the existing home for
   mission-identity affordances.
2. **Auto-expand on selection, persist per-mission collapse.**
   First-time view of a mission shows detail; once the user has
   read it, they can collapse and the sidebar remembers. Different
   missions remember independently because users have different
   relationships with each one.
3. **Edit inline, save on blur — no Save button.** Reduces the
   ritual; matches the rename UX elsewhere (`window.prompt`-based
   in the topbar already commits on Enter). Esc cancels for
   parity.
4. **cwd is read-only path text + a pick button.** Typing a path
   into a textbox is error-prone (typos, ~/ vs $HOME, relative
   paths). The folder picker enforces a real directory. Read-only
   display avoids the "did I save?" question.
5. **Hydrate via existing `mission_get`, not a new field on
   `listSummary`.** The sidebar's `listSummary` API is intentionally
   thin for fast render; loading full detail only on selection
   keeps the list query small. Selected-mission detail comes from
   the workspace's already-loaded `Mission` object passed down to
   the sidebar.

## Implementation phases

### Phase 1 — wire the data through

- `Sidebar.tsx` currently uses `api.mission.listSummary()` for the
  rows. The summary likely omits `goal_override` / `cwd` /
  `started_at`. Either:
  - (a) Hydrate the *selected* mission separately via
    `api.mission.get(id)` and pass full detail down, **or**
  - (b) Extend the summary type to include the three fields. Adds
    bytes to every row regardless of selection.
- Pick (a) — the workspace already has the full `Mission` in scope
  (`MissionWorkspace.tsx`); lift it via a context or callback into
  `Sidebar` for the matching row. Keeps `listSummary` lean.
- Look up the crew's default `goal` for the fallback case (when
  `goal_override` is null) — `mission.crew_id` → `crew.goal`. The
  workspace likely already has this for the topbar's meta row.

### Phase 2 — expanded row component

- New `src/components/SidebarMissionDetail.tsx`:
  - Props: `mission: Mission`, `crew: Crew | null`,
    `onUpdate: (patch) => void`.
  - Layout: vertical, padding `[10, 12]`, gap 8, fill `bg-panel`,
    rounded 6 (match the existing collapsed mission row's chrome).
  - Sub-sections in order: goal, cwd row, crew row, started row,
    collapse chevron in the corner.
  - Truncate goal with `line-clamp-3` + a "show more" button (sets
    local state to remove the clamp). Editable on click: swap to a
    `<textarea>` with autosize via row counting; commit on blur,
    revert on Esc.
  - cwd: `<span title={fullPath}>{middleTruncate(fullPath, 32)}</span>`
    + a folder-pick icon-button that calls a small wrapper around
    `@tauri-apps/plugin-dialog::open({ directory: true })`. On pick,
    call `onUpdate({ cwd })`.
  - Crew: `<Link to={`/crews/${crew.id}`}>{crew.handle}</Link>`.
  - Started: relative time via existing helper (or `Intl.RelativeTimeFormat`),
    re-rendered on a 60s tick.

### Phase 3 — sidebar integration

- In `Sidebar.tsx`'s mission row render, branch on
  `mission.id === currentMissionId`:
  - Selected → render the collapsed row chrome as the *header* and
    drop `<SidebarMissionDetail>` directly below.
  - Not selected → render today's single-row representation.
- Wire the collapse-toggle through `localStorage` (key per
  Scope).

### Phase 4 — backend `mission_update`

- A `mission_update` Tauri command may already exist for rename. If
  yes, extend its payload to accept `goal_override?: string | null`
  and `cwd?: string | null`. If no, add it.
- Validation: trim goal; reject cwd that doesn't exist (return
  error → frontend reverts the edit, surfaces a toast).
- Audit: persist the change to the `missions` row only; no event
  log append unless we decide goal changes should be observable
  to the running crew. v1: no event log, no propagation to
  agents mid-mission. The new value applies to future spawns /
  resumes; in-flight agents keep the original goal in their
  context.

### Phase 5 — verification

- Backend: extend (or add) `mission_update` tests for the new
  fields, including the "cwd must exist" validation.
- Frontend manual smoke:
  1. Start a mission with goal "Wire up event bus watcher" and cwd
     `~/go/src/.../runner`. Sidebar selected row expands; shows the
     goal, cwd, crew name, "started 14m ago".
  2. Click goal → edit → change to "Wire up event bus watcher (v2)"
     → blur → row updates, change persists across reload.
  3. Click cwd pick → choose another folder → cwd row updates.
  4. Pick an invalid (deleted) folder via picker — surface the
     backend's validation error as a toast; row reverts.
  5. Collapse via chevron → next page navigation back to this
     mission → row stays collapsed (per-mission preference).
  6. Light theme: detail panel reads correctly in both themes.

## Verification

- [ ] Selected mission row in the sidebar shows goal, cwd, crew,
      started time.
- [ ] Non-selected missions render unchanged (single-row).
- [ ] Goal editable inline; blur commits, Esc cancels.
- [ ] cwd pick via folder dialog; invalid pick rejected with a
      toast.
- [ ] Crew name links to the crew detail page.
- [ ] Started time updates on a 60s tick.
- [ ] Manual collapse persists per-mission via localStorage.
- [ ] Renders correctly in both light (spec 15) and dark themes via
      existing token utilities.
- [ ] `pnpm exec tsc --noEmit` clean; backend tests for
      `mission_update` pass.

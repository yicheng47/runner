# 01 — Archived tab

> Tracking issue: [#31](https://github.com/yicheng47/runner/issues/31)

## Motivation

Both missions and chats can be archived today (`mission_archive` /
`session_archive`, `missions.status='archived'` / `sessions.archived_at`),
but the only user-facing surface is the sidebar — and the sidebar lists
*active* items only. Once you archive something it disappears with no way
back: no view, no unarchive, no recovery short of editing the SQLite db
by hand. The data is there, the UX isn't.

This feature adds the read-and-restore surface that closes that loop.

## Scope

### In scope (v1)

- A new top-level route `/archived` that lists every archived mission and
  every archived chat (direct session) the user has, grouped into two
  sections. Each row is keyboard-focusable and clickable.
- An entry-point in the sidebar bottom area (near Settings) that
  navigates to `/archived`. Visible always, not gated on having any
  archived items — tells the user the door exists.
- Two backend commands:
  - `mission_list_archived(crew_id?)` — mirrors the active `mission_list`
    shape but filters to `status='archived'`, ordered by `archived_at`
    desc (most-recently-archived first; the column lives in the
    `mission_archive` write path already, just not surfaced).
  - `session_list_archived()` — direct sessions with
    `archived_at IS NOT NULL`, ordered by `archived_at` desc. Mirrors the
    `session_list_recent_direct` row shape.
- Per-row actions:
  - **Open** — navigate to the existing mission workspace / chat page.
    The workspace already handles archived missions (status badge,
    no-write affordances); the chat page renders an archived session as
    a read replay since the live PTY is gone.
  - **Unarchive** — calls a new `mission_unarchive(id)` /
    `session_unarchive(id)` command which clears the archive marker
    (sets `status` back to `stopped` / `complete`, or `archived_at = NULL`)
    and emits a `mission/unarchived` / `session/unarchived` event so the
    sidebar reinstates the row in real time without a page refresh.
- Empty state: "Nothing archived yet" with a one-liner explaining how
  archive works and the keyboard shortcut.

### Out of scope (deferred)

- **Permanent delete.** No data-deletion UX precedent in the app yet;
  spec it separately so we can decide cascade rules (event logs,
  signal-types sidecars, mission_dir on disk) without rushing.
- **Search / filter.** If lists grow long enough to need this, add a
  simple substring filter; not v1.
- **Bulk select / bulk unarchive.** Single-row actions only in v1.
- **Surfacing archived crews or runners.** Out of scope — only missions
  and chats archive today.

### Key decisions

1. **Dedicated route, not a tab inside the sidebar.** The sidebar is
   already dense; squeezing a collapsible "Archived" section under the
   live lists would either eat scroll space or hide itself. A separate
   page also gives room for the two-section layout.
2. **Actions stay row-local.** No top-level "unarchive all" button —
   archive is per-item, unarchive should be too.
3. **Backend split into new commands instead of overloading the existing
   ones.** `mission_list` and `session_list_recent_direct` are called on
   every sidebar event; adding an `archived` flag would make every
   caller think about a state they don't care about, and risks
   accidentally widening the active-list query if a callsite forgets to
   pass the filter. New commands are clearer.

## Implementation phases

### Phase 1 — Backend list + unarchive commands

- New Rust commands in `src-tauri/src/commands/mission.rs`:
  `mission_list_archived(crew_id: Option<String>)` and
  `mission_unarchive(id: String)`.
- New Rust commands in `src-tauri/src/commands/session.rs`:
  `session_list_archived()` and `session_unarchive(session_id: String)`.
- Wire all four into the `tauri::generate_handler![]` block in `lib.rs`.
- Unit tests: archived list returns archived rows only; unarchive clears
  the marker and is idempotent on an already-active row (no-op, no error).
- Emit `mission/unarchived` / `session/unarchived` events from the unarchive
  commands so the sidebar can refresh without polling.

### Phase 2 — Frontend route + sidebar entry

- Add `/archived` to the React Router config inside the `AppShell`
  layout route (so the sidebar stays mounted — same reason
  `MissionWorkspace` and friends sit there).
- New page `src/pages/ArchivedView.tsx` with two sections (`<MissionList>`
  and `<ChatList>`) sharing a row component that takes name/timestamp/
  open-handler/unarchive-handler. Empty state when both lists are empty.
- Sidebar bottom-area button: "Archived" with the `Archive` lucide icon,
  routes to `/archived` via `useNavigate`. Sits above Settings.
- Row click → `useNavigate` to the existing workspace or chat route.
- Unarchive button → calls the new API methods, optimistically removes
  the row from the local list, sidebar refreshes from the event.

### Phase 3 — Pencil design + polish

- Design the route in `design/runners-design.pen` first (per project
  convention) — header, two-column or stacked layout, row hover state,
  empty state. Get sign-off before implementing styling.
- Wire keyboard navigation: ↑/↓ moves focus between rows, `Enter` opens,
  `U` unarchives the focused row.
- Confirmation modal on unarchive? Decide during design — leaning no,
  since unarchive is reversible (re-archive is one click).

## Verification

- [ ] `cargo test -p runner` — new tests cover list filtering and
      unarchive idempotency.
- [ ] Archive a mission via the sidebar kebab → it disappears from the
      sidebar and appears at the top of the Archived view's Missions
      section.
- [ ] Click the row → workspace opens with the archived status badge.
- [ ] Click Unarchive → row leaves the Archived view, mission reappears
      in the sidebar without a page refresh.
- [ ] Same flow for chats (direct sessions).
- [ ] Empty state renders correctly when nothing has ever been archived
      (fresh DB).
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.

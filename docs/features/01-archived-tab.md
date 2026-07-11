# 01 — Archived (settings pane)

> Tracking issue: [#31](https://github.com/yicheng47/runner/issues/31)

## Motivation

Both missions and chats can be archived today (`mission_archive` / `session_archive`, `missions.status='archived'` / `sessions.archived_at`), but the only user-facing surface is the sidebar — and the sidebar lists *active* items only. Once you archive something it disappears with no way back: no view, no unarchive, no recovery short of editing the SQLite db by hand. The data is there, the UX isn't. This feature adds the read-and-restore surface that closes that loop.

**Direction change (2026-07-11):** originally specced as a standalone `/archived` route with its own sidebar entry. Superseded: the surface now lives inside the full-page Settings (impl 0025) as a dedicated pane, following the ChatGPT/codex desktop "Archived tasks" settings page — an **Archived** nav group at the bottom of the settings sidebar, below System. No new top-level menu or sidebar button.

## Surface (per codex reference design)

- Settings sidebar gains a fourth nav group **Archived** with one item, **Archived chats** (icon: `archive`), routed at `/settings/archived`.
- Content column: page title, a "Search archived…" field, and a **type segmented control** (All | Missions | Chats — one page, no per-type tabs; a segmented shows all states at a glance where a dropdown hides them).
- **One flat list, newest-archived first.** No project filter or grouping: Runner has no project entity — cwd is metadata, not identity — so the codex "All projects" dropdown doesn't map. The cwd basename renders as row metadata instead, and search matches **title + cwd**, which covers the "only quill stuff" case. If archived lists grow unwieldy, directory grouping is the v2 lever.
- Row = title + type glyph + archived-at timestamp + cwd basename (mono, dim); right side: **Unarchive** button and a per-row **trash** (permanent delete). Row click opens the existing workspace / chat page.
- Page-level **Delete all** action, top-right, destructive-tinted, confirm-gated.
- Empty state: "Nothing archived yet" with a one-liner explaining archive.

## Scope

### In scope (v1)

- The settings pane above, with search (client-side substring over titles), both filters, and project grouping.
- Backend list commands: `mission_list_archived(crew_id?)` (mirrors `mission_list` shape, `status='archived'`, ordered `archived_at` desc) and `session_list_archived()` (`archived_at IS NOT NULL`, mirrors `session_list_recent_direct` shape).
- Per-row **Open** (navigate to existing workspace/chat; both already render archived state) and **Unarchive** — new `mission_unarchive(id)` / `session_unarchive(id)` clearing the marker and emitting `mission/unarchived` / `session/unarchived` so the app sidebar reinstates the row live.

### In design, staged for v1.5 (ship behind review)

- **Permanent delete** (per-row trash + Delete all). Backend `mission_delete(id)` / `session_delete(id)` with cascade semantics **to be decided**: mission event log (NDJSON on disk), signal sidecars, `mission_dir`, session rows referencing runners. Confirm-gated in UI. If cascade questions drag, v1 ships unarchive-only with the delete affordances hidden — the pane layout doesn't change.

### Out of scope (deferred)

- Bulk select / bulk unarchive (Delete all is the only bulk action).
- Surfacing archived crews or runners — only missions and chats archive today.

### Key decisions

1. **Settings pane, not a route with a sidebar entry.** Supersedes the original decision — the full-page settings (impl 0025) now exists and is the natural home for low-frequency management surfaces; the app sidebar stays reserved for live work. Mirrors codex's Archived-tasks-in-settings placement.
2. **Group by project, filter by type.** Replaces the original two-section (Missions/Chats) layout: grouping by cwd matches how the user thinks about old work, and the type dropdown covers the section split.
3. **Backend split into new commands instead of overloading the active-list ones.** Unchanged from the original spec: `mission_list` / `session_list_recent_direct` run on every sidebar event; a forgotten `archived` flag at one callsite would silently widen the active lists.
4. **Unarchive is not confirm-gated; delete is.** Unarchive is reversible (re-archive is one click); delete is not.

## Feasibility audit (2026-07-11)

Verified against the current code that unarchive is a single UPDATE per type — nothing in the archive paths destroys what restore needs.

- **Chats**: `session_archive` stamps `archived_at` (refused while running) and purges only the in-memory scrollback ring; row + `agent_session_key` + agent JSONL survive. Unarchive = clear `archived_at`; the row rejoins `session_list_recent_direct` and Resume keeps working.
- **Missions**: `mission_archive` kills PTYs, then `stop()` atomically sets `status='completed' + stopped_at + archived_at` and appends the terminal `mission_stopped` log event; event log and slot-session rows (stopped, not archived) survive on disk/in db. Unarchive = clear `archived_at` only; status stays `completed`. Note: today "stop == archive" by design — `completed AND archived_at IS NULL` exists only as migration backfill — so unarchive reintroduces a state every surface already renders, and `mission_reset` already clears the column (precedent). Unarchived missions return as completed, per-slot resumable.
- **No orphans possible**: `crew_delete` refuses with non-archived missions and cascade-deletes archived ones (`missions.crew_id ON DELETE CASCADE`, sessions pre-deleted); `runner_delete` refuses with unarchived chats and hard-deletes the runner's session rows including archived ones. Archived items die with their parent — the pane never shows a dangling row.
- **Phase 4 input**: delete-cascade precedent already exists in `crew_delete` / `runner_delete`; and crew deletion currently leaks `mission_dir` NDJSON logs on disk (db rows go, files stay) — `mission_delete` must remove its dir, and the existing leak deserves a fix of its own.

## Implementation phases

### Phase 1 — Backend list + unarchive commands

- `mission_list_archived` / `mission_unarchive` in `src-tauri/src/commands/mission.rs`; `session_list_archived` / `session_unarchive` in `src-tauri/src/commands/session.rs`; register in `lib.rs`.
- Emit `mission/unarchived` / `session/unarchived` events; sidebar refreshes without polling.
- Unit tests: archived lists filter correctly; unarchive clears the marker and is idempotent on an already-active row.

### Phase 2 — Settings pane

- New `ArchivedPane` under `src/components/settings/`, nav group **Archived** appended to the settings sidebar, route `/settings/archived`.
- Search field + type/project filter dropdowns (client-side over the two fetched lists), project grouping with count badges, row component (title, type glyph, timestamp, Unarchive), empty state.
- Row click navigates; unarchive optimistically removes the row and lets the event refresh the app sidebar.

### Phase 3 — Pencil design + polish

- Design the pane in `design/runner-setting.pen` alongside the other settings screens (codex Archived-tasks reference) before styling lands.
- Keyboard nav: ↑/↓ moves row focus, `Enter` opens, `U` unarchives.

### Phase 4 — Permanent delete (gated on cascade decisions)

- Decide cascade rules (event log files, sidecars, mission_dir, foreign rows); implement `mission_delete` / `session_delete` + confirm dialogs; unhide the trash / Delete all affordances.

## Verification

- [ ] `cargo test --workspace` — list filtering + unarchive idempotency covered.
- [ ] Archive a mission via the sidebar kebab → it appears at the top of its project group in Settings → Archived.
- [ ] Row click → workspace opens with the archived status badge; same for chats.
- [ ] Unarchive → row leaves the pane, item reappears in the app sidebar without refresh.
- [ ] Search narrows rows; type and project filters compose.
- [ ] Empty state renders on a fresh DB.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint` clean.

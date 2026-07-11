# Archived pane in Settings — list + unarchive for missions and chats

## Status

Planned. Feature #31, direction revised 2026-07-11: a pane inside the full-page Settings (impl 0025), not a standalone route. Spec: `docs/features/01-archived-tab.md` (includes the feasibility audit). Design: `design/runner-setting.pen`, screen "Settings — Archived" (`FGYcY`) — committed on this branch.

## Problem

Archiving a mission or chat makes it permanently invisible: every list surface filters `archived_at IS NULL` and nothing can clear the marker short of hand-editing SQLite. The data survives archive entirely (audit in the feature spec: rows, `agent_session_key`, mission event logs, slot-session rows all intact), so this is purely a missing read-and-restore surface.

## Design summary (source of truth: spec + `FGYcY`)

Settings sidebar gains a fourth group **Archived** → item **"Archived chats & missions"** (`archive` icon), routed at `/settings/archived`, present on every settings screen. Content: page title (+ a destructive **Delete all** button in the design — **not rendered in v1**, see Non-Goals); "Search archived…" field; a segmented **All | Missions | Chats** type filter (no per-type tabs, no project filter — Runner has no project entity); **one flat card**, newest-archived first. Row = title, type glyph (`rocket` = mission, `message-square` = chat), archived-at timestamp, cwd basename in dim mono; right side an **Unarchive** button (plus a trash icon in the design — v1-hidden). Row click navigates to the existing workspace/chat page. Empty state: "Nothing archived yet" + one-liner.

## Key decisions

1. **Unarchive is `archived_at = NULL`, nothing else.** Missions keep `status='completed'` — today "stop == archive" (`stop()` is only reached via `mission_archive` and stamps both atomically), so unarchive reintroduces the `completed AND archived_at IS NULL` state that migration backfill already created and every surface already renders. `mission_reset` already clears the column (`repo/mission.rs` reset path), so this is established territory, not a new state machine. Sessions: same single-column clear. Unarchive is idempotent — clearing an already-NULL column updates 0 rows and returns Ok, not an error.
2. **New commands, not flags on active-list commands** (unchanged from the original spec): `mission_list_archived(crew_id?)`, `mission_unarchive(id)`, `session_list_archived()`, `session_unarchive(session_id)`. The active lists run on every sidebar event; an optional `archived` flag risks a forgotten filter silently widening them.
3. **Reuse existing event channels for live sidebar refresh.** The sidebar already listens to `mission/changed` (`Sidebar.tsx:472`) and `session/updated` (`Sidebar.tsx:563`) — unarchive emits those, and the restored row reappears with zero new listeners. (Supersedes the spec's `*/unarchived` event names; same semantics, less plumbing.)
4. **No orphan handling needed.** `crew_delete` cascade-deletes archived missions (`missions.crew_id ON DELETE CASCADE` + explicit session pre-delete); `runner_delete` hard-deletes the runner's session rows including archived ones. The pane can never list a row whose parent is gone.
5. **Merge + filter client-side.** Both lists arrive in one fetch each on pane mount; merged and sorted by `archived_at` desc in the component. Search matches title + cwd (covers the "only quill stuff" case without a project filter). No pagination — archived counts are personal-tool scale; revisit only if it bites.
6. **Delete affordances ship dark in v1.** The design includes per-row trash and Delete all; v1 simply doesn't render them (no flag machinery). Permanent delete is Phase 4 of the feature spec, gated on cascade decisions — note `crew_delete` already leaks `mission_dir` NDJSON logs on disk today; `mission_delete` must clean its dir when it lands.

## Goals

- Settings → Archived lists every archived mission and chat, newest first, searchable, type-filterable.
- Unarchive returns the item to its sidebar list live (no refresh) and keeps it fully functional: archived-then-unarchived chats can resume (`agent_session_key` untouched); unarchived missions open as completed workspaces with per-slot resume.
- Row click opens the existing mission workspace / chat page (both already render archived/completed states).
- Empty state on fresh DBs.

## Non-Goals

- Permanent delete (trash / Delete all render nothing in v1; feature-spec Phase 4).
- Bulk unarchive, pagination, project/directory grouping.
- Archived crews or runners (nothing else archives today).
- Keyboard nav (↑/↓/Enter/U) — polish; ride-along if cheap, else follow-up.

## Implementation phases

### Phase 1 — backend

- `repo/mission.rs`: `list_archived(crew_id?)` (mirror active list shape, `archived_at IS NOT NULL`, order `archived_at DESC`) + `unarchive(id)` (single-column clear, returns affected count). `repo/session.rs`: `list_archived()` (mirror `session_list_recent_direct` row shape) + `unarchive(session_id)`.
- Commands in `commands/mission.rs` / `commands/session.rs`: the four from Key Decision 2; emit `mission/changed` / `session/updated` after successful unarchive; register in `lib.rs`.
- Tests: archived lists exclude active rows and vice versa; unarchive clears the marker; unarchive on an active row is a no-op Ok; mission unarchive leaves `status='completed'` and `stopped_at` untouched.

### Phase 2 — api + pane

- `src/lib/api.ts`: `mission.listArchived` / `mission.unarchive` / `session.listArchived` / `session.unarchive` wrappers.
- `src/components/settings/ArchivedPane.tsx`: fetch-on-mount, merged recency list, search field, segmented filter (reuse the segmented pattern from `AppearancePane`), row component per design (type glyph, timestamp, cwd, Unarchive), optimistic row removal on unarchive, empty state.
- `SettingsPage.tsx`: add the **Archived** nav group + pane route per the design (group sits below System on every screen).

### Phase 3 — navigation + verification

- Row click → `useNavigate` to `/missions/:id` or `/chats/:id` (both handle archived rows via unfiltered `get`s already).
- Checks: `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual smoke (user-run): archive a mission and a chat → both appear in Settings → Archived, newest first, correct glyphs/cwd; search and segmented filters compose; unarchive each → reappears in the sidebar without refresh; unarchived chat resumes; unarchived mission opens as completed and a slot resumes; fresh-DB empty state.

## Relevant code

- `src-tauri/src/commands/mission.rs:1468` — `mission_archive_impl` (kill → `stop()` → unmount); `stop()` at ~289 with the stop==archive comment; `repo/mission.rs:125-160` — `complete_and_archive_if_running` + reset precedent for clearing `archived_at`.
- `src-tauri/src/commands/session.rs:378` — `session_archive`; `repo/session.rs` — `archive` (pattern for `unarchive`).
- `src/components/Sidebar.tsx:472,555,563` — `mission/changed` / `session/archived` / `session/updated` listeners (refresh channels).
- `src/components/settings/` + `src/pages/SettingsPage.tsx` — pane registry and shells from impl 0025; `AppearancePane.tsx` — segmented control pattern.
- `design/runner-setting.pen` — screen `FGYcY`; sidebar Archived groups on all screens.
- `docs/features/01-archived-tab.md` — spec + feasibility audit.

## Open questions

- **MCP parity**: the MCP server exposes `mission_archive` but would lack `mission_unarchive`. Cheap to add alongside Phase 1; decide at review whether to include (leaning yes — this session has personally wanted it).

## References

- Feature #31 (+ 2026-07-11 direction-change comment) · impl 0025 (settings surface) · codex desktop "Archived tasks" (visual reference).

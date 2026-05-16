# 17 — Sidebar folders (Arc-style mixed groupings)

> Tracking issue: [#136](https://github.com/yicheng47/runner/issues/136)

## Motivation

The sidebar today is organized by *type*: a "MISSIONS" section and a
"DIRECT CHATS" section, each a flat list. The grouping is enforced —
the user has no say in it. That falls down once a user has more than
a handful of items, because the right unit of organization isn't
"is this a mission or a chat" — it's *which project does this
belong to*.

A typical workflow has, say:

- Quill (the ebook reader) — mission "fix sync regression", direct
  chat with `@architect` for design questions.
- Runner itself — mission "wire up event bus watcher", direct chat
  with `@reviewer` for ad-hoc review.
- Weave (the trading platform) — mission "backfill 2026 data", two
  direct chats with `@strategist` and `@impl`.

Today these six items live in two flat lists, sorted by recency or
status, with no way to see "everything related to Quill" together.
The user has to mentally re-group on every scroll.

Arc's tab management has the right shape: **user-defined folders**
that hold tabs irrespective of their type. We adopt the same pattern
for missions + direct chats. The user creates folders, drops items
in, and the sidebar reflects their mental model.

This is "ideas backlog" territory — recorded for when the P1/P2
queue clears. It's a meaningful architectural change (new table,
drag-and-drop UI, migration of existing rows), not a quick win.

## Scope

### In scope (v1)

- **`folders` table.** New SQLite table:
  ```sql
  CREATE TABLE folders (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      position INTEGER NOT NULL,
      created_at TEXT NOT NULL
  );
  ```
- **Nullable `folder_id` on `missions` and `sessions`.** New column
  on each, FK to `folders.id ON DELETE SET NULL`. NULL means the
  item lives in the implicit "Inbox" pseudo-folder at the bottom of
  the sidebar.
- **Sidebar UI restructure.** The top-level "MISSIONS" / "DIRECT
  CHATS" sections go away. The sidebar becomes:
  - WORKSPACE nav rows (Runners / Crews / Search) — unchanged.
  - A list of user-defined folders, each collapsible, ordered by
    `position`.
  - Folder header: name + count badge + caret. Click name to rename
    inline; right-click for context menu (rename / delete / move
    up / move down).
  - Folder body: mixed list of mission rows and direct-chat rows,
    rendered with the same row components they use today (no
    visual change to individual rows).
  - "Inbox" pseudo-folder at the bottom: anything with NULL
    folder_id. Always present, can't be deleted or renamed.
- **CRUD affordances.**
  - "New folder" button at the bottom of the folder list (or `+`
    next to the Inbox header).
  - Rename inline via double-click on header or context menu.
  - Delete: confirmation modal, items inside revert to `folder_id =
    NULL` (Inbox).
  - Reorder folders by drag on the folder header.
- **Drag items between folders.** HTML5 drag-and-drop on rows; drop
  target = folder body or header. Updates `folder_id` and persists
  via `mission_update` / `session_update`.
- **Persist collapse state per-folder** in `localStorage` —
  `runner.sidebar.folder.collapsed.<folderId>`.
- **Migration on upgrade.** Existing missions and chats start in
  Inbox (NULL folder_id) — no folder is auto-created. The user
  builds their grouping from scratch.

### Out of scope (deferred)

- **Nested folders.** A folder cannot contain another folder in
  v1. Most users get value from a single layer; nesting doubles
  every CRUD path's complexity.
- **An item in multiple folders.** v1 = single-folder-per-item.
  Many-to-many is rarely needed and balloons UI ("where do I find
  this when it's in three places?").
- **Folder color / icon.** Arc has colored folders and emoji
  icons. v1 ships text-only; we can add a color picker later
  without breaking the schema (add `color TEXT` column with
  default NULL).
- **Auto-folder rules.** "All chats with @architect go to
  Quill" — useful but a separate feature.
- **Folder-scoped notifications / search.** Not in v1; the
  spec-14 notification path and the search palette stay global.
- **Folders for runner templates or crews.** Out — those are
  managed differently (Runners / Crews top-level pages, not the
  sidebar list).
- **Shared folders across machines.** Runner has no sync surface;
  folders are local-only.

### Key decisions

1. **Replace the MISSIONS / DIRECT CHATS split, don't layer on top
   of it.** The user's explicit motivation is rejecting the
   forced-by-type grouping. Keeping MISSIONS/DIRECT CHATS as a
   parallel taxonomy below user folders would create two
   inconsistent organizational systems competing for the same
   sidebar real estate. Commit to folders.
2. **Inbox pseudo-folder is the bottom default.** Always present,
   can't be deleted. New items land there. Users who don't engage
   with folders see a flat list under "Inbox" and lose nothing.
3. **Single-folder-per-item.** v1 simplicity. The schema (FK
   column on the item table) is the smallest change that
   delivers value. Many-to-many is a one-migration upgrade if
   demand surfaces.
4. **No auto-migration of existing rows.** Easy to migrate:
   leave everyone in Inbox, let the user organize. Auto-creating
   a "Default" folder per crew or mission age would be
   presumptuous.
5. **Folder operations go through existing Tauri commands.**
   `folder_create` / `folder_rename` / `folder_delete` /
   `folder_reorder` are new CRUD; assignment uses the existing
   `mission_update` / `session_update` (extended to accept
   `folder_id`). No new event types — folders are pure UI
   organization, not part of the agent protocol.
6. **HTML5 drag-and-drop, not a library.** The sidebar is the
   only place that needs drag-and-drop. A library
   (`@dnd-kit/sortable`, `react-beautiful-dnd`) adds bundle size
   for one surface. Native HTML5 drag events + a small
   `useDragDrop` hook covers the cases we need (drop target
   highlighting, drop-to-reorder, drop-to-folder-body).

## Implementation phases

### Phase 1 — schema + commands

- Migration `0010_folders.sql` (or next free number):
  ```sql
  CREATE TABLE folders (...);
  ALTER TABLE missions ADD COLUMN folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL;
  ALTER TABLE sessions ADD COLUMN folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL;
  ```
- New `commands/folder.rs`:
  - `folder_create(name) -> Folder`
  - `folder_list() -> Vec<Folder>` (ordered by position)
  - `folder_rename(id, name)`
  - `folder_delete(id)` (items inside revert to NULL via FK ON
    DELETE SET NULL)
  - `folder_reorder(id_order: Vec<String>)` — atomic update of
    the `position` column for the supplied id list.
- Extend `mission_update` and `session_update` to accept
  `folder_id?: Option<Option<String>>` (the outer Option is
  "field provided"; the inner is "set to NULL").

### Phase 2 — types + API

- New `Folder` type in `src/lib/types.ts`.
- `src/lib/api.ts`: new `folder` namespace with create/list/rename/
  delete/reorder; extend `mission.update` and `session.update`.

### Phase 3 — sidebar UI

- `Sidebar.tsx`: rip out the MISSIONS / DIRECT CHATS sections,
  replace with `<FolderList>` + the Inbox pseudo-folder.
- New `FolderRow.tsx`: header (name, count, caret, ctx menu) +
  collapsible body containing `<MissionRow>` / `<DirectChatRow>`
  children. Reuses the existing row components — no rendering
  changes to individual items.
- New `useDragDrop` hook in `src/lib/dnd.ts` — wraps HTML5
  `dragstart` / `dragover` / `drop` for the two move kinds we
  need (item-to-folder, folder-to-folder reorder).
- Inline rename via `contentEditable` or controlled `<input>` on
  the header.

### Phase 4 — verification

- Backend tests: CRUD round-trip on `folder_create/list/rename/
  delete/reorder`; cascading `folder_id = NULL` on folder delete.
- Manual smoke:
  1. Fresh install with existing missions + chats. All show up
     in Inbox.
  2. Create folder "Quill". Drag a mission and a chat into it.
     Both render together inside the folder.
  3. Rename folder. Persists.
  4. Collapse "Quill" — sidebar shrinks; reopen the app — folder
     is still collapsed.
  5. Delete "Quill" with items inside. Confirmation modal. After
     delete, items reappear in Inbox.
  6. Drag a mission from one folder to another. Order persists
     across reload.
  7. Reorder folders via drag — position persists.
  8. Multi-window (spec 12): a folder change in window A reflects
     in window B's sidebar within the existing event-bus
     subscription's tick. (Verify the bus carries a
     `folder_changed` event or that the sidebar polls
     post-mutation.)

## Verification

- [ ] `folders` table + nullable FK columns on missions and
      sessions; migrations cleanly upgrade existing installs.
- [ ] Existing items appear under Inbox after upgrade; no
      auto-folder creation.
- [ ] User can create / rename / delete / reorder folders.
- [ ] Drag-and-drop moves items between folders and persists.
- [ ] Inbox pseudo-folder is always present and not editable.
- [ ] Folder collapse state persists per-folder via
      localStorage.
- [ ] `mission_update` / `session_update` accept `folder_id`
      including the set-to-NULL case.
- [ ] No new agent-protocol events; folders are pure UI.
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit`
      clean.
- [ ] Renders correctly in both light (spec 15) and dark themes.

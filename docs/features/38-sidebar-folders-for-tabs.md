# 38 — Sidebar folders for tabs

> Tracking issue: [#281](https://github.com/yicheng47/runner/issues/281).

## Motivation

The sidebar CHAT list currently mirrors the split structure of the chat surface: multi-pane tabs render as accordions exposing every member pane (impl 0023), and single chats render as flat rows. As the chat count grows this is noisy in the wrong direction — the pane rows duplicate what the chat surface already shows, while the list offers no way to group work by project or stream.

This feature inverts the hierarchy the sidebar presents: **Folder → Tab**. Folders are user-created, collapsible groups of tabs. A tab — single-pane or multi-pane — renders as exactly one row; individual panes never appear in the sidebar again. The sidebar becomes an organizational surface (what am I working on) instead of a structural echo of the layout picker (how are my panes split).

Supersedes the closed feature 17 / [#136](https://github.com/yicheng47/runner/issues/136) (Arc-style folders grouping missions + chats, replacing the sections): this design is narrower — folders group **chat tabs only**, inside the CHAT section — but its `folders` table sketch carries over.

## Scope

### In scope (v1)

- **Folder rows in the CHAT list**: name, tab count, expand/collapse chevron. Collapsed folders hide their tabs. Ungrouped tabs render at top level below the folders — no pseudo-"Inbox" folder.
- **Tab rows replace chat rows**: every chat belongs to a tab. A single chat is a single-pane tab (today's flat `SessionRow` becomes its row); a multi-pane tab shows its name plus a split-icon/pane-count badge. The impl 0023 accordion member list is removed.
- **Folder CRUD and tab ordering**: create, rename, collapse/expand. Create a new chat tab inside a folder from its `+` action. Move a tab into/out of a folder via row context menu or drag, and reorder tabs through the same drag interaction. Dragging shows an Arc-style destination divider at the exact persisted insertion point. Pinned tabs remain the first tier within each folder and within the ungrouped list; manual order is preserved inside each tier.
- **Folder delete archives its tabs (decided)**: deleting a folder archives every tab inside it — member sessions get `archived_at` and land in Settings → Archived, same as archiving chats individually. Tabs never silently drop to top level. A single confirmation states the tab count and archive behavior before proceeding.
- **Backend persistence (decided)**: folders and tabs move into SQLite as user data next to sessions — migration `0009` adds `folders (id, name, position, collapsed, created_at)` and `tabs (id, folder_id → folders ON DELETE RESTRICT, name, position, layout, created_at)` where `layout` is a JSON column holding preset, slot→session-id assignments, and split sizes. No `SET NULL` orphaning: `folder_delete` archives the member tabs and removes the folder in one transaction. New `folder_*` / `tab_*` Tauri commands. The frontend `paneLayout` store hydrates from the DB and writes through, replacing the `runner.chat.layout` localStorage persist.
- **One-time import**: on first launch after the migration, the existing `runner.chat.layout` v2 payload seeds the `tabs` table; sessions not covered by the payload get single-pane tabs.

### Out of scope (deferred)

- Nested folders — one level only.
- Folders for missions — the MISSION section is untouched; this is the CHAT list.
- Dragging one chat tab onto another to create a split.
- Per-window folder/tab sets — the tab set stays global; which tab is active remains per-window view state.

### Key decisions

1. **Backend DB is the source of truth** (decided over extending localStorage). With panes gone from the sidebar, tabs are the thing users curate — names, folder membership, order — and that is user data, not view state. DB storage also retires the main-window-only persistence hack: today secondary-window tabs evaporate on restart because only `main` writes `runner.chat.layout` (`paneLayout.ts:451`); with write-through commands every window mutates the same rows. MCP tools gain visibility for free.
2. **Tabs get stable identity.** `PaneLayout` is positional today — no id. Folder membership needs a stable key, so tabs become ULID-keyed rows. This also unlocks later per-tab state (feature 33-style).
3. **Layout detail stays a JSON blob, not normalized pane rows.** Preset/slots/sizes go in one `layout` column; the frontend already sweeps dangling session ids after the chat-list fetch (`paneLayout.ts:825`), so FK-per-pane ceremony buys nothing.
4. **Ephemeral view state stays frontend**: per-window active tab, focused pane, route anchoring. Folder `collapsed` goes in the DB so collapse state syncs across windows and restarts through the one store.
5. **Cross-window sync keeps the existing shape**: mutations broadcast `chat/layout-changed` after the DB write, other windows re-hydrate — same event, DB-backed payload.

## Implementation Phases

### Phase 1 — schema + commands

Migration `0009_folders_tabs.sql`, repo layer, `commands/folder.rs` (`folder_create/list/rename/delete/set_collapsed/reorder`) and tab commands (`tab_upsert/list/delete/move_to_folder`).

### Phase 2 — store rewiring

`paneLayout.ts` hydrates from `tab_list`/`folder_list` instead of localStorage; every mutator writes through; one-time localStorage import; secondary windows gain write access; `chat/layout-changed` fanout re-pointed at DB hydration.

### Phase 3 — sidebar UI

Folder rows with expand/collapse and an add-tab action; tab rows without member panes (retire `ChatTabGroup`'s member list, fold single chats into single-pane tab rows); folder CRUD + context-menu/drag move-to-folder; drag reorder with a destination divider and pinned-first tiers; the single-confirm delete flow (archive-all semantics, tab count in the dialog).

### Phase 4 — cleanup + docs

Drop the localStorage persist path and update arch §3.6 (display grouping is no longer frontend-only).

## Open design questions

- Where folder creation lives: a `+` affordance on the CHAT section header, the context menu, or both.
- Whether tab rows inside a folder show the runner avatar/status the flat rows show today, or a slimmer row.
- Ordering rules: do pinned tabs float within their folder, globally, or both (interaction with #250 group pinning).
- Unarchive after a folder delete: restore the tab structure (tab rows would need their own `archived_at`), or just the sessions, re-wrapped as single-pane tabs (lean: sessions only — tab rows are deleted on archive, matching how archived chats leave the sidebar today).

## Design first

Per the design-first workflow, mock the folder rows, collapsed/expanded states, tab-row-with-pane-badge, and move-to-folder menu in `design/runner-mvp-design.pen` before coding.

## Verification (sketch)

- [ ] Create a folder, move two tabs in, collapse it → rows hide; restart the app → folder, membership, and collapsed state all restore.
- [ ] Multi-pane tab renders as one row with a pane badge; no member rows anywhere in the sidebar.
- [ ] Create a chat from a folder's `+` → its single-pane tab appears inside that folder; drag an existing tab onto or between folder rows → the destination divider matches the final position and membership/order persist after restart. Pinned tabs remain above unpinned tabs.
- [ ] Delete a folder → confirmation states the tab count; on confirm every tab inside is archived and its sessions appear in Settings → Archived; cancel leaves everything untouched. No tab ever falls to top level from a delete.
- [ ] Mutate the tab set from a secondary window → restart → the change survived (no more main-window-only persistence).
- [ ] Existing `runner.chat.layout` payload imports on first launch: prior tabs, names, and pane assignments intact.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.

# 44 — Sidebar node tree (unified nav model)

> Tracking issue: [#318](https://github.com/yicheng47/runner/issues/318)

## Motivation

The sidebar's containment mechanisms accreted one migration at a time, and each new concept invented its own: 0009 hard-coded a one-level structure split across two tables (`folders` + `tabs` via `folder_id` + `position`), 0010 bolted attention watermarks onto tabs, 0011 added a second, incompatible containment mechanism (pointer membership via `sessions.project_id` / `missions.project_id`), and pinning is a third (flags: `sessions.pinned`, `missions.pinned_at`). The result: ordering exists only where 0009's structure happens to provide it (tabs within folders), and every wanted behavior — reorder inside a project group, missions in folders, pinned reordering, mission drag — needs its own bespoke addition.

`folders` and `tabs` are the same idea — a positioned nav node — split into two tables. The fix is one **nodes** table: a single tree with one containment/ordering mechanism, where every sidebar row is a node.

The model (from the spec-43 discussion):

- **Nodes are navigation state; domain objects are content.** Folder and Tab are nav-native (no domain counterpart). Project and Mission stay domain objects; their nodes *reference* them. Sessions are never nodes — they're content behind a tab's layout slots or a mission's slots.
- **Exactly two leaf types**: `tab` (composes direct sessions into panes) and `mission` (references a crew run that owns its own composition). A chat never appears bare — it's always wrapped in a (possibly auto-created single-slot) tab, as `ensure_active_sessions` already guarantees.
- **Two container types**: `folder` (pure nav structure) and `project` (domain object rendered as a container).

Prereq shipped: #315 (unified nav scroll surface, PR #316). Sequencing: this remodel lands **before** #317 — the PINNED section then ships as the `pinned_position` derived view from day one instead of being built on the old pin columns and migrated. Until #317 adds the section, pinned rows keep sorting first inside their containers, driven by `pinned_position`.

## Scope

### Target schema

```sql
CREATE TABLE nodes (
    id                 TEXT PRIMARY KEY,
    parent_id          TEXT REFERENCES nodes(id) ON DELETE RESTRICT,  -- NULL = root
    position           INTEGER NOT NULL,          -- scoped to parent
    type               TEXT NOT NULL,             -- 'folder' | 'project' | 'tab' | 'mission'
    name               TEXT,                      -- folder/tab title (project/mission names live on their domain rows)
    ref_id             TEXT,                      -- projects.id / missions.id for reference types
    layout             TEXT,                      -- tab-only: pane layout JSON (as tabs.layout today)
    pinned_position    INTEGER,                   -- non-NULL = pinned; value orders the PINNED section
    last_completed_at  TEXT,                      -- tab-only attention watermarks (from 0010)
    last_viewed_at     TEXT,
    created_at         TEXT NOT NULL
);
```

- `parent_id` + `position` is the **single** containment/ordering mechanism — folder ordering, project-internal ordering, mixed tab/mission ordering all fall out of it.
- `pinned_position` as a nullable position (not reparenting into a PINNED container) gives pinned ordering *and* unpin-returns-to-origin without remembering origins. PINNED renders as a derived view over `pinned_position IS NOT NULL`.
- Collapse state stays frontend, per 0012's decision.

### Migration (cutover, one migration file)

- `folders` rows → `folder` nodes at root, keeping `position`.
- `tabs` rows → `tab` nodes: parent = the folder's node (or the owning project's node when every member session shares a `project_id`; root otherwise), carrying `layout` + watermarks.
- `projects` rows → `project` nodes at root, in sidebar order.
- Non-archived missions → `mission` nodes (parent = project node when `project_id` set, root otherwise).
- Pin flags → `pinned_position` seeded from the current pinned-first sort order; `sessions.pinned` / `missions.pinned_at` retire from sidebar use.
- Rename `folders` / `tabs` to `*_legacy` after the copy (dogfooding insurance against a migration bug); drop them in a later migration.

### Code impact

- Repo layer: `repo/node.rs` replaces `repo/folder.rs` + `repo/tab.rs`; `ensure_active_sessions` seeds tab nodes. Archive/restore needs no special handling — archiving already removes a chat from the structure (`remove_session`), and the same invariant-repair loop re-creates a node for a restored chat: parented under its project's node when `project_id` is set, appended at the parent's end (original position not remembered, matching today). Mission nodes follow symmetrically: created on `mission_start`, deleted on archive, re-created on unarchive.
- Sidebar renders from one tree: sections become derived views (PINNED = pinned overlay; PROJECT = project nodes; recent list = remaining roots), completing the MISSION/CHAT section merge.
- dnd collapses to one reparent/reposition operation for every drag: tab-into-folder, tab/mission-into-project, mission-into-PINNED, reorder anywhere.

### Key decisions

1. **Write-through on project boundaries.** The tree owns placement and order, but `sessions.project_id` / `missions.project_id` stay authoritative for domain membership (cwd binding, project scoping). Reparenting a tab or mission across a project boundary writes the pointer through. Considered and rejected: tree-only membership (breaks non-sidebar consumers of `project_id`) and pointer-derived children (re-creates today's no-ordering problem).
2. **Type-specific columns, not a payload blob.** Nullable `layout`/watermarks on one table over a JSON payload or per-type side tables — SQLite-pragmatic, matches how `tabs` already works. (`type` is a Rust keyword — the repo struct field maps it as `node_type` via serde rename, like the column-name indirection `serde_rusqlite` already handles.)
3. **Attention stays tab-scoped.** Watermarks move with the tab node; mission attention remains the live-activity roll-up. No unification of the two attention models.

### Out of scope

- Any change to the domain models (`sessions`, `missions`, `projects`, crews) beyond retiring the two pin columns from sidebar use.
- Nesting policy beyond today's shapes (no folders-in-folders, no projects-in-folders); the schema allows a general tree but the app enforces current depth.
- Mission-as-tab data unification (rejected in spec 43).

### To be decided

- Whether the recent/unfiled list is one interleaved run or grouped by kind.
- Exact dnd affordances for mission rows in v1 of this rework.

## Implementation phases

1. **Schema + repo layer** — `nodes` table, migration cutover from `folders`/`tabs`, `repo/node.rs`, seeding hooks (`ensure_active_sessions`, mission start/archive).
2. **Sidebar reads the tree** — sections as derived views over one query; MISSION/CHAT sections merge; pinned-first ordering driven by `pinned_position` (the PINNED section itself arrives with #317).
3. **Unified drag** — single reparent/reposition op wired to dnd for all row types; project write-through on cross-boundary moves.

## Verification

- [ ] Migration preserves every folder, tab (with layout + watermarks), project grouping, and pin, in the same visual order as before.
- [ ] Reorder works inside a folder, inside a project, in the recent list, and in PINNED — same interaction everywhere.
- [ ] Moving a tab into/out of a project updates member sessions' `project_id`; new chats in that project still inherit the right cwd.
- [ ] Unpinning returns a row to its tree position; pinned order survives restart.
- [ ] Mission nodes appear on `mission_start`, nest under their project, and leave on archive.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.

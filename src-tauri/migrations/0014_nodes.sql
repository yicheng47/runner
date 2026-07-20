-- Feature 44: one `nodes` table replaces `folders` + `tabs` + the
-- project_id-pointer grouping + pin flags as the sidebar's containment
-- and ordering mechanism. `parent_id` + `position` is the single tree;
-- containers are `folder` (nav-native) and `project` (references the
-- domain row); leaves are `tab` (pane layout JSON + attention
-- watermarks) and `mission` (references the crew run).
--
-- The SQL here copies rows 1:1 (node id = source row id — all ULIDs,
-- no cross-table collisions). The fiddly parts — resolving each tab's
-- project parent from its layout's member sessions (unanimous project
-- trumps a folder, as the old sidebar rendered it), seeding
-- `pinned_position` from the pin flags, and re-seeding `position` per
-- parent scope over today's visual sort — need JSON parsing and run in
-- the Rust backfill step (`db::backfill_0014_nodes`) inside the same
-- transaction.

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

CREATE INDEX idx_nodes_parent_position ON nodes(parent_id, position, created_at);

INSERT INTO nodes (id, parent_id, position, type, name, ref_id, layout,
                   pinned_position, last_completed_at, last_viewed_at, created_at)
SELECT id, NULL, position, 'folder', name, NULL, NULL, NULL, NULL, NULL, created_at
  FROM folders;

INSERT INTO nodes (id, parent_id, position, type, name, ref_id, layout,
                   pinned_position, last_completed_at, last_viewed_at, created_at)
SELECT id, NULL, position, 'project', NULL, id, NULL, NULL, NULL, NULL, created_at
  FROM projects;

INSERT INTO nodes (id, parent_id, position, type, name, ref_id, layout,
                   pinned_position, last_completed_at, last_viewed_at, created_at)
SELECT id, folder_id, position, 'tab', name, NULL, layout,
       NULL, last_completed_at, last_viewed_at, created_at
  FROM tabs;

INSERT INTO nodes (id, parent_id, position, type, name, ref_id, layout,
                   pinned_position, last_completed_at, last_viewed_at, created_at)
SELECT id, project_id, 0, 'mission', NULL, id, NULL, NULL, NULL, NULL, started_at
  FROM missions
 WHERE archived_at IS NULL;

-- Dogfooding insurance against a migration bug: keep the source tables
-- around renamed; a later migration drops them.
ALTER TABLE folders RENAME TO folders_legacy;
ALTER TABLE tabs RENAME TO tabs_legacy;

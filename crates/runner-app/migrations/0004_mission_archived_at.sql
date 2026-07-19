-- Adds `archived_at` to missions so the workspace can tell a completed
-- mission from one the operator explicitly archived. Mirrors the
-- sessions table's `archived_at` column and the same `archived_at IS
-- NULL` filter idiom used in `session_list_recent_direct`.
--
-- The `status` enum is left alone — a mission can still be running,
-- completed, or aborted. `archived_at IS NOT NULL` is the read-only /
-- hidden-from-search discriminator and lives on its own column so a
-- future split (e.g. listing aborted-but-not-archived runs) is a SQL
-- change, not a schema migration.
--
-- Backfill: today `mission_archive` is the only writer that flips a row
-- to `completed`, so every existing `completed` row was archived. Stamp
-- their `archived_at` with `stopped_at` so post-migration filters treat
-- them as archived. Aborted rows (failed spawn/mount during start) are
-- intentionally left with NULL `archived_at`: they were never
-- successfully archived and should not silently disappear from list().

ALTER TABLE missions ADD COLUMN archived_at TEXT;

UPDATE missions
   SET archived_at = stopped_at
 WHERE status = 'completed' AND archived_at IS NULL;

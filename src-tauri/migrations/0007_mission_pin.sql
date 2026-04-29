-- Migration 0007: pin column on missions.
--
-- Mirrors the per-row pin affordance the workspace UI already exposes
-- (Pin item in the mission context menu / topbar kebab). Pinned
-- missions sort to the top of the sidebar's MISSION list. Pure
-- additive DDL; existing rows default to NULL = unpinned.

ALTER TABLE missions ADD COLUMN pinned_at TEXT;

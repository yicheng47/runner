-- Folder nodes are flattened by `db::backfill_0015_retire_folders` in
-- this migration's transaction before startup can observe the tree.
-- The source tables retained by the 0014 cutover are no longer needed.

DROP TABLE tabs_legacy;
DROP TABLE folders_legacy;

-- Migration 0005: pin direct-chat sessions to the top of the SESSION
-- tray.
--
-- The Pencil design (node `P5CLA` inside `u6woG`) defines a per-session
-- context menu with Pin / Rename / Archive. Renaming uses
-- `sessions.title` (0004) and archiving uses `sessions.archived_at`
-- (0003); pinning needs its own column so the list query can sort
-- pinned rows above running ones regardless of activity time.
--
-- pinned_at is the timestamp the user pinned the session. NULL = not
-- pinned. Storing the timestamp (vs a boolean) lets a future "recent
-- pins" affordance order pins by when they were created without an
-- extra column.

ALTER TABLE sessions ADD COLUMN pinned_at TEXT;

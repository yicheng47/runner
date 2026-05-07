-- Adds runtime metadata columns to sessions for the tmux runtime
-- migration (docs/impls/0004-tmux-session-runtime.md, Step 3).
--
-- All columns nullable so existing rows survive the migration with
-- NULL meaning "legacy portable-pty session — runtime layer can't
-- reattach to this." Once Step 9 lands, the manager treats NULL
-- runtime as a stopped session for display purposes; reattach is
-- only attempted when the runtime metadata is fully populated.
--
-- Names use the runtime-agnostic `runtime_*` prefix rather than
-- `tmux_*` so a future native-pty runtime can repopulate the same
-- columns with its own identifiers without a second migration.

ALTER TABLE sessions ADD COLUMN runtime TEXT;
ALTER TABLE sessions ADD COLUMN runtime_socket TEXT;
ALTER TABLE sessions ADD COLUMN runtime_session TEXT;
ALTER TABLE sessions ADD COLUMN runtime_window TEXT;
ALTER TABLE sessions ADD COLUMN runtime_pane TEXT;
ALTER TABLE sessions ADD COLUMN runtime_cursor INTEGER;

-- Migration 0004: user-facing label on direct-chat sessions.
--
-- With multi-chat-per-runner (see docs/impls/direct-chats.md), a runner
-- can own several parallel direct sessions. The sidebar SESSION tray
-- needs to label them as something other than "@handle direct (1)" /
-- "@handle direct (2)". This column holds an optional user-authored
-- title; when NULL the UI falls back to "@handle · <relative-time>".
--
-- Mission sessions don't use this column (the mission row already has a
-- title), but the column is on `sessions` because keeping the schema
-- shape uniform avoids a sub-table for direct-only fields.

ALTER TABLE sessions ADD COLUMN title TEXT;

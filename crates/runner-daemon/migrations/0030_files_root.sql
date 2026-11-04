-- #634: the folder a session's or mission's Files view was rooted at with `files_set_root`.
-- files_root is an absolute path; NULL means the view opens on the session's or mission's own checkout. It goes with its row.

ALTER TABLE sessions ADD COLUMN files_root TEXT;
ALTER TABLE missions ADD COLUMN files_root TEXT;

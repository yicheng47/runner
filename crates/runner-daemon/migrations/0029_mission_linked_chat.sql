-- #748: a mission links to at most one chat, which it reports to and is grouped with.
-- linked_session_id is the linked direct agent chat; NULL means unlinked. Deleting the chat clears it.
-- report_to_chat is 1 when Runner types reports into the linked chat (the default) and 0 for `--no-report`; it is ignored while unlinked.

ALTER TABLE missions ADD COLUMN linked_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL;
ALTER TABLE missions ADD COLUMN report_to_chat INTEGER NOT NULL DEFAULT 1;

CREATE INDEX idx_missions_linked_session ON missions(linked_session_id);

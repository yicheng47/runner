-- Preserve runtime-only direct chat model and effort choices across
-- respawn. Runner-backed sessions continue to resolve these fields
-- from their persisted runner template.

ALTER TABLE sessions ADD COLUMN agent_model TEXT;
ALTER TABLE sessions ADD COLUMN agent_effort TEXT;

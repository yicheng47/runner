-- Migration 0006: runner-as-template + per-slot identity.
--
-- See docs/impls/crew-slots.md.
--
-- Pre-release, no data preservation. Existing crew_runners rows are
-- dropped; users re-add slots via the new Add Slot affordance.
--
-- Shape changes:
--   - DROP TABLE crew_runners.
--   - CREATE TABLE slots — same role as crew_runners (crew↔runner
--     join with position + lead) plus per-slot identity:
--       id          ULID PK so slots are individually addressable.
--       slot_handle in-crew identity used by mission events,
--                   RUNNER_HANDLE env var, router routing. Unique
--                   within a crew.
--     "At most one lead per crew" is now enforced in the slot
--     commands (transactional clear-others-then-set), not via a
--     partial unique index — keeps the SQL portable and the
--     invariant visible in code.
--   - ALTER TABLE runners DROP COLUMN role — role concept dropped
--     entirely in v0; slot_handle is the only per-slot identity.
--   - ALTER TABLE sessions ADD COLUMN slot_id — mission sessions
--     hook to a slot; direct chats stay NULL.

DROP TABLE crew_runners;

CREATE TABLE slots (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    slot_handle TEXT NOT NULL,
    position INTEGER NOT NULL,
    lead INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    UNIQUE (crew_id, slot_handle),
    UNIQUE (crew_id, position)
);

ALTER TABLE runners DROP COLUMN role;

ALTER TABLE sessions ADD COLUMN slot_id TEXT;

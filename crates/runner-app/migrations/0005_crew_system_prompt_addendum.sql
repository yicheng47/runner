-- Adds `system_prompt_addendum` to crews — Layer 2 of the
-- system-prompt stack (see issue #54). The platform preamble
-- (Layer 1, code) and the runner persona (Layer 3, data) bracket
-- this column; spawned mission workers see all three in order,
-- direct chats see only the persona.
--
-- Nullable / default NULL. Empty / NULL = no addendum, no splice
-- (current behavior). Seeded Build squad rows stay NULL — the
-- seeded persona content is already enough; no backfill.

ALTER TABLE crews ADD COLUMN system_prompt_addendum TEXT;

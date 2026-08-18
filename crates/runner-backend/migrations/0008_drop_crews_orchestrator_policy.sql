-- Drop `crews.orchestrator_policy` — deprecated in #247 (superseded by
-- `system_prompt_addendum`) and since then read-only: never written and
-- spliced into no prompt. Removing the column is behavior-neutral; the
-- frozen legacy values on existing rows are intentionally discarded.
--
-- SQLite 3.35+ supports `ALTER TABLE ... DROP COLUMN` directly; the
-- bundled rusqlite is fine.

ALTER TABLE crews DROP COLUMN orchestrator_policy;

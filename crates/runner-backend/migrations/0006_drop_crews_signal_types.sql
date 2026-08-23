-- Drop `crews.signal_types` — CLI validation now reads from the
-- code-side `runner_core::model::KnownSignalType` enum, so the
-- per-crew JSON allowlist column (and its `$APPDATA/.../signal_types.json`
-- sidecar, removed alongside in feature 20) no longer has a consumer.
--
-- SQLite 3.35+ supports `ALTER TABLE ... DROP COLUMN` directly; the
-- bundled rusqlite is fine. Existing sidecar files under
-- `$APPDATA/runner/crews/<id>/signal_types.json` are harmless leftovers;
-- a curious user can `rm` them.

ALTER TABLE crews DROP COLUMN signal_types;

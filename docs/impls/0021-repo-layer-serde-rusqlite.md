# 0021 — Lite ORM repo layer on serde_rusqlite

> Behavior-preserving reorganization of the persistence layer around persistent objects. No product-visible change, no schema change, no new stored byte formats; the existing test suite is the contract.

## Context

Persistence today is hand-written SQL spread across the command layer. Every table has a hand-rolled `fn row_to_X(&Row) -> rusqlite::Result<X>` mapper (`commands/crew.rs:75`, `commands/mission.rs:80`, `commands/slot.rs:113`, `commands/runner.rs:210`, `commands/session.rs:50`), each repeating the same ceremony: `row.get("col")?` per column, `String → DateTime<Utc>` parsing with a hand-built `FromSqlConversionFailure`, `match`-based enum decoding, and `serde_json::from_str` for JSON-in-TEXT columns. Writes are the mirror image: INSERT/UPDATE strings with positional `params![]` lists that must be kept in sync with the mappers by eye. Statement counts: `commands/mission.rs` 29, `commands/session.rs` 17, `commands/slot.rs` 9, `commands/crew.rs` 6, `commands/runner.rs` 4, plus writers outside the command layer (`session/manager/spawn.rs`, `session/manager/output.rs`, `session/pty_runtime.rs`, `session/codex_capture.rs`, `mcp/tools/crew.rs`) and `db.rs` seeds.

The cost is not the SQL — the queries are fine — it is the by-hand mapping between rows and structs: five mappers, dozens of param lists, and column lists duplicated between SELECTs and mappers with nothing keeping them aligned.

The model types in `model.rs` already derive `Serialize`/`Deserialize` (they double as Tauri IPC DTOs), which is exactly the contract `serde_rusqlite` needs. This plan adds a `repo/` module that owns all row mapping and SQL per table, with mapping derived from serde instead of written by hand.

## Goal

- One `repo::<table>` submodule per persistent object (`crew`, `runner`, `slot`, `mission`, `session`) owning that table's row struct, column list, and CRUD functions.
- Row↔struct mapping is derived (`from_row`, `to_params_named`), not hand-written. All five `row_to_*` mappers deleted.
- Callers (`commands/`, `mcp/tools/`, `session/`) do validation and orchestration and call `repo` functions; they no longer contain INSERT/UPDATE SQL or `rusqlite::Row` handling.
- `db.rs` shrinks to pool, migration runner, schema guard, and seeds.
- Stored byte formats stay identical to today (timestamps via `to_rfc3339`, JSON via `serde_json::to_string`), so old and new rows are indistinguishable and no migration is needed.

## Non-goals

- No async. The stack stays `rusqlite` + r2d2 + sync Tauri commands; async ORMs (ormlite, SQLx, SeaORM's async core) were evaluated and rejected — they color the whole command layer for zero benefit on a local single-file SQLite DB. `sea-orm-sync` (2.0, rusqlite backend) was the runner-up but is a heavyweight framework and a 6-month-old sync port; `serde_rusqlite` reuses the serde derives the models already have.
- No query DSL and no schema generation from structs. SQL stays hand-written and visible, just centralized in `repo/`.
- Migrations stay raw SQL files under `src-tauri/migrations/` run by `db.rs`. Seeds stay SQL.
- No change to the IPC surface: `model.rs` types keep their exact serde shape, `src/lib/types.ts` untouched, no frontend change.
- The event log (NDJSON via `runner-core`) is not a database and is untouched.
- No behavior change anywhere. If a step needs a non-mechanical test change, stop and reassess.

## Decisions

1. **`serde_rusqlite`, thin-repository style.** The crate gives `from_row::<T>(&Row)` / `from_rows`, `to_params_named(&T)` (+ `to_params_named_with_fields` for partial UPDATEs), and `columns_from_statement`. Confirmed type support: `bool ↔ INTEGER`, `Option<T>`, unit-variant enums ↔ TEXT (serde `rename_all` respected — matches `MissionStatus`/`SessionStatus` storing `'running'` etc.), `String`/`i64`/`f64`/`Vec<u8>`. Confirmed limits that shape this plan: `Vec<String>`, `HashMap`, and `serde_json::Value` are NOT usable as single columns; `u64`/`i128` unsupported. Version note: pick the `serde_rusqlite` release whose `rusqlite` dependency matches the workspace's `rusqlite` version exactly, or the `Row` types won't unify.
2. **Every table gets a `Row` struct in `repo/` — the persistent object.** Even where the model type's serde shape happens to match the table today (Slot, Mission, Session), the repo defines its own `XRow` mirroring columns exactly, with `From<XRow> for X` / `From<&X> for XRow` conversions. Rationale: (a) it decouples the DB representation from the IPC DTO permanently — Runner (`args`/`env` structs vs `args_json`/`env_json` TEXT) and Crew (`orchestrator_policy` JSON TEXT) prove the two shapes diverge; (b) it lets the row structs pin storage-stable serde helpers (decision 3) without touching what the frontend receives; (c) uniformity is the framework — every table has exactly one Row struct + one column list + CRUD functions, no per-table special cases to remember.
3. **Storage formats are pinned by shared serde helpers in `repo/serde.rs`, byte-identical to today.** `rfc3339` / `rfc3339_opt` serialize `Timestamp` via `to_rfc3339()` (the `+00:00` format every current write path uses) and deserialize via `str::parse` (accepts both RFC3339 offset spellings, so any historical rows are fine). `json_text` / `json_text_opt` are generic `#[serde(with)] `modules serializing any `Serialize + DeserializeOwned` field through `serde_json` to a TEXT column — used by `RunnerRow.args_json: Vec<String>`, `RunnerRow.env_json: HashMap<String, String>`, and `CrewRow.orchestrator_policy: Option<serde_json::Value>` (deprecated per #247/`285530d`: still read, never written — the Row struct keeps it so `SELECT *`-style reads and the serialized `Crew` shape are unaffected).
4. **Repo functions take `&Connection` (or `&Transaction` via `Deref`), never the pool.** Same convention as today's `crew::get(&conn, id)` — callers own connection acquisition and transaction boundaries, so repo calls compose inside the existing multi-statement transactions in `commands/mission.rs` and `commands/slot.rs` unchanged.
5. **Column lists are a per-table `const COLUMNS: &[&str]`** used to build SELECT lists and INSERT column/param lists, so the statement and the struct can't drift apart silently. A per-table unit test round-trips a fully-populated Row through INSERT → SELECT → `from_row` to prove the pairing.
6. **Join DTOs compose from per-table rows; `#[serde(flatten)]` is not trusted for row deserialization.** `SlotWithRunner` and `SessionRow` (session + `handle`/`runtime`/`lead`/`agent_session_key`) are read from JOIN queries today. serde's `flatten` buffers through an internal `Content` type that drops the type hints `serde_rusqlite` needs for `bool`-from-INTEGER and enum decoding — whether it works is version-dependent trivia we don't want to depend on. Instead: joined SELECTs alias each side's columns and the repo builds the DTO from two `from_row_with_columns` calls (or reads the handful of denormalized extras with plain `row.get`), then assembles the existing DTO struct in Rust. The DTO types themselves and their IPC shape don't change.
7. **Order the migration by blast radius, one phase per table, each phase independently green.** Slot first (smallest real surface, exercises bool/i64/timestamp and the join pattern via `slot_list`), then crew (JSON column + `CrewListItem` join), then runner (both JSON columns + the largest partial-UPDATE surface), then session (most writer call sites, spread across `session/` and `mcp/`), then mission last (biggest file, transaction-heavy paths including the `ensure_first_turn_fits` rollback). A stalled or reverted phase leaves everything before it shipped and everything after it untouched.

## Step 0: Dependency + helpers + spike

Files: `src-tauri/Cargo.toml`, new `src-tauri/src/repo/mod.rs`, new `src-tauri/src/repo/serde.rs`

- Add `serde_rusqlite` (version matched to the workspace `rusqlite` — see decision 1).
- Create `repo/mod.rs` (module wiring) and `repo/serde.rs` with `rfc3339`, `rfc3339_opt`, `json_text`, `json_text_opt` helper modules.
- Spike tests against an in-memory DB proving, before any migration: `bool ↔ INTEGER` round-trip; `MissionStatus`/`SessionStatus` ↔ lowercase TEXT; `Option<Timestamp>` via `rfc3339_opt` writing byte-identical strings to `to_rfc3339()` and reading legacy `+00:00` and `Z` spellings; `Vec<String>`/`HashMap`/`Option<Value>` via `json_text` writing byte-identical strings to the current `serde_json::to_string` calls; `to_params_named_with_fields` driving a partial UPDATE.

Validation: `cargo test -p runner repo` green. If any spike fails, resolve here — nothing else has moved yet.

## Step 1: Slot

Files: new `src-tauri/src/repo/slot.rs`, `src-tauri/src/commands/slot.rs`

- `SlotRow` + `COLUMNS` + CRUD (`insert`, `get`, `list_for_crew`, `list_for_runner`, `update`, `delete`, the position/lead helpers behind `slot_reorder`/`slot_set_lead`). Transaction-using callers keep their existing `tx` boundaries (decision 4).
- `slot_list`'s `SlotWithRunner` join moves to the aliased-columns pattern (decision 6), pulling the runner side via `repo::runner` once that exists — until then it may temporarily call the existing `row_to_runner`; the temporary seam is removed in Step 3.
  - **As implemented:** today's `slot::list` is not a JOIN — it is two queries (slot rows, then `runner::get` per slot) with a comment explaining the readability trade-off. Preserving behavior exactly meant keeping that two-query shape: Step 1 reads slot rows via `repo::slot::list_for_crew` and keeps calling `commands::runner::get` (the temporary `row_to_runner` seam); Step 3 makes `runner::get` repo-backed, which removes the seam without ever introducing an aliased JOIN here. The aliased-columns pattern is used where a real JOIN already existed (`list_crews_for_runner`, `CrewListItem` previews, session list/direct surfaces).
- Delete `row_to_slot`.

Validation: `cargo test -p runner`, focused on `commands::slot` + `db` tests.

## Step 2: Crew

Files: new `src-tauri/src/repo/crew.rs`, `src-tauri/src/commands/crew.rs`, `src-tauri/src/mcp/tools/crew.rs`

- `CrewRow` with `orchestrator_policy` behind `json_text_opt` (read-only field, never in INSERT/UPDATE column lists — matches `285530d`), `system_prompt_addendum`, timestamps via `rfc3339`.
- CRUD for create/update/get/list/delete; `CrewListItem`'s member-preview join stays one query, assembled per decision 6.
- Validation logic (`validate_crew_goal`, addendum normalization) stays in `commands/crew.rs` — the repo persists, it does not police.
- Delete `row_to_crew`.

Validation: `cargo test -p runner`, focused on `commands::crew` (goal-cap and addendum tests must pass unchanged).

## Step 3: Runner

Files: new `src-tauri/src/repo/runner.rs`, `src-tauri/src/commands/runner.rs`

- `RunnerRow` with `args_json`/`env_json` behind `json_text` and `From` conversions to/from `model::Runner` (the field-name divergence `args` ↔ `args_json` lives only here).
- Partial updates use `to_params_named_with_fields` against the existing update semantics (outer-`Option` = leave untouched).
- Delete `row_to_runner`; remove the temporary seam from Step 1.

Validation: `cargo test -p runner`, focused on `commands::runner` (including the `MAX_SYSTEM_PROMPT_BYTES` tests).

## Step 4: Session

Files: new `src-tauri/src/repo/session.rs`, `src-tauri/src/commands/session.rs`, `src-tauri/src/session/manager/spawn.rs`, `src-tauri/src/session/manager/output.rs`, `src-tauri/src/session/pty_runtime.rs`, `src-tauri/src/session/codex_capture.rs`, `src-tauri/src/mcp/tools/crew.rs`

- `SessionRowDb` (naming avoids the existing `commands::session::SessionRow` DTO) covering the sessions table including `agent_session_key` and the legacy runtime columns exactly as written today.
- This is the widest phase by call-site count but the statements are small (status flips, pid/stopped_at updates, key capture). Port them mechanically; do not consolidate or "improve" write paths — some run on the PTY hot path and their timing is load-bearing.
- The `SessionRow` DTO join keeps its shape, assembled per decision 6.
- Delete `row_to_session`.

Validation: `cargo test -p runner` (manager + session tests are the deep coverage here), plus a manual smoke: direct chat spawn/stop/resume, mission spawn, `agent_session_key` capture visible in the UI.

## Step 5: Mission

Files: new `src-tauri/src/repo/mission.rs`, `src-tauri/src/commands/mission.rs`, `src-tauri/src/mcp/tools/*.rs` (mission tools)

- `MissionRow` (status enum, four timestamp columns of which three nullable) + CRUD covering the 29 statements: start/stop/complete/abort transitions, archive/pin/rename, the `ensure_first_turn_fits` rollback-to-aborted paths, and the list/feed queries.
- Transaction boundaries in `mission_start`/`mission_reset` stay exactly where they are; repo calls take the `&Transaction`.
- Delete `row_to_mission`.

Validation: `cargo test -p runner` full; the mission lifecycle tests (including `ensure_first_turn_fits_guards_the_argv_ceiling` and the rollback tests) are the contract.

## Step 6: Sweep and gate

Files: `src-tauri/src/db.rs`, stragglers

- Port `db.rs` seed/test helpers to repo calls where that is a net simplification; leave migration SQL and the schema guard untouched.
- Gates, enforced by grep in the final review: no `fn row_to_` anywhere; no `INSERT INTO` / `UPDATE ` SQL outside `repo/`, `db.rs` (migrations/seeds/schema-guard), and test fixtures; `commands/` and `session/` contain no `rusqlite::Row` imports.

Validation: `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, and a full manual smoke (crew CRUD, runner CRUD, slot reorder, mission start → archive, direct chat, app restart with existing DB — the existing-DB restart specifically proves old-format rows read cleanly).

## Verification

- [ ] `cargo test --workspace` green after every step, not just at the end.
- [ ] Old databases open cleanly: timestamps in both RFC3339 spellings and all existing JSON TEXT values deserialize (Step 0 spike + Step 6 restart smoke).
- [ ] New writes are byte-identical to old writes for every column (spot-check with `sqlite3` on a dev DB: insert the same entity via old build and new build, diff the rows).
- [ ] All five `row_to_*` mappers deleted; grep gates from Step 6 hold.
- [ ] IPC payloads unchanged: `src/lib/types.ts` untouched and `pnpm exec tsc --noEmit` green with zero frontend edits.
- [ ] No behavior change: no test modified except mechanically (renamed symbol, moved import).

// SQLite persistence for crews, runners, missions, and sessions.
//
// Schema lives in migrations/0001_init.sql and mirrors arch §7.1 verbatim.
// The pool is opened once at app start with WAL mode + foreign keys; later
// chunks pull connections from it via Tauri state.

use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

use crate::error::Result;

pub type DbPool = Pool<SqliteConnectionManager>;

// The v0 built-in signal allowlist seeded onto every new crew row. See
// arch §5.3 Layer 2 — the CLI reads this list (exported to a sidecar in C5)
// and rejects unknown `type`s. In MVP this list is write-only from the DB
// layer; users extend it in v0.x.
pub const DEFAULT_SIGNAL_TYPES: &[&str] = &[
    "mission_goal",
    "human_said",
    "ask_lead",
    "ask_human",
    "human_question",
    "human_response",
    "runner_status",
    "inbox_read",
];

#[allow(dead_code)] // Consumed by C5 when it writes the sidecar at $APPDATA/.../signal_types.json.
pub fn default_signal_types_json() -> String {
    serde_json::to_string(DEFAULT_SIGNAL_TYPES).expect("static allowlist must serialize")
}

pub fn open_pool(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(init_connection);
    build_pool(manager, 8, true)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<DbPool> {
    // Tests get schema only — the default-crew seed would pollute the
    // empty starting state most command tests assume.
    let manager = SqliteConnectionManager::memory().with_init(init_connection);
    build_pool(manager, 1, false)
}

fn build_pool(manager: SqliteConnectionManager, max_size: u32, seed: bool) -> Result<DbPool> {
    let pool = Pool::builder().max_size(max_size).build(manager)?;
    let mut conn = pool.get()?;
    run_migrations(&mut conn)?;
    if seed {
        seed_defaults(&mut conn)?;
    }
    Ok(pool)
}

fn init_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;\n\
         PRAGMA foreign_keys = ON;\n\
         PRAGMA busy_timeout = 5000;",
    )
}

// Pre-release squash: the original 0001..0008 collapsed into one
// init file. Future schema migrations resume from 0002.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

// Default-data seed: ships the Build squad starter crew on first launch.
//
// Runs at most once per database. The marker
// `_app_state.default_crew_seeded` records that the seed step has been
// considered for this DB so we don't recreate Build squad if the user
// later deletes everything ("first launch" must mean *first* launch,
// not "any future launch where you happen to have zero crews").
//
// Even on first launch we only apply the SQL when the DB has zero
// crews AND zero runners. If the user has *any* prior data — e.g.
// they ran the build-squad.seed.sh fixture against this DB before
// opening the app — we skip cleanly and still set the marker. This
// avoids the partial-crew failure mode where a colliding runner
// handle would leave Build squad missing its lead, while the start-
// mission UI still treated it as launchable.
//
// Tests skip this entire path so command tests can assume an empty
// starting state.
const DEFAULT_SEED_SQL: &str = include_str!("../migrations/0002_default_crew.sql");

const SEED_MARKER_KEY: &str = "default_crew_seeded";

fn seed_defaults(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _app_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         )",
    )?;
    let already_seeded: bool = conn
        .query_row(
            "SELECT 1 FROM _app_state WHERE key = ?1",
            params![SEED_MARKER_KEY],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if already_seeded {
        return Ok(());
    }

    let crew_count: i64 = conn.query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))?;
    let runner_count: i64 = conn.query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if crew_count == 0 && runner_count == 0 {
        tx.execute_batch(DEFAULT_SEED_SQL)?;
    }
    tx.execute(
        "INSERT INTO _app_state (key, value) VALUES (?1, ?2)",
        params![SEED_MARKER_KEY, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(())
}

fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
         )",
    )?;
    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _migrations",
        [],
        |row| row.get(0),
    )?;
    // Each migration + its `_migrations` bookkeeping row runs in a single
    // IMMEDIATE transaction: a crash mid-apply rolls back the DDL so the next
    // startup retries the same version instead of replaying it onto a
    // partially-migrated schema (which would fail on `CREATE TABLE crews`).
    for (version, sql) in MIGRATIONS {
        if *version > current {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ErrorCode;

    fn insert_crew(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![id, format!("crew-{id}"), "2026-04-22T00:00:00Z"],
        )
        .unwrap();
    }

    fn insert_runner(conn: &Connection, id: &str, handle: &str) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'shell', 'sh', ?4, ?4)",
            params![
                id,
                handle,
                format!("{handle} display"),
                "2026-04-22T00:00:00Z"
            ],
        )
    }

    fn insert_slot(
        conn: &Connection,
        id: &str,
        crew_id: &str,
        runner_id: &str,
        slot_handle: &str,
        position: i64,
        lead: i64,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                crew_id,
                runner_id,
                slot_handle,
                position,
                lead,
                "2026-04-22T00:00:00Z"
            ],
        )
    }

    #[test]
    fn migrations_bootstrap_all_tables() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN
                     ('crews','runners','slots','missions','sessions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn new_crew_is_seeded_with_default_signal_types() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        let raw: String = conn
            .query_row("SELECT signal_types FROM crews WHERE id = 'c1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let parsed: Vec<String> = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed,
            DEFAULT_SIGNAL_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
    }

    // The "at most one lead per crew" invariant moves to the slot
    // commands; covered by the slot_set_lead test in commands::slot.
    // The schema no longer has the partial unique index that used to
    // enforce it.

    #[test]
    fn runner_handle_is_globally_unique() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_runner(&conn, "r1", "shared").unwrap();
        let err = insert_runner(&conn, "r2", "shared").unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn same_runner_can_join_multiple_crews() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_crew(&conn, "c2");
        insert_runner(&conn, "r1", "shared").unwrap();

        insert_slot(&conn, "s1", "c1", "r1", "alpha-c1", 0, 1).unwrap();
        insert_slot(&conn, "s2", "c2", "r1", "alpha-c2", 0, 1).unwrap();
    }

    #[test]
    fn same_runner_can_fill_multiple_slots_in_one_crew() {
        // The whole point of the slot redesign: the same runner
        // template can sit in two slots of the same crew with
        // different in-crew handles.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "claude").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "architect", 0, 1).unwrap();
        insert_slot(&conn, "s2", "c1", "r1", "reviewer", 1, 0).unwrap();
    }

    #[test]
    fn slot_handle_is_unique_per_crew() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_runner(&conn, "r2", "beta").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "lead-slot", 0, 1).unwrap();
        let err = insert_slot(&conn, "s2", "c1", "r2", "lead-slot", 1, 0).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn position_is_unique_per_crew() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_runner(&conn, "r2", "beta").unwrap();

        insert_slot(&conn, "s1", "c1", "r1", "alpha", 0, 1).unwrap();
        let err = insert_slot(&conn, "s2", "c1", "r2", "beta", 0, 0).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn json_blob_columns_roundtrip() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");

        let policy = serde_json::json!([{"when": {"signal": "ask_lead"}, "do": "inject_stdin"}]);
        let signals = serde_json::json!(["custom_a", "custom_b"]);
        conn.execute(
            "UPDATE crews SET orchestrator_policy = ?1, signal_types = ?2 WHERE id = 'c1'",
            params![policy.to_string(), signals.to_string()],
        )
        .unwrap();

        let env = serde_json::json!({"FOO": "bar", "BAZ": "qux"});
        let args = serde_json::json!(["--flag", "--val=1"]);
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                args_json, env_json, created_at, updated_at
             ) VALUES ('r1','test-impl','Impl','shell','sh',?1,?2,?3,?3)",
            params![args.to_string(), env.to_string(), "2026-04-22T00:00:00Z"],
        )
        .unwrap();

        let (policy_raw, signals_raw): (String, String) = conn
            .query_row(
                "SELECT orchestrator_policy, signal_types FROM crews WHERE id = 'c1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&policy_raw).unwrap(),
            policy
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&signals_raw).unwrap(),
            signals
        );

        let (args_raw, env_raw): (String, String) = conn
            .query_row(
                "SELECT args_json, env_json FROM runners WHERE id = 'r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&args_raw).unwrap(),
            args
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&env_raw).unwrap(),
            env
        );
    }

    #[test]
    fn deleting_crew_cascades_slot_rows_only() {
        // Runners are global templates — deleting a crew should strip
        // its slots but leave the runner template intact so other
        // crews (or direct chats) can keep using it.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "alpha", 0, 1).unwrap();

        conn.execute("DELETE FROM crews WHERE id = 'c1'", [])
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners WHERE id = 'r1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let slot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM slots WHERE runner_id = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runner_count, 1, "runner template must survive crew delete");
        assert_eq!(slot_count, 0, "slots cascade with the crew");
    }

    #[test]
    fn seed_defaults_inserts_build_squad_on_empty_db() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        let slot_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 1);
        assert_eq!(runner_count, 3);
        assert_eq!(slot_count, 3);

        let lead_handle: String = conn
            .query_row("SELECT slot_handle FROM slots WHERE lead = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(lead_handle, "architect");
    }

    #[test]
    fn seed_defaults_skips_when_user_has_a_crew() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        insert_crew(&conn, "user-c1");
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 1, "should not seed when crews already exist");
    }

    #[test]
    fn seed_defaults_skips_when_user_has_a_runner_but_no_crew() {
        // The partial-crew failure mode: a user manually created an
        // `architect` runner template (or ran the seed.sh fixture
        // directly into this DB), then opened the app for the first
        // time. Pre-fix, the migration inserted the Build squad crew
        // and inserted only impl + reviewer runners (architect skipped
        // by the per-handle NOT EXISTS guard), then the slot insert
        // for the architect slot couldn't find our runner ID and
        // skipped — leaving Build squad with two slots and no lead.
        // The start-mission UI treated that as launchable, then the
        // backend rejected it. Now the whole seed bails, marker is
        // still set, and we never produce a partial crew.
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        insert_runner(&conn, "user-r1", "architect").unwrap();
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 0, "should not create Build squad");
        assert_eq!(runner_count, 1, "user's runner stays untouched");
    }

    #[test]
    fn seed_defaults_marker_prevents_reseed_after_user_deletes_everything() {
        // First launch: empty DB → seed runs and marker is recorded.
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();

        // User wipes the seeded data — slots cascade with the crew,
        // runners are global templates so we delete them explicitly.
        conn.execute("DELETE FROM crews", []).unwrap();
        conn.execute("DELETE FROM runners", []).unwrap();
        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 0);

        // Next launch: seed sees the marker and skips, even though
        // the DB looks "empty" again.
        seed_defaults(&mut conn).unwrap();
        let crew_count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            crew_count_after, 0,
            "marker must prevent reseeding after deletion"
        );
    }

    #[test]
    fn seed_defaults_is_idempotent_across_reseeds() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();
        seed_defaults(&mut conn).unwrap();

        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        let slot_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runner_count, 3);
        assert_eq!(slot_count, 3);
    }

    #[test]
    fn migrations_are_idempotent_on_reopen() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("runner.db");

        {
            let _pool = open_pool(&path).unwrap();
        }
        let pool = open_pool(&path).unwrap();
        let conn = pool.get().unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            applied,
            MIGRATIONS.len() as i64,
            "each migration should apply exactly once"
        );
    }
}

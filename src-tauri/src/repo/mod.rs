// Repo layer (impl 0021): one submodule per persistent object, owning that
// table's row struct, column list, and CRUD statements. Row<->struct mapping
// is derived through `serde_rusqlite` instead of hand-written mappers; the
// storage byte formats are pinned by the helpers in `repo::serde` so old and
// new rows are indistinguishable.
//
// Conventions:
//   - Functions take `&Connection` (or a `&Transaction` via `Deref`), never
//     the pool — callers own connection acquisition and transaction
//     boundaries.
//   - Every module has a `const COLUMNS: &[&str]` naming the table's columns
//     in row-struct field order; SELECT and INSERT statements are built from
//     it so the statement and the struct can't drift apart silently.
//   - Functions return `rusqlite::Result` — the same contract the legacy
//     `row_to_*` mappers had — so command-layer error shaping (not-found
//     messages, constraint-violation rewrites) stays where it is.
//   - Join DTOs are assembled in Rust from per-table reads or aliased
//     columns; `#[serde(flatten)]` is not used for row deserialization.

pub mod crew;
pub mod mission;
pub mod node;
pub mod project;
pub mod runner;
pub mod serde;
pub mod session;
pub mod slot;

/// Map a serde_rusqlite deserialization error into the same rusqlite error
/// shape the legacy hand-written mappers produced on bad column data.
pub(crate) fn de_err(e: serde_rusqlite::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

/// Map a serde_rusqlite serialization error into rusqlite's ToSql error.
pub(crate) fn ser_err(e: serde_rusqlite::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// `col_a, col_b, ...` — SELECT list for a table's COLUMNS.
pub(crate) fn select_list(columns: &[&str]) -> String {
    columns.join(", ")
}

/// `alias.col_a, alias.col_b, ...` — SELECT list for one side of a JOIN.
/// SQLite reports the bare column name for `alias.col`, so `from_row`
/// still matches the row-struct fields by name.
pub(crate) fn qualified_select_list(alias: &str, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|c| format!("{alias}.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `INSERT INTO <table> (cols...) VALUES (:col...)` built from COLUMNS so
/// the statement and the row struct stay paired.
pub(crate) fn insert_sql(table: &str, columns: &[&str]) -> String {
    let placeholders: Vec<String> = columns.iter().map(|c| format!(":{c}")).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(", "),
        placeholders.join(", ")
    )
}

#[cfg(test)]
mod spike_tests {
    // Step 0 spike: prove every risky mapping against an in-memory DB
    // before any table is migrated. Each test pins a byte format or a
    // type-bridge behavior the per-table modules will rely on.

    use std::collections::HashMap;

    use chrono::Utc;
    use rusqlite::Connection;
    use serde::{Deserialize, Serialize};
    use serde_rusqlite::{from_row, to_params_named, to_params_named_with_fields};

    use crate::model::{MissionStatus, SessionStatus, Timestamp};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct SpikeRow {
        id: String,
        flag: bool,
        mission_status: MissionStatus,
        session_status: SessionStatus,
        #[serde(with = "crate::repo::serde::rfc3339")]
        ts: Timestamp,
        #[serde(with = "crate::repo::serde::rfc3339_opt")]
        ts_opt: Option<Timestamp>,
        #[serde(with = "crate::repo::serde::json_text")]
        args: Vec<String>,
        #[serde(with = "crate::repo::serde::json_text")]
        env: HashMap<String, String>,
        #[serde(with = "crate::repo::serde::json_text_opt")]
        policy: Option<serde_json::Value>,
        note: Option<String>,
    }

    const SPIKE_COLUMNS: &[&str] = &[
        "id",
        "flag",
        "mission_status",
        "session_status",
        "ts",
        "ts_opt",
        "args",
        "env",
        "policy",
        "note",
    ];

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE spike (
                id TEXT PRIMARY KEY,
                flag INTEGER NOT NULL,
                mission_status TEXT NOT NULL,
                session_status TEXT NOT NULL,
                ts TEXT NOT NULL,
                ts_opt TEXT,
                args TEXT NOT NULL,
                env TEXT NOT NULL,
                policy TEXT,
                note TEXT
            )",
        )
        .unwrap();
        conn
    }

    fn insert(conn: &Connection, row: &SpikeRow) {
        conn.execute(
            &super::insert_sql("spike", SPIKE_COLUMNS),
            to_params_named(row).unwrap().to_slice().as_slice(),
        )
        .unwrap();
    }

    fn get(conn: &Connection, id: &str) -> SpikeRow {
        let sql = format!(
            "SELECT {} FROM spike WHERE id = ?1",
            super::select_list(SPIKE_COLUMNS)
        );
        conn.query_row(&sql, rusqlite::params![id], |row| {
            from_row::<SpikeRow>(row).map_err(super::de_err)
        })
        .unwrap()
    }

    fn full_row() -> SpikeRow {
        let now = Utc::now();
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        env.insert("BAZ".to_string(), "qux".to_string());
        SpikeRow {
            id: "full".into(),
            flag: true,
            mission_status: MissionStatus::Running,
            session_status: SessionStatus::Crashed,
            ts: now,
            ts_opt: Some(now),
            args: vec!["--flag".into(), "--val=1".into()],
            env,
            policy: Some(serde_json::json!([
                {"when": {"signal": "ask_lead"}, "do": "inject_stdin"}
            ])),
            note: Some("hello".into()),
        }
    }

    fn minimal_row() -> SpikeRow {
        SpikeRow {
            id: "minimal".into(),
            flag: false,
            mission_status: MissionStatus::Aborted,
            session_status: SessionStatus::Stopped,
            ts: Utc::now(),
            ts_opt: None,
            args: Vec::new(),
            env: HashMap::new(),
            policy: None,
            note: None,
        }
    }

    #[test]
    fn round_trip_fully_populated_row() {
        let conn = conn();
        let row = full_row();
        insert(&conn, &row);
        assert_eq!(get(&conn, "full"), row);
    }

    #[test]
    fn round_trip_null_heavy_row() {
        let conn = conn();
        let row = minimal_row();
        insert(&conn, &row);
        assert_eq!(get(&conn, "minimal"), row);
    }

    #[test]
    fn bool_is_stored_as_integer() {
        let conn = conn();
        insert(&conn, &full_row());
        let (type_of, raw): (String, i64) = conn
            .query_row(
                "SELECT typeof(flag), flag FROM spike WHERE id = 'full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(type_of, "integer");
        assert_eq!(raw, 1);
    }

    #[test]
    fn status_enums_store_lowercase_text() {
        let conn = conn();
        insert(&conn, &full_row());
        insert(&conn, &minimal_row());
        let (m, s): (String, String) = conn
            .query_row(
                "SELECT mission_status, session_status FROM spike WHERE id = 'full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(m, "running");
        assert_eq!(s, "crashed");
        let (m, s): (String, String) = conn
            .query_row(
                "SELECT mission_status, session_status FROM spike WHERE id = 'minimal'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(m, "aborted");
        assert_eq!(s, "stopped");
    }

    #[test]
    fn timestamps_write_byte_identical_to_to_rfc3339() {
        let conn = conn();
        let row = full_row();
        insert(&conn, &row);
        let (ts_raw, ts_opt_raw): (String, String) = conn
            .query_row("SELECT ts, ts_opt FROM spike WHERE id = 'full'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(ts_raw, row.ts.to_rfc3339());
        assert_eq!(ts_opt_raw, row.ts_opt.unwrap().to_rfc3339());
    }

    #[test]
    fn timestamps_read_both_legacy_offset_spellings() {
        let conn = conn();
        // Raw-SQL inserts in today's two historical spellings: the
        // `+00:00` form every `to_rfc3339()` write produced, and the `Z`
        // form used by fixed seed/test timestamps.
        conn.execute(
            "INSERT INTO spike
                (id, flag, mission_status, session_status, ts, ts_opt, args, env, policy, note)
             VALUES ('offset', 0, 'completed', 'running',
                     '2026-04-22T01:02:03.456789+00:00', '2026-04-22T01:02:03+00:00',
                     '[]', '{}', NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO spike
                (id, flag, mission_status, session_status, ts, ts_opt, args, env, policy, note)
             VALUES ('zulu', 0, 'completed', 'running',
                     '2026-04-22T01:02:03.456789Z', '2026-04-22T01:02:03Z',
                     '[]', '{}', NULL, NULL)",
            [],
        )
        .unwrap();

        let offset = get(&conn, "offset");
        let zulu = get(&conn, "zulu");
        assert_eq!(
            offset.ts, zulu.ts,
            "both spellings parse to the same instant"
        );
        assert_eq!(offset.ts_opt, zulu.ts_opt);
        assert_eq!(
            offset.ts,
            "2026-04-22T01:02:03.456789Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn json_text_writes_byte_identical_to_serde_json_to_string() {
        let conn = conn();
        // Single-entry map so the serialized form is deterministic and the
        // byte comparison against `serde_json::to_string` is exact.
        let mut row = full_row();
        row.id = "json".into();
        row.env = HashMap::from([("FOO".to_string(), "bar".to_string())]);
        insert(&conn, &row);

        let (args_raw, env_raw, policy_raw): (String, String, String) = conn
            .query_row(
                "SELECT args, env, policy FROM spike WHERE id = 'json'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(args_raw, serde_json::to_string(&row.args).unwrap());
        assert_eq!(env_raw, serde_json::to_string(&row.env).unwrap());
        assert_eq!(
            policy_raw,
            serde_json::to_string(row.policy.as_ref().unwrap()).unwrap()
        );

        // Empty collections still serialize (as "[]" / "{}"), matching the
        // legacy create path that always wrote `serde_json::to_string`.
        let (args_raw, env_raw): (String, String) = conn
            .query_row("SELECT args, env FROM spike WHERE id = 'json'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(args_raw, serde_json::to_string(&row.args).unwrap());
        assert_eq!(env_raw, serde_json::to_string(&row.env).unwrap());
    }

    #[test]
    fn json_text_reads_existing_stored_shapes() {
        let conn = conn();
        // Raw-SQL insert mirroring today's stored JSON TEXT shapes,
        // including the seed's args_json literal.
        conn.execute(
            r#"INSERT INTO spike
                (id, flag, mission_status, session_status, ts, ts_opt, args, env, policy, note)
             VALUES ('legacy', 1, 'running', 'running',
                     '2026-04-22T00:00:00+00:00', NULL,
                     '["--ask-for-approval","on-request","--sandbox","workspace-write"]',
                     '{"FOO":"bar"}',
                     '[{"when":{"signal":"ask_lead"},"do":"inject_stdin"}]',
                     NULL)"#,
            [],
        )
        .unwrap();
        let row = get(&conn, "legacy");
        assert_eq!(
            row.args,
            vec![
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ]
        );
        assert_eq!(
            row.env,
            HashMap::from([("FOO".to_string(), "bar".to_string())])
        );
        assert_eq!(
            row.policy,
            Some(serde_json::json!([
                {"when": {"signal": "ask_lead"}, "do": "inject_stdin"}
            ]))
        );
    }

    #[test]
    fn to_params_named_with_fields_drives_a_partial_update() {
        let conn = conn();
        let original = full_row();
        insert(&conn, &original);

        let mut updated = original.clone();
        updated.note = Some("rewritten".into());
        updated.session_status = SessionStatus::Stopped;
        updated.ts_opt = None;
        conn.execute(
            "UPDATE spike
                SET note = :note, session_status = :session_status, ts_opt = :ts_opt
              WHERE id = :id",
            to_params_named_with_fields(&updated, &["note", "session_status", "ts_opt", "id"])
                .unwrap()
                .to_slice()
                .as_slice(),
        )
        .unwrap();

        let after = get(&conn, "full");
        assert_eq!(
            after, updated,
            "named fields updated, everything else untouched"
        );
        assert_eq!(after.mission_status, original.mission_status);
        assert_eq!(after.ts, original.ts);
        assert_eq!(after.args, original.args);
    }
}

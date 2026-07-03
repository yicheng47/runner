// `runners` table — global agent templates.
//
// The only table where the row shape and the IPC shape diverge by name:
// the DB stores `args_json` / `env_json` TEXT columns while `model::Runner`
// exposes `args: Vec<String>` / `env: HashMap`. That divergence lives
// entirely in this module's `From` conversions.
//
// Legacy rows may carry NULL `args_json` / `env_json` (fixture and
// hand-inserted rows); those read back as empty collections, matching the
// old `row_to_runner`. Rows written through the repo always serialize the
// collections (`"[]"` / `"{}"` when empty), matching the legacy
// create/update paths that always called `serde_json::to_string`.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named, to_params_named_with_fields};

use crate::model::{Runner, Timestamp};

use super::{de_err, insert_sql, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerRow {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub runtime: String,
    pub command: String,
    #[serde(with = "crate::repo::serde::json_text_opt")]
    pub args_json: Option<Vec<String>>,
    pub working_dir: Option<String>,
    pub system_prompt: Option<String>,
    #[serde(with = "crate::repo::serde::json_text_opt")]
    pub env_json: Option<HashMap<String, String>>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub created_at: Timestamp,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub updated_at: Timestamp,
}

pub const COLUMNS: &[&str] = &[
    "id",
    "handle",
    "display_name",
    "runtime",
    "command",
    "args_json",
    "working_dir",
    "system_prompt",
    "env_json",
    "model",
    "effort",
    "created_at",
    "updated_at",
];

/// `handle` and `created_at` are immutable after create (handle is the
/// runner's identity in events and policy references), so the update
/// column list excludes them — same statement shape as the legacy UPDATE.
const UPDATE_FIELDS: &[&str] = &[
    "display_name",
    "runtime",
    "command",
    "args_json",
    "working_dir",
    "system_prompt",
    "env_json",
    "model",
    "effort",
    "updated_at",
    "id",
];

impl From<RunnerRow> for Runner {
    fn from(r: RunnerRow) -> Self {
        Runner {
            id: r.id,
            handle: r.handle,
            display_name: r.display_name,
            runtime: r.runtime,
            command: r.command,
            args: r.args_json.unwrap_or_default(),
            working_dir: r.working_dir,
            system_prompt: r.system_prompt,
            env: r.env_json.unwrap_or_default(),
            model: r.model,
            effort: r.effort,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

impl From<&Runner> for RunnerRow {
    fn from(r: &Runner) -> Self {
        RunnerRow {
            id: r.id.clone(),
            handle: r.handle.clone(),
            display_name: r.display_name.clone(),
            runtime: r.runtime.clone(),
            command: r.command.clone(),
            args_json: Some(r.args.clone()),
            working_dir: r.working_dir.clone(),
            system_prompt: r.system_prompt.clone(),
            env_json: Some(r.env.clone()),
            model: r.model.clone(),
            effort: r.effort.clone(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub fn insert(conn: &Connection, row: &RunnerRow) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("runners", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

/// Full-row update of every mutable column. The command layer resolves the
/// outer-`Option` (leave-untouched) patch semantics against the existing
/// row before calling.
pub fn update(conn: &Connection, row: &RunnerRow) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE runners
            SET display_name = :display_name,
                runtime = :runtime,
                command = :command,
                args_json = :args_json,
                working_dir = :working_dir,
                system_prompt = :system_prompt,
                env_json = :env_json,
                model = :model,
                effort = :effort,
                updated_at = :updated_at
          WHERE id = :id",
        to_params_named_with_fields(row, UPDATE_FIELDS)
            .map_err(ser_err)?
            .to_slice()
            .as_slice(),
    )
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Runner>> {
    let sql = format!("SELECT {} FROM runners WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<RunnerRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Runner::from))
}

pub fn get_by_handle(conn: &Connection, handle: &str) -> rusqlite::Result<Option<Runner>> {
    let sql = format!(
        "SELECT {} FROM runners WHERE handle = ?1",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![handle], |row| {
        from_row::<RunnerRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Runner::from))
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Runner>> {
    let sql = format!(
        "SELECT {} FROM runners ORDER BY handle ASC",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| from_row::<RunnerRow>(row).map_err(de_err))?;
    rows.map(|r| r.map(Runner::from)).collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM runners WHERE id = ?1", rusqlite::params![id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn full_row() -> RunnerRow {
        let now = Utc::now();
        RunnerRow {
            id: "r-full".into(),
            handle: "full".into(),
            display_name: "Full".into(),
            runtime: "codex".into(),
            command: "codex".into(),
            args_json: Some(vec![
                "--ask-for-approval".into(),
                "on-request".into(),
                "--sandbox".into(),
                "workspace-write".into(),
            ]),
            working_dir: Some("/tmp/work".into()),
            system_prompt: Some("persona".into()),
            env_json: Some(HashMap::from([("FOO".to_string(), "bar".to_string())])),
            model: Some("gpt-5".into()),
            effort: Some("high".into()),
            created_at: now,
            updated_at: now,
        }
    }

    fn minimal_row() -> RunnerRow {
        let now = Utc::now();
        RunnerRow {
            id: "r-min".into(),
            handle: "min".into(),
            display_name: "Min".into(),
            runtime: "shell".into(),
            command: "sh".into(),
            args_json: Some(Vec::new()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(HashMap::new()),
            model: None,
            effort: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            let read = get(&conn, &row.id).unwrap().unwrap();
            assert_eq!(RunnerRow::from(&read), row);
        }
    }

    #[test]
    fn legacy_rows_with_null_json_columns_read_as_empty_collections() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        // Shape of db.rs fixtures and pre-args rows: no args_json/env_json,
        // `Z`-spelled timestamps.
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES ('r-legacy', 'legacy', 'Legacy', 'shell', 'sh',
                       '2026-04-22T00:00:00Z', '2026-04-22T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let runner = get(&conn, "r-legacy").unwrap().unwrap();
        assert!(runner.args.is_empty());
        assert!(runner.env.is_empty());
        assert_eq!(runner.created_at, runner.updated_at);
    }

    #[test]
    fn legacy_json_text_shapes_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        // The exact seed args literal and a stored env map.
        conn.execute(
            r#"INSERT INTO runners (
                id, handle, display_name, runtime, command, args_json, env_json,
                created_at, updated_at
             ) VALUES ('r-seed', 'seed', 'Seed', 'codex', 'codex',
                       '["--ask-for-approval","on-request","--sandbox","workspace-write"]',
                       '{"FOO":"bar"}',
                       '2026-05-03T00:00:00Z', '2026-05-03T00:00:00Z')"#,
            [],
        )
        .unwrap();
        let runner = get(&conn, "r-seed").unwrap().unwrap();
        assert_eq!(
            runner.args,
            vec![
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ]
        );
        assert_eq!(
            runner.env,
            HashMap::from([("FOO".to_string(), "bar".to_string())])
        );
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = full_row();
        insert(&conn, &row).unwrap();
        let (args_raw, env_raw, created_raw): (String, String, String) = conn
            .query_row(
                "SELECT args_json, env_json, created_at FROM runners WHERE id = 'r-full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            args_raw,
            serde_json::to_string(row.args_json.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            env_raw,
            serde_json::to_string(row.env_json.as_ref().unwrap()).unwrap()
        );
        assert_eq!(created_raw, row.created_at.to_rfc3339());

        // Empty collections serialize as "[]" / "{}", not NULL — the shape
        // the legacy create path always wrote.
        insert(&conn, &minimal_row()).unwrap();
        let (args_raw, env_raw): (String, String) = conn
            .query_row(
                "SELECT args_json, env_json FROM runners WHERE id = 'r-min'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(args_raw, "[]");
        assert_eq!(env_raw, "{}");
    }

    #[test]
    fn update_rewrites_mutable_columns_and_preserves_handle_and_created_at() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let original = full_row();
        insert(&conn, &original).unwrap();

        let mut updated = original.clone();
        updated.display_name = "Renamed".into();
        updated.args_json = Some(vec!["--debug".into()]);
        updated.model = None;
        updated.updated_at = Utc::now();
        // Struct-level drift on immutable fields must not reach the DB.
        updated.handle = "hijacked".into();
        update(&conn, &updated).unwrap();

        let read = get(&conn, "r-full").unwrap().unwrap();
        assert_eq!(read.display_name, "Renamed");
        assert_eq!(read.args, vec!["--debug".to_string()]);
        assert_eq!(read.model, None);
        assert_eq!(read.handle, "full", "handle is not in the update list");
        assert_eq!(read.created_at, original.created_at);
        assert_eq!(read.updated_at, updated.updated_at);
    }
}

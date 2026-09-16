// `roles` table — global agent templates.
//
// The only table where the row shape and the IPC shape diverge by name:
// the DB stores `args_json` / `env_json` TEXT columns while `model::Role`
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

use crate::model::{Role, Timestamp};

use super::{de_err, insert_sql, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleRow {
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
/// role's identity in events and policy references), so the update
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

impl From<RoleRow> for Role {
    fn from(r: RoleRow) -> Self {
        Role {
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

impl From<&Role> for RoleRow {
    fn from(r: &Role) -> Self {
        RoleRow {
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

pub fn insert(conn: &Connection, row: &RoleRow) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("roles", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

/// Full-row update of every mutable column. The command layer resolves the
/// outer-`Option` (leave-untouched) patch semantics against the existing
/// row before calling.
pub fn update(conn: &Connection, row: &RoleRow) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE roles
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

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Role>> {
    let sql = format!("SELECT {} FROM roles WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<RoleRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Role::from))
}

pub fn get_by_handle(conn: &Connection, handle: &str) -> rusqlite::Result<Option<Role>> {
    let sql = format!(
        "SELECT {} FROM roles WHERE handle = ?1",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![handle], |row| {
        from_row::<RoleRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Role::from))
}

/// Row mapper for the list queries: an unreadable row (a non-TEXT value in
/// a TEXT column, written by something outside the app — issue #439)
/// degrades to a warn naming the row id instead of failing the whole query
/// and blanking every role surface. `get`/`get_by_handle` still error —
/// an explicitly requested row must not silently vanish.
fn read_or_skip(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Role>> {
    match from_row::<RoleRow>(row) {
        Ok(r) => Ok(Some(Role::from(r))),
        Err(e) => {
            let id: String = row.get(0).unwrap_or_else(|_| "<unreadable>".into());
            log::warn!("roles: skipping unreadable row {id}: {e}");
            Ok(None)
        }
    }
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Role>> {
    let sql = format!(
        "SELECT {} FROM roles ORDER BY handle ASC",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], read_or_skip)?;
    rows.filter_map(|r| r.transpose()).collect()
}

pub fn list_for_crew(conn: &Connection, crew_id: &str) -> rusqlite::Result<Vec<Role>> {
    let sql = format!(
        "SELECT {}
           FROM roles r
          WHERE EXISTS (
                SELECT 1
                  FROM slots s
                 WHERE s.crew_id = ?1
                   AND s.role_id = r.id
          )
          ORDER BY r.handle ASC",
        super::qualified_select_list("r", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![crew_id], read_or_skip)?;
    rows.filter_map(|r| r.transpose()).collect()
}

const SEARCH_PREDICATE: &str = "(
       LOWER(r.handle) LIKE LOWER(?1) ESCAPE '\\'
    OR LOWER(r.display_name) LIKE LOWER(?1) ESCAPE '\\'
)";

pub fn count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM roles", [], |row| row.get(0))
}

#[cfg(test)]
pub(crate) fn table_exists(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'roles'
         )",
        [],
        |row| row.get(0),
    )
}

pub fn count_matching(conn: &Connection, pattern: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM roles r WHERE {SEARCH_PREDICATE}"),
        rusqlite::params![pattern],
        |row| row.get(0),
    )
}

pub fn list_page(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<Role>> {
    let sql = format!(
        "SELECT {}
           FROM roles r
          WHERE {SEARCH_PREDICATE}
          ORDER BY r.handle ASC
          LIMIT ?2 OFFSET ?3",
        super::qualified_select_list("r", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![pattern, limit, offset], read_or_skip)?;
    rows.filter_map(|r| r.transpose()).collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM roles WHERE id = ?1", rusqlite::params![id])
}

#[cfg(test)]
pub(crate) fn delete_all(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM roles", [])
}

pub fn clear_inheriting_slot_agent_overrides(
    conn: &Connection,
    role_id: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots
            SET model_override = NULL, effort_override = NULL
          WHERE role_id = ?1 AND runtime_override IS NULL",
        rusqlite::params![role_id],
    )
}

pub fn unarchived_direct_session_ids(
    conn: &Connection,
    role_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id
           FROM sessions
          WHERE role_id = ?1
            AND mission_id IS NULL
            AND slot_id IS NULL
            AND archived_at IS NULL
          ORDER BY started_at ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![role_id], |row| row.get(0))?;
    rows.collect()
}

pub fn affected_crews(conn: &Connection, role_id: &str) -> rusqlite::Result<Vec<(String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT crew_id, MAX(lead)
           FROM slots
          WHERE role_id = ?1
          GROUP BY crew_id",
    )?;
    let rows = stmt.query_map(rusqlite::params![role_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
    })?;
    rows.collect()
}

pub fn delete_sessions(conn: &Connection, role_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM sessions WHERE role_id = ?1",
        rusqlite::params![role_id],
    )
}

pub fn session_count(conn: &Connection, role_id: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE role_id = ?1",
        rusqlite::params![role_id],
        |row| row.get(0),
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityRow {
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    pub last_started_at: Option<String>,
    pub direct_session_id: Option<String>,
}

pub fn activity(conn: &Connection, role_id: &str) -> rusqlite::Result<ActivityRow> {
    let active_sessions = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE role_id = ?1 AND status = 'running'",
        rusqlite::params![role_id],
        |row| row.get(0),
    )?;
    let active_missions = conn.query_row(
        "SELECT COUNT(DISTINCT mission_id) FROM sessions
          WHERE role_id = ?1 AND status = 'running' AND mission_id IS NOT NULL",
        rusqlite::params![role_id],
        |row| row.get(0),
    )?;
    let crew_count = conn.query_row(
        "SELECT COUNT(DISTINCT crew_id) FROM slots WHERE role_id = ?1",
        rusqlite::params![role_id],
        |row| row.get(0),
    )?;
    let last_started_at = conn.query_row(
        "SELECT MAX(started_at) FROM sessions WHERE role_id = ?1",
        rusqlite::params![role_id],
        |row| row.get(0),
    )?;
    let direct_session_id = conn
        .query_row(
            "SELECT id FROM sessions
              WHERE role_id = ?1
                AND status = 'running'
                AND mission_id IS NULL
                AND slot_id IS NULL
                AND archived_at IS NULL
              ORDER BY started_at DESC
              LIMIT 1",
            rusqlite::params![role_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(ActivityRow {
        active_sessions,
        active_missions,
        crew_count,
        last_started_at,
        direct_session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn full_row() -> RoleRow {
        let now = Utc::now();
        RoleRow {
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

    fn minimal_row() -> RoleRow {
        let now = Utc::now();
        RoleRow {
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
            assert_eq!(RoleRow::from(&read), row);
        }
    }

    #[test]
    fn legacy_rows_with_null_json_columns_read_as_empty_collections() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        // Shape of db.rs fixtures and pre-args rows: no args_json/env_json,
        // `Z`-spelled timestamps.
        conn.execute(
            "INSERT INTO roles (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES ('r-legacy', 'legacy', 'Legacy', 'shell', 'sh',
                       '2026-04-22T00:00:00Z', '2026-04-22T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let role = get(&conn, "r-legacy").unwrap().unwrap();
        assert!(role.args.is_empty());
        assert!(role.env.is_empty());
        assert_eq!(role.created_at, role.updated_at);
    }

    #[test]
    fn legacy_json_text_shapes_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        // The exact seed args literal and a stored env map.
        conn.execute(
            r#"INSERT INTO roles (
                id, handle, display_name, runtime, command, args_json, env_json,
                created_at, updated_at
             ) VALUES ('r-seed', 'seed', 'Seed', 'codex', 'codex',
                       '["--ask-for-approval","on-request","--sandbox","workspace-write"]',
                       '{"FOO":"bar"}',
                       '2026-05-03T00:00:00Z', '2026-05-03T00:00:00Z')"#,
            [],
        )
        .unwrap();
        let role = get(&conn, "r-seed").unwrap().unwrap();
        assert_eq!(
            role.args,
            vec![
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ]
        );
        assert_eq!(
            role.env,
            HashMap::from([("FOO".to_string(), "bar".to_string())])
        );
    }

    #[test]
    fn list_skips_unreadable_rows_and_keeps_readable_ones() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert(&conn, &full_row()).unwrap();
        // The issue #439 repro: a BLOB in a TEXT column, only writable from
        // outside the app.
        conn.execute(
            "INSERT INTO roles (
                id, handle, display_name, runtime, command, args_json,
                system_prompt, created_at, updated_at
             ) VALUES ('bad', 'bad', 'Bad', 'codex', 'codex', '[]',
                       x'deadbeef', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let handles =
            |roles: Vec<Role>| roles.into_iter().map(|r| r.handle).collect::<Vec<String>>();
        assert_eq!(handles(list(&conn).unwrap()), ["full"]);
        assert_eq!(handles(list_page(&conn, "%", 10, 0).unwrap()), ["full"]);
        // An explicitly requested row still errors.
        assert!(get(&conn, "bad").is_err());
        assert!(get_by_handle(&conn, "bad").is_err());
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = full_row();
        insert(&conn, &row).unwrap();
        let (args_raw, env_raw, created_raw): (String, String, String) = conn
            .query_row(
                "SELECT args_json, env_json, created_at FROM roles WHERE id = 'r-full'",
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
                "SELECT args_json, env_json FROM roles WHERE id = 'r-min'",
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

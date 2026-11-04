// `missions` table — the runtime container for a crew run.
//
// Lifecycle orchestration (event-log writes, router/bus mounting,
// rollback decisions) stays in `ops::mission`; this module owns the
// row struct and the statements. Transaction boundaries belong to callers.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named};

use crate::model::{LinkedChat, Mission, MissionStatus, Timestamp};

use super::{de_err, insert_sql, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionRow {
    pub id: String,
    pub crew_id: Option<String>,
    pub project_id: Option<String>,
    pub title: String,
    pub status: MissionStatus,
    pub goal_override: Option<String>,
    pub cwd: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub started_at: Timestamp,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub stopped_at: Option<Timestamp>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub pinned_at: Option<Timestamp>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub archived_at: Option<Timestamp>,
    pub linked_session_id: Option<String>,
    pub report_to_chat: bool,
}

pub const COLUMNS: &[&str] = &[
    "id",
    "crew_id",
    "project_id",
    "title",
    "status",
    "goal_override",
    "cwd",
    "started_at",
    "stopped_at",
    "pinned_at",
    "archived_at",
    "linked_session_id",
    "report_to_chat",
];

impl From<MissionRow> for Mission {
    fn from(r: MissionRow) -> Self {
        Mission {
            id: r.id,
            crew_id: r.crew_id,
            project_id: r.project_id,
            title: r.title,
            status: r.status,
            goal_override: r.goal_override,
            cwd: r.cwd,
            started_at: r.started_at,
            stopped_at: r.stopped_at,
            pinned_at: r.pinned_at,
            archived_at: r.archived_at,
            linked_chat: None,
        }
    }
}

impl From<&Mission> for MissionRow {
    fn from(m: &Mission) -> Self {
        MissionRow {
            id: m.id.clone(),
            crew_id: m.crew_id.clone(),
            project_id: m.project_id.clone(),
            title: m.title.clone(),
            status: m.status,
            goal_override: m.goal_override.clone(),
            cwd: m.cwd.clone(),
            started_at: m.started_at,
            stopped_at: m.stopped_at,
            pinned_at: m.pinned_at,
            archived_at: m.archived_at,
            linked_session_id: m.linked_chat.as_ref().map(|chat| chat.session_id.clone()),
            report_to_chat: m.linked_chat.as_ref().is_none_or(|chat| chat.reports),
        }
    }
}

/// The row as a `Mission`, with its linked chat's title and archive state
/// read from the chat's row. A link to a row that is no longer a direct
/// chat reads as unlinked.
fn hydrate(conn: &Connection, row: MissionRow) -> rusqlite::Result<Mission> {
    let linked_chat = match &row.linked_session_id {
        Some(id) => super::session::get_direct(conn, id)?.map(|chat| LinkedChat {
            session_id: id.clone(),
            title: super::session::direct_title(&chat),
            archived: chat.row.archived_at.is_some(),
            reports: row.report_to_chat,
        }),
        None => None,
    };
    let mut mission = Mission::from(row);
    mission.linked_chat = linked_chat;
    Ok(mission)
}

fn hydrate_all(
    conn: &Connection,
    rows: impl Iterator<Item = rusqlite::Result<MissionRow>>,
) -> rusqlite::Result<Vec<Mission>> {
    rows.collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|row| hydrate(conn, row))
        .collect()
}

pub fn insert(conn: &Connection, row: &MissionRow) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("missions", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Mission>> {
    // Intentionally no `archived_at` filter — opening an archived
    // mission by direct URL has to still resolve so the workspace can
    // render it read-only.
    let sql = format!(
        "SELECT {} FROM missions WHERE id = ?1",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<MissionRow>(row).map_err(de_err)
    })
    .optional()?
    .map(|row| hydrate(conn, row))
    .transpose()
}

/// Non-archived missions, optionally filtered by crew. Pinned missions
/// float to the top, then most-recently-started. `archived_at IS NULL`
/// is the single chokepoint that hides archived missions from every
/// listing surface.
pub fn list(conn: &Connection, crew_id: Option<&str>) -> rusqlite::Result<Vec<Mission>> {
    let sql = format!(
        "SELECT {}
           FROM missions
           WHERE (?1 IS NULL OR crew_id = ?1)
             AND archived_at IS NULL
           ORDER BY pinned_at IS NULL, pinned_at DESC, started_at DESC",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![crew_id], |row| {
        from_row::<MissionRow>(row).map_err(de_err)
    })?;
    hydrate_all(conn, rows)
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Mission>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM missions ORDER BY started_at, id",
        select_list(COLUMNS)
    ))?;
    let rows = stmt.query_map([], |row| from_row::<MissionRow>(row).map_err(de_err))?;
    hydrate_all(conn, rows)
}

/// Non-archived missions linked to `session_id`, oldest first.
pub fn list_linked(conn: &Connection, session_id: &str) -> rusqlite::Result<Vec<Mission>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {}
           FROM missions
          WHERE linked_session_id = ?1
            AND archived_at IS NULL
          ORDER BY started_at, id",
        select_list(COLUMNS)
    ))?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        from_row::<MissionRow>(row).map_err(de_err)
    })?;
    hydrate_all(conn, rows)
}

/// A mission and its chat link only within one project (#748), so moving
/// either into another project drops the link.
pub fn unlink_across_projects(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET linked_session_id = NULL, report_to_chat = 1
          WHERE linked_session_id IS NOT NULL
            AND project_id IS NOT
                (SELECT s.project_id FROM sessions s WHERE s.id = missions.linked_session_id)",
        [],
    )
}

pub fn set_link(
    conn: &Connection,
    id: &str,
    session_id: Option<&str>,
    report_to_chat: bool,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions SET linked_session_id = ?1, report_to_chat = ?2 WHERE id = ?3",
        rusqlite::params![session_id, report_to_chat, id],
    )
}

/// Archived missions, optionally filtered by crew, newest-archived
/// first — the Settings → Archived pane's read surface. Mirror image
/// of `list()`.
pub fn list_archived(conn: &Connection, crew_id: Option<&str>) -> rusqlite::Result<Vec<Mission>> {
    let sql = format!(
        "SELECT {}
           FROM missions
           WHERE (?1 IS NULL OR crew_id = ?1)
             AND archived_at IS NOT NULL
           ORDER BY archived_at DESC",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![crew_id], |row| {
        from_row::<MissionRow>(row).map_err(de_err)
    })?;
    hydrate_all(conn, rows)
}

/// Terminal stop: flip `running` -> `completed` and stamp `archived_at`
/// atomically with the status (stopping through `mission_archive` always
/// archives). The `WHERE status = 'running'` guard makes racing stops
/// mutually exclusive — the loser updates 0 rows. Missions already out
/// of `running` archive through `archive_if_stopped` instead.
pub fn complete_and_archive_if_running(
    conn: &Connection,
    id: &str,
    stopped_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET status = 'completed', stopped_at = ?1, archived_at = ?1
          WHERE id = ?2 AND status = 'running'",
        rusqlite::params![stopped_at.to_rfc3339(), id],
    )
}

/// Visibility-only archive for missions already out of `running` (#376):
/// legacy restored rows (completed + unarchived, impl 0026) and aborted spawns.
/// Stamps `archived_at` and leaves the lifecycle columns alone — their
/// terminal stop already happened. The `archived_at IS NULL` guard makes
/// racing archives mutually exclusive; the status guard keeps running
/// missions on the stop-and-archive path above.
pub fn archive_if_stopped(
    conn: &Connection,
    id: &str,
    archived_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET archived_at = ?1
          WHERE id = ?2 AND status != 'running' AND archived_at IS NULL",
        rusqlite::params![archived_at.to_rfc3339(), id],
    )
}

/// Roll a half-open mission to `aborted` after a start failure.
pub fn abort(conn: &Connection, id: &str, stopped_at: Timestamp) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET status = 'aborted', stopped_at = ?1
          WHERE id = ?2",
        rusqlite::params![stopped_at.to_rfc3339(), id],
    )
}

pub fn unarchive(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET status = 'running', stopped_at = NULL, archived_at = NULL
          WHERE id = ?1 AND (archived_at IS NOT NULL OR status != 'running')",
        rusqlite::params![id],
    )
}

pub fn delete_many(conn: &Connection, ids: &[String]) -> rusqlite::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    conn.execute(
        &format!("DELETE FROM missions WHERE id IN ({placeholders})"),
        rusqlite::params_from_iter(ids),
    )
}

pub fn ids_for_crew(conn: &Connection, crew_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM missions WHERE crew_id = ?1")?;
    let ids = stmt.query_map(rusqlite::params![crew_id], |row| row.get(0))?;
    ids.collect()
}

pub fn unarchived_ids_for_crew(conn: &Connection, crew_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id
           FROM missions
          WHERE crew_id = ?1
            AND archived_at IS NULL
          ORDER BY started_at ASC",
    )?;
    let ids = stmt.query_map(rusqlite::params![crew_id], |row| row.get(0))?;
    ids.collect()
}

/// Every mission that is running and not archived, oldest first.
pub fn running_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM missions
           WHERE status = 'running' AND archived_at IS NULL
           ORDER BY started_at ASC",
    )?;
    let ids = stmt.query_map([], |row| row.get(0))?;
    ids.collect()
}

pub fn unbind_project(conn: &Connection, project_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions SET project_id = NULL WHERE project_id = ?1",
        rusqlite::params![project_id],
    )
}

pub fn set_pinned_at(
    conn: &Connection,
    id: &str,
    pinned_at: Option<Timestamp>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions SET pinned_at = ?1 WHERE id = ?2",
        rusqlite::params![pinned_at.map(|t| t.to_rfc3339()), id],
    )
}

/// The Files view's saved root (#634); `None` for no row or no saved root.
pub fn files_root(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT files_root FROM missions WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

pub fn set_files_root(conn: &Connection, id: &str, root: Option<&str>) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions SET files_root = ?2 WHERE id = ?1",
        rusqlite::params![id, root],
    )
}

pub fn set_title(conn: &Connection, id: &str, title: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions SET title = ?1 WHERE id = ?2",
        rusqlite::params![title, id],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn seed_crew(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![id, format!("crew-{id}"), "2026-04-22T00:00:00Z"],
        )
        .unwrap();
    }

    fn full_row() -> MissionRow {
        let now = Utc::now();
        MissionRow {
            id: "m-full".into(),
            crew_id: Some("c1".into()),
            project_id: None,
            title: "Ship it".into(),
            status: MissionStatus::Completed,
            goal_override: Some("override".into()),
            cwd: Some("/tmp/work".into()),
            started_at: now,
            stopped_at: Some(now),
            pinned_at: Some(now),
            archived_at: Some(now),
            linked_session_id: None,
            report_to_chat: true,
        }
    }

    fn minimal_row() -> MissionRow {
        MissionRow {
            id: "m-min".into(),
            crew_id: Some("c1".into()),
            project_id: None,
            title: "Bare".into(),
            status: MissionStatus::Running,
            goal_override: None,
            cwd: None,
            started_at: Utc::now(),
            stopped_at: None,
            pinned_at: None,
            archived_at: None,
            linked_session_id: None,
            report_to_chat: true,
        }
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            let read = get(&conn, &row.id).unwrap().unwrap();
            assert_eq!(MissionRow::from(&read), row);
        }
    }

    #[test]
    fn legacy_rows_in_both_timestamp_spellings_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, goal_override, cwd, started_at, stopped_at)
             VALUES ('m-z', 'c1', 't', 'aborted', NULL, NULL,
                     '2026-04-22T00:00:00Z', '2026-04-22T01:00:00Z'),
                    ('m-o', 'c1', 't', 'aborted', NULL, NULL,
                     '2026-04-22T00:00:00+00:00', '2026-04-22T01:00:00+00:00')",
            [],
        )
        .unwrap();
        let zulu = get(&conn, "m-z").unwrap().unwrap();
        let offset = get(&conn, "m-o").unwrap().unwrap();
        assert_eq!(zulu.started_at, offset.started_at);
        assert_eq!(zulu.stopped_at, offset.stopped_at);
        assert_eq!(zulu.status, MissionStatus::Aborted);
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        let row = minimal_row();
        insert(&conn, &row).unwrap();
        let (started_raw, status_raw): (String, String) = conn
            .query_row(
                "SELECT started_at, status FROM missions WHERE id = 'm-min'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(started_raw, row.started_at.to_rfc3339());
        assert_eq!(status_raw, "running");
    }

    #[test]
    fn complete_and_archive_only_flips_running_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        insert(&conn, &minimal_row()).unwrap();

        let stopped_at = Utc::now();
        assert_eq!(
            complete_and_archive_if_running(&conn, "m-min", stopped_at).unwrap(),
            1
        );
        let m = get(&conn, "m-min").unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Completed);
        assert_eq!(m.stopped_at, m.archived_at, "archive stamps atomically");

        // Losing racer: no longer running, 0 rows.
        assert_eq!(
            complete_and_archive_if_running(&conn, "m-min", Utc::now()).unwrap(),
            0
        );
    }

    #[test]
    fn list_archived_is_the_mirror_image_of_list_newest_first() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        insert(&conn, &minimal_row()).unwrap(); // active — hidden here
        let mut older = full_row();
        older.id = "m-arch-old".into();
        older.archived_at = Some(Utc::now() - chrono::Duration::hours(2));
        insert(&conn, &older).unwrap();
        let mut newer = full_row();
        newer.id = "m-arch-new".into();
        insert(&conn, &newer).unwrap();

        let archived = list_archived(&conn, Some("c1")).unwrap();
        let ids: Vec<&str> = archived.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["m-arch-new", "m-arch-old"]);

        let active: Vec<String> = list(&conn, Some("c1"))
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            active,
            vec!["m-min"],
            "archived rows stay off the active list"
        );
    }

    #[test]
    fn unarchive_reopens_the_mission_and_preserves_its_pin() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        let row = full_row(); // completed + stopped_at + pinned_at + archived_at
        insert(&conn, &row).unwrap();

        assert_eq!(unarchive(&conn, "m-full").unwrap(), 1);
        let m = get(&conn, "m-full").unwrap().unwrap();
        assert_eq!(m.archived_at, None);
        assert_eq!(m.status, MissionStatus::Running);
        assert_eq!(m.stopped_at, None);
        assert_eq!(m.pinned_at, row.pinned_at, "pin survives unarchive");

        // Idempotent: an already-active row is a 0-row no-op, not an error.
        assert_eq!(unarchive(&conn, "m-full").unwrap(), 0);
    }

    #[test]
    fn list_hides_archived_and_floats_pinned() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        insert(&conn, &full_row()).unwrap(); // archived — hidden
        insert(&conn, &minimal_row()).unwrap();
        let mut pinned = minimal_row();
        pinned.id = "m-pinned".into();
        pinned.started_at = Utc::now() - chrono::Duration::hours(1);
        pinned.pinned_at = Some(Utc::now());
        insert(&conn, &pinned).unwrap();

        let listed = list(&conn, Some("c1")).unwrap();
        let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["m-pinned", "m-min"]);
    }
}

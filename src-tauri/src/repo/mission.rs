// `missions` table — the runtime container for a crew run.
//
// Lifecycle orchestration (event-log writes, router/bus mounting,
// rollback decisions) stays in `commands::mission`; this module owns the
// row struct and the statements. Transaction boundaries are the
// caller's: every function takes `&Connection` and composes inside the
// existing `mission_start` / `mission_reset` transactions unchanged.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named};

use crate::model::{Mission, MissionStatus, Timestamp};

use super::{de_err, insert_sql, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionRow {
    pub id: String,
    pub crew_id: String,
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
}

pub const COLUMNS: &[&str] = &[
    "id",
    "crew_id",
    "title",
    "status",
    "goal_override",
    "cwd",
    "started_at",
    "stopped_at",
    "pinned_at",
    "archived_at",
];

impl From<MissionRow> for Mission {
    fn from(r: MissionRow) -> Self {
        Mission {
            id: r.id,
            crew_id: r.crew_id,
            title: r.title,
            status: r.status,
            goal_override: r.goal_override,
            cwd: r.cwd,
            started_at: r.started_at,
            stopped_at: r.stopped_at,
            pinned_at: r.pinned_at,
            archived_at: r.archived_at,
        }
    }
}

impl From<&Mission> for MissionRow {
    fn from(m: &Mission) -> Self {
        MissionRow {
            id: m.id.clone(),
            crew_id: m.crew_id.clone(),
            title: m.title.clone(),
            status: m.status,
            goal_override: m.goal_override.clone(),
            cwd: m.cwd.clone(),
            started_at: m.started_at,
            stopped_at: m.stopped_at,
            pinned_at: m.pinned_at,
            archived_at: m.archived_at,
        }
    }
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
    .optional()
    .map(|opt| opt.map(Mission::from))
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
    rows.map(|r| r.map(Mission::from)).collect()
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
    rows.map(|r| r.map(Mission::from)).collect()
}

/// Terminal stop: flip `running` -> `completed` and stamp `archived_at`
/// atomically with the status (a terminal stop is by definition an
/// archive). The `WHERE status = 'running'` guard makes racing stops
/// mutually exclusive — the loser updates 0 rows.
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

/// Roll a half-open mission to `aborted` (spawn/mount/first-turn
/// failures during start or reset).
pub fn abort(conn: &Connection, id: &str, stopped_at: Timestamp) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET status = 'aborted', stopped_at = ?1
          WHERE id = ?2",
        rusqlite::params![stopped_at.to_rfc3339(), id],
    )
}

/// Mission reset: back to `running` with a fresh `started_at`;
/// `stopped_at` and `archived_at` clear in lockstep with the status flip
/// so a freshly-reset live mission never vanishes from `list()`.
pub fn reset_to_running(
    conn: &Connection,
    id: &str,
    started_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET status = 'running',
                started_at = ?1,
                stopped_at = NULL,
                archived_at = NULL
          WHERE id = ?2",
        rusqlite::params![started_at.to_rfc3339(), id],
    )
}

/// Unarchive: clear the archive marker and nothing else — status and
/// `stopped_at` stay, so an unarchived mission reappears exactly as the
/// `completed AND archived_at IS NULL` rows the migration backfill
/// created. The `IS NOT NULL` guard makes a repeat call a 0-row no-op.
pub fn unarchive(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE missions
            SET archived_at = NULL
          WHERE id = ?1 AND archived_at IS NOT NULL",
        rusqlite::params![id],
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
            crew_id: "c1".into(),
            title: "Ship it".into(),
            status: MissionStatus::Completed,
            goal_override: Some("override".into()),
            cwd: Some("/tmp/work".into()),
            started_at: now,
            stopped_at: Some(now),
            pinned_at: Some(now),
            archived_at: Some(now),
        }
    }

    fn minimal_row() -> MissionRow {
        MissionRow {
            id: "m-min".into(),
            crew_id: "c1".into(),
            title: "Bare".into(),
            status: MissionStatus::Running,
            goal_override: None,
            cwd: None,
            started_at: Utc::now(),
            stopped_at: None,
            pinned_at: None,
            archived_at: None,
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
    fn reset_to_running_clears_terminal_stamps() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        insert(&conn, &full_row()).unwrap();

        let fresh_start = Utc::now();
        assert_eq!(reset_to_running(&conn, "m-full", fresh_start).unwrap(), 1);
        let m = get(&conn, "m-full").unwrap().unwrap();
        assert_eq!(m.status, MissionStatus::Running);
        assert_eq!(m.started_at, fresh_start);
        assert_eq!(m.stopped_at, None);
        assert_eq!(m.archived_at, None);
        assert!(m.pinned_at.is_some(), "pin survives a reset");
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
    fn unarchive_clears_only_the_archive_marker() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        let row = full_row(); // completed + stopped_at + pinned_at + archived_at
        insert(&conn, &row).unwrap();

        assert_eq!(unarchive(&conn, "m-full").unwrap(), 1);
        let m = get(&conn, "m-full").unwrap().unwrap();
        assert_eq!(m.archived_at, None);
        assert_eq!(m.status, MissionStatus::Completed, "status is preserved");
        assert_eq!(m.stopped_at, row.stopped_at, "stopped_at is preserved");
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

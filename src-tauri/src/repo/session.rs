// `sessions` table — one PTY run (mission slot or direct chat).
//
// Named `SessionRowDb` to stay clear of the `commands::session::SessionRow`
// IPC DTO. Covers every column including `agent_session_key` and the
// legacy `runtime_*` metadata columns exactly as written today.
//
// The write functions mirror the legacy statements one-for-one — status
// flips, pid/stopped_at updates, key capture — because several run on the
// PTY hot path where statement shape and timing are load-bearing. Do not
// consolidate them.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named};

use crate::model::{Session, SessionStatus, Timestamp};

use super::{de_err, insert_sql, qualified_select_list, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRowDb {
    pub id: String,
    pub mission_id: Option<String>,
    pub runner_id: Option<String>,
    pub slot_id: Option<String>,
    pub cwd: Option<String>,
    pub status: SessionStatus,
    pub pid: Option<i64>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub started_at: Option<Timestamp>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub stopped_at: Option<Timestamp>,
    pub agent_session_key: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub archived_at: Option<Timestamp>,
    pub title: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub pinned_at: Option<Timestamp>,
    /// PTY-runtime metadata (`pty`, legacy tmux) — not the agent kind;
    /// that is `agent_runtime`.
    pub runtime: Option<String>,
    pub runtime_socket: Option<String>,
    pub runtime_session: Option<String>,
    pub runtime_window: Option<String>,
    pub runtime_pane: Option<String>,
    pub runtime_cursor: Option<i64>,
    pub agent_runtime: Option<String>,
    pub agent_command: Option<String>,
}

impl SessionRowDb {
    /// Fresh `running` row with every optional column NULL — spawn sites
    /// fill in what they persist and leave the rest.
    pub fn new_running(id: String) -> Self {
        SessionRowDb {
            id,
            mission_id: None,
            runner_id: None,
            slot_id: None,
            cwd: None,
            status: SessionStatus::Running,
            pid: None,
            started_at: None,
            stopped_at: None,
            agent_session_key: None,
            archived_at: None,
            title: None,
            pinned_at: None,
            runtime: None,
            runtime_socket: None,
            runtime_session: None,
            runtime_window: None,
            runtime_pane: None,
            runtime_cursor: None,
            agent_runtime: None,
            agent_command: None,
        }
    }
}

pub const COLUMNS: &[&str] = &[
    "id",
    "mission_id",
    "runner_id",
    "slot_id",
    "cwd",
    "status",
    "pid",
    "started_at",
    "stopped_at",
    "agent_session_key",
    "archived_at",
    "title",
    "pinned_at",
    "runtime",
    "runtime_socket",
    "runtime_session",
    "runtime_window",
    "runtime_pane",
    "runtime_cursor",
    "agent_runtime",
    "agent_command",
];

pub fn insert(conn: &Connection, row: &SessionRowDb) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("sessions", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

pub fn get_row(conn: &Connection, id: &str) -> rusqlite::Result<Option<SessionRowDb>> {
    let sql = format!(
        "SELECT {} FROM sessions WHERE id = ?1",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<SessionRowDb>(row).map_err(de_err)
    })
    .optional()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
}

pub fn delete_all_for_mission(conn: &Connection, mission_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM sessions WHERE mission_id = ?1",
        rusqlite::params![mission_id],
    )
}

/// Persist the runtime-side identity after the PTY forks.
pub fn update_runtime_metadata(
    conn: &Connection,
    id: &str,
    runtime: &str,
    runtime_session: &str,
    pid: Option<i32>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET runtime = ?2,
                runtime_session = ?3,
                pid = ?4
          WHERE id = ?1",
        rusqlite::params![id, runtime, runtime_session, pid],
    )
}

/// Resume an existing row in place: same id, same conversation thread.
/// A key the resume plan assigned wins; otherwise the stored key is kept.
pub fn resume_in_place(
    conn: &Connection,
    id: &str,
    started_at: Timestamp,
    assigned_key: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET status = 'running',
                pid = NULL,
                started_at = ?2,
                stopped_at = NULL,
                agent_session_key = COALESCE(?3, agent_session_key)
          WHERE id = ?1",
        rusqlite::params![id, started_at.to_rfc3339(), assigned_key],
    )
}

/// Terminal status flip after a PTY exits (or a pending spawn is
/// cancelled / fails). `status` is `stopped` or `crashed`.
pub fn set_exit_status(
    conn: &Connection,
    id: &str,
    status: SessionStatus,
    stopped_at: Timestamp,
) -> rusqlite::Result<usize> {
    let status = match status {
        SessionStatus::Running => "running",
        SessionStatus::Stopped => "stopped",
        SessionStatus::Crashed => "crashed",
    };
    conn.execute(
        "UPDATE sessions
            SET status = ?1, stopped_at = ?2
          WHERE id = ?3",
        rusqlite::params![status, stopped_at.to_rfc3339(), id],
    )
}

/// Resume-failure flip: the prior conversation was rejected, so the row
/// crashes AND forgets its key so the next launch starts fresh.
pub fn set_crashed_clearing_key(
    conn: &Connection,
    id: &str,
    stopped_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET status = ?1, stopped_at = ?2,
                agent_session_key = NULL
          WHERE id = ?3",
        rusqlite::params!["crashed", stopped_at.to_rfc3339(), id],
    )
}

/// Guarded capture of a codex conversation key. `agent_session_key IS
/// NULL` so a concurrent writer's key is never clobbered; `started_at`
/// equality so a stale watcher from a prior incarnation of the row can't
/// write into a later stop/resume. Returns whether the row was updated.
pub fn capture_agent_session_key(
    conn: &Connection,
    id: &str,
    agent_session_key: &str,
    expected_row_started_at: &str,
) -> rusqlite::Result<bool> {
    conn.execute(
        "UPDATE sessions
            SET agent_session_key = ?2
          WHERE id = ?1
            AND agent_session_key IS NULL
            AND started_at = ?3",
        rusqlite::params![id, agent_session_key, expected_row_started_at],
    )
    .map(|updated| updated > 0)
}

/// Startup cleanup: demote rows still marked `running` from a prior app
/// process to `stopped`, preserving any prior `stopped_at`.
pub fn cleanup_stale_running(conn: &Connection, now: Timestamp) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET status = 'stopped',
                stopped_at = COALESCE(stopped_at, ?1)
            WHERE status = 'running'",
        rusqlite::params![now.to_rfc3339()],
    )
}

/// Soft-delete: refused for running rows (kill first).
pub fn archive(conn: &Connection, id: &str, archived_at: Timestamp) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET archived_at = ?2
          WHERE id = ?1
            AND status != 'running'",
        rusqlite::params![id, archived_at.to_rfc3339()],
    )
}

/// Archive every non-archived session of a mission (mission reset).
pub fn archive_all_for_mission(
    conn: &Connection,
    mission_id: &str,
    archived_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET archived_at = ?1
          WHERE mission_id = ?2 AND archived_at IS NULL",
        rusqlite::params![archived_at.to_rfc3339(), mission_id],
    )
}

pub fn set_title(conn: &Connection, id: &str, title: Option<&str>) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET title = ?2 WHERE id = ?1",
        rusqlite::params![id, title],
    )
}

pub fn set_pinned_at(
    conn: &Connection,
    id: &str,
    pinned_at: Option<Timestamp>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET pinned_at = ?2 WHERE id = ?1",
        rusqlite::params![id, pinned_at.map(|t| t.to_rfc3339())],
    )
}

/// One mission session joined with its slot/runner labels — feeds the
/// `commands::session::SessionRow` IPC DTO.
#[derive(Debug, Clone)]
pub struct MissionSessionRow {
    pub session: Session,
    pub agent_session_key: Option<String>,
    pub handle: String,
    pub runtime: String,
    pub lead: bool,
}

fn session_from_row_db(row: SessionRowDb) -> rusqlite::Result<Session> {
    // Mission-session surfaces INNER JOIN runners, so runner_id is always
    // present; NULL would mean the query and this assembly drifted apart.
    let runner_id = row.runner_id.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Null,
            "mission session row missing runner_id".into(),
        )
    })?;
    Ok(Session {
        id: row.id,
        mission_id: row.mission_id,
        runner_id,
        slot_id: row.slot_id,
        cwd: row.cwd,
        status: row.status,
        pid: row.pid,
        started_at: row.started_at,
        stopped_at: row.stopped_at,
    })
}

/// Non-archived sessions of a mission in slot-roster order, joined with
/// the slot handle (template handle as fallback for pre-slot rows), the
/// runner's runtime, and the lead flag. Assembled per decision 6: session
/// side via `from_row`, denormalized extras via plain `row.get`.
pub fn list_for_mission(
    conn: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<MissionSessionRow>> {
    let sql = format!(
        "SELECT {},
                COALESCE(sl.slot_handle, r.handle) AS handle,
                r.runtime AS runner_runtime,
                COALESCE(sl.lead, 0) AS lead
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
           LEFT JOIN slots sl ON sl.id = s.slot_id
          WHERE s.mission_id = ?1
            AND s.archived_at IS NULL
          ORDER BY COALESCE(sl.position, 0) ASC, s.started_at ASC",
        qualified_select_list("s", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![mission_id], |row| {
        let db_row = from_row::<SessionRowDb>(row).map_err(de_err)?;
        let handle: String = row.get("handle")?;
        let runtime: String = row.get("runner_runtime")?;
        let lead: bool = row.get("lead")?;
        let agent_session_key = db_row.agent_session_key.clone();
        Ok(MissionSessionRow {
            session: session_from_row_db(db_row)?,
            agent_session_key,
            handle,
            runtime,
            lead,
        })
    })?;
    rows.collect()
}

/// A direct-chat session row plus the runner-template labels the sidebar
/// needs. `runner_*` fields are None for runtime-only chats (#195).
/// Direct chats are exactly rows with no mission and no slot; legacy
/// orphaned mission-slot rows can have `mission_id NULL` after the old
/// FK behavior, but their `slot_id` must keep them off this surface.
#[derive(Debug, Clone)]
pub struct DirectSessionRow {
    pub row: SessionRowDb,
    pub runner_handle: Option<String>,
    pub runner_display_name: Option<String>,
    pub runner_runtime: Option<String>,
    pub runner_command: Option<String>,
}

const DIRECT_EXTRAS: &str = "r.handle    AS runner_handle,
                r.display_name AS runner_display_name,
                r.runtime   AS runner_runtime,
                r.command   AS runner_command";

fn direct_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DirectSessionRow> {
    Ok(DirectSessionRow {
        row: from_row::<SessionRowDb>(row).map_err(de_err)?,
        runner_handle: row.get("runner_handle")?,
        runner_display_name: row.get("runner_display_name")?,
        runner_runtime: row.get("runner_runtime")?,
        runner_command: row.get("runner_command")?,
    })
}

/// Flat list of every un-archived direct session. Sort key:
///   1. pinned first (pinned_at NOT NULL)
///   2. then running before stopped/crashed
///   3. then by most-recent activity (stopped_at if set, else started_at)
pub fn list_recent_direct(conn: &Connection) -> rusqlite::Result<Vec<DirectSessionRow>> {
    let sql = format!(
        "SELECT {}, {DIRECT_EXTRAS}
           FROM sessions s
           LEFT JOIN runners r ON r.id = s.runner_id
          WHERE s.mission_id IS NULL
            AND s.slot_id IS NULL
            AND s.archived_at IS NULL
          ORDER BY CASE WHEN s.pinned_at IS NOT NULL THEN 0 ELSE 1 END,
                   CASE WHEN s.status = 'running'    THEN 0 ELSE 1 END,
                   COALESCE(s.stopped_at, s.started_at) DESC",
        qualified_select_list("s", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], direct_row)?;
    rows.collect()
}

/// Unfiltered single-row lookup for a direct-chat session — archived rows
/// ARE returned (the chat page renders them read-only; see the
/// archived-row tests in `commands::session`). Mission sessions are not:
/// this surface is for direct chats only.
pub fn get_direct(conn: &Connection, id: &str) -> rusqlite::Result<Option<DirectSessionRow>> {
    let sql = format!(
        "SELECT {}, {DIRECT_EXTRAS}
           FROM sessions s
           LEFT JOIN runners r ON r.id = s.runner_id
          WHERE s.id = ?1
            AND s.mission_id IS NULL
            AND s.slot_id IS NULL",
        qualified_select_list("s", COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![id], direct_row)
        .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn seed_runner(conn: &Connection, id: &str, handle: &str) {
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'shell', 'sh', ?4, ?4)",
            rusqlite::params![
                id,
                handle,
                format!("{handle} display"),
                "2026-04-22T00:00:00Z"
            ],
        )
        .unwrap();
    }

    fn full_row() -> SessionRowDb {
        let now = Utc::now();
        SessionRowDb {
            id: "sess-full".into(),
            mission_id: None,
            runner_id: Some("r1".into()),
            slot_id: None,
            cwd: Some("/tmp/work".into()),
            status: SessionStatus::Stopped,
            pid: Some(4242),
            started_at: Some(now),
            stopped_at: Some(now),
            agent_session_key: Some("2f6e0f2e-key".into()),
            archived_at: Some(now),
            title: Some("my chat".into()),
            pinned_at: Some(now),
            runtime: Some("pty".into()),
            runtime_socket: Some("sock".into()),
            runtime_session: Some("rt-sess".into()),
            runtime_window: Some("w0".into()),
            runtime_pane: Some("p0".into()),
            runtime_cursor: Some(7),
            agent_runtime: Some("codex".into()),
            agent_command: Some("codex".into()),
        }
    }

    fn minimal_row() -> SessionRowDb {
        SessionRowDb::new_running("sess-min".into())
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_runner(&conn, "r1", "alpha");
        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            assert_eq!(get_row(&conn, &row.id).unwrap().unwrap(), row);
        }
    }

    #[test]
    fn legacy_rows_in_todays_stored_formats_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_runner(&conn, "r1", "alpha");
        // Both timestamp spellings, a captured agent key, and legacy tmux
        // runtime metadata — the shapes real upgraded databases carry.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, cwd, status, pid, started_at, stopped_at,
                 agent_session_key, runtime, runtime_socket, runtime_session,
                 runtime_window, runtime_pane, runtime_cursor)
             VALUES ('sess-legacy', NULL, 'r1', '/tmp', 'crashed', 123,
                     '2026-04-22T01:02:03.456789+00:00', '2026-04-22T02:00:00Z',
                     'abcd-uuid', 'tmux', '/tmp/sock', 'tmux-0', '@1', '%1', 42)",
            [],
        )
        .unwrap();
        let row = get_row(&conn, "sess-legacy").unwrap().unwrap();
        assert_eq!(row.status, SessionStatus::Crashed);
        assert_eq!(row.pid, Some(123));
        assert_eq!(row.agent_session_key.as_deref(), Some("abcd-uuid"));
        assert_eq!(row.runtime.as_deref(), Some("tmux"));
        assert_eq!(row.runtime_cursor, Some(42));
        assert_eq!(
            row.started_at.unwrap(),
            "2026-04-22T01:02:03.456789Z".parse::<Timestamp>().unwrap()
        );
        assert_eq!(
            row.stopped_at.unwrap(),
            "2026-04-22T02:00:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_runner(&conn, "r1", "alpha");
        let row = full_row();
        insert(&conn, &row).unwrap();
        let (started_raw, status_raw): (String, String) = conn
            .query_row(
                "SELECT started_at, status FROM sessions WHERE id = 'sess-full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(started_raw, row.started_at.unwrap().to_rfc3339());
        assert_eq!(status_raw, "stopped");
    }

    #[test]
    fn capture_agent_session_key_is_guarded_by_null_key_and_started_at() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_runner(&conn, "r1", "alpha");
        let mut row = minimal_row();
        let started = Utc::now();
        row.runner_id = Some("r1".into());
        row.started_at = Some(started);
        insert(&conn, &row).unwrap();
        let started_str = started.to_rfc3339();

        // Stale watcher (wrong started_at) must not write.
        assert!(!capture_agent_session_key(
            &conn,
            "sess-min",
            "key-a",
            "1999-01-01T00:00:00+00:00"
        )
        .unwrap());
        // Matching guard writes.
        assert!(capture_agent_session_key(&conn, "sess-min", "key-a", &started_str).unwrap());
        // Second writer must not clobber.
        assert!(!capture_agent_session_key(&conn, "sess-min", "key-b", &started_str).unwrap());
        let stored: String = conn
            .query_row(
                "SELECT agent_session_key FROM sessions WHERE id = 'sess-min'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "key-a");
    }

    #[test]
    fn list_for_mission_matches_todays_dto_shape() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        seed_runner(&conn, "r1", "template");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at) VALUES ('c1', 'C', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('sl1', 'c1', 'r1', 'coder', 0, 1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m1', 'c1', 't', 'running', ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, slot_id, status, started_at, agent_session_key)
             VALUES ('se1', 'm1', 'r1', 'sl1', 'running', ?1, 'key-1')",
            rusqlite::params![now],
        )
        .unwrap();
        // Archived row must be filtered out.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, slot_id, status, started_at, archived_at)
             VALUES ('se-archived', 'm1', 'r1', 'sl1', 'stopped', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();

        let rows = list_for_mission(&conn, "m1").unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.session.id, "se1");
        assert_eq!(row.session.runner_id, "r1");
        assert_eq!(row.session.status, SessionStatus::Running);
        assert_eq!(row.handle, "coder", "slot handle wins over template handle");
        assert_eq!(row.runtime, "shell");
        assert!(row.lead);
        assert_eq!(row.agent_session_key.as_deref(), Some("key-1"));
    }

    #[test]
    fn direct_session_surfaces_join_runner_labels_and_keep_archived_semantics() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        seed_runner(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at, agent_session_key)
             VALUES ('d1', NULL, 'r1', 'stopped', ?1, 'resume-key')",
            rusqlite::params![now],
        )
        .unwrap();
        // Runtime-only chat (#195): no runner row behind it.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at, agent_runtime, agent_command)
             VALUES ('d2', NULL, NULL, 'running', ?1, 'codex', 'codex')",
            rusqlite::params![now],
        )
        .unwrap();
        // Archived: hidden from the list, still returned by get_direct.
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at, archived_at)
             VALUES ('d3', NULL, 'r1', 'stopped', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        // Legacy orphan from the old mission FK: no mission, but still
        // slot-bound, so it must never masquerade as a direct chat.
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, slot_id, status, started_at)
             VALUES ('orphan-slot', NULL, 'r1', 'slot-old', 'stopped', ?1)",
            rusqlite::params![now],
        )
        .unwrap();

        let listed = list_recent_direct(&conn).unwrap();
        let ids: Vec<&str> = listed.iter().map(|d| d.row.id.as_str()).collect();
        assert_eq!(ids, vec!["d2", "d1"], "running first, archived hidden");
        let d1 = listed.iter().find(|d| d.row.id == "d1").unwrap();
        assert_eq!(d1.runner_handle.as_deref(), Some("alpha"));
        assert_eq!(d1.runner_runtime.as_deref(), Some("shell"));
        let d2 = listed.iter().find(|d| d.row.id == "d2").unwrap();
        assert_eq!(d2.runner_handle, None);
        assert_eq!(d2.row.agent_runtime.as_deref(), Some("codex"));

        let archived = get_direct(&conn, "d3").unwrap().unwrap();
        assert!(archived.row.archived_at.is_some());
        assert!(
            get_direct(&conn, "orphan-slot").unwrap().is_none(),
            "slot-bound orphaned mission sessions must stay off direct-chat surfaces"
        );
    }
}

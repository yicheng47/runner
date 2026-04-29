// Session Tauri commands — thin wrappers over `session::SessionManager`.
//
// Spawn happens inside `mission_start` (see `commands::mission`), so there's
// no `session_spawn` here. The commands below let the frontend:
//   - list persisted sessions for a mission (including ones that have exited)
//   - inject bytes into a live session's stdin
//   - kill a live session
//
// `session/output` and `session/exit` events flow from the PTY reader threads
// directly via `AppHandle::emit`; the frontend subscribes without going
// through a command.

use std::sync::Arc;

use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    commands::runner,
    error::{Error, Result},
    model::{Session, SessionStatus, Timestamp},
    session::manager::{OutputEvent, SessionEvents, SpawnedSession, TauriSessionEvents},
    AppState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    #[serde(flatten)]
    pub session: Session,
    /// Handle of the runner this session instantiates — denormalized so the
    /// frontend can render `@coder`-style labels without a second lookup.
    pub handle: String,
    /// Whether this runner is the lead for the mission's crew.
    pub lead: bool,
}

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let status: String = row.get("status")?;
    let started_at: Option<String> = row.get("started_at")?;
    let stopped_at: Option<String> = row.get("stopped_at")?;

    let status = match status.as_str() {
        "running" => SessionStatus::Running,
        "stopped" => SessionStatus::Stopped,
        "crashed" => SessionStatus::Crashed,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown session status {other:?}").into(),
            ))
        }
    };
    let parse_ts = |s: String| -> rusqlite::Result<Timestamp> {
        s.parse().map_err(|e: chrono::ParseError| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    };
    Ok(SessionRow {
        session: Session {
            id: row.get("id")?,
            mission_id: row.get("mission_id")?,
            runner_id: row.get("runner_id")?,
            cwd: row.get("cwd")?,
            status,
            pid: row.get("pid")?,
            started_at: started_at.map(parse_ts).transpose()?,
            stopped_at: stopped_at.map(parse_ts).transpose()?,
        },
        handle: row.get("handle")?,
        lead: row.get("lead")?,
    })
}

#[tauri::command]
pub async fn session_list(
    state: State<'_, AppState>,
    mission_id: String,
) -> Result<Vec<SessionRow>> {
    // Order by the crew-scoped position of the runner within this mission's
    // crew, so the UI renders sessions in the same slot order as the Crew
    // Detail roster. `runners` is globally scoped post-C5.5a so we join
    // through `missions` + `crew_runners` to get the crew-local position.
    let conn = state.db.get()?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.mission_id, s.runner_id, s.cwd, s.status, s.pid,
                s.started_at, s.stopped_at, r.handle,
                COALESCE(cr.lead, 0) AS lead
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
           JOIN missions m ON m.id = s.mission_id
           LEFT JOIN crew_runners cr
                  ON cr.crew_id = m.crew_id AND cr.runner_id = s.runner_id
          WHERE s.mission_id = ?1
          ORDER BY cr.position ASC, s.started_at ASC",
    )?;
    let rows = stmt.query_map(params![mission_id], row_to_session)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[tauri::command]
pub async fn session_inject_stdin(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<()> {
    state.sessions.inject_stdin(&session_id, text.as_bytes())
}

#[tauri::command]
pub async fn session_kill(state: State<'_, AppState>, session_id: String) -> Result<()> {
    state.sessions.kill(&session_id)
}

#[tauri::command]
pub async fn session_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<()> {
    state.sessions.resize(&session_id, cols, rows)
}

#[tauri::command]
pub async fn session_output_snapshot(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<OutputEvent>> {
    Ok(state.sessions.output_snapshot(&session_id))
}

/// One row per direct-chat *session* in the sidebar SESSION tray. Each
/// runner can host multiple parallel chats — see
/// docs/impls/direct-chats.md — so the tray is flat (not collapsed per
/// runner). Stopped/crashed rows stay listed because they can be
/// resumed via `session_resume`, which preserves the row's id and
/// `agent_session_key`.
///
/// Click behavior on the frontend:
///   - `status = "running"` → attach to the live PTY.
///   - `status = "stopped" | "crashed"` → call `session_resume` (the
///     respawn happens server-side; the row stays the same), then
///     attach.
#[derive(Debug, Clone, Serialize)]
pub struct DirectSessionEntry {
    pub session_id: String,
    pub runner_id: String,
    pub handle: String,
    pub status: SessionStatus,
    /// User-authored label. NULL → frontend derives a default from
    /// handle + start time. Set via `session_rename`.
    pub title: Option<String>,
    /// Per-chat cwd override stored on the row at spawn. NULL means
    /// the chat falls back to the runner's `working_dir` on
    /// resume/spawn. Surfaced for the chat header's meta line.
    pub cwd: Option<String>,
    pub started_at: Option<Timestamp>,
    pub stopped_at: Option<Timestamp>,
    /// `true` iff `agent_session_key IS NOT NULL`. Lets the UI
    /// distinguish "stopped but resumable" from "stopped and
    /// forgotten" without shipping the raw key down.
    pub resumable: bool,
    /// `true` iff `pinned_at IS NOT NULL`. Pinned rows render with a
    /// pin glyph and sort to the top of the tray.
    pub pinned: bool,
}

#[tauri::command]
pub async fn session_list_recent_direct(
    state: State<'_, AppState>,
) -> Result<Vec<DirectSessionEntry>> {
    let conn = state.db.get()?;
    // Flat list: every un-archived direct session. Sort key:
    //   1. pinned first (pinned_at NOT NULL)
    //   2. then running before stopped/crashed
    //   3. then by most-recent activity (stopped_at if set, else
    //      started_at)
    let mut stmt = conn.prepare(
        "SELECT s.id        AS session_id,
                s.runner_id AS runner_id,
                r.handle    AS handle,
                s.status    AS status,
                s.title     AS title,
                s.cwd       AS cwd,
                s.started_at,
                s.stopped_at,
                CASE WHEN s.agent_session_key IS NOT NULL THEN 1 ELSE 0 END AS resumable,
                CASE WHEN s.pinned_at         IS NOT NULL THEN 1 ELSE 0 END AS pinned
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
          WHERE s.mission_id IS NULL
            AND s.archived_at IS NULL
          ORDER BY CASE WHEN s.pinned_at IS NOT NULL THEN 0 ELSE 1 END,
                   CASE WHEN s.status = 'running'    THEN 0 ELSE 1 END,
                   COALESCE(s.stopped_at, s.started_at) DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let status: String = row.get("status")?;
        let status = match status.as_str() {
            "running" => SessionStatus::Running,
            "stopped" => SessionStatus::Stopped,
            "crashed" => SessionStatus::Crashed,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    format!("unknown session status {other:?}").into(),
                ))
            }
        };
        let parse_ts = |s: String| -> rusqlite::Result<Timestamp> {
            s.parse().map_err(|e: chrono::ParseError| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        };
        let started_at: Option<String> = row.get("started_at")?;
        let stopped_at: Option<String> = row.get("stopped_at")?;
        let resumable: i64 = row.get("resumable")?;
        let pinned: i64 = row.get("pinned")?;
        Ok(DirectSessionEntry {
            session_id: row.get("session_id")?,
            runner_id: row.get("runner_id")?,
            handle: row.get("handle")?,
            status,
            title: row.get("title")?,
            cwd: row.get("cwd")?,
            started_at: started_at.map(parse_ts).transpose()?,
            stopped_at: stopped_at.map(parse_ts).transpose()?,
            resumable: resumable != 0,
            pinned: pinned != 0,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Soft-delete a session: hides it from the SESSION sidebar tray. The row
/// stays in the table so a future Archived workspace surface can still
/// surface it. Running sessions cannot be archived — kill them first.
#[tauri::command]
pub async fn session_archive(state: State<'_, AppState>, session_id: String) -> Result<()> {
    let conn = state.db.get()?;
    let now = chrono::Utc::now().to_rfc3339();
    let updated = conn.execute(
        "UPDATE sessions
            SET archived_at = ?2
          WHERE id = ?1
            AND status != 'running'",
        params![session_id, now],
    )?;
    if updated == 0 {
        return Err(Error::msg(
            "session not found or still running (kill before archiving)".to_string(),
        ));
    }
    // Drop the in-memory output buffer for this row. Forget intentionally
    // keeps the buffer alive across PTY exits so the chat can be reopened
    // and replayed; archive is the explicit "I'm done with this chat"
    // signal, so we let the buffer go.
    state.sessions.purge_session_buffers(&session_id);
    Ok(())
}

/// Set or clear the user-facing label for a direct-chat session. Pass
/// `None` (or an all-whitespace string, treated as None) to revert to
/// the auto-derived label (`@handle · <time>`). Trims surrounding
/// whitespace before persisting.
#[tauri::command]
pub async fn session_rename(
    state: State<'_, AppState>,
    session_id: String,
    title: Option<String>,
) -> Result<()> {
    let normalized: Option<String> = title.and_then(|t| {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let conn = state.db.get()?;
    let updated = conn.execute(
        "UPDATE sessions SET title = ?2 WHERE id = ?1",
        params![session_id, normalized],
    )?;
    if updated == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    Ok(())
}

/// Pin or unpin a direct-chat session in the SESSION sidebar tray.
/// Pinned sessions sort above running sessions in
/// `session_list_recent_direct` regardless of last activity. Setting
/// `pinned = false` clears `pinned_at`.
#[tauri::command]
pub async fn session_pin(
    state: State<'_, AppState>,
    session_id: String,
    pinned: bool,
) -> Result<()> {
    let conn = state.db.get()?;
    let updated = if pinned {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE sessions SET pinned_at = ?2 WHERE id = ?1",
            params![session_id, now],
        )?
    } else {
        conn.execute(
            "UPDATE sessions SET pinned_at = NULL WHERE id = ?1",
            params![session_id],
        )?
    };
    if updated == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    Ok(())
}

/// Respawn an existing direct-chat session row. Reuses the row's id and
/// `agent_session_key` so the agent CLI continues the prior conversation
/// (claude-code: `--resume <uuid>`; codex: `codex resume <uuid>` once the
/// key-capture path lands). See `SessionManager::resume` for the detailed
/// contract — refused for running rows, mission-scoped rows, and archived
/// rows.
#[tauri::command]
pub async fn session_resume(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    session_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let emitter: Arc<dyn SessionEvents> = Arc::new(TauriSessionEvents(app));
    state
        .sessions
        .resume(
            &session_id,
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| Error::msg(format!("session_resume: {e}")))
}

/// Spawn a "direct chat" session for a runner — a PTY with no parent
/// mission, no orchestrator, no event log (C8.5). Used by the Runner
/// Detail page's "Chat now" button: the user picks a working directory
/// and gets a one-on-one terminal with the agent's CLI.
///
/// `cwd` defaults to the runner's own `working_dir` when None — that's
/// what the spawn path resolves anyway, but exposing it on the row gives
/// future UI surfaces (session list, recent chats) something to show
/// without a second lookup against the runner config.
#[tauri::command]
pub async fn session_start_direct(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    runner_id: String,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    // Look up the runner under a short-lived connection so we don't hold
    // a pool slot across the spawn (which itself grabs a connection to
    // insert the `sessions` row).
    let runner = {
        let conn = state.db.get()?;
        runner::get(&conn, &runner_id)?
    };
    let emitter: Arc<dyn SessionEvents> = Arc::new(TauriSessionEvents(app));
    let spawned = state
        .sessions
        .spawn_direct(
            &runner,
            cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| Error::msg(format!("session_start_direct: {e}")))?;
    Ok(spawned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    /// Mirrors the SELECT in `session_list_recent_direct` so we can
    /// exercise the ORDER BY without a Tauri State. Returns
    /// (session_id, status, pinned) in the order the tray will render.
    fn list_recent_direct(conn: &rusqlite::Connection) -> Vec<(String, String, bool)> {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.status,
                        CASE WHEN s.pinned_at IS NOT NULL THEN 1 ELSE 0 END AS pinned
                   FROM sessions s
                   JOIN runners r ON r.id = s.runner_id
                  WHERE s.mission_id IS NULL
                    AND s.archived_at IS NULL
                  ORDER BY CASE WHEN s.pinned_at IS NOT NULL THEN 0 ELSE 1 END,
                           CASE WHEN s.status = 'running'    THEN 0 ELSE 1 END,
                           COALESCE(s.stopped_at, s.started_at) DESC",
            )
            .unwrap();
        stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            let status: String = r.get(1)?;
            let pinned: i64 = r.get(2)?;
            Ok((id, status, pinned != 0))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    #[test]
    fn pinned_sessions_sort_above_running_and_stopped() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now();
        let runner_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, role, runtime, command,
                 created_at, updated_at)
             VALUES (?1, 'r', 'R', 'test', 'shell', '/bin/sh', ?2, ?2)",
            params![runner_id, now.to_rfc3339()],
        )
        .unwrap();

        let insert =
            |id: &str, status: &str, started_offset_secs: i64, pinned: bool, archived: bool| {
                let started = (now - chrono::Duration::seconds(started_offset_secs)).to_rfc3339();
                let pinned_at: Option<String> = if pinned { Some(now.to_rfc3339()) } else { None };
                let archived_at: Option<String> = if archived {
                    Some(now.to_rfc3339())
                } else {
                    None
                };
                conn.execute(
                    "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at,
                     pinned_at, archived_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
                    params![id, runner_id, status, started, pinned_at, archived_at],
                )
                .unwrap();
            };

        // newest running, but not pinned
        insert("running-new", "running", 60, false, false);
        // older running, but pinned
        insert("running-pinned", "running", 600, true, false);
        // very old stopped, pinned
        insert("stopped-pinned-old", "stopped", 3600, true, false);
        // recent stopped, not pinned
        insert("stopped-recent", "stopped", 120, false, false);
        // archived: must not appear at all, even if pinned
        insert("pinned-archived", "stopped", 30, true, true);

        let rows = list_recent_direct(&conn);
        let ids: Vec<&str> = rows.iter().map(|(id, _, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                // pinned bucket sorted by recency: pinned-running (600s
                // old) is more recent than pinned-stopped (3600s old).
                "running-pinned",
                "stopped-pinned-old",
                // unpinned bucket: running before stopped, then recency.
                "running-new",
                "stopped-recent",
            ]
        );
        // Sanity: archived row never makes the list, regardless of pin.
        assert!(
            !ids.contains(&"pinned-archived"),
            "archived rows must be excluded from the SESSION tray"
        );
    }
}

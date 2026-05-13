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

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

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
            slot_id: row.get("slot_id")?,
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
    // Order by the slot-scoped position within this mission's crew so
    // the UI renders sessions in the same slot order as the Crew
    // Detail roster. The session's `slot_id` is the direct join key
    // into `slots`; `handle` is the slot's in-crew handle, and the
    // template handle is no longer used in mission contexts.
    // `r.handle` (template) is kept on the row for fallback display
    // (legacy mission sessions before 0006 have no slot_id).
    let conn = state.db.get()?;
    // archived_at IS NULL filters out the dead session rows that
    // `mission_reset` (and any future archive path) leaves behind: a
    // reset wipes the run context and inserts fresh PTY rows for the
    // same (mission_id, slot_id) pair, so without this filter the
    // sidebar would stack the old stopped row alongside the new
    // running one for every slot.
    let mut stmt = conn.prepare(
        "SELECT s.id, s.mission_id, s.runner_id, s.slot_id, s.cwd, s.status, s.pid,
                s.started_at, s.stopped_at,
                COALESCE(sl.slot_handle, r.handle) AS handle,
                COALESCE(sl.lead, 0) AS lead
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
           LEFT JOIN slots sl ON sl.id = s.slot_id
          WHERE s.mission_id = ?1
            AND s.archived_at IS NULL
          ORDER BY COALESCE(sl.position, 0) ASC, s.started_at ASC",
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

/// Restore NSPasteboard for a PNG paste that came in through the
/// webview, so the agent CLI's NSPasteboard read returns the real
/// bytes and renders its native `[Image x]` placeholder in the prompt.
///
/// Why: when the user presses Cmd+V over the WKWebView, WebKit
/// materializes the image clipboard item into a `File` object (a temp
/// file under the hood). As a side effect NSPasteboard's `public.png`
/// representation becomes the OS-rendered icon of that temp file
/// rather than the original screenshot bytes. The agent CLI's
/// subsequent clipboard read then returns the icon, not the screenshot
/// (#79).
///
/// Fix: the frontend grabs the original bytes off the `ClipboardEvent`
/// File before they reach the child process, ships them here, we
/// write them to a `NamedTempFile`, and use `osascript` to repopulate
/// NSPasteboard with the real bytes. The frontend then injects Ctrl-V
/// (`\x16`); the agent's existing paste-attach flow runs unchanged.
///
/// PNG-only. AppleScript writes the bytes verbatim into the
/// `public.png` pasteboard flavor, so non-PNG payloads would end up
/// labeled PNG with non-PNG bytes. The frontend filters to
/// `image/png` for the same reason. JPEG/GIF/WebP support is a
/// follow-up that would need either a per-MIME OSType map or a
/// transcode step.
///
/// macOS-only; on other platforms this is a no-op (the embedded
/// webview's paste behavior on Linux/Windows hasn't been audited and
/// the runner doesn't ship there yet).
#[tauri::command]
pub async fn session_paste_image(bytes: Vec<u8>) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        // NamedTempFile so the file is removed on drop — pasted
        // screenshots can be sensitive and shouldn't accumulate in
        // $TMPDIR. The OS would eventually reap them, but explicit
        // cleanup is cheaper and matches the value's lifetime:
        // osascript reads the bytes into NSPasteboard synchronously,
        // and after that we don't need the file.
        let mut tmp = tempfile::Builder::new()
            .prefix("runner-paste-")
            .suffix(".png")
            .tempfile_in(std::env::temp_dir())?;
        tmp.write_all(&bytes)?;
        tmp.flush()?;

        let path_str = tmp.path().to_string_lossy();
        // AppleScript: read the temp file's bytes and write them to
        // NSPasteboard's `public.png` representation. `«class PNGf»`
        // is the four-char OSType code for PNG. The `{:?}` debug
        // format quotes the path with `\\` / `\"` escapes that
        // AppleScript also accepts, so paths with spaces or quotes
        // pass through safely.
        let script = format!(
            "set the clipboard to (read POSIX file {:?} as «class PNGf»)",
            path_str
        );
        let status = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status()
            .map_err(|e| Error::msg(format!("osascript spawn failed: {e}")))?;
        // tmp drops here either way — file is removed regardless of
        // osascript's success.
        if !status.success() {
            return Err(Error::msg(format!(
                "osascript exited with status {:?}",
                status.code()
            )));
        }
        Ok(())
    }
}

/// One row per direct-chat *session* in the sidebar SESSION tray. Each
/// runner can host multiple parallel chats — see
/// docs/impls/0003-direct-chats.md — so the tray is flat (not collapsed per
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
    /// When set, the session has been archived: hidden from the SESSION
    /// tray and `session_list_recent_direct`. Returned here so
    /// `session_get` (unfiltered) can tell the chat page to render
    /// read-only when the user navigates to an archived session by
    /// direct URL. `listRecentDirect` filters these out at SQL, so
    /// rows from that surface always carry `archived_at: None`.
    pub archived_at: Option<Timestamp>,
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
                s.archived_at,
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
        let archived_at: Option<String> = row.get("archived_at")?;
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
            archived_at: archived_at.map(parse_ts).transpose()?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Unfiltered single-row lookup for a direct-chat session.
///
/// `session_list_recent_direct` hides archived rows (`archived_at IS
/// NOT NULL`) from the SESSION tray, but the `/runners/:handle/chat/
/// :sessionId` route still mounts when the user navigates by direct
/// URL. RunnerChat falls back to this helper when the list lookup
/// misses so it can detect an archived row and render the workspace
/// read-only (no PTY attach, no Resume, no live composer) instead of
/// silently failing to find the session.
///
/// Returns `None` if the id doesn't exist. Mission sessions are not
/// returned here — this command is for direct chats only, matching
/// the surface that `listRecentDirect` covers.
///
/// The SQL lives in `get_direct(...)` below so the test module can
/// exercise the real query against an in-memory DB without spinning
/// up an `AppState`. If a future refactor adds `AND archived_at IS
/// NULL` to that helper's WHERE clause, the archived-row tests fail
/// and the chat-page lockdown stays honest.
#[tauri::command]
pub async fn session_get(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<DirectSessionEntry>> {
    let conn = state.db.get()?;
    get_direct(&conn, &session_id)
}

fn get_direct(conn: &rusqlite::Connection, session_id: &str) -> Result<Option<DirectSessionEntry>> {
    let mut stmt = conn.prepare(
        "SELECT s.id        AS session_id,
                s.runner_id AS runner_id,
                r.handle    AS handle,
                s.status    AS status,
                s.title     AS title,
                s.cwd       AS cwd,
                s.started_at,
                s.stopped_at,
                s.archived_at,
                CASE WHEN s.agent_session_key IS NOT NULL THEN 1 ELSE 0 END AS resumable,
                CASE WHEN s.pinned_at         IS NOT NULL THEN 1 ELSE 0 END AS pinned
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
          WHERE s.id = ?1
            AND s.mission_id IS NULL",
    )?;
    let row = stmt
        .query_row(params![session_id], |row| {
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
            let archived_at: Option<String> = row.get("archived_at")?;
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
                archived_at: archived_at.map(parse_ts).transpose()?,
            })
        })
        .optional()?;
    Ok(row)
}

/// Soft-delete a session: hides it from the SESSION sidebar tray. The row
/// stays in the table so a future Archived workspace surface can still
/// surface it. Running sessions cannot be archived — kill them first.
///
/// Emits a `session/archived` Tauri event after the row flips so the
/// sidebar's CHAT list can refresh — without it, archiving from the
/// chat page (RunnerChat's SessionEnded overlay) would archive the
/// row but leave the sidebar stale until something else triggered a
/// refresh.
#[tauri::command]
pub async fn session_archive(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<()> {
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
    let _ = app.emit(
        "session/archived",
        serde_json::json!({ "session_id": session_id }),
    );
    Ok(())
}

/// Set or clear the user-facing label for a direct-chat session. Pass
/// `None` (or an all-whitespace string, treated as None) to revert to
/// the auto-derived label (`@handle · <time>`). Trims surrounding
/// whitespace before persisting.
///
/// Emits a `session/updated` Tauri event after the row flips so the
/// sidebar's CHAT list can refresh — without it, renaming from the
/// chat-page kebab would update the row but leave the sidebar's
/// title stale until some other refresh trigger fires.
#[tauri::command]
pub async fn session_rename(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
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
    let _ = app.emit(
        "session/updated",
        serde_json::json!({ "session_id": session_id }),
    );
    Ok(())
}

/// Pin or unpin a direct-chat session in the SESSION sidebar tray.
/// Pinned sessions sort above running sessions in
/// `session_list_recent_direct` regardless of last activity. Setting
/// `pinned = false` clears `pinned_at`.
///
/// Emits a `session/updated` Tauri event after the row flips so the
/// sidebar's CHAT list can refresh — same rationale as
/// `session_rename` above.
#[tauri::command]
pub async fn session_pin(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
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
    let _ = app.emit(
        "session/updated",
        serde_json::json!({ "session_id": session_id }),
    );
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
    let spawned = state
        .sessions
        .resume(
            &session_id,
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| Error::msg(format!("session_resume: {e}")))?;
    // Fresh-fallback for a lead slot: the prior claude-code conversation
    // file was missing, so the resume degraded to a `--session-id` fresh
    // spawn. The bus's mission_goal handler is suppressed on resume by
    // `mission_attach`'s reconstruction watermark, so without this call
    // the lead's fresh agent comes up with no system context. Fire the
    // launch prompt manually through the registered router.
    if spawned.fresh_fallback_lead {
        if let Some(mission_id) = spawned.mission_id.as_deref() {
            if let Some(router) = state.routers.get(mission_id) {
                router.fire_lead_launch_prompt();
            }
        }
    }
    Ok(spawned)
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
    // Compose the persona first-user-turn body upstream so the spawn
    // path can deliver it via the positional `[PROMPT]` argv at
    // process boot — eliminating the post-spawn paste race the
    // verify loop was working around. Direct chats are off-bus, so
    // the body is just the runner's `system_prompt` (no worker
    // coordination preamble). See
    // `docs/impls/0007-spawn-time-prompt-delivery.md`.
    let first_turn =
        crate::router::prompt::compose_direct_first_turn(runner.system_prompt.as_deref());
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
            first_turn,
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
                (id, handle, display_name, runtime, command,
                 created_at, updated_at)
             VALUES (?1, 'r', 'R', 'shell', '/bin/sh', ?2, ?2)",
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

    fn seed_runner(conn: &rusqlite::Connection) -> String {
        let runner_id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command,
                 created_at, updated_at)
             VALUES (?1, 'r', 'R', 'shell', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        runner_id
    }

    fn insert_direct_session(
        conn: &rusqlite::Connection,
        runner_id: &str,
        archived: bool,
    ) -> String {
        let id = ulid::Ulid::new().to_string();
        let now = Utc::now();
        let archived_at: Option<String> = if archived {
            Some(now.to_rfc3339())
        } else {
            None
        };
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at, archived_at)
             VALUES (?1, NULL, ?2, 'stopped', ?3, ?4)",
            params![id, runner_id, now.to_rfc3339(), archived_at],
        )
        .unwrap();
        id
    }

    #[test]
    fn session_get_returns_archived_row() {
        // Whole reason this command exists: listRecentDirect filters
        // archived rows, so RunnerChat needs an unfiltered fallback to
        // detect an archived direct-URL navigation and render
        // read-only. A future refactor that adds `archived_at IS NULL`
        // to get_direct's WHERE breaks the chat-page lockdown — this
        // test fails if it ever does.
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let session_id = insert_direct_session(&conn, &runner_id, /*archived*/ true);

        let row = get_direct(&conn, &session_id).unwrap();
        let row = row.expect("archived row must be returned");
        assert_eq!(row.session_id, session_id);
        assert!(
            row.archived_at.is_some(),
            "archived_at must be populated for archived rows"
        );
    }

    #[test]
    fn session_get_populates_archived_at_when_set() {
        // Belt-and-suspenders for the column round-trip: archived
        // direct sessions returned by get_direct must carry the
        // timestamp the row was archived with, not a recoded `now()`.
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        let archived_at = "2025-01-01T00:00:00+00:00";
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at, archived_at)
             VALUES (?1, NULL, ?2, 'stopped', ?3, ?4)",
            params![id, runner_id, now, archived_at],
        )
        .unwrap();

        let row = get_direct(&conn, &id).unwrap().unwrap();
        let got = row.archived_at.expect("archived_at populated").to_rfc3339();
        assert_eq!(got, archived_at);
    }

    #[test]
    fn session_get_returns_none_for_unknown_id() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = get_direct(&conn, "01HZUNKNOWNUNKNOWNUNKNOWN").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn session_get_returns_none_for_mission_session() {
        // `mission_id IS NULL` filter scopes this command to direct
        // chats only — mission sessions go through `session_list`
        // instead. Dropping that filter would let an archived
        // mission's PTY row leak into RunnerChat's lookup and confuse
        // the read-only branch (mission sessions don't have a
        // /runners/:handle/chat URL).
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);

        // Seed a crew + mission so the FK is satisfied. The mission
        // table's other NOT NULL columns are populated minimally.
        let crew_id = ulid::Ulid::new().to_string();
        let mission_id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
            params![crew_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
            params![mission_id, crew_id, now],
        )
        .unwrap();
        let session_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at)
             VALUES (?1, ?2, ?3, 'stopped', ?4)",
            params![session_id, mission_id, runner_id, now],
        )
        .unwrap();

        let row = get_direct(&conn, &session_id).unwrap();
        assert!(
            row.is_none(),
            "mission sessions must not leak through session_get"
        );
    }
}

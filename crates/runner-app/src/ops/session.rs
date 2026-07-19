// Session command bodies — thin wrappers over `session::SessionManager`.
//
// Spawn happens inside `mission_start` (see `ops::mission`), so there's
// no `session_spawn` here. The commands below let the frontend:
//   - list persisted sessions for a mission (including ones that have exited)
//   - inject bytes into a live session's stdin
//   - kill a live session
//
// `session/output` and `session/exit` events flow from the PTY reader threads
// onto the app event channel; the frontend subscribes without going
// through a command.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    model::{Runner, Session, SessionStatus, Timestamp},
    ops::{project, runner},
    repo,
    session::manager::{
        runtime_direct_runner, OutputEvent, SessionActivityState, SessionEvents, SpawnedSession,
    },
    AppCore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    #[serde(flatten)]
    pub session: Session,
    /// Handle of the runner this session instantiates — denormalized so the
    /// frontend can render `@coder`-style labels without a second lookup.
    pub handle: String,
    /// Effective runtime kind for this session (`"claude-code"`,
    /// `"codex"`, `"shell"`, …): `sessions.agent_runtime` when the row
    /// recorded one (runtime-override spawns), else the runner row's
    /// `runtime`. Denormalized onto SessionRow so the frontend's
    /// terminal pane can gate per-runtime UX decisions
    /// (clear-on-resize for full-screen TUIs, etc.) without a second
    /// runner lookup. See docs/impls/archive/0011 §"Per-runtime clear-on-resize".
    pub runtime: String,
    /// Whether this runner is the lead for the mission's crew.
    pub lead: bool,
    /// Native agent conversation key captured from the spawned agent.
    /// NULL means capture is pending, unavailable, or intentionally
    /// failed closed.
    pub agent_session_key: Option<String>,
}

pub fn list_for_mission(conn: &rusqlite::Connection, mission_id: &str) -> Result<Vec<SessionRow>> {
    // Sessions render in the same slot order as the Crew Detail roster;
    // `handle` is the slot's in-crew handle with the template handle as
    // fallback for legacy pre-slot rows, and archived rows (mission
    // reset leftovers) are filtered out. The ordering, join, and filter
    // semantics live in `repo::session::list_for_mission`.
    let rows = repo::session::list_for_mission(conn, mission_id)?;
    Ok(rows
        .into_iter()
        .map(|row| SessionRow {
            session: row.session,
            handle: row.handle,
            runtime: row.runtime,
            lead: row.lead,
            agent_session_key: row.agent_session_key,
        })
        .collect())
}

pub fn session_list(state: &AppCore, mission_id: &str) -> Result<Vec<SessionRow>> {
    let conn = state.db.get()?;
    list_for_mission(&conn, mission_id)
}

pub fn session_inject_stdin(state: &AppCore, session_id: &str, text: &str) -> Result<()> {
    state
        .sessions
        .inject_direct_stdin(session_id, text.as_bytes(), &state.session_events())
}

pub fn session_kill(state: &AppCore, session_id: &str) -> Result<()> {
    state.sessions.kill(session_id)
}

pub fn session_activity_snapshot(state: &AppCore) -> BTreeMap<String, SessionActivityState> {
    state.sessions.activity_snapshot()
}

pub fn session_resize(state: &AppCore, session_id: &str, cols: u16, rows: u16) -> Result<()> {
    state.sessions.resize(session_id, cols, rows, &state.db)
}

pub fn session_output_snapshot(state: &AppCore, session_id: &str) -> Result<Vec<OutputEvent>> {
    Ok(state.sessions.output_snapshot(session_id))
}

/// The seq the output ring had reached when the session's most
/// recent resume started (0 for sessions that never resumed). A
/// dedicated read command rather than a field on the snapshot or the
/// resume RPC: the snapshot's return shape is consumed as a bare
/// array by terminal replay, and a resume can be triggered from
/// another window (impl 0018), so the pill effects can't rely on the
/// resume response reaching them.
pub fn session_replay_watermark(state: &AppCore, session_id: &str) -> Result<u64> {
    Ok(state.sessions.replay_watermark(session_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PasteImageFormat {
    extension: &'static str,
    pasteboard_class: &'static str,
}

fn paste_image_format(mime_type: &str) -> Result<PasteImageFormat> {
    let normalized = mime_type.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "image/png" => Ok(PasteImageFormat {
            extension: "png",
            pasteboard_class: "PNGf",
        }),
        "image/jpeg" | "image/jpg" => Ok(PasteImageFormat {
            extension: "jpg",
            pasteboard_class: "JPEG",
        }),
        _ => Err(Error::msg(format!(
            "unsupported clipboard image type {mime_type:?}"
        ))),
    }
}

/// Restore NSPasteboard for an image paste that came in through the
/// webview, so the agent CLI's NSPasteboard read returns the real
/// bytes and renders its native `[Image x]` placeholder in the prompt.
///
/// Why: when the user presses Cmd+V over the WKWebView, WebKit
/// materializes the image clipboard item into a `File` object (a temp
/// file under the hood). As a side effect NSPasteboard's image
/// representation can become the OS-rendered icon of that temp file
/// rather than the original image bytes. The agent CLI's subsequent
/// clipboard read then returns the icon, not the copied image (#79).
///
/// Fix: the frontend grabs the original bytes off the `ClipboardEvent`
/// File before they reach the child process, ships them here with the
/// image MIME type, we write them to a `NamedTempFile`, and use
/// `osascript` to repopulate NSPasteboard with the matching image
/// flavor. The frontend then injects Ctrl-V (`\x16`); the agent's
/// existing paste-attach flow runs unchanged.
///
/// macOS-only; on other platforms this is a no-op (the embedded
/// webview's paste behavior on Linux/Windows hasn't been audited and
/// the runner doesn't ship there yet).
pub fn session_paste_image(bytes: Vec<u8>, mime_type: &str) -> Result<()> {
    let format = paste_image_format(mime_type)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        let _ = format;
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
        let suffix = format!(".{}", format.extension);
        let mut tmp = tempfile::Builder::new()
            .prefix("runner-paste-")
            .suffix(&suffix)
            .tempfile_in(std::env::temp_dir())?;
        tmp.write_all(&bytes)?;
        tmp.flush()?;

        let path_str = tmp.path().to_string_lossy();
        // AppleScript: read the temp file's bytes and write them to
        // NSPasteboard's matching image representation. The
        // `pasteboard_class` values are fixed OSType codes selected
        // from the MIME allowlist above. The `{:?}` debug format
        // quotes the path with `\\` / `\"` escapes that AppleScript
        // also accepts, so paths with spaces or quotes pass through
        // safely.
        let script = format!(
            "set the clipboard to (read POSIX file {:?} as «class {}»)",
            path_str, format.pasteboard_class
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
/// docs/impls/archive/0003-direct-chats.md — so the tray is flat (not collapsed per
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
    pub project_id: Option<String>,
    pub runner_id: Option<String>,
    pub handle: Option<String>,
    pub agent_runtime: String,
    pub agent_command: String,
    pub display_name: String,
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
    /// Native agent conversation key for the active direct chat.
    /// `session_list_recent_direct` intentionally returns NULL here;
    /// `session_get` is the full-detail path RunnerChat uses for the
    /// visible row.
    pub agent_session_key: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
pub struct StartDirectSessionOutput {
    #[serde(flatten)]
    pub session: SpawnedSession,
    pub project_id: Option<String>,
    pub cwd: Option<String>,
}

/// Assemble the IPC entry from a repo direct-session row. `ship_key`
/// distinguishes the two surfaces: `session_get` returns the raw
/// `agent_session_key`, while the recent list intentionally ships NULL.
fn direct_entry_from_repo(
    d: repo::session::DirectSessionRow,
    ship_key: bool,
) -> Result<DirectSessionEntry> {
    let handle = d.runner_handle;
    let agent_runtime = d
        .row
        .agent_runtime
        .or(d.runner_runtime)
        .ok_or_else(|| Error::msg(format!("session {} has no agent_runtime", d.row.id)))?;
    let agent_command = d
        .row
        .agent_command
        .or(d.runner_command)
        .ok_or_else(|| Error::msg(format!("session {} has no agent_command", d.row.id)))?;
    let display_name = d
        .runner_display_name
        .filter(|_| handle.is_some())
        .unwrap_or_else(|| crate::router::runtime::runtime_display_name(&agent_runtime));
    Ok(DirectSessionEntry {
        session_id: d.row.id,
        project_id: d.row.project_id,
        runner_id: d.row.runner_id,
        handle,
        agent_runtime,
        agent_command,
        display_name,
        status: d.row.status,
        title: d.row.title,
        cwd: d.row.cwd,
        started_at: d.row.started_at,
        stopped_at: d.row.stopped_at,
        resumable: d.row.agent_session_key.is_some(),
        agent_session_key: if ship_key {
            d.row.agent_session_key
        } else {
            None
        },
        pinned: d.row.pinned_at.is_some(),
        archived_at: d.row.archived_at,
    })
}

pub fn session_list_recent_direct(state: &AppCore) -> Result<Vec<DirectSessionEntry>> {
    let conn = state.db.get()?;
    repo::session::list_recent_direct(&conn)?
        .into_iter()
        .map(|d| direct_entry_from_repo(d, /*ship_key*/ false))
        .collect()
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
/// Returns `None` if the id doesn't exist. Mission sessions and
/// slot-bound legacy orphans are not returned here — this command is
/// for direct chats only, matching the surface that `listRecentDirect`
/// covers.
///
/// The SQL lives in `get_direct(...)` below so the test module can
/// exercise the real query against an in-memory DB without spinning
/// up an `AppCore`. If a future refactor adds `AND archived_at IS
/// NULL` to that helper's WHERE clause, the archived-row tests fail
/// and the chat-page lockdown stays honest.
pub fn session_get(state: &AppCore, session_id: &str) -> Result<Option<DirectSessionEntry>> {
    let conn = state.db.get()?;
    get_direct(&conn, session_id)
}

fn get_direct(conn: &rusqlite::Connection, session_id: &str) -> Result<Option<DirectSessionEntry>> {
    repo::session::get_direct(conn, session_id)?
        .map(|d| direct_entry_from_repo(d, /*ship_key*/ true))
        .transpose()
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
pub fn session_archive(state: &AppCore, session_id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let updated = repo::session::archive(&tx, session_id, chrono::Utc::now())?;
    if updated == 0 {
        return Err(Error::msg(
            "session not found or still running (kill before archiving)".to_string(),
        ));
    }
    repo::tab::remove_session(&tx, session_id)?;
    tx.commit()?;
    // Drop the in-memory output buffer for this row. Forget intentionally
    // keeps the buffer alive across PTY exits so the chat can be reopened
    // and replayed; archive is the explicit "I'm done with this chat"
    // signal, so we let the buffer go.
    state.sessions.purge_session_buffers(session_id);
    state.events.emit(
        "session/archived",
        &serde_json::json!({ "session_id": session_id }),
    );
    Ok(())
}

/// Clear a direct session's archive marker so it rejoins the SESSION
/// tray (Settings → Archived restore). Single-column flip — status,
/// title, and `agent_session_key` survive, so a restored chat can
/// still resume. Idempotent: unarchiving an active direct chat is a
/// no-op Ok. Unknown ids and mission/slot-bound rows error — the
/// Archived pane only lists direct chats, and restoring a reset
/// leftover would leak it back into `session_list` for its mission.
///
/// Emits a `session/updated` Tauri event after the flip — the same
/// channel `session_rename` / `session_pin` use — so the sidebar's
/// CHAT list picks the restored row back up without a refresh.
pub fn session_unarchive(state: &AppCore, session_id: &str) -> Result<()> {
    let conn = state.db.get()?;
    let updated = repo::session::unarchive_direct(&conn, session_id)?;
    if updated == 0 {
        // Split the no-op (already-active direct chat) from the two
        // refusals the scoped UPDATE can't distinguish.
        match repo::session::get_row(&conn, session_id)? {
            None => {
                return Err(Error::msg(format!("session not found: {session_id}")));
            }
            Some(row) if row.mission_id.is_some() || row.slot_id.is_some() => {
                return Err(Error::msg(format!(
                    "session {session_id} is mission-scoped; only direct chats can be unarchived"
                )));
            }
            Some(_) => {}
        }
    }
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
    );
    Ok(())
}

/// Permanently delete an archived direct chat (Settings → Archived
/// delete, feature 01 Phase 4). Refused for non-archived and
/// mission-scoped rows — archive is the reversible step, delete is
/// not. Runner keeps nothing else on disk for a direct chat (the
/// agent's own JSONL belongs to the agent runtime), so the row delete
/// is the whole cleanup; the scrollback buffer was already purged at
/// archive time.
pub fn session_delete(state: &AppCore, session_id: &str) -> Result<()> {
    let conn = state.db.get()?;
    let deleted = repo::session::delete_archived_direct(&conn, session_id)?;
    if deleted == 0 {
        // Split not-found from the two refusals the scoped DELETE
        // can't distinguish, mirroring `session_unarchive`.
        return match repo::session::get_row(&conn, session_id)? {
            None => Err(Error::msg(format!("session not found: {session_id}"))),
            Some(row) if row.mission_id.is_some() || row.slot_id.is_some() => Err(Error::msg(
                format!("session {session_id} is mission-scoped; only direct chats can be deleted"),
            )),
            Some(_) => Err(Error::msg(format!(
                "session {session_id} is not archived; archive it before deleting"
            ))),
        };
    }
    Ok(())
}

/// Archived direct sessions, newest-archived first — the Settings →
/// Archived pane's chat list. Same DTO as `session_list_recent_direct`
/// (keys withheld); archived mission-slot rows stay off this surface.
pub fn session_list_archived(state: &AppCore) -> Result<Vec<DirectSessionEntry>> {
    let conn = state.db.get()?;
    repo::session::list_archived_direct(&conn)?
        .into_iter()
        .map(|d| direct_entry_from_repo(d, /*ship_key*/ false))
        .collect()
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
pub fn session_rename(state: &AppCore, session_id: &str, title: Option<String>) -> Result<()> {
    let normalized: Option<String> = title.and_then(|t| {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let conn = state.db.get()?;
    let updated = repo::session::set_title(&conn, session_id, normalized.as_deref())?;
    if updated == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
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
pub fn session_pin(state: &AppCore, session_id: &str, pinned: bool) -> Result<()> {
    let conn = state.db.get()?;
    let pinned_at = if pinned {
        Some(chrono::Utc::now())
    } else {
        None
    };
    let updated = repo::session::set_pinned_at(&conn, session_id, pinned_at)?;
    if updated == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
    );
    Ok(())
}

/// Respawn an existing direct-chat session row. Reuses the row's id and
/// `agent_session_key` so the agent CLI continues the prior conversation
/// (claude-code: `--resume <uuid>`; codex: `codex resume <uuid>` once the
/// key-capture path lands). See `SessionManager::resume` for the detailed
/// contract — refused for running rows, mission-scoped rows, and archived
/// rows.
pub fn session_resume(
    state: &AppCore,
    session_id: &str,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    // Dims decide the PTY fork size; None forks at the 80×24 default and
    // a TUI's instant banner then renders garbled at pane width. Logged
    // so a garbled-pane report can be traced back to its resume geometry.
    log::info!("session_resume: session={session_id} cols={cols:?} rows={rows:?}");
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let spawned = state
        .sessions
        .resume(
            session_id,
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
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
    );
    Ok(spawned)
}

/// Spawn a "direct chat" session for a runner — a PTY with no parent
/// mission, no orchestrator, no event log (C8.5). Used by the Runner
/// Detail page's "Chat now" button: the user picks a working directory
/// and gets a one-on-one terminal with the agent's CLI.
///
/// Working-directory precedence is explicit `cwd`, project cwd, then the
/// runner's own `working_dir`.
pub(crate) fn resolve_direct_start(
    conn: &rusqlite::Connection,
    runner_id: &str,
    project_id: Option<&str>,
    cwd: Option<String>,
) -> Result<(Runner, Option<String>)> {
    let cwd = project::resolve_cwd(conn, project_id, cwd)?;
    let runner = runner::get(conn, runner_id)?;
    let effective_cwd = cwd.or_else(|| runner.working_dir.clone());
    Ok((runner, effective_cwd))
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_direct_impl(
    state: &AppCore,
    runner_id: String,
    runtime: Option<String>,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<StartDirectSessionOutput> {
    let (runner, effective_cwd) = {
        let conn = state.db.get()?;
        resolve_direct_start(&conn, &runner_id, project_id.as_deref(), cwd)?
    };
    let first_turn =
        crate::router::prompt::compose_direct_first_turn(runner.system_prompt.as_deref());
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let session = state
        .sessions
        .spawn_direct(
            &runner,
            runtime.as_deref(),
            project_id.as_deref(),
            effective_cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
            first_turn,
        )
        .map_err(|e| Error::msg(format!("session_start_direct: {e}")))?;
    Ok(StartDirectSessionOutput {
        session,
        project_id,
        cwd: effective_cwd,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_direct(
    state: &AppCore,
    runner_id: String,
    runtime: Option<String>,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    Ok(session_start_direct_impl(state, runner_id, runtime, project_id, cwd, cols, rows)?.session)
}

pub fn session_start_runtime(
    state: &AppCore,
    runtime: &str,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let runner = runtime_direct_runner(runtime, None)?;
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let spawned = state
        .sessions
        .spawn_runtime_direct(
            &runner,
            project_id.as_deref(),
            cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| Error::msg(format!("session_start_runtime: {e}")))?;
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": spawned.id }),
    );
    Ok(spawned)
}

pub fn session_set_project(
    state: &AppCore,
    session_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<()> {
    let mut conn = state.db.get()?;
    repo::session::set_project_for_direct_sessions(&mut conn, &session_ids, project_id.as_deref())
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                Error::msg("one or more direct sessions were not found or are archived")
            }
            error => error.into(),
        })?;
    if let Some(session_id) = session_ids.first() {
        state.events.emit(
            "session/updated",
            &serde_json::json!({ "session_id": session_id }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;
    use rusqlite::params;

    #[test]
    fn paste_image_format_maps_png_and_jpeg_to_pasteboard_classes() {
        assert_eq!(
            paste_image_format("image/png").unwrap(),
            PasteImageFormat {
                extension: "png",
                pasteboard_class: "PNGf",
            }
        );
        assert_eq!(
            paste_image_format("image/jpeg").unwrap(),
            PasteImageFormat {
                extension: "jpg",
                pasteboard_class: "JPEG",
            }
        );
    }

    #[test]
    fn paste_image_format_normalizes_case_and_jpg_alias() {
        assert_eq!(
            paste_image_format(" IMAGE/JPG ").unwrap(),
            PasteImageFormat {
                extension: "jpg",
                pasteboard_class: "JPEG",
            }
        );
    }

    #[test]
    fn paste_image_format_rejects_unsupported_image_types() {
        let err = paste_image_format("image/gif").unwrap_err().to_string();
        assert!(err.contains("unsupported clipboard image type"));
    }

    /// Mirrors the SELECT in `session_list_recent_direct` so we can
    /// exercise the ORDER BY without a Tauri State. Returns
    /// (session_id, status, pinned) in the order the tray will render.
    fn list_recent_direct(conn: &rusqlite::Connection) -> Vec<(String, String, bool)> {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.status,
                        CASE WHEN s.pinned_at IS NOT NULL THEN 1 ELSE 0 END AS pinned
                   FROM sessions s
                   LEFT JOIN runners r ON r.id = s.runner_id
                  WHERE s.mission_id IS NULL
                    AND s.slot_id IS NULL
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

    #[test]
    fn resolve_direct_start_defaults_cwd_from_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();

        let (runner, cwd) =
            resolve_direct_start(&conn, &runner_id, Some(&project.id), None).unwrap();

        assert_eq!(runner.id, runner_id);
        assert_eq!(cwd.as_deref(), Some("/project"));
    }

    #[test]
    fn resolve_direct_start_explicit_cwd_overrides_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();

        let (_, cwd) = resolve_direct_start(
            &conn,
            &runner_id,
            Some(&project.id),
            Some("/override".into()),
        )
        .unwrap();

        assert_eq!(cwd.as_deref(), Some("/override"));
    }

    #[test]
    fn resolve_direct_start_unknown_project_creates_no_session() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);

        let error = resolve_direct_start(&conn, &runner_id, Some("missing"), None).unwrap_err();

        assert_eq!(error.to_string(), "project not found: missing");
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(session_count, 0);
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
    fn delete_archived_direct_only_deletes_archived_direct_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let active = insert_direct_session(&conn, &runner_id, false);
        let archived = insert_direct_session(&conn, &runner_id, true);
        // Archived slot leftover (mission reset): must survive this
        // path — it dies with its mission, not through chat delete.
        let slot_bound = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, slot_id, runner_id, status, started_at, archived_at)
             VALUES (?1, NULL, 'slot-1', ?2, 'stopped', ?3, ?3)",
            params![slot_bound, runner_id, Utc::now().to_rfc3339()],
        )
        .unwrap();

        assert_eq!(
            repo::session::delete_archived_direct(&conn, &active).unwrap(),
            0,
            "active chats must be archived before deletion"
        );
        assert_eq!(
            repo::session::delete_archived_direct(&conn, &slot_bound).unwrap(),
            0,
            "slot-bound rows must not be deletable as chats"
        );
        assert_eq!(
            repo::session::delete_archived_direct(&conn, &archived).unwrap(),
            1
        );

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2, "only the archived direct row is gone");
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
    fn session_get_returns_agent_session_key_for_direct_chat() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        let key = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at, agent_session_key)
             VALUES (?1, NULL, ?2, 'stopped', ?3, ?4)",
            params![id, runner_id, now, key],
        )
        .unwrap();

        let row = get_direct(&conn, &id).unwrap().unwrap();
        assert_eq!(row.agent_session_key.as_deref(), Some(key.as_str()));
        assert!(row.resumable);
    }

    #[test]
    fn session_list_returns_agent_session_key_for_mission_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn);
        let crew_id = ulid::Ulid::new().to_string();
        let slot_id = ulid::Ulid::new().to_string();
        let mission_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        let key = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
            params![crew_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, ?2, ?3, 'coder', 0, 1, ?4)",
            params![slot_id, crew_id, runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
            params![mission_id, crew_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, slot_id, status, started_at, agent_session_key)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6)",
            params![session_id, mission_id, runner_id, slot_id, now, key],
        )
        .unwrap();

        let rows = list_for_mission(&conn, &mission_id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session.id, session_id);
        assert_eq!(rows[0].handle, "coder");
        assert_eq!(rows[0].agent_session_key.as_deref(), Some(key.as_str()));
    }

    #[test]
    fn session_list_prefers_recorded_effective_runtime() {
        // Feature 41: mission rows spawned under a slot runtime
        // override record the effective runtime in
        // `sessions.agent_runtime`; session_list must surface that
        // (not the runner row's default) so terminal UX gating and
        // badges track the engine actually running.
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = seed_runner(&conn); // runtime 'shell'
        let crew_id = ulid::Ulid::new().to_string();
        let slot_id = ulid::Ulid::new().to_string();
        let mission_id = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
            params![crew_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, ?2, ?3, 'coder', 0, 1, ?4)",
            params![slot_id, crew_id, runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
            params![mission_id, crew_id, now],
        )
        .unwrap();
        let overridden = ulid::Ulid::new().to_string();
        let plain = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, slot_id, status, started_at, agent_runtime)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, 'claude-code'),
                    (?6, ?2, ?3, ?4, 'running', ?5, NULL)",
            params![overridden, mission_id, runner_id, slot_id, now, plain],
        )
        .unwrap();

        let rows = list_for_mission(&conn, &mission_id).unwrap();
        let runtime_for = |id: &str| {
            rows.iter()
                .find(|r| r.session.id == id)
                .map(|r| r.runtime.clone())
                .unwrap()
        };
        assert_eq!(runtime_for(&overridden), "claude-code");
        assert_eq!(runtime_for(&plain), "shell");
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

// Session command bodies — thin wrappers over `session::SessionManager`.
//
// Spawn happens inside `mission_start` (see `ops::mission`), so there's
// no `session_spawn` here. The commands below let the frontend:
//   - list persisted sessions for a mission (including ones that have exited)
//   - inject bytes into a live session's stdin
//   - kill a live session
//
// PTY output flows synchronously to the native terminal registry; lifecycle
// events continue over the app event channel.

use crate::model::{CodexSpeed, Runtime};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::{
    error::{Error, Result},
    model::{Role, SessionStatus},
    ops::{
        project::{self, ProjectScope},
        role,
    },
    repo,
    session::manager::{
        runtime_direct_role, SessionActivityState, SessionEvents, SessionUpdatedEvent,
        SpawnedSession,
    },
    AppCore,
};

pub use runner_core::protocol::session::SessionRow;

pub fn list_for_mission(
    conn: &rusqlite::Connection,
    app_data_dir: &std::path::Path,
    mission_id: &str,
) -> Result<Vec<SessionRow>> {
    let rows = repo::session::list_for_mission(conn, mission_id)?;
    let mut rows: Vec<_> = rows
        .into_iter()
        .map(|row| SessionRow {
            session: row.session,
            live_title: row.live_title,
            handle: row.handle,
            runtime: row.runtime,
            role_handle: String::new(),
            lead: false,
            agent_session_key: row.agent_session_key,
        })
        .collect();
    let mission = super::mission::get(conn, mission_id)?;
    let roster = super::mission::roster::load(conn, app_data_dir, &mission)?;
    for row in &mut rows {
        if let Some(member) = roster
            .slots
            .iter()
            .find(|member| row.session.slot_id.as_deref() == Some(&member.slot.id))
        {
            row.handle = member.slot.slot_handle.clone();
            row.role_handle = member.role.handle.clone();
        }
    }
    rows.retain(|row| {
        roster
            .file
            .slots
            .iter()
            .any(|slot| row.session.slot_id.as_deref() == Some(&slot.id))
    });
    rows.sort_by_key(|row| {
        roster
            .slots
            .iter()
            .position(|member| row.session.slot_id.as_deref() == Some(&member.slot.id))
            .unwrap_or(usize::MAX)
    });
    Ok(rows)
}

pub fn session_list(state: &AppCore, mission_id: &str) -> Result<Vec<SessionRow>> {
    let conn = state.db.get()?;
    list_for_mission(&conn, &state.app_data_dir, mission_id)
}

pub fn session_kill(state: &AppCore, session_id: &str) -> Result<()> {
    state.sessions.kill(session_id)?;
    if let Err(error) = state.routers.clear_interruption(session_id) {
        log::warn!("clear interruption for stopped session {session_id}: {error}");
    }
    Ok(())
}

pub fn session_status_snapshot(
    state: &AppCore,
) -> Result<BTreeMap<String, crate::session::status::AgentStatus>> {
    use crate::session::status::{AgentStatus, Lifecycle};
    let mut statuses = state.sessions.status_snapshot();
    let interrupted = state.routers.interrupted_sessions();
    let conn = state.db.get()?;
    for row in repo::session::status_rows(&conn)? {
        let interrupted_since = interrupted.get(&row.id).copied();
        let status = statuses.entry(row.id).or_insert(AgentStatus {
            lifecycle: Lifecycle::Running,
            ..Default::default()
        });
        status.unread_since = row.unread_since;
        match row.status.as_str() {
            "stopped" => {
                status.lifecycle = Lifecycle::Stopped;
                status.error_since = None;
                status.failed_since = None;
            }
            "crashed" => {
                status.lifecycle = Lifecycle::Error;
                status.failed_since = None;
                let stopped = row
                    .stopped_at
                    .as_deref()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                    .map(|time| time.timestamp_millis());
                let acknowledged = row
                    .error_acknowledged_at
                    .as_deref()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                    .map(|time| time.timestamp_millis());
                status.error_since =
                    stopped.filter(|time| acknowledged.is_none_or(|viewed| viewed < *time));
            }
            _ => status.interrupted_since = interrupted_since,
        }
    }
    Ok(statuses)
}

pub fn session_activity_snapshot(state: &AppCore) -> BTreeMap<String, SessionActivityState> {
    state.sessions.activity_snapshot()
}

/// Live agent processes per runtime, across direct chats, mission slots,
/// drawers and forks: the session manager's attached processes, keyed by
/// each session row's runtime. A process without a row, such as a CLI
/// update in Settings → Agents, is not a session and is not counted.
pub fn live_session_counts(state: &AppCore) -> Result<HashMap<Runtime, usize>> {
    let conn = state.db.get()?;
    count_by_runtime(&conn, &state.sessions.live_session_ids())
}

fn count_by_runtime(
    conn: &rusqlite::Connection,
    session_ids: &[String],
) -> Result<HashMap<Runtime, usize>> {
    let mut counts = HashMap::new();
    for session_id in session_ids {
        if let Some(runtime) = repo::session::effective_runtime(conn, session_id)?
            .as_deref()
            .and_then(Runtime::parse)
        {
            *counts.entry(runtime).or_default() += 1;
        }
    }
    Ok(counts)
}

/// Paths of files referenced by the platform clipboard, in clipboard order.
pub fn session_clipboard_file_paths() -> Vec<String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            System::{
                DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
                Ole::CF_HDROP,
            },
            UI::Shell::DragQueryFileW,
        };

        // GPUI 0.3.3 passes a locked pointer to DragQueryFileW, which expects the HDROP handle.
        if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
            return Vec::new();
        }
        let paths = (|| {
            let hdrop = unsafe { GetClipboardData(CF_HDROP as u32) };
            if hdrop.is_null() {
                return Vec::new();
            }
            let count = unsafe { DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0) };
            (0..count)
                .filter_map(|index| {
                    let length = unsafe { DragQueryFileW(hdrop, index, std::ptr::null_mut(), 0) };
                    if length == 0 {
                        return None;
                    }
                    let mut buffer = vec![0u16; length as usize + 1];
                    let copied = unsafe {
                        DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), buffer.len() as u32)
                    };
                    (copied > 0).then(|| String::from_utf16_lossy(&buffer[..copied as usize]))
                })
                .collect()
        })();
        unsafe { CloseClipboard() };
        paths
    }

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
        use objc2_foundation::NSURL;

        // SAFETY: AppKit's own flavor constant, read-only.
        let file_url_type = unsafe { NSPasteboardTypeFileURL };
        let Some(items) = NSPasteboard::generalPasteboard().pasteboardItems() else {
            return Vec::new();
        };
        items
            .to_vec()
            .into_iter()
            .filter_map(|item| {
                let url_string = item.stringForType(file_url_type)?;
                let url = NSURL::URLWithString(&url_string)?;
                if !url.isFileURL() {
                    return None;
                }
                Some(url.path()?.to_string())
            })
            .collect()
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}

pub use runner_core::protocol::session::DirectSessionEntry;

pub use runner_core::protocol::session::StartDirectSessionOutput;

/// Assemble the IPC entry from a repo direct-session row. `ship_key`
/// distinguishes the two surfaces: `session_get` returns the raw
/// `agent_session_key`, while the recent list intentionally ships NULL.
fn direct_entry_from_repo(
    d: repo::session::DirectSessionRow,
    ship_key: bool,
) -> Result<DirectSessionEntry> {
    let handle = d.role_handle;
    let has_key = d.row.agent_session_key.is_some();
    let is_direct = d.row.mission_id.is_none();
    let is_archived = d.row.archived_at.is_some();
    let agent_runtime = d
        .row
        .agent_runtime
        .ok_or_else(|| Error::msg(format!("session {} has no agent_runtime", d.row.id)))?;
    let agent_command = d
        .row
        .launch_definition
        .as_ref()
        .map(|definition| definition.command.clone())
        .or(d.row.agent_command)
        .ok_or_else(|| Error::msg(format!("session {} has no agent_command", d.row.id)))?;
    let display_name = d
        .role_display_name
        .filter(|_| handle.is_some())
        .unwrap_or_else(|| {
            crate::runtimes::for_key(&agent_runtime)
                .catalog()
                .map(|catalog| catalog.display_name.to_string())
                .unwrap_or_else(|| agent_runtime.to_string())
        });
    let native_fork = crate::runtimes::for_key(&agent_runtime)
        .catalog()
        .is_some_and(|catalog| catalog.native_fork);
    Ok(DirectSessionEntry {
        claude_account: d.row.launch_definition.as_ref().and_then(|definition| {
            definition.claude_account.clone().or_else(|| {
                (agent_runtime == "claude-code")
                    .then(|| definition.env.get("CLAUDE_CONFIG_DIR"))
                    .flatten()
                    .map(|folder| runner_core::protocol::ClaudeSessionAccount {
                        account_id: None,
                        folder: folder.clone(),
                        native: false,
                        pinned: true,
                        origin_folder: folder.clone(),
                        permission_mode: "default".into(),
                    })
            })
        }),
        session_id: d.row.id,
        project_id: d.row.project_id,
        role_id: d.row.role_id,
        handle,
        agent_runtime,
        agent_command,
        agent_model: d
            .row
            .launch_definition
            .as_ref()
            .and_then(|definition| definition.model.clone()),
        agent_effort: d
            .row
            .launch_definition
            .as_ref()
            .and_then(|definition| definition.effort.clone()),
        display_name,
        status: d.row.status,
        title: d.row.title,
        live_title: d.row.live_title,
        cwd: d.row.cwd,
        started_at: d.row.started_at,
        stopped_at: d.row.stopped_at,
        resumable: has_key,
        native_fork,
        forkable: native_fork && has_key && is_direct && !is_archived,
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
/// NOT NULL`) from the SESSION tray, but the `/roles/:handle/chat/
/// :sessionId` route still mounts when the user navigates by direct
/// URL. RoleChat falls back to this helper when the list lookup
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

pub fn session_get_with_status(state: &AppCore, session_id: &str) -> Result<serde_json::Value> {
    let mut row = if let Some(direct) = session_get(state, session_id)? {
        serde_json::to_value(direct)?
    } else {
        let conn = state.db.get()?;
        let row = repo::session::get_row(&conn, session_id)?
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        serde_json::to_value(row)?
    };
    let object = row
        .as_object_mut()
        .ok_or_else(|| Error::msg("session row did not serialize as an object"))?;
    object.remove("launch_definition_json");
    let session = repo::session::get_row(&*state.db.get()?, session_id)?
        .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
    let definition = session.launch_definition.as_ref();
    object.insert(
        "agent_model".into(),
        serde_json::to_value(definition.and_then(|definition| definition.model.as_ref()))?,
    );
    object.insert(
        "agent_effort".into(),
        serde_json::to_value(definition.and_then(|definition| definition.effort.as_ref()))?,
    );
    object.insert(
        "agent_speed".into(),
        serde_json::to_value(definition.and_then(|definition| definition.codex_speed))?,
    );
    object.insert(
        "agent_runtime".into(),
        serde_json::to_value(session.agent_runtime)?,
    );
    object.insert(
        "agent_command".into(),
        serde_json::to_value(
            definition
                .map(|definition| &definition.command)
                .or(session.agent_command.as_ref()),
        )?,
    );
    object.insert(
        "agent_status".into(),
        serde_json::to_value(state.sessions.agent_status(session_id))?,
    );
    object.insert(
        "activity".into(),
        serde_json::to_value(state.sessions.activity_snapshot().remove(session_id))?,
    );
    Ok(row)
}

fn get_direct(conn: &rusqlite::Connection, session_id: &str) -> Result<Option<DirectSessionEntry>> {
    repo::session::get_direct(conn, session_id)?
        .map(|d| direct_entry_from_repo(d, /*ship_key*/ true))
        .transpose()
}

fn ensure_archivable_session(conn: &rusqlite::Connection, session_id: &str) -> Result<()> {
    if repo::session::effective_runtime(conn, session_id)?
        .as_deref()
        .and_then(Runtime::parse)
        .is_some_and(Runtime::is_shell)
    {
        return Err(Error::msg("terminal sessions cannot be archived"));
    }
    Ok(())
}

/// Soft-delete a session: hides it from the SESSION sidebar tray. The row
/// stays in the table so a future Archived workspace surface can still
/// surface it. Running sessions cannot be archived — kill them first.
/// Terminal sessions are rejected because closing their pane permanently
/// closes the shell instead of preserving a conversation.
///
/// Emits a `session/archived` app event after the row flips so the
/// sidebar's CHAT list can refresh — without it, archiving from the
/// chat page (RoleChat's SessionEnded overlay) would archive the
/// row but leave the sidebar stale until something else triggered a
/// refresh.
pub fn session_archive(state: &AppCore, session_id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    ensure_archivable_session(&conn, session_id)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let updated = repo::session::archive(&tx, session_id, chrono::Utc::now())?;
    if updated == 0 {
        return Err(Error::msg(
            "session not found or still running (kill before archiving)".to_string(),
        ));
    }
    repo::node::remove_session(&tx, session_id)?;
    tx.commit()?;
    state.sessions.forget_session_state(session_id);
    state.session_events().archived(&SessionUpdatedEvent {
        session_id: session_id.to_owned(),
        mission_id: None,
    });
    Ok(())
}

/// Clear a direct session's archive marker so it rejoins the SESSION
/// tray (Settings → Archived restore). Single-column flip — status,
/// title, and `agent_session_key` survive, so a restored chat can
/// still resume. Idempotent: unarchiving an active direct chat is a
/// no-op Ok. Unknown ids and mission/slot-bound rows error — the
/// Archived pane only lists direct chats, and restoring a mission-bound
/// row would leak it back into `session_list` for its mission.
///
/// Emits a `session/updated` app event after the flip — the same
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
/// not. An Antigravity chat's `--log-file` goes with the row; the
/// agent's own JSONL belongs to the agent runtime, and the scrollback
/// buffer was already purged at archive time.
pub fn session_delete(state: &AppCore, session_id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    match repo::session::get_row(&tx, session_id)? {
        None => return Err(Error::msg(format!("session not found: {session_id}"))),
        Some(row) if row.mission_id.is_some() || row.slot_id.is_some() => {
            return Err(Error::msg(format!(
                "session {session_id} is mission-scoped; only direct chats can be deleted"
            )))
        }
        Some(row) if row.archived_at.is_none() => {
            return Err(Error::msg(format!(
                "session {session_id} is not archived; archive it before deleting"
            )))
        }
        Some(_) => {}
    }
    delete_rows(&tx, &[session_id.to_owned()])?;
    tx.commit()?;
    for runtime in Runtime::ALL {
        crate::runtimes::adapter(runtime).remove_capture(&state.app_data_dir, session_id);
    }
    Ok(())
}

/// Delete sessions and the rows keyed by them, one statement per table,
/// inside the caller's transaction. Every path that removes a session comes
/// through here, so none depends on the `session_attention` cascade (#619).
pub(crate) fn delete_rows(tx: &rusqlite::Transaction<'_>, ids: &[String]) -> Result<usize> {
    repo::session_attention::delete_for_sessions(tx, ids)?;
    Ok(repo::session::delete_many(tx, ids)?)
}

pub(crate) fn close_shell_row(conn: &mut rusqlite::Connection, session_id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    match repo::session::effective_runtime(&tx, session_id)?.as_deref() {
        Some(runtime) if Runtime::parse(runtime).is_some_and(Runtime::is_shell) => {}
        Some(_) => {
            return Err(Error::msg(format!(
                "session {session_id} is not a shell session"
            )))
        }
        None => return Ok(()),
    }
    for row in repo::node::list(&tx)?
        .into_iter()
        .filter(|row| row.node_type == repo::node::NodeType::Tab)
    {
        let stored: serde_json::Value =
            serde_json::from_str(row.layout.as_deref().unwrap_or("{}"))?;
        if !stored
            .get("slots")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|slots| slots.iter().any(|slot| slot.as_str() == Some(session_id)))
        {
            continue;
        }
        let mut layout = runner_core::pane_layout::PaneLayout::from_node_row(&row)
            .map_err(|error| Error::msg(error.to_string()))?;
        let pane = layout
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
            .map(|leaf| leaf.id.clone());
        if let Some(pane) = pane {
            if !layout.close_pane(&pane) {
                layout.remove_session(session_id);
            }
        }
        if layout.all_session_ids().is_empty() {
            repo::node::delete(&tx, &row.id)?;
        } else {
            repo::node::set_layout(
                &tx,
                &row.id,
                &layout
                    .serialize()
                    .map_err(|error| Error::msg(error.to_string()))?,
            )?;
        }
    }
    repo::node::remove_session(&tx, session_id)?;
    if delete_rows(&tx, &[session_id.to_owned()])? == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn retire_chat_drawers(conn: &mut rusqlite::Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for row in repo::node::list(&tx)?
        .into_iter()
        .filter(|row| row.node_type == repo::node::NodeType::Tab)
    {
        let Some(raw) = row.layout.as_deref() else {
            continue;
        };
        let mut layout: serde_json::Value = serde_json::from_str(raw)?;
        let Some(drawer) = layout
            .as_object_mut()
            .and_then(|layout| layout.remove("drawer"))
        else {
            continue;
        };
        let shells = drawer
            .get("shells")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        for id in shells {
            if repo::session::effective_runtime(&tx, id)?.as_deref() == Some("shell") {
                repo::node::remove_session(&tx, id)?;
                delete_rows(&tx, &[id.to_owned()])?;
            }
        }
        if repo::node::get(&tx, &row.id)?.is_some() {
            tx.execute(
                "UPDATE nodes SET layout = ?2 WHERE id = ?1",
                rusqlite::params![row.id, layout.to_string()],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn close_exited_shells(conn: &mut rusqlite::Connection) -> Result<()> {
    let ids = {
        let mut stmt = conn.prepare("SELECT id FROM sessions WHERE mission_id IS NULL AND agent_runtime = 'shell' AND status != 'running' AND resume_on_launch = 0")?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in ids {
        close_shell_row(conn, &id)?;
    }
    Ok(())
}

/// Permanently close a shell-backed terminal session. A running child is
/// stopped first, then the pane's node slot and session row are removed in
/// one transaction. Chat sessions are rejected because closing their panes
/// must preserve the conversation for the sidebar.
pub fn session_close(state: &AppCore, session_id: &str) -> Result<()> {
    let status = {
        let conn = state.db.get()?;
        match repo::session::effective_runtime(&conn, session_id)?.as_deref() {
            Some(runtime) if Runtime::parse(runtime).is_some_and(Runtime::is_shell) => {}
            Some(_) => {
                return Err(Error::msg(format!(
                    "session {session_id} is not a shell session"
                )))
            }
            None => return Err(Error::msg(format!("session not found: {session_id}"))),
        }
        repo::session::get_row(&conn, session_id)?
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?
            .status
    };
    if status == SessionStatus::Running {
        state.sessions.kill(session_id)?;
    }
    let mut conn = state.db.get()?;
    close_shell_row(&mut conn, session_id)?;
    state.sessions.forget_session_state(session_id);
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
    );
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(())
}

/// Report whether a shell-backed terminal currently owns a foreground
/// process group beyond the shell itself. Chat sessions are rejected.
pub fn session_shell_has_foreground_process(state: &AppCore, session_id: &str) -> Result<bool> {
    let conn = state.db.get()?;
    match repo::session::effective_runtime(&conn, session_id)?.as_deref() {
        Some(runtime) if Runtime::parse(runtime).is_some_and(Runtime::is_shell) => {
            state.sessions.has_foreground_process(session_id)
        }
        Some(_) => Err(Error::msg(format!(
            "session {session_id} is not a shell session"
        ))),
        None => Err(Error::msg(format!("session not found: {session_id}"))),
    }
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
/// Emits a `session/updated` app event after the row flips so the
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
/// Pin a session and its tab node together: the whole tab pins (every
/// member session's `pinned_at` writes through) because the sidebar
/// renders pin state from the node (feature 44).
pub(crate) fn pin_session_and_tab(
    conn: &mut rusqlite::Connection,
    session_id: &str,
    pinned: bool,
) -> Result<Option<String>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let pinned_at = pinned.then(chrono::Utc::now);
    let updated = repo::session::set_pinned_at(&tx, session_id, pinned_at)?;
    if updated == 0 {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    repo::node::ensure_active_sessions(&tx)?;
    let tab = repo::node::find_for_session(&tx, session_id)?;
    if let Some(tab) = tab.as_ref() {
        for member in repo::node::session_ids(tab) {
            if member != session_id {
                repo::session::set_pinned_at(&tx, &member, pinned_at)?;
            }
        }
        repo::node::set_pinned(&tx, &tab.id, pinned)?;
    }
    tx.commit()?;
    Ok(tab.map(|tab| tab.id))
}

/// `pinned = false` clears the pin.
///
/// Emits `session/updated` (row content) and `chat/layout-changed`
/// (the tab node's pin drives sidebar ordering) after the flip.
pub fn session_pin(state: &AppCore, session_id: &str, pinned: bool) -> Result<()> {
    let mut conn = state.db.get()?;
    pin_session_and_tab(&mut conn, session_id, pinned)?;
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": session_id }),
    );
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(())
}

/// Respawn an existing session row. Reuses the row's id and
/// `agent_session_key` so the agent CLI continues the prior conversation
/// (claude-code: `--resume <uuid>`; codex: `codex resume <uuid>` once the
/// key-capture path lands). See `SessionManager::resume` for the detailed
/// contract — refused for running and archived rows.
pub fn session_resume(
    state: &AppCore,
    session_id: &str,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    // Dims decide the PTY fork size; None uses the persisted last-applied
    // size before falling back to 80×24. Logged so a garbled-pane report
    // can be traced back to its resume geometry.
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
        .map_err(|e| e.context("session_resume"))?;
    state.events.emit(
        "session/updated",
        &crate::session::manager::SessionUpdatedEvent {
            session_id: session_id.to_string(),
            mission_id: spawned.mission_id.clone(),
        },
    );
    Ok(spawned)
}

pub fn session_restart(
    state: &AppCore,
    session_id: &str,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let row = {
        let conn = state.db.get()?;
        repo::session::get_row(&conn, session_id)?
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?
    };
    let mission_id = row
        .mission_id
        .as_deref()
        .ok_or_else(|| Error::msg("only mission slots can be restarted"))?;
    let router = state
        .routers
        .get(mission_id)
        .ok_or_else(|| Error::msg("mission router is not mounted"))?;
    let spawned = state
        .sessions
        .restart(
            session_id,
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            Arc::new(state.session_events()),
        )
        .map_err(|error| error.context("session_restart"))?;
    if let Err(error) = router.record_slot_restart(
        &spawned.handle,
        session_id,
        row.agent_session_key.as_deref(),
    ) {
        let message = format!("@{} restarted, but its restart notification could not be recorded. Tell the crew to re-send its task and context. {error}", spawned.handle);
        log::warn!("session_restart: {message}");
        state.events.emit(
            "session/warning",
            &crate::session::manager::WarningEvent {
                session_id: session_id.to_owned(),
                mission_id: spawned.mission_id.clone(),
                kind: "slot_restart_notification_failed".into(),
                message,
            },
        );
    }
    state.events.emit(
        "session/updated",
        &SessionUpdatedEvent {
            session_id: session_id.to_string(),
            mission_id: spawned.mission_id.clone(),
        },
    );
    Ok(spawned)
}

pub fn session_fork(
    state: &AppCore,
    source_session_id: &str,
    title: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let spawned = state
        .sessions
        .spawn_fork(
            source_session_id,
            title,
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|error| error.context("session_fork"))?;
    log::info!(
        "session_fork: source={source_session_id} new={} cols={cols:?} rows={rows:?}",
        spawned.id
    );
    state.events.emit(
        "session/updated",
        &crate::session::manager::SessionUpdatedEvent {
            session_id: spawned.id.clone(),
            mission_id: None,
        },
    );
    Ok(spawned)
}

fn finish_launch_resume(conn: &mut rusqlite::Connection, session_id: &str) -> Result<()> {
    repo::session::finish_resume_on_launch(conn, session_id)?;
    // A shell can exit before its launch claim is released by this caller.
    if repo::session::effective_runtime(conn, session_id)?.as_deref() == Some("shell")
        && repo::session::get_row(conn, session_id)?
            .is_some_and(|row| row.status != SessionStatus::Running && !row.resume_on_launch)
    {
        close_shell_row(conn, session_id)?;
    }
    Ok(())
}

pub fn session_resume_on_launch(
    state: &AppCore,
    session_id: &str,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    log::info!("session_resume: session={session_id} cols={cols:?} rows={rows:?} automatic=true");
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let result = state.sessions.resume_on_launch(
        session_id,
        cols,
        rows,
        &state.app_data_dir,
        state.db.clone(),
        emitter,
    );
    match state.db.get() {
        Ok(mut conn) => {
            if let Err(error) = finish_launch_resume(&mut conn, session_id) {
                log::warn!("session_resume: launch claim update failed for {session_id}: {error}");
            }
        }
        Err(error) => {
            log::warn!("session_resume: launch claim connection failed for {session_id}: {error}");
        }
    }
    let spawned = result.map_err(|error| {
        log::warn!("session_resume: automatic resume of {session_id} failed: {error}");
        error.context("session_resume")
    })?;
    state.events.emit(
        "session/updated",
        &crate::session::manager::SessionUpdatedEvent {
            session_id: session_id.to_string(),
            mission_id: spawned.mission_id.clone(),
        },
    );
    Ok(spawned)
}

pub fn session_take_resume_on_launch(state: &AppCore, session_id: &str) -> Result<bool> {
    let mut conn = state.db.get()?;
    Ok(repo::session::take_resume_on_launch_for_session(&mut conn, session_id)?.is_some())
}

/// Spawn a "direct chat" session for a role — a PTY with no parent
/// mission, no orchestrator, no event log (C8.5). Used by the Role
/// Detail page's "Chat now" button: the user picks a working directory
/// and gets a one-on-one terminal with the agent's CLI.
pub(crate) fn resolve_direct_start(
    conn: &rusqlite::Connection,
    role_id: &str,
    project_id: Option<&str>,
    cwd: Option<String>,
) -> Result<(Role, Option<String>, Option<String>)> {
    resolve_direct_start_in(
        conn,
        role_id,
        &ProjectScope::or_infer(project_id.map(str::to_owned)),
        cwd,
    )
}

/// Working-directory precedence is explicit `cwd`, project cwd, then the
/// role's own `working_dir`.
pub(crate) fn resolve_direct_start_in(
    conn: &rusqlite::Connection,
    role_id: &str,
    scope: &ProjectScope,
    cwd: Option<String>,
) -> Result<(Role, Option<String>, Option<String>)> {
    let role = role::get(conn, role_id)?;
    let cwd = match scope {
        ProjectScope::Project(_) => cwd,
        ProjectScope::Infer | ProjectScope::Root => cwd.or_else(|| role.working_dir.clone()),
    };
    let (project_id, cwd) = project::resolve_scope(conn, scope, cwd)?;
    Ok((role, project_id, cwd))
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_direct_impl_with_speed(
    state: &AppCore,
    role_id: String,
    runtime: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    speed: Option<CodexSpeed>,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<StartDirectSessionOutput> {
    let (role, project_id, cwd) = {
        let conn = state.db.get()?;
        resolve_direct_start(&conn, &role_id, project_id.as_deref(), cwd)?
    };
    spawn_direct_chat(
        state, &role, runtime, model, effort, speed, project_id, cwd, cols, rows,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_direct_chat(
    state: &AppCore,
    role: &Role,
    runtime: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    speed: Option<CodexSpeed>,
    project_id: Option<String>,
    effective_cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<StartDirectSessionOutput> {
    let first_turn =
        crate::router::prompt::compose_direct_first_turn(role.system_prompt.as_deref());
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let session = state
        .sessions
        .spawn_direct_with_speed(
            role,
            runtime.as_deref(),
            model.as_deref(),
            effort.as_deref(),
            speed,
            project_id.as_deref(),
            effective_cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
            first_turn,
        )
        .map_err(|e| e.context("session_start_direct"))?;
    Ok(StartDirectSessionOutput {
        session,
        project_id,
        cwd: effective_cwd,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_direct(
    state: &AppCore,
    role_id: String,
    runtime: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    scope: ProjectScope,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    session_start_direct_with_speed(
        state, role_id, runtime, model, effort, None, scope, cwd, cols, rows,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_direct_with_speed(
    state: &AppCore,
    role_id: String,
    runtime: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    speed: Option<CodexSpeed>,
    scope: ProjectScope,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let (role, project_id, cwd) = {
        let conn = state.db.get()?;
        resolve_direct_start_in(&conn, &role_id, &scope, cwd)?
    };
    Ok(spawn_direct_chat(
        state, &role, runtime, model, effort, speed, project_id, cwd, cols, rows,
    )?
    .session)
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_runtime(
    state: &AppCore,
    runtime: &str,
    scope: ProjectScope,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    model: Option<String>,
    effort: Option<String>,
) -> Result<StartDirectSessionOutput> {
    session_start_runtime_with_speed(state, runtime, scope, cwd, cols, rows, model, effort, None)
}

#[allow(clippy::too_many_arguments)]
pub fn session_start_runtime_with_speed(
    state: &AppCore,
    runtime: &str,
    scope: ProjectScope,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    model: Option<String>,
    effort: Option<String>,
    speed: Option<CodexSpeed>,
) -> Result<StartDirectSessionOutput> {
    let (project_id, cwd) = {
        let conn = state.db.get()?;
        project::resolve_scope(&conn, &scope, cwd)?
    };
    let mut role = runtime_direct_role(runtime, None, model.as_deref(), effort.as_deref())?;
    if crate::runtimes::for_key(runtime).capabilities().codex_speed {
        role.codex_speed = speed;
    }
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let spawned = state
        .sessions
        .spawn_runtime_direct(
            &role,
            project_id.as_deref(),
            cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| e.context("session_start_runtime"))?;
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": spawned.id }),
    );
    Ok(StartDirectSessionOutput {
        session: spawned,
        project_id,
        cwd,
    })
}

fn resolve_shell_command(shell: Option<String>) -> Result<String> {
    let shell = crate::shell_path::configured_shell(shell);
    #[cfg(target_os = "linux")]
    {
        shell
            .or_else(crate::shell_path::account_login_shell)
            .ok_or_else(|| Error::msg("Cannot determine the account login shell"))
    }
    #[cfg(not(target_os = "linux"))]
    Ok(shell.unwrap_or_else(|| {
        #[cfg(unix)]
        {
            "/bin/zsh".to_string()
        }
        #[cfg(windows)]
        {
            windows_shell_command(
                &std::env::var("PATH").unwrap_or_default(),
                std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string()),
            )
        }
    }))
}

#[cfg(windows)]
fn windows_shell_command(path: &str, comspec: String) -> String {
    ["pwsh.exe", "powershell.exe"]
        .into_iter()
        .find_map(|command| crate::runtime_status::find_executable(command, path))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or(comspec)
}

/// Spawn the user's configured login shell for a caller that may pass only a
/// directory: without `project_id`, the project is inferred from `cwd`.
pub fn session_start_shell(
    state: &AppCore,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    session_start_shell_in(state, ProjectScope::or_infer(project_id), cwd, cols, rows)
}

/// Spawn the user's configured login shell as a runtime-only direct session
/// for a terminal pane. An explicit cwd wins over the project's directory;
/// no agent prompt, hooks, Role bus environment, or bundled CLI is added.
pub fn session_start_shell_in(
    state: &AppCore,
    scope: ProjectScope,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    let (project_id, cwd) = {
        let conn = state.db.get()?;
        project::resolve_scope(&conn, &scope, cwd)?
    };
    let command = resolve_shell_command(std::env::var("SHELL").ok())?;
    let role = runtime_direct_role(Runtime::Shell.key(), Some(&command), None, None)?;
    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let spawned = state
        .sessions
        .spawn_runtime_direct(
            &role,
            project_id.as_deref(),
            cwd.as_deref(),
            cols,
            rows,
            &state.app_data_dir,
            state.db.clone(),
            emitter,
        )
        .map_err(|e| Error::msg(format!("session_start_shell: {e}")))?;
    state.events.emit(
        "session/updated",
        &serde_json::json!({ "session_id": spawned.id }),
    );
    Ok(spawned)
}

pub fn session_stop(state: &AppCore, session_id: &str) -> Result<()> {
    let conn = state.db.get()?;
    if crate::repo::session::get_row(&conn, session_id)?.is_none() {
        return Err(Error::msg(format!("session not found: {session_id}")));
    }
    drop(conn);
    session_kill(state, session_id)
}

pub fn session_archive_direct(state: &AppCore, session_id: &str) -> Result<()> {
    session_archive_direct_with_missions(state, session_id, false).map(|_| ())
}

pub fn session_archive_direct_with_missions(
    state: &AppCore,
    session_id: &str,
    with_missions: bool,
) -> Result<Vec<String>> {
    let direct = match session_get(state, session_id)? {
        Some(direct) => direct,
        None => {
            let conn = state.db.get()?;
            return match crate::repo::session::get_row(&conn, session_id)? {
                Some(_) => Err(crate::error::Error::msg(format!(
                    "session {session_id} is mission-scoped; only direct chats can be archived"
                ))),
                None => Err(crate::error::Error::msg(format!(
                    "session not found: {session_id}"
                ))),
            };
        }
    };
    if crate::model::Runtime::parse(&direct.agent_runtime)
        .is_some_and(crate::model::Runtime::is_shell)
    {
        return Err(crate::error::Error::msg(format!(
            "session {session_id} is a terminal; terminals close rather than archive"
        )));
    }
    let linked_missions = if with_missions {
        let conn = state.db.get()?;
        crate::repo::mission::list_linked(&conn, session_id)?
    } else {
        Vec::new()
    };
    if direct.status == crate::model::SessionStatus::Running {
        session_kill(state, session_id)?;
        if !state
            .sessions
            .live_session_ids()
            .iter()
            .any(|live| live == session_id)
        {
            let conn = state.db.get()?;
            if crate::repo::session::get_row(&conn, session_id)?
                .is_some_and(|row| row.status == crate::model::SessionStatus::Running)
            {
                crate::repo::session::set_exit_status(
                    &conn,
                    session_id,
                    crate::model::SessionStatus::Stopped,
                    chrono::Utc::now(),
                )?;
            }
        }
    }
    session_archive(state, session_id)?;
    let mut archived_mission_ids = Vec::with_capacity(linked_missions.len());
    for mission in linked_missions {
        match crate::daemon::block_on(crate::ops::mission::mission_archive_impl(
            state,
            mission.id.clone(),
        )) {
            Ok(archived) => {
                state.events.emit(
                    "mission/changed",
                    &serde_json::json!({ "mission_id": archived.id }),
                );
                archived_mission_ids.push(archived.id);
            }
            Err(error) => {
                if !crate::ops::mission::mission_get(state, &mission.id)
                    .is_ok_and(|row| row.archived_at.is_some())
                {
                    return Err(error);
                }
            }
        }
    }
    Ok(archived_mission_ids)
}

pub fn session_start_chat(
    state: &AppCore,
    args: runner_core::protocol::StartDirectSessionArgs,
) -> Result<StartDirectSessionOutput> {
    match (args.role_id, args.runtime) {
        (Some(role_id), runtime) => session_start_direct_impl_with_speed(
            state,
            role_id,
            runtime.map(|runtime| runtime.to_string()),
            args.model,
            args.effort,
            args.speed,
            args.project_id,
            args.cwd,
            None,
            None,
        ),
        (None, Some(runtime)) => session_start_runtime_with_speed(
            state,
            runtime.key(),
            crate::ops::project::ProjectScope::or_infer(args.project_id),
            args.cwd,
            None,
            None,
            args.model,
            args.effort,
            args.speed,
        ),
        (None, None) => Err(Error::msg("one of role_id or runtime is required")),
    }
}

pub fn session_list_with_activity(state: &AppCore) -> Result<Vec<serde_json::Value>> {
    let sessions = session_list_recent_direct(state)?;
    let statuses = state.sessions.status_snapshot();
    Ok(sessions
        .into_iter()
        .map(|session| {
            let activity = statuses
                .get(&session.session_id)
                .map(|status| status.observation.activity)
                .unwrap_or_default();
            let mut value = serde_json::to_value(session).expect("session row serializes");
            value["activity"] = serde_json::json!(activity);
            value
        })
        .collect())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod client_tests;

#[cfg(test)]
mod launch_definition_output_tests;

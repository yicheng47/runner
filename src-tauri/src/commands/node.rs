// Sidebar node-tree commands (feature 44) — the unified surface that
// replaced `commands::folder` + `commands::tab`. One tree query feeds
// every sidebar section; one reparent/reposition op backs every drag.

use chrono::Utc;
use serde::Deserialize;
use tauri::{Emitter, Manager, State};

use crate::error::{Error, Result};
use crate::repo;
use crate::repo::node::{NodeRow, NodeType};
use crate::session::manager::SessionActivityState;
use crate::windows::Subject;
use crate::AppState;

const ATTENTION_CHANGED_EVENT: &str = "chat/tab-attention-changed";
const LAYOUT_CHANGED_EVENT: &str = "chat/layout-changed";

fn emit_layout_changed(app: &tauri::AppHandle) {
    let _ = app.emit(LAYOUT_CHANGED_EVENT, serde_json::json!({}));
}

fn clean_folder_name(name: String) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::msg("folder name cannot be empty"));
    }
    Ok(name.to_owned())
}

fn validate_layout(layout: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(layout)
        .map(|_| ())
        .map_err(|e| Error::msg(format!("invalid tab layout: {e}")))
}

#[derive(Debug, Deserialize)]
pub struct NodeTabUpsertInput {
    pub id: String,
    /// Scope for a NEW tab node; an existing node keeps its stored
    /// placement — reparenting/reordering go through `node_move` only,
    /// so a layout/name write can never scramble sibling positions.
    pub parent_id: Option<String>,
    pub name: String,
    pub layout: String,
}

#[derive(Debug, Deserialize)]
pub struct NodeTabImportInput {
    pub name: String,
    pub position: i64,
    pub layout: String,
}

#[tauri::command]
pub fn node_list(state: State<'_, AppState>) -> Result<Vec<NodeRow>> {
    let mut conn = state.db.get()?;
    Ok(repo::node::list_with_repair(&mut conn)?)
}

#[tauri::command]
pub fn node_folder_create(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
) -> Result<NodeRow> {
    let conn = state.db.get()?;
    let row = repo::node::create_folder(&conn, &clean_folder_name(name)?)?;
    emit_layout_changed(&app);
    Ok(row)
}

/// Rename a folder or tab node. Project and mission rows keep their
/// names on the domain tables — rename those through `project_rename`
/// / `mission_rename`.
#[tauri::command]
pub fn node_rename(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<NodeRow> {
    let conn = state.db.get()?;
    let node =
        repo::node::get(&conn, &id)?.ok_or_else(|| Error::msg(format!("node not found: {id}")))?;
    let name = match node.node_type {
        NodeType::Folder => clean_folder_name(name)?,
        NodeType::Tab => name.trim().to_owned(),
        NodeType::Project | NodeType::Mission => {
            return Err(Error::msg(
                "project and mission names live on their domain rows",
            ))
        }
    };
    repo::node::rename(&conn, &id, &name)?;
    let row = repo::node::get(&conn, &id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    emit_layout_changed(&app);
    Ok(row)
}

#[tauri::command]
pub fn node_tab_upsert(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    input: NodeTabUpsertInput,
) -> Result<NodeRow> {
    validate_layout(&input.layout)?;
    ulid::Ulid::from_string(&input.id)
        .map_err(|_| Error::msg(format!("invalid node id: {}", input.id)))?;
    let mut conn = state.db.get()?;
    let existing = repo::node::get(&conn, &input.id)?;
    if let Some(existing) = existing.as_ref() {
        if existing.node_type != NodeType::Tab {
            return Err(Error::msg(format!("node {} is not a tab", input.id)));
        }
    }
    let (parent_id, position) = match existing.as_ref() {
        Some(row) => (row.parent_id.clone(), row.position),
        None => {
            let position = repo::node::next_position(&conn, input.parent_id.as_deref())?;
            (input.parent_id, position)
        }
    };
    let row = NodeRow {
        id: input.id,
        parent_id,
        position,
        node_type: NodeType::Tab,
        name: Some(input.name.trim().to_owned()),
        ref_id: None,
        layout: Some(input.layout),
        pinned_position: existing.as_ref().and_then(|row| row.pinned_position),
        last_completed_at: existing
            .as_ref()
            .and_then(|row| row.last_completed_at.clone()),
        last_viewed_at: existing.as_ref().and_then(|row| row.last_viewed_at.clone()),
        created_at: existing
            .as_ref()
            .map(|row| row.created_at.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
    };
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::node::upsert_move_not_copy(&tx, &row)?;
    tx.commit()?;
    emit_layout_changed(&app);
    Ok(row)
}

/// Delete a tab node (closing a chat tab). Folders go through
/// `node_folder_delete`; mission nodes leave via mission archive.
#[tauri::command]
pub fn node_delete(state: State<'_, AppState>, app: tauri::AppHandle, id: String) -> Result<()> {
    let conn = state.db.get()?;
    if let Some(node) = repo::node::get(&conn, &id)? {
        if node.node_type != NodeType::Tab {
            return Err(Error::msg(format!("node {id} is not a tab")));
        }
        repo::node::delete(&conn, &id)?;
    }
    emit_layout_changed(&app);
    Ok(())
}

/// The unified reparent/reposition op behind every sidebar drag.
/// `ordered_ids` is the complete new ordering of the destination
/// scope's children (moved node included). Crossing a project boundary
/// writes `sessions.project_id` / `missions.project_id` through.
#[tauri::command]
pub fn node_move(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    parent_id: Option<String>,
    ordered_ids: Vec<String>,
) -> Result<Vec<NodeRow>> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let moved_type = repo::node::get(&tx, &id)?
        .ok_or_else(|| Error::msg(format!("node not found: {id}")))?
        .node_type;
    if let Some(parent_id) = parent_id.as_deref() {
        if repo::node::get(&tx, parent_id)?.is_none() {
            return Err(Error::msg(format!("node not found: {parent_id}")));
        }
    }
    repo::node::move_and_reorder(&tx, &id, parent_id.as_deref(), &ordered_ids)
        .map_err(|error| Error::msg(format!("move node: {error}")))?;
    let rows = repo::node::list(&tx)?;
    tx.commit()?;
    emit_layout_changed(&app);
    // A cross-project move rewrites domain pointers — nudge the
    // surfaces that render them.
    match moved_type {
        NodeType::Tab => {
            let _ = app.emit("session/updated", serde_json::json!({}));
        }
        NodeType::Mission => {
            let _ = app.emit("mission/changed", serde_json::json!({}));
        }
        NodeType::Folder | NodeType::Project => {}
    }
    Ok(rows)
}

/// Pin/unpin a tab or mission row. The node's `pinned_position` is
/// what the sidebar renders; the legacy domain flags
/// (`sessions.pinned_at` for the tab's members, `missions.pinned_at`)
/// are written through because non-sidebar surfaces (the tray sort,
/// MCP mission listings) still read them.
#[tauri::command]
pub fn node_set_pinned(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    pinned: bool,
) -> Result<NodeRow> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let node =
        repo::node::get(&tx, &id)?.ok_or_else(|| Error::msg(format!("node not found: {id}")))?;
    let pinned_at = pinned.then(Utc::now);
    match node.node_type {
        NodeType::Tab => {
            for session_id in repo::node::session_ids(&node) {
                repo::session::set_pinned_at(&tx, &session_id, pinned_at)?;
            }
        }
        NodeType::Mission => {
            if let Some(mission_id) = node.ref_id.as_deref() {
                repo::mission::set_pinned_at(&tx, mission_id, pinned_at)?;
            }
        }
        NodeType::Folder | NodeType::Project => {
            return Err(Error::msg("only tab and mission rows can be pinned"));
        }
    }
    repo::node::set_pinned(&tx, &id, pinned)?;
    let row = repo::node::get(&tx, &id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    tx.commit()?;
    emit_layout_changed(&app);
    match node.node_type {
        NodeType::Tab => {
            let _ = app.emit("session/updated", serde_json::json!({}));
        }
        NodeType::Mission => {
            let _ = app.emit("mission/changed", serde_json::json!({}));
        }
        _ => {}
    }
    Ok(row)
}

/// A container's direct children, split for the archive-everything
/// delete path shared by folders and projects.
pub(crate) struct ContainerChildren {
    pub session_ids: Vec<String>,
    pub missions: Vec<(String, crate::model::MissionStatus)>,
}

pub(crate) fn container_children(
    conn: &rusqlite::Connection,
    parent_id: &str,
) -> Result<ContainerChildren> {
    let children: Vec<_> = repo::node::list(conn)?
        .into_iter()
        .filter(|row| row.parent_id.as_deref() == Some(parent_id))
        .collect();
    let session_ids: Vec<String> = children
        .iter()
        .filter(|row| row.node_type == NodeType::Tab)
        .flat_map(repo::node::session_ids)
        .collect();
    let missions: Vec<(String, crate::model::MissionStatus)> = children
        .iter()
        .filter(|row| row.node_type == NodeType::Mission)
        .filter_map(|row| row.ref_id.clone())
        .filter_map(|mission_id| {
            repo::mission::get(conn, &mission_id)
                .ok()
                .flatten()
                .map(|mission| (mission_id, mission.status))
        })
        .collect();
    Ok(ContainerChildren {
        session_ids,
        missions,
    })
}

/// What one atomic sweep step decided for a member mission.
pub(crate) enum MissionArchiveStep {
    /// Gone, already archived, or stamped — node cleaned up in the
    /// same transaction. Nothing left to do.
    Done,
    /// The mission is running — the caller must take the full archive
    /// path, AFTER this step's transaction has released.
    NeedsFullArchive,
}

/// One atomic step of the archive-all mission sweep: re-read the
/// mission and, when it is not running, stamp `archived_at` and delete
/// its node inside the SAME Immediate transaction — there is no
/// observable state between the stamp and the node cleanup for a
/// concurrent `mission_reset` to slip into. Running missions are left
/// untouched here; the caller runs the full archive path with no
/// transaction held.
///
/// Deliberately NO bus/router teardown in the non-running and
/// already-archived branches: those states have no live runtime by
/// invariant, and an unconditional teardown after commit is exactly
/// the window in which a reset-spawned fresh run could be disconnected.
pub(crate) fn archive_mission_step(
    conn: &mut rusqlite::Connection,
    mission_id: &str,
) -> Result<MissionArchiveStep> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let Some(mission) = repo::mission::get(&tx, mission_id)? else {
        return Ok(MissionArchiveStep::Done); // deleted meanwhile
    };
    if mission.archived_at.is_some() {
        // Already archived (possibly by another window) — remove the
        // node atomically with that observation.
        repo::node::delete_mission_node(&tx, mission_id)?;
        tx.commit()?;
        return Ok(MissionArchiveStep::Done);
    }
    if mission.status == crate::model::MissionStatus::Running {
        return Ok(MissionArchiveStep::NeedsFullArchive); // tx drops unwritten
    }
    tx.execute(
        "UPDATE missions SET archived_at = ?2
         WHERE id = ?1 AND archived_at IS NULL AND status != 'running'",
        rusqlite::params![mission_id, Utc::now().to_rfc3339()],
    )?;
    repo::node::delete_mission_node(&tx, mission_id)?;
    tx.commit()?;
    Ok(MissionArchiveStep::Done)
}

/// Archive a container's member missions, one by one, each a complete
/// self-consistent operation: running missions go through the full
/// mission-archive path (PTY kills, terminal event, bus/router
/// unmount); stopped ones stamp + drop their node atomically via
/// `archive_mission_step`. The caller's snapshot is advisory only —
/// every decision comes from a fresh transactional read, so a reset
/// landing between steps either wins cleanly (the next step sees a
/// running mission and demands the full path) or happens after the
/// mission is durably archived (a legitimate revival).
pub(crate) async fn archive_child_missions(
    state: &AppState,
    missions: &[(String, crate::model::MissionStatus)],
) -> Result<()> {
    for (mission_id, _snapshot_status) in missions {
        loop {
            let step = {
                let mut conn = state.db.get()?;
                archive_mission_step(&mut conn, mission_id)?
            };
            match step {
                MissionArchiveStep::Done => break,
                MissionArchiveStep::NeedsFullArchive => {
                    match crate::commands::mission::mission_archive_impl(state, mission_id.clone())
                        .await
                    {
                        Ok(_) => break,
                        Err(error) => {
                            // A concurrent archive can win the status
                            // race; if the mission ended up archived
                            // anyway the goal is met — the next step
                            // cleans the node. Otherwise surface it.
                            let archived = {
                                let conn = state.db.get()?;
                                repo::mission::get(&conn, mission_id)?
                                    .is_some_and(|m| m.archived_at.is_some())
                            };
                            if archived {
                                continue;
                            }
                            return Err(error);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn kill_running_children(state: &AppState, session_ids: &[String]) -> Result<()> {
    for session_id in session_ids {
        let running = {
            let conn = state.db.get()?;
            repo::session::get_row(&conn, session_id)?
                .is_some_and(|row| row.status == crate::model::SessionStatus::Running)
        };
        if running {
            state
                .sessions
                .kill(session_id)
                .map_err(|e| Error::msg(format!("stop session {session_id}: {e}")))?;
        }
    }
    Ok(())
}

/// Delete a folder, archiving every node below it — member missions
/// first, then member tabs' chats and the folder node in one
/// transaction. Returns the archived chat session ids for buffer
/// purge + event fanout.
pub(crate) async fn folder_delete_impl(state: &AppState, id: &str) -> Result<Vec<String>> {
    let children = {
        let conn = state.db.get()?;
        let node = repo::node::get(&conn, id)?
            .ok_or_else(|| Error::msg(format!("folder not found: {id}")))?;
        if node.node_type != NodeType::Folder {
            return Err(Error::msg(format!("node {id} is not a folder")));
        }
        container_children(&conn, id)?
    };

    archive_child_missions(state, &children.missions).await?;
    kill_running_children(state, &children.session_ids)?;

    let archived_ids = {
        let mut conn = state.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let archived_ids = repo::node::delete_container_tabs_and_archive(&tx, id)?;
        if repo::node::delete_folder_after_tabs(&tx, id)? == 0 {
            return Err(Error::msg(format!("folder not found: {id}")));
        }
        tx.commit()?;
        archived_ids
    };
    for session_id in &archived_ids {
        state.sessions.purge_session_buffers(session_id);
    }
    Ok(archived_ids)
}

#[tauri::command]
pub async fn node_folder_delete(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<()> {
    let result = folder_delete_impl(&state, &id).await;
    // Mission archives commit one by one BEFORE the folder transaction,
    // so even a failed delete may have durably archived children —
    // invalidate every consuming surface regardless of outcome.
    let _ = app.emit("mission/changed", serde_json::json!({}));
    let _ = app.emit("session/updated", serde_json::json!({}));
    emit_layout_changed(&app);
    let archived_ids = result?;
    for session_id in &archived_ids {
        let _ = app.emit(
            "session/archived",
            serde_json::json!({ "session_id": session_id }),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn node_mark_viewed(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    member_ids: Vec<String>,
) -> Result<NodeRow> {
    mark_node_viewed_for_window(&state, &app, window.label(), &id, member_ids)
}

fn mark_node_viewed_for_window<R: tauri::Runtime>(
    state: &AppState,
    app: &tauri::AppHandle<R>,
    window_label: &str,
    id: &str,
    member_ids: Vec<String>,
) -> Result<NodeRow> {
    state.windows.mark_focused(window_label);
    state.windows.set_subjects(
        window_label,
        member_ids.into_iter().map(Subject::DirectChat).collect(),
    );
    let conn = state.db.get()?;
    let row = repo::node::mark_viewed(&conn, id, Utc::now())?
        .ok_or_else(|| Error::msg(format!("node not found: {id}")))?;
    let _ = app.emit(ATTENTION_CHANGED_EVENT, serde_json::json!({ "tab_id": id }));
    crate::broadcast_focus_map(app);
    Ok(row)
}

pub(crate) fn record_session_completion<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: &str,
) -> Result<()> {
    let Some(state) = app.try_state::<AppState>() else {
        return Ok(());
    };
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::node::ensure_active_sessions(&tx)?;
    let Some(tab) = repo::node::find_for_session(&tx, session_id)? else {
        tx.commit()?;
        return Ok(());
    };
    let member_ids = repo::node::session_ids(&tab);
    let activity = state.sessions.activity_snapshot();
    if member_ids
        .iter()
        .any(|id| activity.get(id) == Some(&SessionActivityState::Busy))
    {
        tx.commit()?;
        return Ok(());
    }
    if !state.sessions.take_completion_armed(&member_ids) {
        tx.commit()?;
        return Ok(());
    }
    let viewed = state.windows.any_focused_displaying(&member_ids);
    let row = repo::node::record_completion(&tx, &tab.id, viewed, Utc::now())?;
    tx.commit()?;
    if row.is_some() {
        let _ = app.emit(
            ATTENTION_CHANGED_EVENT,
            serde_json::json!({ "tab_id": tab.id }),
        );
    }
    Ok(())
}

pub(crate) fn mark_direct_sessions_viewed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    session_ids: &[String],
) -> Result<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::node::ensure_active_sessions(&tx)?;
    let mut tab_ids = Vec::new();
    for session_id in session_ids {
        if let Some(tab) = repo::node::find_for_session(&tx, session_id)? {
            if !tab_ids.contains(&tab.id) {
                tab_ids.push(tab.id);
            }
        }
    }
    let now = Utc::now();
    for tab_id in &tab_ids {
        repo::node::mark_viewed(&tx, tab_id, now)?;
    }
    tx.commit()?;
    if !tab_ids.is_empty() {
        let _ = app.emit(
            ATTENTION_CHANGED_EVENT,
            serde_json::json!({ "tab_ids": tab_ids }),
        );
    }
    Ok(())
}

/// One-time cold-start import of localStorage-era tabs, kept from the
/// 0009 cutover: only applies when the tree has no tab nodes yet.
#[tauri::command]
pub fn node_import_once(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    tabs: Vec<NodeTabImportInput>,
) -> Result<Vec<NodeRow>> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing: i64 =
        tx.query_row("SELECT COUNT(*) FROM nodes WHERE type = 'tab'", [], |row| {
            row.get(0)
        })?;
    if existing == 0 {
        for tab in tabs {
            validate_layout(&tab.layout)?;
            let row =
                repo::node::create_tab(&tx, None, tab.name.trim(), tab.position, &tab.layout)?;
            repo::node::upsert_move_not_copy(&tx, &row)?;
        }
    }
    repo::node::ensure_active_sessions(&tx)?;
    tx.commit()?;
    let rows = repo::node::list(&conn)?;
    emit_layout_changed(&app);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use chrono::{DateTime, FixedOffset, Utc};
    use tauri::{Listener, Manager};

    use super::*;
    use crate::db;
    use crate::event_bus::BusRegistry;
    use crate::mcp::McpHandle;
    use crate::router::RouterRegistry;
    use crate::session::manager::TauriSessionEvents;
    use crate::session::runtime::{
        OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
        SpawnSpec,
    };
    use crate::session::SessionManager;
    use crate::shell_path::LoginShellEnv;
    use crate::windows::WindowRegistry;

    struct InertRuntime;

    impl SessionRuntime for InertRuntime {
        fn spawn(&self, _spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }

        fn stop(&self, _session: &RuntimeSession) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }

        fn send_bytes(&self, _session: &RuntimeSession, _bytes: &[u8]) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }

        fn send_key(&self, _session: &RuntimeSession, _key: &str) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }

        fn resize(&self, _session: &RuntimeSession, _cols: u16, _rows: u16) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }

        fn status(&self, _session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Err(RuntimeError::Msg("unused test runtime".into()))
        }
    }

    fn test_app_in(app_data_dir: PathBuf) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(AppState {
            db: Arc::new(db::open_in_memory().unwrap()),
            app_data_dir,
            sessions: SessionManager::new(LoginShellEnv::default(), Arc::new(InertRuntime)),
            buses: BusRegistry::new(),
            routers: RouterRegistry::new(),
            mcp: Arc::new(McpHandle::new()),
            windows: Arc::new(WindowRegistry::new()),
        });
        app
    }

    fn test_app() -> tauri::App<tauri::test::MockRuntime> {
        test_app_in(PathBuf::new())
    }

    fn create_tab(state: &AppState, session_ids: &[&str]) -> NodeRow {
        let layout = serde_json::json!({
            "preset": if session_ids.len() == 1 { "single" } else { "cols-2" },
            "slots": session_ids,
            "sizes": {},
        });
        let conn = state.db.get().unwrap();
        repo::node::create_tab(&conn, None, "chat", 0, &layout.to_string()).unwrap()
    }

    fn attention_events<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Arc<AtomicUsize> {
        let count = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&count);
        app.listen(ATTENTION_CHANGED_EVENT, move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        count
    }

    fn parsed(value: Option<&str>) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value.expect("timestamp")).unwrap()
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn seed_stopped_session(state: &AppState, id: &str) {
        let conn = state.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, archived_at) VALUES (?1, 'stopped', NULL)",
            [id],
        )
        .unwrap();
    }

    fn seed_mission_with_status(
        state: &AppState,
        id: &str,
        project_id: Option<&str>,
        status: &str,
    ) {
        let conn = state.db.get().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at, project_id)
             VALUES (?1, 'c1', 'M', ?3, '2026-07-01T00:00:00Z', ?2)",
            rusqlite::params![id, project_id, status],
        )
        .unwrap();
    }

    fn seed_mission(state: &AppState, id: &str, project_id: Option<&str>) {
        seed_mission_with_status(state, id, project_id, "aborted");
    }

    #[test]
    fn folder_delete_archives_missions_and_chats_below() {
        let app = test_app();
        let state = app.state::<AppState>();
        seed_stopped_session(&state, "s1");
        seed_mission(&state, "m1", None);
        let folder = {
            let conn = state.db.get().unwrap();
            let folder = repo::node::create_folder(&conn, "F").unwrap();
            repo::node::create_tab(
                &conn,
                Some(&folder.id),
                "",
                0,
                r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
            )
            .unwrap();
            let mission_node = repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
            repo::node::reparent_append(&conn, &mission_node.id, Some(&folder.id)).unwrap();
            folder
        };

        let archived = block_on(folder_delete_impl(&state, &folder.id)).unwrap();
        assert_eq!(archived, vec!["s1".to_string()]);

        let conn = state.db.get().unwrap();
        let mission_archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(mission_archived.is_some(), "member mission archives too");
        let session_archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM sessions WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(session_archived.is_some());
        assert!(repo::node::get(&conn, &folder.id).unwrap().is_none());
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "folder, tab, and mission nodes all gone");
    }

    /// Failure injection: the final transaction fails (a tab member
    /// session is missing), AFTER a member mission already archived.
    /// The mission archive must stay durable, the folder and tab must
    /// survive the rollback — the command layer's unconditional
    /// invalidation exists precisely for this partial state.
    #[test]
    fn folder_delete_partial_failure_keeps_folder_and_durable_mission_archive() {
        let app = test_app();
        let state = app.state::<AppState>();
        seed_mission(&state, "m1", None);
        let (folder, tab) = {
            let conn = state.db.get().unwrap();
            let folder = repo::node::create_folder(&conn, "F").unwrap();
            // Layout references a session that does not exist — the
            // archive UPDATE in the final tx hits 0 rows and errors.
            let tab = repo::node::create_tab(
                &conn,
                Some(&folder.id),
                "",
                0,
                r#"{"preset":"single","slots":["ghost"],"sizes":{}}"#,
            )
            .unwrap();
            let mission_node = repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
            repo::node::reparent_append(&conn, &mission_node.id, Some(&folder.id)).unwrap();
            (folder, tab)
        };

        assert!(block_on(folder_delete_impl(&state, &folder.id)).is_err());

        let conn = state.db.get().unwrap();
        let mission_archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            mission_archived.is_some(),
            "the mission archive committed before the failure and stays durable"
        );
        assert!(repo::node::get(&conn, &folder.id).unwrap().is_some());
        assert!(repo::node::get(&conn, &tab.id).unwrap().is_some());
    }

    /// The post-stamp/pre-cleanup boundary is gone: one step stamps
    /// and deletes the node in a single transaction, so a
    /// `mission_reset` can only land fully before (step sees running,
    /// demands the full path) or fully after (a legitimate revival the
    /// sweep then refuses to stamp and never tears down).
    #[test]
    fn mission_archive_step_is_atomic_against_reset_revival() {
        let app = test_app();
        let state = app.state::<AppState>();
        seed_mission(&state, "m1", None); // aborted
        {
            let conn = state.db.get().unwrap();
            repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
        }

        // Step 1: stamp + node delete commit together.
        {
            let mut conn = state.db.get().unwrap();
            assert!(matches!(
                archive_mission_step(&mut conn, "m1").unwrap(),
                MissionArchiveStep::Done
            ));
        }
        {
            let conn = state.db.get().unwrap();
            let archived: Option<String> = conn
                .query_row(
                    "SELECT archived_at FROM missions WHERE id = 'm1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(archived.is_some());
            assert!(
                repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
                    .unwrap()
                    .is_none()
            );
        }

        // A reset landing AFTER the step legitimately revives the
        // mission: archive marker cleared, node re-created, run live.
        {
            let conn = state.db.get().unwrap();
            repo::mission::reset_to_running(&conn, "m1", chrono::Utc::now()).unwrap();
            repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
        }

        // A stale sweep continuation must now refuse to touch it: no
        // stamp, no node delete — it demands the full archive path.
        {
            let mut conn = state.db.get().unwrap();
            assert!(matches!(
                archive_mission_step(&mut conn, "m1").unwrap(),
                MissionArchiveStep::NeedsFullArchive
            ));
        }
        let conn = state.db.get().unwrap();
        let (status, archived_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "running");
        assert_eq!(archived_at, None, "revived mission must not be stamped");
        assert!(
            repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
                .unwrap()
                .is_some(),
            "the fresh node from the reset survives the refused step"
        );
    }

    /// Concurrency guard at the snapshot/action boundary: the caller's
    /// status snapshot says `aborted`, but the mission was reset to
    /// `running` in the gap (another window). The stamp path must NOT
    /// fire — the loop re-reads and takes the full archive path, which
    /// terminates the run properly (status flips to completed) instead
    /// of stamping `archived_at` onto a still-running mission.
    #[test]
    fn stale_status_snapshot_never_stamps_a_running_mission() {
        let dir = tempfile::tempdir().unwrap();
        let app = test_app_in(dir.path().to_path_buf());
        let state = app.state::<AppState>();
        seed_mission_with_status(&state, "m1", None, "running");
        {
            let conn = state.db.get().unwrap();
            repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
        }

        block_on(archive_child_missions(
            &state,
            &[("m1".to_string(), crate::model::MissionStatus::Aborted)],
        ))
        .unwrap();

        let conn = state.db.get().unwrap();
        let (status, archived_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            status, "completed",
            "the full archive path ran (stale stamp would have left it 'running')"
        );
        assert!(archived_at.is_some());
        assert!(
            repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn project_delete_archives_children_and_unbinds_pointers() {
        let app = test_app();
        let state = app.state::<AppState>();
        let project = {
            let conn = state.db.get().unwrap();
            crate::repo::project::create(&conn, "P", "/tmp/p").unwrap()
        };
        {
            let conn = state.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, status, project_id) VALUES ('s1', 'stopped', ?1)",
                [&project.id],
            )
            .unwrap();
        }
        seed_mission(&state, "m1", Some(&project.id));
        {
            let conn = state.db.get().unwrap();
            let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
            repo::node::create_tab(
                &conn,
                Some(&project_node.id),
                "",
                0,
                r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
            )
            .unwrap();
            repo::node::ensure_mission_node(&conn, "m1", Some(&project.id)).unwrap();
        }

        let archived = block_on(crate::commands::project::project_delete_impl(
            &state,
            &project.id,
        ))
        .unwrap();
        assert_eq!(archived, vec!["s1".to_string()]);

        let conn = state.db.get().unwrap();
        assert!(crate::repo::project::get(&conn, &project.id)
            .unwrap()
            .is_none());
        let (session_archived, session_project): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT archived_at, project_id FROM sessions WHERE id = 's1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(session_archived.is_some());
        assert_eq!(
            session_project, None,
            "row delete unbinds the archived chat's pointer"
        );
        let mission_archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(mission_archived.is_some());
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "project, tab, and mission nodes all gone");
    }

    #[test]
    fn armed_final_idle_in_focused_window_completes_and_views_tab() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        state.windows.mark_focused("main");
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_completed_at, row.last_viewed_at);
        assert!(row.last_completed_at.is_some());
    }

    #[test]
    fn armed_final_idle_in_background_marks_tab_unread() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_some());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn spontaneous_settle_does_not_complete_tab_or_emit_invalidation() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        let observed = attention_events(app.handle());
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());
        assert_eq!(observed.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn armed_member_waits_for_busy_peer_before_completing_tab() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a", "b"]);
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("b", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());

        state
            .sessions
            .publish_direct_activity("b", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_some());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn completion_arm_is_consumed_after_recording() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        let observed = attention_events(app.handle());
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);
        let first = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap()
            .last_completed_at
            .expect("armed settle should record completion");

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_completed_at.as_deref(), Some(first.as_str()));
        assert_eq!(observed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn activation_and_focus_return_advance_viewed_and_emit_invalidation() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        let observed = attention_events(app.handle());
        let first_completion = Utc::now();
        repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, first_completion)
            .unwrap();

        let activated = mark_node_viewed_for_window(
            &state,
            app.handle(),
            "main",
            &tab.id,
            vec!["a".to_string()],
        )
        .unwrap();
        assert!(
            parsed(activated.last_viewed_at.as_deref())
                >= parsed(activated.last_completed_at.as_deref())
        );
        assert_eq!(
            state.windows.focused_direct_sessions("main"),
            ["a".to_string()]
        );
        assert_eq!(observed.load(Ordering::SeqCst), 1);

        state.windows.mark_blurred("main");
        let second_completion = first_completion + chrono::Duration::seconds(1);
        repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, second_completion)
            .unwrap();
        state.windows.mark_focused("main");
        let visible = state.windows.focused_direct_sessions("main");
        mark_direct_sessions_viewed(app.handle(), &state, &visible).unwrap();

        let focused = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(
            parsed(focused.last_viewed_at.as_deref())
                >= parsed(focused.last_completed_at.as_deref())
        );
        assert_eq!(observed.load(Ordering::SeqCst), 2);
    }
}

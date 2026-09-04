// Sidebar node-tree commands (feature 44). One tree query feeds every
// sidebar section; one reparent/reposition op backs every drag.

use chrono::Utc;
use serde::Deserialize;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::events::EventChannel;
use crate::repo;
use crate::repo::node::{NodeRow, NodeType};
use crate::session::manager::SessionActivityState;
use crate::session::SessionManager;
use crate::windows::{Subject, WindowRegistry};
use crate::AppCore;

const ATTENTION_CHANGED_EVENT: &str = "chat/tab-attention-changed";
const LAYOUT_CHANGED_EVENT: &str = "chat/layout-changed";

fn emit_layout_changed(state: &AppCore) {
    state
        .events
        .emit(LAYOUT_CHANGED_EVENT, &serde_json::json!({}));
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

pub fn node_list(state: &AppCore) -> Result<Vec<NodeRow>> {
    let mut conn = state.db.get()?;
    Ok(repo::node::list_with_repair(&mut conn)?)
}

/// Rename a tab node. Project and mission rows keep their names on the
/// domain tables — rename those through `project_rename` / `mission_rename`.
pub fn node_rename(state: &AppCore, id: String, name: String) -> Result<NodeRow> {
    let conn = state.db.get()?;
    let node =
        repo::node::get(&conn, &id)?.ok_or_else(|| Error::msg(format!("node not found: {id}")))?;
    let name = match node.node_type {
        NodeType::Tab => name.trim().to_owned(),
        NodeType::Project | NodeType::Mission => {
            return Err(Error::msg(
                "project and mission names live on their domain rows",
            ))
        }
    };
    repo::node::rename(&conn, &id, &name)?;
    let row = repo::node::get(&conn, &id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    emit_layout_changed(state);
    Ok(row)
}

pub fn node_tab_upsert(state: &AppCore, input: NodeTabUpsertInput) -> Result<NodeRow> {
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
    emit_layout_changed(state);
    Ok(row)
}

pub fn node_mission_layout_set(state: &AppCore, node_id: &str, layout: String) -> Result<NodeRow> {
    validate_layout(&layout)?;
    let mut conn = state.db.get()?;
    let node = repo::node::get(&conn, node_id)?
        .ok_or_else(|| Error::msg(format!("node not found: {node_id}")))?;
    if node.node_type != NodeType::Mission {
        return Err(Error::msg(format!("node {node_id} is not a mission")));
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for session_id in repo::node::session_ids_from_layout(&layout) {
        repo::node::remove_session_except(&tx, &session_id, Some(node_id))?;
    }
    tx.execute(
        "UPDATE nodes SET layout = ?2 WHERE id = ?1",
        rusqlite::params![node_id, layout],
    )?;
    let row = repo::node::get(&tx, node_id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    tx.commit()?;
    emit_layout_changed(state);
    Ok(row)
}

/// Delete a tab node (closing a chat tab). Mission nodes leave via
/// mission archive.
pub fn node_delete(state: &AppCore, id: String) -> Result<()> {
    let conn = state.db.get()?;
    if let Some(node) = repo::node::get(&conn, &id)? {
        if node.node_type != NodeType::Tab {
            return Err(Error::msg(format!("node {id} is not a tab")));
        }
        repo::node::delete(&conn, &id)?;
    }
    emit_layout_changed(state);
    Ok(())
}

/// The unified reparent/reposition op behind every sidebar drag.
/// `ordered_ids` is the complete new ordering of the destination scope's
/// unpinned children (the moved node included when it is unpinned). Crossing
/// a project boundary writes domain `project_id` pointers through.
pub fn node_move(
    state: &AppCore,
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
    emit_layout_changed(state);
    // A cross-project move rewrites domain pointers — nudge the
    // surfaces that render them.
    match moved_type {
        NodeType::Tab => {
            state.events.emit("session/updated", &serde_json::json!({}));
        }
        NodeType::Mission => {
            state.events.emit("mission/changed", &serde_json::json!({}));
        }
        NodeType::Project => {}
    }
    Ok(rows)
}

/// Rewrite the complete global PINNED order. Parent-scoped positions remain
/// dormant and untouched until each row is unpinned.
pub fn node_reorder_pinned(state: &AppCore, ordered_ids: Vec<String>) -> Result<Vec<NodeRow>> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::node::reorder_pinned(&tx, &ordered_ids)
        .map_err(|error| Error::msg(format!("reorder pinned nodes: {error}")))?;
    let rows = repo::node::list(&tx)?;
    tx.commit()?;
    emit_layout_changed(state);
    Ok(rows)
}

/// Pin/unpin a tab or mission row. The node's `pinned_position` is
/// what the sidebar renders; the legacy domain flags
/// (`sessions.pinned_at` for the tab's members, `missions.pinned_at`)
/// are written through because non-sidebar surfaces (the tray sort,
/// MCP mission listings) still read them.
pub fn node_set_pinned(state: &AppCore, id: String, pinned: bool) -> Result<NodeRow> {
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
        NodeType::Project => {
            return Err(Error::msg("only tab and mission rows can be pinned"));
        }
    }
    repo::node::set_pinned(&tx, &id, pinned)?;
    let row = repo::node::get(&tx, &id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    tx.commit()?;
    emit_layout_changed(state);
    match node.node_type {
        NodeType::Tab => {
            state.events.emit("session/updated", &serde_json::json!({}));
        }
        NodeType::Mission => {
            state.events.emit("mission/changed", &serde_json::json!({}));
        }
        _ => {}
    }
    Ok(row)
}

/// A project's direct children, split for its archive-everything delete.
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
/// its node inside the SAME Immediate transaction so reconciliation
/// cannot recreate the node between those writes. Running missions are
/// left untouched here; the caller runs the full archive path with no
/// transaction held.
///
/// Deliberately NO bus/router teardown in the non-running and
/// already-archived branches: those states have no live runtime by
/// invariant, while running missions are routed to the full archive
/// path that owns teardown.
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
/// every decision comes from a fresh transactional read so stale
/// snapshots cannot stamp a running mission.
pub(crate) async fn archive_child_missions(
    state: &AppCore,
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
                    match crate::ops::mission::mission_archive_impl(state, mission_id.clone()).await
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

pub(crate) fn kill_running_children(state: &AppCore, session_ids: &[String]) -> Result<()> {
    let mut failures = Vec::new();
    for session_id in session_ids {
        let running = match state.db.get() {
            Ok(conn) => match repo::session::get_row(&conn, session_id) {
                Ok(row) => {
                    row.is_some_and(|row| row.status == crate::model::SessionStatus::Running)
                }
                Err(error) => {
                    failures.push(format!("{session_id}: {error}"));
                    continue;
                }
            },
            Err(error) => {
                failures.push(format!("{session_id}: {error}"));
                continue;
            }
        };
        if running {
            if let Err(error) = state.sessions.kill(session_id) {
                failures.push(format!("{session_id}: {error}"));
            }
        }
    }
    if !failures.is_empty() {
        return Err(Error::msg(format!(
            "failed to stop child sessions: {}",
            failures.join("; ")
        )));
    }
    Ok(())
}

/// Body of the `node_mark_viewed` command. The window label comes from
/// the invoking window (resolved by the frontend shell), not trusted
/// from the caller.
pub fn node_mark_viewed(
    state: &AppCore,
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
    state.events.emit(
        ATTENTION_CHANGED_EVENT,
        &serde_json::json!({ "tab_id": id }),
    );
    state.broadcast_focus_map();
    Ok(row)
}

/// Takes the state pieces individually (not `&AppCore`) because the main
/// caller is `CoreSessionEvents::status`, which holds the session manager
/// only weakly to avoid an Arc cycle.
pub(crate) fn record_session_completion(
    db: &DbPool,
    sessions: &SessionManager,
    windows: &WindowRegistry,
    events: &EventChannel,
    session_id: &str,
) -> Result<()> {
    let mut conn = db.get()?;
    if repo::session::effective_runtime(&conn, session_id)?.as_deref() == Some("shell") {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::node::ensure_active_sessions(&tx)?;
    let Some(tab) = repo::node::find_for_session(&tx, session_id)? else {
        tx.commit()?;
        return Ok(());
    };
    let mut member_ids = Vec::new();
    for id in repo::node::session_ids(&tab) {
        if repo::session::effective_runtime(&tx, &id)?.as_deref() != Some("shell") {
            member_ids.push(id);
        }
    }
    let activity = sessions.activity_snapshot();
    if member_ids
        .iter()
        .any(|id| activity.get(id) == Some(&SessionActivityState::Busy))
    {
        tx.commit()?;
        return Ok(());
    }
    if !sessions.take_completion_armed(&member_ids) {
        tx.commit()?;
        return Ok(());
    }
    let viewed = windows.any_focused_displaying(&member_ids);
    let row = repo::node::record_completion(&tx, &tab.id, viewed, Utc::now())?;
    tx.commit()?;
    if row.is_some() {
        events.emit(
            ATTENTION_CHANGED_EVENT,
            &serde_json::json!({ "tab_id": tab.id }),
        );
    }
    Ok(())
}

pub fn mark_direct_sessions_viewed(state: &AppCore, session_ids: &[String]) -> Result<()> {
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
        state.events.emit(
            ATTENTION_CHANGED_EVENT,
            &serde_json::json!({ "tab_ids": tab_ids }),
        );
    }
    Ok(())
}

/// One-time cold-start import of localStorage-era tabs, kept from the
/// 0009 cutover: only applies when the tree has no tab nodes yet.
pub fn node_import_once(state: &AppCore, tabs: Vec<NodeTabImportInput>) -> Result<Vec<NodeRow>> {
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
    emit_layout_changed(state);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use chrono::{DateTime, FixedOffset, Utc};
    use tokio::sync::broadcast;

    use super::*;
    use crate::db;
    use crate::event_bus::BusRegistry;
    use crate::events::AppEvent;
    use crate::mcp::McpHandle;
    use crate::router::RouterRegistry;
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

    fn test_core_in(app_data_dir: PathBuf) -> AppCore {
        let runtime_shell_env = Arc::new(std::sync::RwLock::new(LoginShellEnv::default()));
        let runtime_discovery = Arc::new(std::sync::RwLock::new(
            crate::shell_path::DiscoveryState::startup(None, None),
        ));
        AppCore {
            db: Arc::new(db::open_in_memory().unwrap()),
            app_data_dir,
            sessions: SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                Arc::new(InertRuntime),
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: BusRegistry::new(),
            routers: RouterRegistry::new(),
            mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
            mcp: Arc::new(McpHandle::new()),
            windows: Arc::new(WindowRegistry::new()),
            events: EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        }
    }

    fn test_core() -> AppCore {
        test_core_in(PathBuf::new())
    }

    fn create_tab(state: &AppCore, session_ids: &[&str]) -> NodeRow {
        let layout = serde_json::json!({
            "preset": if session_ids.len() == 1 { "single" } else { "cols-2" },
            "slots": session_ids,
            "sizes": {},
        });
        let conn = state.db.get().unwrap();
        repo::node::create_tab(&conn, None, "chat", 0, &layout.to_string()).unwrap()
    }

    /// Count `chat/tab-attention-changed` events delivered since the
    /// receiver subscribed. Every emit in these tests happens on the
    /// calling thread, so draining afterwards observes them all.
    fn drain_attention_count(rx: &mut broadcast::Receiver<AppEvent>) -> usize {
        let mut count = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev.name == ATTENTION_CHANGED_EVENT {
                count += 1;
            }
        }
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

    fn seed_mission_with_status(state: &AppCore, id: &str, project_id: Option<&str>, status: &str) {
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

    fn seed_mission(state: &AppCore, id: &str, project_id: Option<&str>) {
        seed_mission_with_status(state, id, project_id, "aborted");
    }

    #[test]
    fn mission_layout_set_requires_a_mission_and_moves_shells_out_of_tabs() {
        let state = test_core();
        seed_mission(&state, "m1", None);
        let (mission_node, tab) = {
            let conn = state.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('drawer-shell', 'stopped', 'shell', '/bin/zsh')",
                [],
            )
            .unwrap();
            let mission_node = repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
            let tab = repo::node::create_tab(
                &conn,
                None,
                "terminal",
                0,
                r#"{"preset":"single","slots":["drawer-shell"],"sizes":{}}"#,
            )
            .unwrap();
            (mission_node, tab)
        };
        let mut events = state.events.subscribe();
        let layout =
            r#"{"drawer":{"open":true,"height":280,"shells":["drawer-shell"],"active":0}}"#;

        let row = node_mission_layout_set(&state, &mission_node.id, layout.into()).unwrap();

        assert_eq!(row.layout.as_deref(), Some(layout));
        let conn = state.db.get().unwrap();
        assert!(repo::node::get(&conn, &tab.id).unwrap().is_none());
        assert_eq!(repo::node::session_ids(&row), ["drawer-shell"]);
        assert_eq!(events.try_recv().unwrap().name, LAYOUT_CHANGED_EVENT);

        let tab = repo::node::create_tab(
            &conn,
            None,
            "chat",
            0,
            r#"{"preset":"single","slots":[],"sizes":{}}"#,
        )
        .unwrap();
        drop(conn);
        let error = node_mission_layout_set(&state, &tab.id, "{}".into()).unwrap_err();
        assert!(error.to_string().contains("is not a mission"));
    }

    #[test]
    fn mission_archive_closes_drawer_shells_and_tolerates_stale_ids() {
        let archive_state = test_core();
        seed_mission(&archive_state, "archive-me", None);
        {
            let conn = archive_state.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('archive-shell', 'stopped', 'shell', '/bin/zsh')",
                [],
            )
            .unwrap();
            let node = repo::node::ensure_mission_node(&conn, "archive-me", None).unwrap();
            conn.execute(
                "UPDATE nodes SET layout = ?2 WHERE id = ?1",
                rusqlite::params![
                    node.id,
                    r#"{"drawer":{"open":true,"height":280,"shells":["archive-shell","missing-shell"],"active":0}}"#
                ],
            )
            .unwrap();
        }

        block_on(crate::ops::mission::mission_archive_impl(
            &archive_state,
            "archive-me".into(),
        ))
        .unwrap();

        let conn = archive_state.db.get().unwrap();
        assert!(repo::session::get_row(&conn, "archive-shell")
            .unwrap()
            .is_none());
        assert!(
            repo::node::find_by_ref(&conn, NodeType::Mission, "archive-me")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn mission_delete_closes_shells_if_an_archived_row_retains_a_drawer_node() {
        // Normal archive deletes the node; this covers interrupted or legacy persisted state.
        let delete_state = test_core();
        seed_mission(&delete_state, "delete-me", None);
        {
            let conn = delete_state.db.get().unwrap();
            conn.execute(
                "UPDATE missions SET archived_at = '2026-09-04T00:00:00Z'
                 WHERE id = 'delete-me'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('delete-shell', 'stopped', 'shell', '/bin/zsh')",
                [],
            )
            .unwrap();
            let node = repo::node::ensure_mission_node(&conn, "delete-me", None).unwrap();
            conn.execute(
                "UPDATE nodes SET layout = ?2 WHERE id = ?1",
                rusqlite::params![
                    node.id,
                    r#"{"drawer":{"open":true,"height":280,"shells":["delete-shell"],"active":0}}"#
                ],
            )
            .unwrap();
        }

        crate::ops::mission::mission_delete(&delete_state, "delete-me").unwrap();

        let conn = delete_state.db.get().unwrap();
        assert!(repo::session::get_row(&conn, "delete-shell")
            .unwrap()
            .is_none());
        assert!(repo::mission::get(&conn, "delete-me").unwrap().is_none());
    }

    #[test]
    fn archive_mission_step_stamps_and_deletes_node_atomically() {
        let state = test_core();
        seed_mission(&state, "m1", None);
        {
            let conn = state.db.get().unwrap();
            repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
        }

        {
            let mut conn = state.db.get().unwrap();
            assert!(matches!(
                archive_mission_step(&mut conn, "m1").unwrap(),
                MissionArchiveStep::Done
            ));
        }

        let conn = state.db.get().unwrap();
        let archived_at: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM missions WHERE id = 'm1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(archived_at.is_some());
        assert!(
            repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
                .unwrap()
                .is_none()
        );
    }

    /// Concurrency guard at the snapshot/action boundary: the caller's
    /// status snapshot says `aborted`, but the mission is running when
    /// the action executes. The stamp path must NOT fire — the loop
    /// re-reads and takes the full archive path, which terminates the
    /// run properly (status flips to completed) instead of stamping
    /// `archived_at` onto a still-running mission.
    #[test]
    fn stale_status_snapshot_never_stamps_a_running_mission() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_core_in(dir.path().to_path_buf());
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
        let state = test_core();
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

        let archived = block_on(crate::ops::project::project_delete_impl(
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
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        state.windows.mark_focused("main");
        let events = state.session_events();

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
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        let events = state.session_events();

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
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let events = state.session_events();

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
        assert_eq!(drain_attention_count(&mut rx), 0);
    }

    #[test]
    fn armed_member_waits_for_busy_peer_before_completing_tab() {
        let state = test_core();
        let tab = create_tab(&state, &["a", "b"]);
        let events = state.session_events();

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
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let events = state.session_events();

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
        assert_eq!(drain_attention_count(&mut rx), 1);
    }

    #[test]
    fn shell_activity_never_records_tab_completion() {
        let state = test_core();
        state
            .db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command)
                 VALUES ('shell', 'running', ?1, 'shell', '/bin/zsh')",
                [Utc::now().to_rfc3339()],
            )
            .unwrap();
        let tab = create_tab(&state, &["shell"]);
        let events = state.session_events();
        state.sessions.arm_completion("shell");

        state.sessions.publish_direct_activity(
            "shell",
            SessionActivityState::Busy,
            "test",
            &events,
        );
        state.sessions.publish_direct_activity(
            "shell",
            SessionActivityState::Idle,
            "test",
            &events,
        );

        let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn shell_peer_does_not_block_or_trigger_chat_completion() {
        let state = test_core();
        let conn = state.db.get().unwrap();
        let now = Utc::now().to_rfc3339();
        for (id, runtime, command) in [("chat", "codex", "codex"), ("shell", "shell", "/bin/zsh")] {
            conn.execute(
                "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command)
                 VALUES (?1, 'running', ?2, ?3, ?4)",
                rusqlite::params![id, now, runtime, command],
            )
            .unwrap();
        }
        drop(conn);
        let tab = create_tab(&state, &["chat", "shell"]);
        let events = state.session_events();
        state.sessions.arm_completion("chat");

        state
            .sessions
            .publish_direct_activity("chat", SessionActivityState::Busy, "test", &events);
        state.sessions.publish_direct_activity(
            "shell",
            SessionActivityState::Busy,
            "test",
            &events,
        );
        state.sessions.publish_direct_activity(
            "shell",
            SessionActivityState::Idle,
            "test",
            &events,
        );
        let before = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(before.last_completed_at.is_none());

        state.sessions.publish_direct_activity(
            "shell",
            SessionActivityState::Busy,
            "test",
            &events,
        );
        state
            .sessions
            .publish_direct_activity("chat", SessionActivityState::Idle, "test", &events);
        let after = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(after.last_completed_at.is_some());
    }

    #[test]
    fn activation_and_focus_return_advance_viewed_and_emit_invalidation() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let first_completion = Utc::now();
        repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, first_completion)
            .unwrap();

        let activated = node_mark_viewed(&state, "main", &tab.id, vec!["a".to_string()]).unwrap();
        assert!(
            parsed(activated.last_viewed_at.as_deref())
                >= parsed(activated.last_completed_at.as_deref())
        );
        assert_eq!(
            state.windows.focused_direct_sessions("main"),
            ["a".to_string()]
        );
        assert_eq!(drain_attention_count(&mut rx), 1);

        state.windows.mark_blurred("main");
        let second_completion = first_completion + chrono::Duration::seconds(1);
        repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, second_completion)
            .unwrap();
        state.windows.mark_focused("main");
        let visible = state.windows.focused_direct_sessions("main");
        mark_direct_sessions_viewed(&state, &visible).unwrap();

        let focused = repo::node::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(
            parsed(focused.last_viewed_at.as_deref())
                >= parsed(focused.last_completed_at.as_deref())
        );
        assert_eq!(drain_attention_count(&mut rx), 1);
    }
}

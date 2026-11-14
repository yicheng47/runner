// Sidebar node-tree commands (feature 44). One tree query feeds every
// sidebar section; one reparent/reposition op backs every drag.

use crate::model::Runtime;
use chrono::Utc;
use std::collections::HashSet;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::events::EventChannel;
use crate::repo;
use crate::repo::node::{NodeRow, NodeType};
#[cfg(test)]
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

/// A chat or mission that left its project dropped the links between them.
fn emit_unlinked(state: &AppCore, unlinked: usize) {
    if unlinked > 0 {
        state.events.emit("mission/changed", &serde_json::json!({}));
    }
}

fn validate_layout(layout: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(layout)
        .map(|_| ())
        .map_err(|e| Error::msg(format!("invalid tab layout: {e}")))
}

pub use runner_core::protocol::node::NodeTabUpsertInput;

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
    let (row, unlinked) = persist_tab_layout(state, input)?;
    emit_unlinked(state, unlinked);
    state.sessions.reconcile_tab_control(&row.id, &state.db)?;
    emit_layout_changed(state);
    Ok(row)
}

fn persist_tab_layout(state: &AppCore, input: NodeTabUpsertInput) -> Result<(NodeRow, usize)> {
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
            let position = repo::node::new_node_position(&conn, input.parent_id.as_deref())?;
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
    let project_id = repo::node::effective_project(&tx, row.parent_id.as_deref())?;
    let unlinked = repo::node::write_layout_project(
        &tx,
        row.layout.as_deref().expect("tab layout is present"),
        project_id.as_deref(),
    )?;
    tx.commit()?;
    drop(conn);
    Ok((row, unlinked))
}

pub fn node_tab_delete(state: &AppCore, node_id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let node = repo::node::get(&tx, node_id)?
        .ok_or_else(|| Error::msg(format!("node not found: {node_id}")))?;
    if node.node_type != NodeType::Tab {
        return Err(Error::msg(format!("node {node_id} is not a tab")));
    }
    if !repo::node::session_ids(&node).is_empty() {
        return Err(Error::msg(format!("tab {node_id} still has sessions")));
    }
    repo::node::delete(&tx, node_id)?;
    tx.commit()?;
    emit_layout_changed(state);
    Ok(())
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
    let mission_id = node
        .ref_id
        .as_deref()
        .ok_or_else(|| Error::msg(format!("mission node {node_id} has no mission")))?;
    let project_id = repo::mission::get(&tx, mission_id)?
        .ok_or_else(|| Error::msg(format!("mission not found: {mission_id}")))?
        .project_id;
    for session_id in repo::node::session_ids_from_layout(&layout) {
        repo::node::remove_session_except(&tx, &session_id, Some(node_id))?;
    }
    let unlinked = repo::node::write_layout_project(&tx, &layout, project_id.as_deref())?;
    repo::node::set_layout(&tx, node_id, &layout)?;
    let row = repo::node::get(&tx, node_id)?.ok_or_else(|| Error::msg("node disappeared"))?;
    tx.commit()?;
    emit_layout_changed(state);
    emit_unlinked(state, unlinked);
    Ok(row)
}

pub(crate) fn append_order(
    conn: &rusqlite::Connection,
    parent_id: Option<&str>,
    moved_id: &str,
) -> Result<Vec<String>> {
    let moved = repo::node::get(conn, moved_id)?
        .ok_or_else(|| Error::msg(format!("node not found: {moved_id}")))?;
    let mut order: Vec<String> = repo::node::list(conn)?
        .into_iter()
        .filter(|row| {
            row.parent_id.as_deref() == parent_id
                && row.pinned_position.is_none()
                && row.id != moved_id
        })
        .map(|row| row.id)
        .collect();
    if moved.pinned_position.is_none() {
        order.push(moved_id.to_owned());
    }
    Ok(order)
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
    let unlinked = repo::node::move_and_reorder(&tx, &id, parent_id.as_deref(), &ordered_ids)
        .map_err(|error| Error::msg(format!("move node: {error}")))?;
    let rows = repo::node::list(&tx)?;
    tx.commit()?;
    emit_layout_changed(state);
    if moved_type != NodeType::Mission {
        emit_unlinked(state, unlinked);
    }
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
    let mut node =
        repo::node::get(&tx, &id)?.ok_or_else(|| Error::msg(format!("node not found: {id}")))?;
    // Pinning a linked mission pins its chat's group (#748).
    if node.node_type == NodeType::Mission {
        if let Some(head) = node
            .ref_id
            .as_deref()
            .map(|mission| crate::ops::mission::link::group_head(&tx, mission))
            .transpose()?
            .flatten()
        {
            node = head;
        }
    }
    let id = node.id.clone();
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
    repo::mission::archive_if_stopped(&tx, mission_id, Utc::now())?;
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

/// Delete a project's member tabs after archiving their chats and deleting
/// their terminals. The project node itself deletes afterwards (a node with
/// children cannot be deleted). Returns the archived chat ids followed by the
/// deleted terminal ids.
pub(crate) fn delete_container_tabs_and_archive(
    tx: &rusqlite::Transaction<'_>,
    container_id: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    repo::node::check_container_tabs_removable(tx, container_id)?;
    let rows = repo::node::container_tabs(tx, container_id)?;
    let session_ids: HashSet<String> = rows.iter().flat_map(repo::node::session_ids).collect();
    let archived_at = Utc::now();
    let mut archived_ids = Vec::new();
    let mut deleted_ids = Vec::new();
    for id in &session_ids {
        let shell = repo::session::effective_runtime(tx, id)?
            .as_deref()
            .and_then(Runtime::parse)
            .is_some_and(Runtime::is_shell);
        if shell {
            deleted_ids.push(id.clone());
        } else {
            repo::session::archive(tx, id, archived_at)?;
            archived_ids.push(id.clone());
        }
    }
    super::session::delete_rows(tx, &deleted_ids)?;
    for row in &rows {
        repo::node::delete(tx, &row.id)?;
    }
    Ok((archived_ids, deleted_ids))
}

/// Body of the `node_mark_viewed` command. The window label comes from
/// the invoking window (resolved by the frontend shell), not trusted
/// from the caller.
pub fn node_mark_viewed(
    state: &AppCore,
    window_label: &str,
    id: &str,
    member_ids: Vec<String>,
    viewed_session_id: Option<&str>,
) -> Result<Option<NodeRow>> {
    node_mark_viewed_with(state, id, viewed_session_id, || {
        state.windows.mark_focused(window_label);
        state.windows.set_subjects(
            window_label,
            member_ids.into_iter().map(Subject::DirectChat).collect(),
        );
        state
            .windows
            .set_viewed_session(window_label, viewed_session_id);
        Ok(())
    })
}

pub(crate) fn node_mark_viewed_with(
    state: &AppCore,
    id: &str,
    viewed_session_id: Option<&str>,
    update_window: impl FnOnce() -> Result<()>,
) -> Result<Option<NodeRow>> {
    let viewed_ids: Vec<_> = viewed_session_id.into_iter().map(str::to_owned).collect();
    let row = {
        let mut conn = state.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // An exit/focus callback can arrive after archiving removed its tab.
        let Some(tab) = repo::node::get(&tx, id)? else {
            return Ok(None);
        };
        let now = Utc::now().to_rfc3339();
        for session_id in &viewed_ids {
            repo::session_attention::mark_viewed(&tx, session_id, &now)?;
        }
        let row = if repo::session_attention::any_unread(&tx, &repo::node::session_ids(&tab))? {
            Some(tab)
        } else {
            repo::node::mark_viewed(&tx, id, Utc::now())?
        };
        tx.commit()?;
        row
    };
    state.sessions.mark_status_viewed(&viewed_ids);
    update_window()?;
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
    if repo::session::effective_runtime(&conn, session_id)?
        .as_deref()
        .and_then(Runtime::parse)
        .is_some_and(Runtime::is_shell)
    {
        return Ok(());
    }
    if !sessions.take_completion_armed(&[session_id.to_owned()]) {
        return Ok(());
    }
    let viewed = windows.any_focused_displaying(&[session_id.to_owned()]);
    sessions.record_unread(session_id, viewed);
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::session_attention::record_completion(
        &tx,
        session_id,
        viewed,
        Utc::now().timestamp_millis(),
    )?;
    repo::node::ensure_active_sessions(&tx)?;
    let Some(tab) = repo::node::find_for_session(&tx, session_id)? else {
        tx.commit()?;
        return Ok(());
    };
    let viewed =
        viewed && !repo::session_attention::any_unread(&tx, &repo::node::session_ids(&tab))?;
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
    for session_id in session_ids {
        repo::session_attention::mark_viewed(&tx, session_id, &now.to_rfc3339())?;
    }
    for tab_id in &tab_ids {
        if let Some(tab) = repo::node::get(&tx, tab_id)? {
            if !repo::session_attention::any_unread(&tx, &repo::node::session_ids(&tab))? {
                repo::node::mark_viewed(&tx, tab_id, now)?;
            }
        }
    }
    state.sessions.mark_status_viewed(session_ids);
    tx.commit()?;
    state.events.emit(
        ATTENTION_CHANGED_EVENT,
        &serde_json::json!({ "tab_ids": tab_ids, "session_ids": session_ids }),
    );
    Ok(())
}

#[cfg(test)]
mod tests;

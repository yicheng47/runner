// `nodes` table — the sidebar navigation tree (feature 44).
//
// Every sidebar row is a node; `parent_id` + `position` is the single
// containment/ordering mechanism. Container types `folder` (nav-native,
// owns its name) and `project` (references `projects.id`); leaf types
// `tab` (carries the pane layout JSON + attention watermarks) and
// `mission` (references `missions.id`). Sessions are never nodes —
// they're content behind a tab's layout slots.
//
// `pinned_position` non-NULL = pinned; the value orders the PINNED
// overlay. Unpinning just clears it, returning the row to its tree
// position. The tree owns placement/order only: `sessions.project_id` /
// `missions.project_id` stay authoritative for domain membership, so
// reparenting across a project boundary writes the pointer through
// (see `move_and_reorder`).

use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_row;

use super::{de_err, select_list};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Folder,
    Project,
    Tab,
    Mission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub position: i64,
    /// Column is named `type` in SQL; `type` is a Rust keyword.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: Option<String>,
    pub ref_id: Option<String>,
    pub layout: Option<String>,
    pub pinned_position: Option<i64>,
    pub last_completed_at: Option<String>,
    pub last_viewed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredLayout {
    #[serde(default)]
    slots: Vec<Option<String>>,
}

const COLUMNS: &[&str] = &[
    "id",
    "parent_id",
    "position",
    "type",
    "name",
    "ref_id",
    "layout",
    "pinned_position",
    "last_completed_at",
    "last_viewed_at",
    "created_at",
];

/// One tree query. Within each parent scope pinned rows sort first by
/// `pinned_position`, then everything by `position, created_at` — the
/// same order the sidebar renders, so backend and frontend agree.
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<NodeRow>> {
    let sql = format!(
        "SELECT {} FROM nodes
         ORDER BY parent_id IS NOT NULL, parent_id,
                  pinned_position IS NULL, pinned_position,
                  position, created_at",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], |row| from_row(row).map_err(de_err))?
        .collect();
    rows
}

pub fn list_with_repair(conn: &mut Connection) -> rusqlite::Result<Vec<NodeRow>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure_active_sessions(&tx)?;
    let rows = list(&tx)?;
    tx.commit()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<NodeRow>> {
    let sql = format!("SELECT {} FROM nodes WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, [id], |row| from_row(row).map_err(de_err))
        .optional()
}

pub fn upsert(conn: &Connection, row: &NodeRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO nodes (
             id, parent_id, position, type, name, ref_id, layout,
             pinned_position, last_completed_at, last_viewed_at, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
             parent_id = excluded.parent_id,
             position = excluded.position,
             name = excluded.name,
             layout = excluded.layout",
        rusqlite::params![
            row.id,
            row.parent_id,
            row.position,
            node_type_str(row.node_type),
            row.name,
            row.ref_id,
            row.layout,
            row.pinned_position,
            row.last_completed_at,
            row.last_viewed_at,
            row.created_at,
        ],
    )?;
    Ok(())
}

fn node_type_str(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::Folder => "folder",
        NodeType::Project => "project",
        NodeType::Tab => "tab",
        NodeType::Mission => "mission",
    }
}

/// Structural upsert for tab nodes: any member session already living in
/// another tab is moved out of it (a session renders in exactly one tab).
pub fn upsert_move_not_copy(conn: &Connection, row: &NodeRow) -> rusqlite::Result<()> {
    if let Some(layout) = row.layout.as_deref() {
        for session_id in session_ids_from_layout(layout) {
            remove_session_except(conn, &session_id, Some(&row.id))?;
        }
    }
    upsert(conn, row)
}

pub fn next_position(conn: &Connection, parent_id: Option<&str>) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM nodes WHERE parent_id IS ?1",
        [parent_id],
        |row| row.get(0),
    )
}

pub fn create_folder(conn: &Connection, name: &str) -> rusqlite::Result<NodeRow> {
    let row = NodeRow {
        id: ulid::Ulid::new().to_string(),
        parent_id: None,
        position: next_position(conn, None)?,
        node_type: NodeType::Folder,
        name: Some(name.to_owned()),
        ref_id: None,
        layout: None,
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: Utc::now().to_rfc3339(),
    };
    upsert(conn, &row)?;
    Ok(row)
}

pub fn create_tab(
    conn: &Connection,
    parent_id: Option<&str>,
    name: &str,
    position: i64,
    layout: &str,
) -> rusqlite::Result<NodeRow> {
    let row = NodeRow {
        id: ulid::Ulid::new().to_string(),
        parent_id: parent_id.map(str::to_owned),
        position,
        node_type: NodeType::Tab,
        name: Some(name.to_owned()),
        ref_id: None,
        layout: Some(layout.to_owned()),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: Utc::now().to_rfc3339(),
    };
    upsert(conn, &row)?;
    Ok(row)
}

fn create_ref_node(
    conn: &Connection,
    node_type: NodeType,
    ref_id: &str,
    parent_id: Option<&str>,
) -> rusqlite::Result<NodeRow> {
    let row = NodeRow {
        id: ulid::Ulid::new().to_string(),
        parent_id: parent_id.map(str::to_owned),
        position: next_position(conn, parent_id)?,
        node_type,
        name: None,
        ref_id: Some(ref_id.to_owned()),
        layout: None,
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: Utc::now().to_rfc3339(),
    };
    upsert(conn, &row)?;
    Ok(row)
}

pub fn find_by_ref(
    conn: &Connection,
    node_type: NodeType,
    ref_id: &str,
) -> rusqlite::Result<Option<NodeRow>> {
    let sql = format!(
        "SELECT {} FROM nodes WHERE type = ?1 AND ref_id = ?2",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, [node_type_str(node_type), ref_id], |row| {
        from_row(row).map_err(de_err)
    })
    .optional()
}

/// Project node for a project row, created at the root end if missing.
pub fn ensure_project_node(conn: &Connection, project_id: &str) -> rusqlite::Result<NodeRow> {
    match find_by_ref(conn, NodeType::Project, project_id)? {
        Some(row) => Ok(row),
        None => create_ref_node(conn, NodeType::Project, project_id, None),
    }
}

/// Mission node, parented under the mission's project node when
/// `project_id` is set, appended at the parent's end. Idempotent.
pub fn ensure_mission_node(
    conn: &Connection,
    mission_id: &str,
    project_id: Option<&str>,
) -> rusqlite::Result<NodeRow> {
    if let Some(row) = find_by_ref(conn, NodeType::Mission, mission_id)? {
        return Ok(row);
    }
    let parent = match project_id {
        Some(project_id) => Some(ensure_project_node(conn, project_id)?.id),
        None => None,
    };
    create_ref_node(conn, NodeType::Mission, mission_id, parent.as_deref())
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM nodes WHERE id = ?1", [id])
}

pub fn delete_mission_node(conn: &Connection, mission_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM nodes WHERE type = 'mission' AND ref_id = ?1",
        [mission_id],
    )
}

/// Remove a project's node, reparenting any children to the root end
/// first (the self-FK is ON DELETE RESTRICT). Domain unbinding is the
/// caller's `projects` delete — `project_id` pointers go NULL there.
pub fn delete_project_node(conn: &Connection, project_id: &str) -> rusqlite::Result<()> {
    let Some(node) = find_by_ref(conn, NodeType::Project, project_id)? else {
        return Ok(());
    };
    reparent_children_to_root(conn, &node.id)?;
    delete(conn, &node.id)?;
    Ok(())
}

fn reparent_children_to_root(conn: &Connection, parent_id: &str) -> rusqlite::Result<()> {
    let children: Vec<String> = conn
        .prepare(
            "SELECT id FROM nodes WHERE parent_id = ?1
             ORDER BY position, created_at",
        )?
        .query_map([parent_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in children {
        let position = next_position(conn, None)?;
        conn.execute(
            "UPDATE nodes SET parent_id = NULL, position = ?2 WHERE id = ?1",
            rusqlite::params![id, position],
        )?;
    }
    Ok(())
}

pub fn rename(conn: &Connection, id: &str, name: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE nodes SET name = ?2 WHERE id = ?1",
        rusqlite::params![id, name],
    )
}

/// Pin appends at the PINNED end (an already-pinned row keeps its
/// slot); unpin clears — the row returns to its tree position for free.
pub fn set_pinned(conn: &Connection, id: &str, pinned: bool) -> rusqlite::Result<usize> {
    if pinned {
        conn.execute(
            "UPDATE nodes
             SET pinned_position = COALESCE(pinned_position,
                 (SELECT COALESCE(MAX(n2.pinned_position) + 1, 0) FROM nodes n2))
             WHERE id = ?1",
            [id],
        )
    } else {
        conn.execute(
            "UPDATE nodes SET pinned_position = NULL WHERE id = ?1",
            [id],
        )
    }
}

/// The unified reparent/reposition op behind every sidebar drag.
///
/// Nesting is enforced here at the app layer (the schema allows a
/// general tree): containers (`folder`, `project`) live at root only;
/// leaves (`tab`, `mission`) live at root or under one container.
/// Folder deletion archives everything below the folder — member
/// tabs' chats and member missions alike (`commands::node::
/// folder_delete_impl`). The sidebar additionally refuses to DRAG a
/// node out of a project scope (leaving a project is an explicit menu
/// action); that is a UI affordance, not a shape rule, so it lives in
/// the frontend's `canDropInScope`, not here.
///
/// Crossing a project boundary writes the domain pointer through:
/// a tab updates its member sessions' `sessions.project_id`, a mission
/// updates `missions.project_id` plus its sessions — the pointers stay
/// authoritative for cwd binding and project scoping.
pub fn move_and_reorder(
    tx: &Transaction<'_>,
    id: &str,
    parent_id: Option<&str>,
    ordered_ids: &[String],
) -> rusqlite::Result<()> {
    let node = get(tx, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let parent = match parent_id {
        Some(parent_id) => Some(get(tx, parent_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?),
        None => None,
    };
    let parent_type = parent.as_ref().map(|p| p.node_type);
    let allowed = match node.node_type {
        NodeType::Folder | NodeType::Project => parent_type.is_none(),
        NodeType::Tab | NodeType::Mission => matches!(
            parent_type,
            None | Some(NodeType::Folder) | Some(NodeType::Project)
        ),
    };
    if !allowed {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "cannot nest a {} node under this parent",
            node_type_str(node.node_type)
        )));
    }

    let old_project = effective_project(tx, node.parent_id.as_deref())?;
    let new_project = match (&parent, parent_type) {
        (Some(p), Some(NodeType::Project)) => p.ref_id.clone(),
        _ => None,
    };
    if old_project != new_project {
        write_project_through(tx, &node, new_project.as_deref())?;
    }

    tx.execute(
        "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
        rusqlite::params![id, parent_id],
    )?;
    let actual: Vec<String> = tx
        .prepare("SELECT id FROM nodes WHERE parent_id IS ?1")?
        .query_map([parent_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let expected: HashSet<&str> = actual.iter().map(String::as_str).collect();
    let provided: HashSet<&str> = ordered_ids.iter().map(String::as_str).collect();
    if ordered_ids.len() != actual.len()
        || provided.len() != ordered_ids.len()
        || provided != expected
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "ordered node ids do not match destination scope".to_owned(),
        ));
    }
    for (position, node_id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "UPDATE nodes SET position = ?2 WHERE id = ?1",
            rusqlite::params![node_id, position as i64],
        )?;
    }
    Ok(())
}

/// Reparent a node to the end of a new scope without reordering the
/// destination — the move-to-project menu paths use this; drags go
/// through `move_and_reorder`. Callers own any domain write-through.
pub fn reparent_append(
    conn: &Connection,
    id: &str,
    parent_id: Option<&str>,
) -> rusqlite::Result<usize> {
    let position = next_position(conn, parent_id)?;
    conn.execute(
        "UPDATE nodes SET parent_id = ?2, position = ?3 WHERE id = ?1",
        rusqlite::params![id, parent_id, position],
    )
}

/// Re-derive a tab node's placement from its members' `sessions.
/// project_id` pointers, after a pointer write that may cover only
/// some members: a unanimous non-NULL project wins (project placement
/// trumps a folder, matching the pre-node sidebar's derived grouping);
/// mixed or absent membership leaves a folder placement alone but
/// moves the tab out of a project node to the root end.
pub fn reconcile_tab_placement(conn: &Connection, tab_id: &str) -> rusqlite::Result<()> {
    let Some(tab) = get(conn, tab_id)? else {
        return Ok(());
    };
    if tab.node_type != NodeType::Tab {
        return Ok(());
    }
    let members = session_ids(&tab);
    let mut unanimous: Option<String> = None;
    let mut all_share = !members.is_empty();
    for (index, session_id) in members.iter().enumerate() {
        let project_id: Option<Option<String>> = conn
            .query_row(
                "SELECT project_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        let project_id = project_id.flatten();
        if project_id.is_none() || (index > 0 && project_id != unanimous) {
            all_share = false;
            break;
        }
        unanimous = project_id;
    }
    if all_share {
        if let Some(project_id) = unanimous {
            let parent = ensure_project_node(conn, &project_id)?;
            if tab.parent_id.as_deref() != Some(parent.id.as_str()) {
                reparent_append(conn, &tab.id, Some(&parent.id))?;
            }
            return Ok(());
        }
    }
    if effective_project(conn, tab.parent_id.as_deref())?.is_some() {
        reparent_append(conn, &tab.id, None)?;
    }
    Ok(())
}

/// The project a node belongs to by placement: its parent when the
/// parent is a project node.
fn effective_project(
    conn: &Connection,
    parent_id: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let Some(parent_id) = parent_id else {
        return Ok(None);
    };
    Ok(get(conn, parent_id)?
        .filter(|p| p.node_type == NodeType::Project)
        .and_then(|p| p.ref_id))
}

fn write_project_through(
    conn: &Connection,
    node: &NodeRow,
    project_id: Option<&str>,
) -> rusqlite::Result<()> {
    match node.node_type {
        NodeType::Tab => {
            for session_id in node
                .layout
                .as_deref()
                .map(session_ids_from_layout)
                .unwrap_or_default()
            {
                conn.execute(
                    "UPDATE sessions SET project_id = ?2
                     WHERE id = ?1 AND mission_id IS NULL AND slot_id IS NULL",
                    rusqlite::params![session_id, project_id],
                )?;
            }
        }
        NodeType::Mission => {
            let Some(mission_id) = node.ref_id.as_deref() else {
                return Ok(());
            };
            conn.execute(
                "UPDATE missions SET project_id = ?2 WHERE id = ?1",
                rusqlite::params![mission_id, project_id],
            )?;
            conn.execute(
                "UPDATE sessions SET project_id = ?2 WHERE mission_id = ?1",
                rusqlite::params![mission_id, project_id],
            )?;
        }
        NodeType::Folder | NodeType::Project => {}
    }
    Ok(())
}

pub fn session_ids(row: &NodeRow) -> Vec<String> {
    row.layout
        .as_deref()
        .map(session_ids_from_layout)
        .unwrap_or_default()
}

pub fn session_ids_from_layout(layout: &str) -> Vec<String> {
    serde_json::from_str::<StoredLayout>(layout)
        .map(|layout| layout.slots.into_iter().flatten().collect())
        .unwrap_or_default()
}

pub fn find_for_session(conn: &Connection, session_id: &str) -> rusqlite::Result<Option<NodeRow>> {
    Ok(list(conn)?
        .into_iter()
        .filter(|row| row.node_type == NodeType::Tab)
        .find(|row| session_ids(row).iter().any(|id| id == session_id)))
}

fn max_timestamp(values: &[Option<&str>], now: chrono::DateTime<Utc>) -> String {
    values
        .iter()
        .flatten()
        .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .chain(std::iter::once(now))
        .max()
        .expect("now always supplies a timestamp")
        .to_rfc3339()
}

pub fn record_completion(
    conn: &Connection,
    id: &str,
    viewed: bool,
    now: chrono::DateTime<Utc>,
) -> rusqlite::Result<Option<NodeRow>> {
    let Some(row) = get(conn, id)? else {
        return Ok(None);
    };
    let completed_at = max_timestamp(&[row.last_completed_at.as_deref()], now);
    let viewed_at = viewed.then(|| {
        max_timestamp(
            &[row.last_viewed_at.as_deref(), Some(completed_at.as_str())],
            now,
        )
    });
    conn.execute(
        "UPDATE nodes
         SET last_completed_at = ?2,
             last_viewed_at = CASE WHEN ?3 THEN ?4 ELSE last_viewed_at END
         WHERE id = ?1",
        rusqlite::params![id, completed_at, viewed, viewed_at],
    )?;
    get(conn, id)
}

pub fn mark_viewed(
    conn: &Connection,
    id: &str,
    now: chrono::DateTime<Utc>,
) -> rusqlite::Result<Option<NodeRow>> {
    let Some(row) = get(conn, id)? else {
        return Ok(None);
    };
    let viewed_at = max_timestamp(
        &[
            row.last_viewed_at.as_deref(),
            row.last_completed_at.as_deref(),
        ],
        now,
    );
    conn.execute(
        "UPDATE nodes SET last_viewed_at = ?2 WHERE id = ?1",
        rusqlite::params![id, viewed_at],
    )?;
    get(conn, id)
}

/// Rust mirror of the frontend's `tabHasUnreadCompletion`
/// (chatAttention.ts): parsed comparison, not string order —
/// chrono's RFC3339 fractional-second width varies.
fn has_unread_completion(completed_at: Option<&str>, viewed_at: Option<&str>) -> bool {
    let Some(completed) =
        completed_at.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return false;
    };
    let Some(viewed) = viewed_at.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return true;
    };
    completed > viewed
}

/// Startup sweep, run alongside the stale-`running` session demotion:
/// a completion recorded by a prior app process points at output whose
/// in-memory ring died with it, so a carried-over unread dot would
/// advertise a result the app can no longer show. Stamp those tabs
/// viewed. Returns the number of tab nodes cleared.
pub fn clear_unread_on_startup(
    conn: &Connection,
    now: chrono::DateTime<Utc>,
) -> rusqlite::Result<usize> {
    let mut cleared = 0;
    for row in list(conn)? {
        if !has_unread_completion(
            row.last_completed_at.as_deref(),
            row.last_viewed_at.as_deref(),
        ) {
            continue;
        }
        let viewed_at = max_timestamp(
            &[
                row.last_viewed_at.as_deref(),
                row.last_completed_at.as_deref(),
            ],
            now,
        );
        cleared += conn.execute(
            "UPDATE nodes SET last_viewed_at = ?2 WHERE id = ?1",
            rusqlite::params![row.id, viewed_at],
        )?;
    }
    Ok(cleared)
}

/// Invariant repair for the whole tree, run inside the caller's
/// transaction on every tree read:
///
///   - every `projects` row has a project node (created at the root
///     end); project nodes whose row is gone are removed, children
///     reparented to root;
///   - every non-archived mission has a mission node (under its
///     project node when bound); mission nodes whose mission is
///     archived or deleted are removed — this is what re-creates a
///     node on unarchive/reset without special-case handling;
///   - every active direct session is covered by exactly the tab
///     layouts; uncovered sessions get a fresh single-slot tab node,
///     parented under their project's node when `project_id` is set,
///     appended at the parent's end (original position not
///     remembered, matching pre-node archive/restore behavior).
pub fn ensure_active_sessions(conn: &Connection) -> rusqlite::Result<()> {
    // Projects.
    let project_ids: Vec<String> = conn
        .prepare("SELECT id FROM projects ORDER BY position, created_at")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for project_id in &project_ids {
        ensure_project_node(conn, project_id)?;
    }
    let stale_projects: Vec<String> = conn
        .prepare(
            "SELECT ref_id FROM nodes
             WHERE type = 'project'
               AND ref_id NOT IN (SELECT id FROM projects)",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for project_id in &stale_projects {
        delete_project_node(conn, project_id)?;
    }

    // Missions.
    let missions: Vec<(String, Option<String>)> = conn
        .prepare(
            "SELECT id, project_id FROM missions
             WHERE archived_at IS NULL
             ORDER BY started_at, id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (mission_id, project_id) in &missions {
        ensure_mission_node(conn, mission_id, project_id.as_deref())?;
    }
    conn.execute(
        "DELETE FROM nodes
         WHERE type = 'mission'
           AND ref_id NOT IN
               (SELECT id FROM missions WHERE archived_at IS NULL)",
        [],
    )?;

    // Direct sessions -> tab nodes.
    let covered: HashSet<String> = list(conn)?
        .iter()
        .filter(|row| row.node_type == NodeType::Tab)
        .flat_map(session_ids)
        .collect();
    let mut stmt = conn.prepare(
        "SELECT id, project_id FROM sessions
         WHERE mission_id IS NULL AND slot_id IS NULL AND archived_at IS NULL
         ORDER BY COALESCE(started_at, stopped_at), id",
    )?;
    let sessions: Vec<(String, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for (session_id, project_id) in sessions.into_iter().filter(|(id, _)| !covered.contains(id)) {
        let parent = match project_id.as_deref() {
            Some(project_id) => Some(ensure_project_node(conn, project_id)?.id),
            None => None,
        };
        let layout = serde_json::json!({
            "preset": "single",
            "slots": [session_id],
            "sizes": {},
        })
        .to_string();
        let position = next_position(conn, parent.as_deref())?;
        create_tab(conn, parent.as_deref(), "", position, &layout)?;
    }
    Ok(())
}

pub fn remove_session(conn: &Connection, session_id: &str) -> rusqlite::Result<()> {
    remove_session_except(conn, session_id, None)
}

fn remove_session_except(
    conn: &Connection,
    session_id: &str,
    preserved_node_id: Option<&str>,
) -> rusqlite::Result<()> {
    for row in list(conn)? {
        if row.node_type != NodeType::Tab || preserved_node_id == Some(row.id.as_str()) {
            continue;
        }
        let Some(layout_text) = row.layout.as_deref() else {
            continue;
        };
        let Ok(mut layout) = serde_json::from_str::<serde_json::Value>(layout_text) else {
            continue;
        };
        let Some(slots) = layout.get_mut("slots").and_then(|v| v.as_array_mut()) else {
            continue;
        };
        let mut changed = false;
        for slot in slots.iter_mut() {
            if slot.as_str() == Some(session_id) {
                *slot = serde_json::Value::Null;
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        if slots.iter().all(serde_json::Value::is_null) {
            delete(conn, &row.id)?;
        } else {
            conn.execute(
                "UPDATE nodes SET layout = ?2 WHERE id = ?1",
                rusqlite::params![row.id, layout.to_string()],
            )?;
        }
    }
    Ok(())
}

/// Delete a container's member tabs after archiving their sessions —
/// the container node itself deletes afterwards (self-FK is RESTRICT).
/// Used by folder AND project deletion, which both archive everything
/// below the container.
pub fn delete_container_tabs_and_archive(
    tx: &Transaction<'_>,
    container_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let rows = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {} FROM nodes
             WHERE parent_id = ?1 AND type = 'tab'
             ORDER BY position, created_at",
            select_list(COLUMNS)
        ))?;
        let rows = stmt
            .query_map([container_id], |row| from_row(row).map_err(de_err))?
            .collect::<rusqlite::Result<Vec<NodeRow>>>()?;
        rows
    };
    let session_ids: HashSet<String> = rows.iter().flat_map(session_ids).collect();
    let archived_at = Utc::now().to_rfc3339();
    for id in &session_ids {
        let updated = tx.execute(
            "UPDATE sessions SET archived_at = ?2
             WHERE id = ?1 AND mission_id IS NULL AND slot_id IS NULL AND status != 'running'",
            rusqlite::params![id, archived_at],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "session {id} is missing or still running"
            )));
        }
    }
    tx.execute(
        "DELETE FROM nodes WHERE parent_id = ?1 AND type = 'tab'",
        [container_id],
    )?;
    Ok(session_ids.into_iter().collect())
}

pub fn delete_folder_after_tabs(tx: &Transaction<'_>, id: &str) -> rusqlite::Result<usize> {
    tx.execute("DELETE FROM nodes WHERE id = ?1 AND type = 'folder'", [id])
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use chrono::TimeZone;

    use super::NodeType;
    use crate::db;

    fn parsed(value: Option<&str>) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(value.expect("timestamp")).unwrap()
    }

    fn insert_session(conn: &rusqlite::Connection, id: &str, project_id: Option<&str>) {
        conn.execute(
            "INSERT INTO sessions (id, status, project_id, archived_at)
             VALUES (?1, 'stopped', ?2, NULL)",
            rusqlite::params![id, project_id],
        )
        .unwrap();
    }

    fn insert_mission(
        conn: &rusqlite::Connection,
        id: &str,
        project_id: Option<&str>,
        archived: bool,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let archived_at = archived.then_some("2026-07-02T00:00:00Z");
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at,
                                   project_id, archived_at)
             VALUES (?1, 'c1', 'M', 'running', '2026-07-01T00:00:00Z', ?2, ?3)",
            rusqlite::params![id, project_id, archived_at],
        )
        .unwrap();
    }

    #[test]
    fn attention_watermarks_default_null_and_remain_monotonic() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = super::create_tab(
            &conn,
            None,
            "chat",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        assert_eq!(row.last_completed_at, None);
        assert_eq!(row.last_viewed_at, None);

        let first = chrono::Utc.timestamp_opt(200, 0).single().unwrap();
        let earlier = chrono::Utc.timestamp_opt(100, 0).single().unwrap();
        let later = chrono::Utc.timestamp_opt(300, 0).single().unwrap();
        let completed = super::record_completion(&conn, &row.id, false, first)
            .unwrap()
            .unwrap();
        assert_eq!(completed.last_viewed_at, None);

        let viewed = super::mark_viewed(&conn, &row.id, earlier)
            .unwrap()
            .unwrap();
        assert!(
            parsed(viewed.last_viewed_at.as_deref()) >= parsed(viewed.last_completed_at.as_deref())
        );

        let regressed = super::record_completion(&conn, &row.id, false, earlier)
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed(regressed.last_completed_at.as_deref()),
            parsed(viewed.last_completed_at.as_deref())
        );

        let unread = super::record_completion(&conn, &row.id, false, later)
            .unwrap()
            .unwrap();
        assert!(
            parsed(unread.last_completed_at.as_deref()) > parsed(unread.last_viewed_at.as_deref())
        );

        let visible_completion = super::record_completion(&conn, &row.id, true, later)
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed(visible_completion.last_completed_at.as_deref()),
            parsed(visible_completion.last_viewed_at.as_deref())
        );
    }

    #[test]
    fn startup_clear_stamps_only_unread_completions_viewed() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let unread = super::create_tab(
            &conn,
            None,
            "unread",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        let viewed = super::create_tab(
            &conn,
            None,
            "viewed",
            1,
            r#"{"preset":"single","slots":["s2"],"sizes":{}}"#,
        )
        .unwrap();
        let fresh = super::create_tab(
            &conn,
            None,
            "fresh",
            2,
            r#"{"preset":"single","slots":["s3"],"sizes":{}}"#,
        )
        .unwrap();

        let completed_at = chrono::Utc.timestamp_opt(200, 0).single().unwrap();
        super::record_completion(&conn, &unread.id, false, completed_at).unwrap();
        super::record_completion(&conn, &viewed.id, true, completed_at).unwrap();
        let viewed_before = super::get(&conn, &viewed.id).unwrap().unwrap();

        let now = chrono::Utc.timestamp_opt(300, 0).single().unwrap();
        assert_eq!(super::clear_unread_on_startup(&conn, now).unwrap(), 1);

        let unread_after = super::get(&conn, &unread.id).unwrap().unwrap();
        assert!(
            parsed(unread_after.last_viewed_at.as_deref())
                >= parsed(unread_after.last_completed_at.as_deref())
        );
        let viewed_after = super::get(&conn, &viewed.id).unwrap().unwrap();
        assert_eq!(viewed_after.last_viewed_at, viewed_before.last_viewed_at);
        let fresh_after = super::get(&conn, &fresh.id).unwrap().unwrap();
        assert_eq!(fresh_after.last_viewed_at, None);

        assert_eq!(super::clear_unread_on_startup(&conn, now).unwrap(), 0);
    }

    #[test]
    fn structural_upsert_preserves_attention_watermarks_and_pin() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let mut stale = super::create_tab(
            &conn,
            None,
            "chat",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        let completed_at = chrono::Utc.timestamp_opt(200, 0).single().unwrap();
        super::record_completion(&conn, &stale.id, true, completed_at).unwrap();
        super::set_pinned(&conn, &stale.id, true).unwrap();
        let before = super::get(&conn, &stale.id).unwrap().unwrap();

        stale.name = Some("renamed".into());
        stale.layout = Some(r#"{"preset":"cols-2","slots":["s1",null],"sizes":{}}"#.into());
        super::upsert_move_not_copy(&conn, &stale).unwrap();

        let folder = super::create_folder(&conn, "Project").unwrap();
        let peer = super::create_tab(
            &conn,
            Some(&folder.id),
            "peer",
            0,
            r#"{"preset":"single","slots":["s2"],"sizes":{}}"#,
        )
        .unwrap();
        let mut conn = conn;
        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &stale.id,
            Some(&folder.id),
            &[peer.id, stale.id.clone()],
        )
        .unwrap();
        tx.commit().unwrap();

        let after = super::get(&conn, &stale.id).unwrap().unwrap();
        assert_eq!(after.name.as_deref(), Some("renamed"));
        assert_eq!(after.layout, stale.layout);
        assert_eq!(after.parent_id, Some(folder.id));
        assert_eq!(after.position, 1);
        assert_eq!(after.last_completed_at, before.last_completed_at);
        assert_eq!(after.last_viewed_at, before.last_viewed_at);
        assert_eq!(after.pinned_position, before.pinned_position);
        assert!(after.pinned_position.is_some());
    }

    #[test]
    fn ensure_active_sessions_creates_stable_single_tabs_once() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_session(&conn, "s1", None);
        super::ensure_active_sessions(&conn).unwrap();
        let first = super::list(&conn).unwrap();
        super::ensure_active_sessions(&conn).unwrap();
        let second = super::list(&conn).unwrap();
        assert_eq!(first, second);
        assert_eq!(super::session_ids(&first[0]), ["s1"]);
    }

    #[test]
    fn ensure_active_sessions_parents_project_bound_chats_under_their_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        insert_session(&conn, "s1", Some(&project.id));
        insert_session(&conn, "s2", None);

        super::ensure_active_sessions(&conn).unwrap();

        let rows = super::list(&conn).unwrap();
        let project_node = rows
            .iter()
            .find(|row| row.node_type == NodeType::Project)
            .expect("project node repaired");
        assert_eq!(project_node.ref_id.as_deref(), Some(project.id.as_str()));
        let bound = rows
            .iter()
            .find(|row| super::session_ids(row) == ["s1"])
            .unwrap();
        assert_eq!(bound.parent_id.as_deref(), Some(project_node.id.as_str()));
        let loose = rows
            .iter()
            .find(|row| super::session_ids(row) == ["s2"])
            .unwrap();
        assert_eq!(loose.parent_id, None);
    }

    #[test]
    fn ensure_active_sessions_repairs_mission_nodes() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        insert_mission(&conn, "m-live", Some(&project.id), false);
        insert_mission(&conn, "m-arch", None, true);
        // Stale node for the archived mission, as if archive crashed
        // before removing it.
        super::ensure_mission_node(&conn, "m-arch", None).unwrap();
        conn.execute(
            "UPDATE missions SET archived_at = '2026-07-02T00:00:00Z' WHERE id = 'm-arch'",
            [],
        )
        .unwrap();

        super::ensure_active_sessions(&conn).unwrap();

        let live = super::find_by_ref(&conn, NodeType::Mission, "m-live")
            .unwrap()
            .expect("live mission node repaired");
        let project_node = super::find_by_ref(&conn, NodeType::Project, &project.id)
            .unwrap()
            .unwrap();
        assert_eq!(live.parent_id.as_deref(), Some(project_node.id.as_str()));
        assert!(super::find_by_ref(&conn, NodeType::Mission, "m-arch")
            .unwrap()
            .is_none());
    }

    #[test]
    fn concurrent_tree_reads_seed_one_tab() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open_pool(&dir.path().join("runner.db")).unwrap();
        {
            let conn = pool.get().unwrap();
            insert_session(&conn, "s1", None);
        }

        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let pool = pool.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut conn = pool.get().unwrap();
                    barrier.wait();
                    super::list_with_repair(&mut conn).unwrap()
                })
            })
            .collect();
        barrier.wait();
        for handle in handles {
            let rows = handle.join().unwrap();
            assert_eq!(
                rows.iter().flat_map(super::session_ids).collect::<Vec<_>>(),
                ["s1"]
            );
        }

        let conn = pool.get().unwrap();
        let rows = super::list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(super::session_ids(&rows[0]), ["s1"]);
    }

    #[test]
    fn remove_session_deletes_empty_tab_and_preserves_other_members() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = super::create_tab(
            &conn,
            None,
            "pair",
            0,
            r#"{"preset":"cols-2","slots":["a","b"],"sizes":{}}"#,
        )
        .unwrap();
        super::remove_session(&conn, "a").unwrap();
        assert_eq!(
            super::session_ids(&super::get(&conn, &row.id).unwrap().unwrap()),
            ["b"]
        );
        super::remove_session(&conn, "b").unwrap();
        assert!(super::get(&conn, &row.id).unwrap().is_none());
    }

    #[test]
    fn folder_delete_archives_members_before_restricted_delete() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        insert_session(&conn, "s1", None);
        let folder = super::create_folder(&conn, "Project").unwrap();
        super::create_tab(
            &conn,
            Some(&folder.id),
            "",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        assert!(conn
            .execute("DELETE FROM nodes WHERE id = ?1", [&folder.id])
            .is_err());

        let tx = conn.transaction().unwrap();
        let ids = super::delete_container_tabs_and_archive(&tx, &folder.id).unwrap();
        assert_eq!(ids, ["s1"]);
        super::delete_folder_after_tabs(&tx, &folder.id).unwrap();
        tx.commit().unwrap();

        let archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM sessions WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(archived.is_some());
        assert!(super::get(&conn, &folder.id).unwrap().is_none());
        assert!(super::list(&conn).unwrap().is_empty());
    }

    #[test]
    fn upsert_moves_sessions_out_of_other_tabs() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let source = super::create_tab(
            &conn,
            None,
            "source",
            0,
            r#"{"preset":"cols-2","slots":["a","b"],"sizes":{}}"#,
        )
        .unwrap();
        let mut target = super::create_tab(
            &conn,
            None,
            "target",
            1,
            r#"{"preset":"single","slots":[null],"sizes":{}}"#,
        )
        .unwrap();
        target.layout = Some(r#"{"preset":"single","slots":["b"],"sizes":{}}"#.to_owned());
        super::upsert_move_not_copy(&conn, &target).unwrap();

        assert_eq!(
            super::session_ids(&super::get(&conn, &source.id).unwrap().unwrap()),
            ["a"]
        );
        assert_eq!(
            super::session_ids(&super::get(&conn, &target.id).unwrap().unwrap()),
            ["b"]
        );
    }

    #[test]
    fn move_and_reorder_changes_scope_and_persists_exact_order() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let folder = super::create_folder(&conn, "Project").unwrap();
        let a = super::create_tab(
            &conn,
            None,
            "A",
            0,
            r#"{"preset":"single","slots":["a"],"sizes":{}}"#,
        )
        .unwrap();
        let b = super::create_tab(
            &conn,
            None,
            "B",
            1,
            r#"{"preset":"single","slots":["b"],"sizes":{}}"#,
        )
        .unwrap();
        let c = super::create_tab(
            &conn,
            Some(&folder.id),
            "C",
            0,
            r#"{"preset":"single","slots":["c"],"sizes":{}}"#,
        )
        .unwrap();

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::move_and_reorder(&tx, &b.id, Some(&folder.id), &[b.id.clone(), c.id.clone()])
            .unwrap();
        tx.commit().unwrap();

        let rows = super::list(&conn).unwrap();
        let grouped: Vec<_> = rows
            .iter()
            .filter(|row| row.parent_id.as_deref() == Some(folder.id.as_str()))
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(grouped, [b.id.as_str(), c.id.as_str()]);
        assert_eq!(
            rows.iter()
                .filter(|row| row.parent_id.is_none() && row.node_type == NodeType::Tab)
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            [a.id.as_str()]
        );

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            super::move_and_reorder(&tx, &a.id, Some(&folder.id), std::slice::from_ref(&a.id),)
                .is_err()
        );
        drop(tx);
        assert!(super::get(&conn, &a.id)
            .unwrap()
            .unwrap()
            .parent_id
            .is_none());
    }

    #[test]
    fn move_into_and_out_of_project_writes_session_pointer_through() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        let project_node = super::ensure_project_node(&conn, &project.id).unwrap();
        insert_session(&conn, "s1", None);
        insert_session(&conn, "s2", None);
        let tab = super::create_tab(
            &conn,
            None,
            "pair",
            0,
            r#"{"preset":"cols-2","slots":["s1","s2"],"sizes":{}}"#,
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &tab.id,
            Some(&project_node.id),
            std::slice::from_ref(&tab.id),
        )
        .unwrap();
        tx.commit().unwrap();
        for id in ["s1", "s2"] {
            let project_id: Option<String> = conn
                .query_row("SELECT project_id FROM sessions WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
        }

        let root_ids: Vec<String> = {
            let rows = super::list(&conn).unwrap();
            rows.iter()
                .filter(|row| row.parent_id.is_none() && row.id != tab.id)
                .map(|row| row.id.clone())
                .chain(std::iter::once(tab.id.clone()))
                .collect()
        };
        let tx = conn.transaction().unwrap();
        super::move_and_reorder(&tx, &tab.id, None, &root_ids).unwrap();
        tx.commit().unwrap();
        for id in ["s1", "s2"] {
            let project_id: Option<String> = conn
                .query_row("SELECT project_id FROM sessions WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(project_id, None);
        }
    }

    #[test]
    fn moving_a_mission_across_a_project_boundary_writes_pointers_through() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        let project_node = super::ensure_project_node(&conn, &project.id).unwrap();
        insert_mission(&conn, "m1", None, false);
        conn.execute(
            "INSERT INTO sessions (id, mission_id, status) VALUES ('ms1', 'm1', 'stopped')",
            [],
        )
        .unwrap();
        let node = super::ensure_mission_node(&conn, "m1", None).unwrap();

        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &node.id,
            Some(&project_node.id),
            std::slice::from_ref(&node.id),
        )
        .unwrap();
        tx.commit().unwrap();

        let mission_project: Option<String> = conn
            .query_row("SELECT project_id FROM missions WHERE id = 'm1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(mission_project.as_deref(), Some(project.id.as_str()));
        let session_project: Option<String> = conn
            .query_row(
                "SELECT project_id FROM sessions WHERE id = 'ms1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(session_project.as_deref(), Some(project.id.as_str()));
    }

    #[test]
    fn nesting_stays_within_todays_shapes() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let outer = super::create_folder(&conn, "Outer").unwrap();
        let inner = super::create_folder(&conn, "Inner").unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        let project_node = super::ensure_project_node(&conn, &project.id).unwrap();
        insert_mission(&conn, "m1", None, false);
        let mission_node = super::ensure_mission_node(&conn, "m1", None).unwrap();

        // Folders can't nest under folders; projects can't nest at all.
        for (id, parent) in [
            (inner.id.as_str(), outer.id.as_str()),
            (project_node.id.as_str(), outer.id.as_str()),
        ] {
            let tx = conn.transaction().unwrap();
            assert!(
                super::move_and_reorder(&tx, id, Some(parent), &[id.to_owned()]).is_err(),
                "nesting {id} under {parent} must be refused"
            );
            drop(tx);
        }

        // Leaves go under either container: mission -> folder and
        // mission -> project are both allowed.
        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &mission_node.id,
            Some(&outer.id),
            std::slice::from_ref(&mission_node.id),
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            super::get(&conn, &mission_node.id)
                .unwrap()
                .unwrap()
                .parent_id
                .as_deref(),
            Some(outer.id.as_str())
        );
        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &mission_node.id,
            Some(&project_node.id),
            std::slice::from_ref(&mission_node.id),
        )
        .unwrap();
        tx.commit().unwrap();
        // ...and moving from the project back to a folder clears the
        // written-through project pointer.
        let tx = conn.transaction().unwrap();
        super::move_and_reorder(
            &tx,
            &mission_node.id,
            Some(&outer.id),
            std::slice::from_ref(&mission_node.id),
        )
        .unwrap();
        tx.commit().unwrap();
        let mission_project: Option<String> = conn
            .query_row("SELECT project_id FROM missions WHERE id = 'm1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(mission_project, None);
    }

    #[test]
    fn reconcile_tab_placement_follows_member_pointers() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        insert_session(&conn, "a", None);
        insert_session(&conn, "b", None);
        let tab = super::create_tab(
            &conn,
            None,
            "pair",
            0,
            r#"{"preset":"cols-2","slots":["a","b"],"sizes":{}}"#,
        )
        .unwrap();

        // Half the members carry the project -> the tab stays at root.
        conn.execute(
            "UPDATE sessions SET project_id = ?1 WHERE id = 'a'",
            [&project.id],
        )
        .unwrap();
        super::reconcile_tab_placement(&conn, &tab.id).unwrap();
        assert_eq!(super::get(&conn, &tab.id).unwrap().unwrap().parent_id, None);

        // Unanimous membership wins even over a folder placement.
        let folder = super::create_folder(&conn, "F").unwrap();
        super::reparent_append(&conn, &tab.id, Some(&folder.id)).unwrap();
        conn.execute(
            "UPDATE sessions SET project_id = ?1 WHERE id = 'b'",
            [&project.id],
        )
        .unwrap();
        super::reconcile_tab_placement(&conn, &tab.id).unwrap();
        let project_node = super::find_by_ref(&conn, NodeType::Project, &project.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            super::get(&conn, &tab.id)
                .unwrap()
                .unwrap()
                .parent_id
                .as_deref(),
            Some(project_node.id.as_str())
        );

        // Mixed again -> out of the project node, back to root.
        conn.execute("UPDATE sessions SET project_id = NULL WHERE id = 'a'", [])
            .unwrap();
        super::reconcile_tab_placement(&conn, &tab.id).unwrap();
        assert_eq!(super::get(&conn, &tab.id).unwrap().unwrap().parent_id, None);

        // A foldered tab with mixed membership keeps its folder.
        super::reparent_append(&conn, &tab.id, Some(&folder.id)).unwrap();
        super::reconcile_tab_placement(&conn, &tab.id).unwrap();
        assert_eq!(
            super::get(&conn, &tab.id)
                .unwrap()
                .unwrap()
                .parent_id
                .as_deref(),
            Some(folder.id.as_str())
        );
    }

    /// The finalization boundary of archive-all container deletes: a
    /// child that "arrived late" (missed by the pre-delete archive
    /// sweep, e.g. moved in by another window) must fail the guarded
    /// container delete via the self-FK's ON DELETE RESTRICT — never
    /// be silently detached or dropped.
    #[test]
    fn container_delete_is_blocked_by_remaining_children() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "A", "/tmp/a").unwrap();
        let project_node = super::ensure_project_node(&conn, &project.id).unwrap();
        insert_mission(&conn, "m1", Some(&project.id), false);
        let mission_node = super::ensure_mission_node(&conn, "m1", Some(&project.id)).unwrap();

        let tx = conn.transaction().unwrap();
        let archived = super::delete_container_tabs_and_archive(&tx, &project_node.id).unwrap();
        assert!(archived.is_empty());
        assert!(
            super::delete(&tx, &project_node.id).is_err(),
            "RESTRICT must block the project-node delete while a child remains"
        );
        drop(tx); // rollback

        assert!(super::get(&conn, &project_node.id).unwrap().is_some());
        assert_eq!(
            super::get(&conn, &mission_node.id)
                .unwrap()
                .unwrap()
                .parent_id
                .as_deref(),
            Some(project_node.id.as_str())
        );

        // Folder variant hits the same guard through
        // delete_folder_after_tabs.
        let folder = super::create_folder(&conn, "F").unwrap();
        super::reparent_append(&conn, &mission_node.id, Some(&folder.id)).unwrap();
        let tx = conn.transaction().unwrap();
        super::delete_container_tabs_and_archive(&tx, &folder.id).unwrap();
        assert!(super::delete_folder_after_tabs(&tx, &folder.id).is_err());
        drop(tx);
        assert!(super::get(&conn, &folder.id).unwrap().is_some());
    }

    #[test]
    fn pin_appends_and_unpin_returns_to_tree_position() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let a = super::create_tab(
            &conn,
            None,
            "A",
            0,
            r#"{"preset":"single","slots":["a"],"sizes":{}}"#,
        )
        .unwrap();
        let b = super::create_tab(
            &conn,
            None,
            "B",
            1,
            r#"{"preset":"single","slots":["b"],"sizes":{}}"#,
        )
        .unwrap();

        super::set_pinned(&conn, &b.id, true).unwrap();
        super::set_pinned(&conn, &a.id, true).unwrap();
        let pinned: Vec<(String, Option<i64>)> = super::list(&conn)
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.pinned_position))
            .collect();
        assert_eq!(
            pinned,
            [(b.id.clone(), Some(0)), (a.id.clone(), Some(1))],
            "pinned rows sort first by pin order"
        );

        // Re-pinning keeps the slot.
        super::set_pinned(&conn, &b.id, true).unwrap();
        assert_eq!(
            super::get(&conn, &b.id).unwrap().unwrap().pinned_position,
            Some(0)
        );

        super::set_pinned(&conn, &b.id, false).unwrap();
        let order: Vec<String> = super::list(&conn)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            order,
            [a.id.clone(), b.id.clone()],
            "unpinned row returns behind the remaining pinned row"
        );
        assert_eq!(
            super::get(&conn, &b.id).unwrap().unwrap().position,
            1,
            "tree position untouched by pin round-trip"
        );
    }
}

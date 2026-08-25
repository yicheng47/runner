use crate::error::{Error, Result};
use crate::repo;
use crate::repo::project::ProjectRow;
use crate::session::manager::{SessionEvents, SessionUpdatedEvent};
use crate::AppCore;

fn clean_value(value: String, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::msg(format!("project {label} cannot be empty")));
    }
    Ok(value.to_owned())
}

fn emit_changed(state: &AppCore) {
    state.events.emit("project/changed", &serde_json::json!({}));
}

pub fn list(conn: &rusqlite::Connection) -> Result<Vec<ProjectRow>> {
    Ok(repo::project::list(conn)?)
}

pub fn get(conn: &rusqlite::Connection, id: &str) -> Result<ProjectRow> {
    repo::project::get(conn, id)?.ok_or_else(|| Error::msg(format!("project not found: {id}")))
}

pub(crate) fn resolve_cwd(
    conn: &rusqlite::Connection,
    project_id: Option<&str>,
    cwd: Option<String>,
) -> Result<Option<String>> {
    let Some(project_id) = project_id else {
        return Ok(cwd);
    };
    let project = get(conn, project_id)?;
    Ok(cwd.or(Some(project.cwd)))
}

pub fn project_list(state: &AppCore) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    list(&conn)
}

pub fn project_create(state: &AppCore, name: String, cwd: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    let row = repo::project::create(
        &conn,
        &clean_value(name, "name")?,
        &clean_value(cwd, "cwd")?,
    )?;
    repo::node::ensure_project_node(&conn, &row.id)?;
    emit_changed(state);
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(row)
}

pub fn project_rename(state: &AppCore, id: String, name: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::rename(&conn, &id, &clean_value(name, "name")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(state);
    Ok(row)
}

pub fn project_set_cwd(state: &AppCore, id: String, cwd: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::set_cwd(&conn, &id, &clean_value(cwd, "cwd")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(state);
    Ok(row)
}

pub fn project_reorder(state: &AppCore, ordered_ids: Vec<String>) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    repo::project::reorder(&conn, &ordered_ids)?;
    let rows = repo::project::list(&conn)?;
    emit_changed(state);
    Ok(rows)
}

/// Delete a project after archiving member missions and chats and closing member
/// terminals. Missions archive first as complete self-consistent operations;
/// member tabs and the project node and row then change in one transaction. The
/// project row's ON DELETE SET NULL unbinds the archived rows' pointers, so
/// restored items come back unfiled. Returns archived chat ids for event fanout.
pub(crate) async fn project_delete_impl(state: &AppCore, id: &str) -> Result<Vec<String>> {
    let (project_node, children) = {
        let conn = state.db.get()?;
        if repo::project::get(&conn, id)?.is_none() {
            return Err(Error::msg(format!("project not found: {id}")));
        }
        let node = repo::node::find_by_ref(&conn, repo::node::NodeType::Project, id)?;
        let children = match node.as_ref() {
            Some(node) => crate::ops::node::container_children(&conn, &node.id)?,
            None => crate::ops::node::ContainerChildren {
                session_ids: Vec::new(),
                missions: Vec::new(),
            },
        };
        (node, children)
    };

    crate::ops::node::archive_child_missions(state, &children.missions).await?;
    crate::ops::node::kill_running_children(state, &children.session_ids)?;

    let (archived_ids, deleted_ids) = {
        let mut conn = state.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Tabs are re-queried inside the transaction (not the earlier
        // snapshot), so late arrivals archive too — or fail the guard
        // if their sessions are still running.
        let removed = match project_node.as_ref() {
            Some(node) => repo::node::delete_container_tabs_and_archive(&tx, &node.id)?,
            None => (Vec::new(), Vec::new()),
        };
        // A plain RESTRICT-guarded delete: a child that arrived during
        // the archive gap (another window moving a mission in) fails
        // the whole transaction loudly instead of being silently
        // reparented and unbound.
        if let Some(node) = project_node.as_ref() {
            repo::node::delete(&tx, &node.id)?;
        }
        if repo::project::delete(&tx, id)? == 0 {
            return Err(Error::msg(format!("project not found: {id}")));
        }
        tx.commit()?;
        removed
    };
    for session_id in archived_ids.iter().chain(&deleted_ids) {
        state.sessions.forget_session_state(session_id);
    }
    Ok(archived_ids)
}

pub async fn project_delete(state: &AppCore, id: String) -> Result<()> {
    let result = project_delete_impl(state, &id).await;
    // Mission archives commit one by one BEFORE the final transaction,
    // so even a failed delete may have durably archived children —
    // invalidate every consuming surface regardless of outcome.
    emit_changed(state);
    state.events.emit("mission/changed", &serde_json::json!({}));
    state.events.emit("session/updated", &serde_json::json!({}));
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    let archived_ids = result?;
    for session_id in &archived_ids {
        state.session_events().archived(&SessionUpdatedEvent {
            session_id: session_id.clone(),
            mission_id: None,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clean_value, resolve_cwd};
    use crate::{db, repo};

    #[test]
    fn clean_value_trims_and_rejects_blank() {
        assert_eq!(clean_value("  Runner  ".into(), "name").unwrap(), "Runner");
        assert!(clean_value("  ".into(), "cwd").is_err());
    }

    #[test]
    fn resolve_cwd_defaults_from_project_and_preserves_override() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();

        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), None).unwrap(),
            Some("/project".into())
        );
        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), Some("/override".into())).unwrap(),
            Some("/override".into())
        );
    }

    #[test]
    fn resolve_cwd_rejects_unknown_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();

        let error = resolve_cwd(&conn, Some("missing"), None).unwrap_err();

        assert_eq!(error.to_string(), "project not found: missing");
    }
}

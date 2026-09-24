use crate::error::{Error, Result};
use crate::model::{MissionStatus, SessionStatus};
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

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct LiveMembers {
    pub session_ids: Vec<String>,
    pub mission_ids: Vec<String>,
}

pub(crate) fn live_members(conn: &rusqlite::Connection, id: &str) -> Result<LiveMembers> {
    get(conn, id)?;
    let Some(node) = repo::node::find_by_ref(conn, repo::node::NodeType::Project, id)? else {
        return Ok(LiveMembers::default());
    };
    let children = crate::ops::node::container_children(conn, &node.id)?;
    let mut session_ids = Vec::new();
    for session_id in children.session_ids {
        if repo::session::get_row(conn, &session_id)?
            .is_some_and(|row| row.status == SessionStatus::Running)
        {
            session_ids.push(session_id);
        }
    }
    let mission_ids = children
        .missions
        .into_iter()
        .filter(|(_, status)| *status == MissionStatus::Running)
        .map(|(id, _)| id)
        .collect();
    Ok(LiveMembers {
        session_ids,
        mission_ids,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct ProjectDeleteOutcome {
    pub archived_mission_ids: Vec<String>,
    pub archived_session_ids: Vec<String>,
}

pub(crate) fn resolve_cwd(
    conn: &rusqlite::Connection,
    project_id: Option<&str>,
    cwd: Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    if let Some(project_id) = project_id {
        let project = get(conn, project_id)?;
        return Ok((Some(project.id), cwd.or(Some(project.cwd))));
    }
    let project_id = cwd
        .as_deref()
        .map(|cwd| repo::project::find_for_path(conn, cwd))
        .transpose()?
        .flatten()
        .map(|project| project.id);
    Ok((project_id, cwd))
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

/// Delete a project after archiving member missions and chats and closing member
/// terminals. Missions archive first as complete self-consistent operations;
/// member tabs and the project node and row then change in one transaction,
/// which unbinds the archived rows' pointers, so restored items come back
/// unfiled. Returns archived member ids for event fanout.
pub(crate) async fn project_delete_impl(state: &AppCore, id: &str) -> Result<ProjectDeleteOutcome> {
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
            Some(node) => super::node::delete_container_tabs_and_archive(&tx, &node.id)?,
            None => (Vec::new(), Vec::new()),
        };
        // Refused while the node has children: a child that arrived during
        // the archive gap (another window moving a mission in) fails
        // the whole transaction loudly instead of being silently
        // reparented and unbound.
        if let Some(node) = project_node.as_ref() {
            repo::node::delete(&tx, &node.id)?;
        }
        repo::session::unbind_project(&tx, id)?;
        repo::mission::unbind_project(&tx, id)?;
        if repo::project::delete(&tx, id)? == 0 {
            return Err(Error::msg(format!("project not found: {id}")));
        }
        tx.commit()?;
        removed
    };
    for session_id in archived_ids.iter().chain(&deleted_ids) {
        state.sessions.forget_session_state(session_id);
    }
    Ok(ProjectDeleteOutcome {
        archived_mission_ids: children.missions.into_iter().map(|(id, _)| id).collect(),
        archived_session_ids: archived_ids,
    })
}

pub async fn project_delete(state: &AppCore, id: String) -> Result<ProjectDeleteOutcome> {
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
    let outcome = result?;
    for session_id in &outcome.archived_session_ids {
        state.session_events().archived(&SessionUpdatedEvent {
            session_id: session_id.clone(),
            mission_id: None,
        });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::{clean_value, resolve_cwd};
    use crate::{db, repo};

    fn create_dir(root: &std::path::Path, relative: &str) -> String {
        let path = root.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn clean_value_trims_and_rejects_blank() {
        assert_eq!(clean_value("  Runner  ".into(), "name").unwrap(), "Runner");
        assert!(clean_value("  ".into(), "cwd").is_err());
    }

    #[test]
    fn resolve_cwd_defaults_from_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();

        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), None).unwrap(),
            (Some(project.id), Some("/project".into()))
        );
    }

    #[test]
    fn resolve_cwd_infers_an_exact_project_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cwd = create_dir(temp.path(), "runner");
        let project = repo::project::create(&conn, "Runner", &cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, None, Some(cwd.clone())).unwrap(),
            (Some(project.id), Some(cwd))
        );
    }

    #[test]
    fn resolve_cwd_infers_a_project_from_a_deep_descendant() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = create_dir(temp.path(), "runner");
        let cwd = create_dir(temp.path(), "runner/.worktrees/feat-680/src");
        let project = repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, None, Some(cwd.clone())).unwrap(),
            (Some(project.id), Some(cwd))
        );
    }

    #[test]
    fn resolve_cwd_picks_the_longest_project_ancestor() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let parent_cwd = create_dir(temp.path(), "yicheng47");
        let project_cwd = create_dir(temp.path(), "yicheng47/runner");
        let cwd = create_dir(temp.path(), "yicheng47/runner/.worktrees/feat-680");
        repo::project::create(&conn, "All repos", &parent_cwd).unwrap();
        let runner = repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, None, Some(cwd.clone())).unwrap(),
            (Some(runner.id), Some(cwd))
        );
    }

    #[test]
    fn resolve_cwd_does_not_match_a_string_prefix_sibling() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = create_dir(temp.path(), "yicheng47/runner");
        let cwd = create_dir(temp.path(), "yicheng47/runner-wt");
        repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, None, Some(cwd.clone())).unwrap(),
            (None, Some(cwd))
        );
    }

    #[test]
    fn resolve_cwd_lexical_fallback_uses_path_components() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = temp.path().join("missing/runner");
        let descendant = project_cwd.join(".worktrees/feat-680");
        let sibling = temp.path().join("missing/runner-wt");
        let project =
            repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref()).unwrap();

        assert_eq!(
            resolve_cwd(&conn, None, Some(descendant.to_string_lossy().into_owned()))
                .unwrap()
                .0,
            Some(project.id)
        );
        assert_eq!(
            resolve_cwd(&conn, None, Some(sibling.to_string_lossy().into_owned()))
                .unwrap()
                .0,
            None
        );
    }

    #[test]
    fn resolve_cwd_leaves_an_unbound_path_unfiled() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cwd = create_dir(temp.path(), "unbound");

        assert_eq!(
            resolve_cwd(&conn, None, Some(cwd.clone())).unwrap(),
            (None, Some(cwd))
        );
        assert_eq!(resolve_cwd(&conn, None, None).unwrap(), (None, None));
    }

    #[test]
    fn resolve_cwd_explicit_project_beats_an_inferable_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let explicit_cwd = create_dir(temp.path(), "explicit");
        let inferred_cwd = create_dir(temp.path(), "inferred");
        let cwd = create_dir(temp.path(), "inferred/worktree");
        let explicit = repo::project::create(&conn, "Explicit", &explicit_cwd).unwrap();
        repo::project::create(&conn, "Inferred", &inferred_cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, Some(&explicit.id), Some(cwd.clone())).unwrap(),
            (Some(explicit.id), Some(cwd))
        );
    }

    #[test]
    fn resolve_cwd_explicit_cwd_beats_the_projects_bound_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = create_dir(temp.path(), "project");
        let cwd = create_dir(temp.path(), "override");
        let project = repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), Some(cwd.clone())).unwrap(),
            (Some(project.id), Some(cwd))
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

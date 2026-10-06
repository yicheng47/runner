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

pub use runner_core::protocol::project::ProjectDeleteOutcome;

pub use runner_core::protocol::project::ProjectScope;

/// Decide the project and working directory of a new session or mission.
/// `Root` never consults the directory.
pub(crate) fn resolve_scope(
    conn: &rusqlite::Connection,
    scope: &ProjectScope,
    cwd: Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    match scope {
        ProjectScope::Infer => resolve_cwd(conn, None, cwd),
        ProjectScope::Root => Ok((None, cwd)),
        ProjectScope::Project(project_id) => resolve_cwd(conn, Some(project_id), cwd),
    }
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

pub fn project_create_checked(state: &AppCore, name: String, cwd: String) -> Result<ProjectRow> {
    let cwd = cwd.trim();
    if !std::path::Path::new(cwd).is_absolute()
        || !std::fs::metadata(cwd).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(Error::msg(
            "cwd must be an absolute path to an existing directory",
        ));
    }
    project_create(state, name, cwd.to_owned())
}

pub async fn project_delete_checked(
    state: &AppCore,
    id: String,
    force: bool,
) -> Result<ProjectDeleteOutcome> {
    if !force {
        let conn = state.db.get()?;
        let live = live_members(&conn, &id)?;
        if !live.session_ids.is_empty() || !live.mission_ids.is_empty() {
            return Err(Error::msg("project has running members"));
        }
    }
    project_delete(state, id).await
}

#[cfg(test)]
mod tests {
    use super::{clean_value, resolve_cwd};
    use super::{resolve_scope, ProjectScope};
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
    fn scope_constructors_read_a_missing_project_by_caller() {
        assert_eq!(ProjectScope::or_infer(None), ProjectScope::Infer);
        assert_eq!(ProjectScope::or_root(None), ProjectScope::Root);
        for scope in [
            ProjectScope::or_infer(Some("p".into())),
            ProjectScope::or_root(Some("p".into())),
        ] {
            assert_eq!(scope, ProjectScope::Project("p".into()));
        }
    }

    #[test]
    fn resolve_scope_root_never_infers_from_the_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = create_dir(temp.path(), "runner");
        let cwd = create_dir(temp.path(), "runner/.worktrees/fix-718");
        repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_scope(&conn, &ProjectScope::Root, Some(project_cwd.clone())).unwrap(),
            (None, Some(project_cwd))
        );
        assert_eq!(
            resolve_scope(&conn, &ProjectScope::Root, Some(cwd.clone())).unwrap(),
            (None, Some(cwd))
        );
        assert_eq!(
            resolve_scope(&conn, &ProjectScope::Root, None).unwrap(),
            (None, None)
        );
    }

    #[test]
    fn resolve_scope_infers_or_takes_the_named_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let project_cwd = create_dir(temp.path(), "runner");
        let cwd = create_dir(temp.path(), "runner/.worktrees/fix-718");
        let project = repo::project::create(&conn, "Runner", &project_cwd).unwrap();

        assert_eq!(
            resolve_scope(&conn, &ProjectScope::Infer, Some(cwd.clone())).unwrap(),
            (Some(project.id.clone()), Some(cwd))
        );
        assert_eq!(
            resolve_scope(&conn, &ProjectScope::Project(project.id.clone()), None).unwrap(),
            (Some(project.id), Some(project_cwd))
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

#[cfg(test)]
mod client_tests {
    use crate::ops::project;
    use crate::repo::node::NodeType;
    use crate::repo::project::ProjectRow;
    use crate::test_support::test_core;
    use crate::{db, repo, AppCore};
    fn result_json(result: impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(result).unwrap()
    }
    fn seed_members(state: &AppCore, session_status: &str, mission_status: &str) -> ProjectRow {
        let conn = state.db.get().unwrap();
        let project = repo::project::create(&conn, "Project", "/project").unwrap();
        let node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, project_id) VALUES ('chat', ?1, ?2)",
            rusqlite::params![session_status, project.id],
        )
        .unwrap();
        repo::node::create_tab(
            &conn,
            Some(&node.id),
            "Chat",
            0,
            r#"{"preset":"single","slots":["chat"],"sizes":{}}"#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at, project_id)
             VALUES ('mission', 'crew', 'Mission', ?1, '2026-09-10T00:00:00Z', ?2)",
            rusqlite::params![mission_status, project.id],
        )
        .unwrap();
        repo::node::ensure_mission_node(&conn, "mission", Some(&project.id)).unwrap();
        project
    }

    #[tokio::test]
    async fn project_create_validates_cwd_and_appends_project_and_node() {
        let handler = test_core();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "file").unwrap();
        for cwd in [
            "relative".to_owned(),
            dir.path().join("missing").to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            "  ".to_owned(),
        ] {
            let error =
                project::project_create_checked(&handler, "Project".into(), cwd).unwrap_err();

            assert_eq!(
                error.to_string(),
                "cwd must be an absolute path to an existing directory"
            );
        }
        let previous = {
            let conn = handler.db.get().unwrap();
            assert!(repo::project::list(&conn).unwrap().is_empty());
            assert!(repo::node::list(&conn).unwrap().is_empty());
            let project = repo::project::create(&conn, "Previous", "/previous").unwrap();
            conn.execute(
                "UPDATE projects SET position = 7 WHERE id = ?1",
                [&project.id],
            )
            .unwrap();
            repo::node::ensure_project_node(&conn, &project.id).unwrap()
        };
        let error = project::project_create_checked(
            &handler,
            "  ".into(),
            dir.path().to_string_lossy().into_owned(),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "project name cannot be empty");

        let mut events = handler.events.subscribe();
        let created = result_json(
            project::project_create_checked(
                &handler,
                "  New project  ".into(),
                format!("  {}  ", dir.path().display()),
            )
            .unwrap(),
        );
        let conn = handler.db.get().unwrap();
        let row = repo::project::get(&conn, created["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(created, serde_json::json!(row));
        assert_eq!(row.name, "New project");
        assert_eq!(row.cwd, dir.path().to_string_lossy());
        assert_eq!(row.position, 8);
        let node = repo::node::find_by_ref(&conn, NodeType::Project, &row.id)
            .unwrap()
            .unwrap();
        assert_eq!(node.position, previous.position + 1);
        assert_eq!(events.try_recv().unwrap().name, "project/changed");
        assert_eq!(events.try_recv().unwrap().name, "chat/layout-changed");
    }

    #[tokio::test]
    async fn project_rename_rejects_unknown_and_returns_updated_name() {
        let handler = test_core();
        let error =
            project::project_rename(&handler, "missing".into(), "Renamed".into()).unwrap_err();

        assert_eq!(error.to_string(), "project not found: missing");
        let row = project::project_create(&handler, "Old".into(), "/project".into()).unwrap();
        let mut events = handler.events.subscribe();
        let renamed = result_json(
            project::project_rename(&handler, row.id.clone(), "  Renamed  ".into()).unwrap(),
        );
        assert_eq!(renamed["id"], row.id);
        assert_eq!(renamed["name"], "Renamed");
        assert_eq!(events.try_recv().unwrap().name, "project/changed");
    }

    #[tokio::test]
    async fn project_delete_refuses_running_members_without_mutations() {
        for (session_status, mission_status) in [
            ("running", "aborted"),
            ("stopped", "running"),
            ("running", "running"),
        ] {
            let handler = test_core();
            let project = seed_members(&handler, session_status, mission_status);
            let mut events = handler.events.subscribe();
            let error = project::project_delete_checked(&handler, project.id.clone(), false)
                .await
                .unwrap_err();

            assert_eq!(error.to_string(), "project has running members");
            assert_eq!(
                serde_json::to_value(
                    project::live_members(&handler.db.get().unwrap(), &project.id).unwrap()
                )
                .unwrap(),
                serde_json::json!({
                    "session_ids": if session_status == "running" { vec!["chat"] } else { vec![] },
                    "mission_ids": if mission_status == "running" { vec!["mission"] } else { vec![] },
                })
            );
            let conn = handler.db.get().unwrap();
            assert!(repo::project::get(&conn, &project.id).unwrap().is_some());
            assert_eq!(repo::node::list(&conn).unwrap().len(), 3);
            let session = repo::session::get_row(&conn, "chat").unwrap().unwrap();
            assert!(session.archived_at.is_none());
            assert_eq!(
                session.status,
                if session_status == "running" {
                    crate::model::SessionStatus::Running
                } else {
                    crate::model::SessionStatus::Stopped
                }
            );
            let mission = repo::mission::get(&conn, "mission").unwrap().unwrap();
            assert!(mission.archived_at.is_none());
            assert_eq!(
                mission.status,
                if mission_status == "running" {
                    crate::model::MissionStatus::Running
                } else {
                    crate::model::MissionStatus::Aborted
                }
            );
            assert!(events.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn project_delete_archives_stopped_members_and_returns_both_lists() {
        let handler = test_core();
        let project = seed_members(&handler, "stopped", "aborted");
        let mut events = handler.events.subscribe();
        let outcome = result_json(
            project::project_delete_checked(&handler, project.id.clone(), false)
                .await
                .unwrap(),
        );
        assert_eq!(
            outcome,
            serde_json::json!({
                "archived_session_ids": ["chat"], "archived_mission_ids": ["mission"],
            })
        );
        let conn = handler.db.get().unwrap();
        assert!(repo::project::get(&conn, &project.id).unwrap().is_none());
        assert!(repo::node::list(&conn).unwrap().is_empty());
        let session = repo::session::get_row(&conn, "chat").unwrap().unwrap();
        assert!(session.archived_at.is_some());
        assert!(session.project_id.is_none());
        let mission = repo::mission::get(&conn, "mission").unwrap().unwrap();
        assert!(mission.archived_at.is_some());
        assert!(mission.project_id.is_none());
        let mut names = Vec::new();
        while let Ok(event) = events.try_recv() {
            names.push(event.name);
        }
        for expected in [
            "project/changed",
            "mission/changed",
            "session/updated",
            "chat/layout-changed",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[tokio::test]
    async fn project_delete_force_bypasses_guard_and_reaches_running_row_check() {
        let handler = test_core();
        let project = seed_members(&handler, "running", "aborted");
        let error = project::project_delete_checked(&handler, project.id.clone(), true)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("session chat is missing or still running"),
            "{error}"
        );
        let conn = handler.db.get().unwrap();
        assert!(repo::project::get(&conn, &project.id).unwrap().is_some());
    }

    #[tokio::test]
    async fn project_delete_rejects_unknown_id_with_or_without_force() {
        let handler = test_core();
        for force in [false, true] {
            let error = project::project_delete_checked(&handler, "missing".into(), force)
                .await
                .unwrap_err();

            assert_eq!(error.to_string(), "project not found: missing");
        }
    }

    #[test]
    fn project_discovery_lists_and_gets_bound_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let created = repo::project::create(&conn, "Runner", "/runner").unwrap();

        let listed = project::list(&conn).unwrap();
        let fetched = project::get(&conn, &created.id).unwrap();

        assert_eq!(listed, vec![created.clone()]);
        assert_eq!(fetched, created);
    }
}

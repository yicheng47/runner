use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::Error;
use crate::mcp::server::RunnerMcpHandler;
use crate::ops::project;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectIdArgs {
    /// Project ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectCreateArgs {
    /// Project name. Trimmed and must not be empty.
    pub name: String,
    /// Absolute path to an existing directory.
    pub cwd: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectRenameArgs {
    /// Project ID.
    pub id: String,
    /// New project name. Trimmed and must not be empty.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectDeleteArgs {
    /// Project ID.
    pub id: String,
    /// Stop running members before deleting. Defaults to false.
    #[serde(default)]
    pub force: bool,
}

fn command_error(error: Error) -> ErrorData {
    match error {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[tool_router(router = project_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(
        description = "Create a project bound to an existing directory, appended in sidebar order."
    )]
    pub async fn project_create(
        &self,
        Parameters(ProjectCreateArgs { name, cwd }): Parameters<ProjectCreateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let cwd = cwd.trim();
        if !std::path::Path::new(cwd).is_absolute()
            || !std::fs::metadata(cwd).is_ok_and(|metadata| metadata.is_dir())
        {
            return Err(ErrorData::invalid_request(
                "cwd must be an absolute path to an existing directory",
                None,
            ));
        }
        let project =
            project::project_create(&self.state, name, cwd.to_owned()).map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&project)?]))
    }

    #[tool(description = "Rename a project.")]
    pub async fn project_rename(
        &self,
        Parameters(ProjectRenameArgs { id, name }): Parameters<ProjectRenameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = project::project_rename(&self.state, id, name).map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&project)?]))
    }

    #[tool(
        description = "Delete a project and archive its members. Refuses running members unless force is true. Returns archived mission and chat session IDs."
    )]
    pub async fn project_delete(
        &self,
        Parameters(ProjectDeleteArgs { id, force }): Parameters<ProjectDeleteArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if !force {
            let conn = self
                .state
                .db
                .get()
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            let live = project::live_members(&conn, &id).map_err(command_error)?;
            if !live.session_ids.is_empty() || !live.mission_ids.is_empty() {
                return Err(ErrorData::invalid_request(
                    "project has running members",
                    Some(serde_json::json!(live)),
                ));
            }
        }
        let outcome = project::project_delete(&self.state, id)
            .await
            .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&outcome)?]))
    }

    #[tool(description = "List all projects in sidebar order, including their bound cwd.")]
    pub async fn project_list(&self) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let projects = project::list(&conn).map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&projects)?]))
    }

    #[tool(description = "Get a project by ID, including its bound cwd.")]
    pub async fn project_get(
        &self,
        Parameters(ProjectIdArgs { id }): Parameters<ProjectIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let project = project::get(&conn, &id).map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&project)?]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::node::NodeType;
    use crate::repo::project::ProjectRow;
    use crate::test_support::test_core;
    use crate::{db, repo, AppCore};

    fn result_json(result: CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
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
        let handler = RunnerMcpHandler::new(test_core());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "file").unwrap();
        for cwd in [
            "relative".to_owned(),
            dir.path().join("missing").to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            "  ".to_owned(),
        ] {
            let error = handler
                .project_create(Parameters(ProjectCreateArgs {
                    name: "Project".into(),
                    cwd,
                }))
                .await
                .unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
            assert_eq!(
                error.message,
                "cwd must be an absolute path to an existing directory"
            );
        }
        let previous = {
            let conn = handler.state.db.get().unwrap();
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
        let error = handler
            .project_create(Parameters(ProjectCreateArgs {
                name: "  ".into(),
                cwd: dir.path().to_string_lossy().into_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "project name cannot be empty");

        let mut events = handler.state.events.subscribe();
        let created = result_json(
            handler
                .project_create(Parameters(ProjectCreateArgs {
                    name: "  New project  ".into(),
                    cwd: format!("  {}  ", dir.path().display()),
                }))
                .await
                .unwrap(),
        );
        let conn = handler.state.db.get().unwrap();
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
        let handler = RunnerMcpHandler::new(test_core());
        let error = handler
            .project_rename(Parameters(ProjectRenameArgs {
                id: "missing".into(),
                name: "Renamed".into(),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "project not found: missing");
        let row = project::project_create(&handler.state, "Old".into(), "/project".into()).unwrap();
        let mut events = handler.state.events.subscribe();
        let renamed = result_json(
            handler
                .project_rename(Parameters(ProjectRenameArgs {
                    id: row.id.clone(),
                    name: "  Renamed  ".into(),
                }))
                .await
                .unwrap(),
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
            let handler = RunnerMcpHandler::new(test_core());
            let project = seed_members(&handler.state, session_status, mission_status);
            let mut events = handler.state.events.subscribe();
            let args = serde_json::from_value(serde_json::json!({ "id": project.id })).unwrap();
            let error = handler.project_delete(Parameters(args)).await.unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
            assert_eq!(error.message, "project has running members");
            assert_eq!(
                error.data.unwrap(),
                serde_json::json!({
                    "session_ids": if session_status == "running" { vec!["chat"] } else { vec![] },
                    "mission_ids": if mission_status == "running" { vec!["mission"] } else { vec![] },
                })
            );
            let conn = handler.state.db.get().unwrap();
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
        let handler = RunnerMcpHandler::new(test_core());
        let project = seed_members(&handler.state, "stopped", "aborted");
        let mut events = handler.state.events.subscribe();
        let outcome = result_json(
            handler
                .project_delete(Parameters(ProjectDeleteArgs {
                    id: project.id.clone(),
                    force: false,
                }))
                .await
                .unwrap(),
        );
        assert_eq!(
            outcome,
            serde_json::json!({
                "archived_session_ids": ["chat"], "archived_mission_ids": ["mission"],
            })
        );
        let conn = handler.state.db.get().unwrap();
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
        let handler = RunnerMcpHandler::new(test_core());
        let project = seed_members(&handler.state, "running", "aborted");
        let error = handler
            .project_delete(Parameters(ProjectDeleteArgs {
                id: project.id.clone(),
                force: true,
            }))
            .await
            .unwrap_err();
        assert!(
            error
                .message
                .contains("session chat is missing or still running"),
            "{error}"
        );
        let conn = handler.state.db.get().unwrap();
        assert!(repo::project::get(&conn, &project.id).unwrap().is_some());
    }

    #[tokio::test]
    async fn project_delete_rejects_unknown_id_with_or_without_force() {
        let handler = RunnerMcpHandler::new(test_core());
        for force in [false, true] {
            let error = handler
                .project_delete(Parameters(ProjectDeleteArgs {
                    id: "missing".into(),
                    force,
                }))
                .await
                .unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
            assert_eq!(error.message, "project not found: missing");
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

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use rusqlite::{params, Connection};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::Emitter;

use crate::commands::crew;
use crate::mcp::server::RunnerMcpHandler;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CrewIdArgs {
    /// Crew ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCrewArgs {
    /// Crew ID.
    pub id: String,
    /// Fields to update. Omitted fields are preserved.
    pub input: crew::UpdateCrewInput,
}

fn unarchived_mission_session_ids_for_crew(
    conn: &Connection,
    crew_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT m.id
           FROM missions m
           JOIN sessions s ON s.mission_id = m.id
          WHERE m.crew_id = ?1
            AND s.archived_at IS NULL
          ORDER BY m.started_at ASC",
    )?;
    let ids = stmt
        .query_map(params![crew_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

#[tool_router(router = crew_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List all crews, including runner counts and member previews.")]
    pub async fn crew_list(&self) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let crews =
            crew::list(&conn).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&crews)?]))
    }

    #[tool(description = "Get a crew by ID.")]
    pub async fn crew_get(
        &self,
        Parameters(CrewIdArgs { id }): Parameters<CrewIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let crew =
            crew::get(&conn, &id).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&crew)?]))
    }

    #[tool(description = "Create a new crew.")]
    pub async fn crew_create(
        &self,
        Parameters(input): Parameters<crew::CreateCrewInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let crew = crew::create(&conn, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.app_handle.emit("crew/changed", ()).ok();
        Ok(CallToolResult::success(vec![Content::json(&crew)?]))
    }

    #[tool(description = "Update a crew by ID. Omitted fields are preserved.")]
    pub async fn crew_update(
        &self,
        Parameters(UpdateCrewArgs { id, input }): Parameters<UpdateCrewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let crew = crew::update(&conn, &id, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.app_handle.emit("crew/changed", ()).ok();
        Ok(CallToolResult::success(vec![Content::json(&crew)?]))
    }

    #[tool(description = "Delete a crew by ID. Slot rows are removed; runner templates are kept.")]
    pub async fn crew_delete(
        &self,
        Parameters(CrewIdArgs { id }): Parameters<CrewIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let affected_missions = unarchived_mission_session_ids_for_crew(&conn, &id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        if !affected_missions.is_empty() {
            return Err(ErrorData::invalid_request(
                format!(
                    "crew {id} has unarchived mission sessions; archive those sessions first: {}",
                    affected_missions.join(", ")
                ),
                None,
            ));
        }
        crew::delete(&conn, &id).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.app_handle.emit("crew/changed", ()).ok();
        self.state.app_handle.emit("slot/changed", ()).ok();
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({ "deleted": true, "id": id }),
        )?]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn unarchived_mission_session_ids_for_crew_blocks_stopped_and_running_sessions() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = "2026-06-19T00:00:00Z";
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-live', 'Live', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-other', 'Other', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES ('runner-1', 'runner1', 'Runner 1', 'shell', 'sh', ?1, ?1)",
            params![now],
        )
        .unwrap();
        for (mission_id, crew_id) in [
            ("mission-live", "crew-live"),
            ("mission-stopped", "crew-live"),
            ("mission-archived-session", "crew-live"),
            ("mission-other", "crew-other"),
        ] {
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES (?1, ?2, ?1, 'running', ?3)",
                params![mission_id, crew_id, now],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at)
             VALUES ('session-live', 'mission-live', 'runner-1', 'running', ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at)
             VALUES ('session-stopped', 'mission-stopped', 'runner-1', 'stopped', ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at, archived_at)
             VALUES ('session-archived', 'mission-archived-session', 'runner-1', 'stopped', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, status, started_at)
             VALUES ('session-other', 'mission-other', 'runner-1', 'running', ?1)",
            params![now],
        )
        .unwrap();

        let ids = unarchived_mission_session_ids_for_crew(&conn, "crew-live").unwrap();
        assert_eq!(
            ids,
            vec!["mission-live".to_string(), "mission-stopped".to_string()]
        );
    }
}

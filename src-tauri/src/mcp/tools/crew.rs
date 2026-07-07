use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::Emitter;

use crate::commands::crew;
use crate::error::Error;
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

fn command_error(e: Error) -> ErrorData {
    match e {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
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
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        crew::delete(&mut conn, &id).map_err(command_error)?;
        self.state.app_handle.emit("crew/changed", ()).ok();
        self.state.app_handle.emit("slot/changed", ()).ok();
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({ "deleted": true, "id": id }),
        )?]))
    }
}

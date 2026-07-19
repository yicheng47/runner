use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::Error;
use crate::mcp::server::RunnerMcpHandler;
use crate::ops::runner;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunnerIdArgs {
    /// Runner ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunnerHandleArgs {
    /// Runner handle without the leading @.
    pub handle: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRunnerArgs {
    /// Runner ID.
    pub id: String,
    /// Fields to update. Omitted fields are preserved.
    pub input: runner::UpdateRunnerInput,
}

fn command_error(e: Error) -> ErrorData {
    match e {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[tool_router(router = runner_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List all runner templates.")]
    pub async fn runner_list(&self) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let runners =
            runner::list(&conn).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&runners)?]))
    }

    #[tool(description = "Get a runner template by ID.")]
    pub async fn runner_get(
        &self,
        Parameters(RunnerIdArgs { id }): Parameters<RunnerIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let runner =
            runner::get(&conn, &id).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&runner)?]))
    }

    #[tool(description = "Get a runner template by handle.")]
    pub async fn runner_get_by_handle(
        &self,
        Parameters(RunnerHandleArgs { handle }): Parameters<RunnerHandleArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let runner = runner::get_by_handle(&conn, &handle)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&runner)?]))
    }

    #[tool(description = "Create a new runner template.")]
    pub async fn runner_create(
        &self,
        Parameters(input): Parameters<runner::CreateRunnerInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let runner = runner::create(&conn, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("runner/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&runner)?]))
    }

    #[tool(description = "Update a runner template by ID. Omitted fields are preserved.")]
    pub async fn runner_update(
        &self,
        Parameters(UpdateRunnerArgs { id, input }): Parameters<UpdateRunnerArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let runner = runner::update(&conn, &id, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("runner/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&runner)?]))
    }

    #[tool(
        description = "Delete a runner template by ID. Live sessions for that runner are killed first."
    )]
    pub async fn runner_delete(
        &self,
        Parameters(RunnerIdArgs { id }): Parameters<RunnerIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        {
            let conn = self
                .state
                .db
                .get()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            runner::ensure_delete_allowed(&conn, &id).map_err(command_error)?;
        }
        self.state
            .sessions
            .kill_all_for_runner(&id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        runner::delete(&mut conn, &id).map_err(command_error)?;
        self.state.events.emit("runner/changed", &());
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({ "deleted": true, "id": id }),
        )?]))
    }
}

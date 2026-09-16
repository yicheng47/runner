use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::Error;
use crate::mcp::server::RunnerMcpHandler;
use crate::ops::role;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoleIdArgs {
    /// Role ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoleHandleArgs {
    /// Role handle without the leading @.
    pub handle: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRoleArgs {
    /// Role ID.
    pub id: String,
    /// Fields to update. Omitted fields are preserved.
    pub input: role::UpdateRoleInput,
}

fn command_error(e: Error) -> ErrorData {
    match e {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[tool_router(router = role_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List all saved configurations used as a crew role.")]
    pub async fn role_list(&self) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let roles =
            role::list(&conn).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&roles)?]))
    }

    #[tool(description = "Get a crew role by ID.")]
    pub async fn role_get(
        &self,
        Parameters(RoleIdArgs { id }): Parameters<RoleIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let role =
            role::get(&conn, &id).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&role)?]))
    }

    #[tool(description = "Get a crew role by handle.")]
    pub async fn role_get_by_handle(
        &self,
        Parameters(RoleHandleArgs { handle }): Parameters<RoleHandleArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let role = role::get_by_handle(&conn, &handle)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&role)?]))
    }

    #[tool(description = "Create a crew role.")]
    pub async fn role_create(
        &self,
        Parameters(input): Parameters<role::CreateRoleInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let role = role::create(&conn, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("role/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&role)?]))
    }

    #[tool(description = "Update a crew role by ID. Omitted fields are preserved.")]
    pub async fn role_update(
        &self,
        Parameters(UpdateRoleArgs { id, input }): Parameters<UpdateRoleArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let role = role::update(&conn, &id, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("role/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&role)?]))
    }

    #[tool(description = "Delete a crew role by ID. Live sessions for that role are killed first.")]
    pub async fn role_delete(
        &self,
        Parameters(RoleIdArgs { id }): Parameters<RoleIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        {
            let conn = self
                .state
                .db
                .get()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            role::ensure_delete_allowed(&conn, &id).map_err(command_error)?;
        }
        self.state
            .sessions
            .kill_all_for_role(&id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        role::delete(&mut conn, &id).map_err(command_error)?;
        self.state.events.emit("role/changed", &());
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({ "deleted": true, "id": id }),
        )?]))
    }
}

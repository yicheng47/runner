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

fn command_error(error: Error) -> ErrorData {
    match error {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[tool_router(router = project_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
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
    use crate::{db, ops::project, repo};

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

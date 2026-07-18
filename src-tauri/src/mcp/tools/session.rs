use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::commands::session;
use crate::error::Error;
use crate::mcp::server::RunnerMcpHandler;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartDirectSessionArgs {
    /// Runner template ID.
    pub runner_id: String,
    /// Optional runtime override (registry name, e.g. "codex" or
    /// "claude-code"). Omit to use the runner's own runtime. When it
    /// differs, the chat spawns that engine with registry defaults
    /// while the runner's persona (system prompt, working dir, env)
    /// carries over.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Optional project membership. Its cwd is used when cwd is omitted.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Optional working-directory override.
    #[serde(default)]
    pub cwd: Option<String>,
}

fn command_error(error: Error) -> ErrorData {
    match error {
        Error::Msg(message) => ErrorData::invalid_request(message, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[tool_router(router = session_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(
        description = "Start a direct chat for a runner. A project's cwd is used unless cwd is explicitly provided."
    )]
    pub async fn session_start_direct(
        &self,
        Parameters(args): Parameters<StartDirectSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let output = session::session_start_direct_impl(
            &app_state,
            &self.state.app_handle,
            args.runner_id,
            args.runtime,
            args.project_id,
            args.cwd,
            None,
            None,
        )
        .map_err(command_error)?;
        Ok(CallToolResult::success(vec![Content::json(&output)?]))
    }
}

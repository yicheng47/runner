use tauri::State;

use runner_app::error::Result;
use runner_app::ops::mcp::{self, McpConfigSnippet, McpIntegrationStatus};

use crate::AppState;

#[tauri::command]
pub async fn mcp_integration_status(state: State<'_, AppState>) -> Result<McpIntegrationStatus> {
    mcp::mcp_integration_status(&state)
}

#[tauri::command]
pub async fn mcp_set_integration(
    state: State<'_, AppState>,
    client: String,
    enabled: bool,
) -> Result<()> {
    mcp::mcp_set_integration(&state, &client, enabled)
}

#[tauri::command]
pub async fn mcp_config_snippet(state: State<'_, AppState>) -> Result<McpConfigSnippet> {
    mcp::mcp_config_snippet(&state)
}

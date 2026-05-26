use serde::Serialize;
use tauri::State;

use crate::error::Result;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct McpConfigSnippet {
    pub claude_code: String,
    pub codex: String,
}

#[tauri::command]
pub async fn mcp_config_snippet(state: State<'_, AppState>) -> Result<McpConfigSnippet> {
    let runner_bin = state
        .app_data_dir
        .join("bin")
        .join("runner")
        .to_string_lossy()
        .to_string();

    let claude_code = serde_json::json!({
        "mcpServers": {
            "runner": {
                "type": "stdio",
                "command": runner_bin,
                "args": ["mcp"]
            }
        }
    });

    let codex = format!("[mcp_servers.runner]\ncommand = \"{runner_bin}\"\nargs = [\"mcp\"]\n");

    Ok(McpConfigSnippet {
        claude_code: serde_json::to_string_pretty(&claude_code).unwrap_or_default(),
        codex,
    })
}

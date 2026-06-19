use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::error::{Error, Result};
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct McpConfigSnippet {
    pub claude_code: String,
    pub codex: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpIntegrationStatus {
    pub environment: String,
    pub binary_path: String,
    pub socket_path: String,
    pub claude_code: McpClientStatus,
    pub codex: McpClientStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpClientStatus {
    pub registered: bool,
    pub matches_current: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub config_path: String,
    pub error: Option<String>,
}

impl McpClientStatus {
    fn empty(path: &Path) -> Self {
        Self {
            registered: false,
            matches_current: false,
            command: None,
            args: Vec::new(),
            config_path: path.to_string_lossy().to_string(),
            error: None,
        }
    }

    fn error(path: &Path, error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::empty(path)
        }
    }
}

enum Client {
    ClaudeCode,
    Codex,
}

impl Client {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "claude_code" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            other => Err(Error::msg(format!(
                "unknown MCP client: {other:?} (expected claude_code or codex)"
            ))),
        }
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::msg("HOME env var not set"))
}

fn claude_code_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude.json"))
}

fn codex_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn mcp_binary_path(state: &AppState) -> String {
    state
        .app_data_dir
        .join("bin")
        .join(crate::cli_install::MCP_DEST_BIN_NAME)
        .to_string_lossy()
        .to_string()
}

fn socket_path(state: &AppState) -> String {
    state
        .app_data_dir
        .join("mcp.sock")
        .to_string_lossy()
        .to_string()
}

fn environment_label() -> String {
    if cfg!(debug_assertions) {
        "Development".to_string()
    } else {
        "Production".to_string()
    }
}

fn args_match_current(args: &[String]) -> bool {
    args.is_empty()
}

fn claude_code_entry(binary_path: &str) -> serde_json::Value {
    json!({
        "type": "stdio",
        "command": binary_path
    })
}

fn json_args(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn claude_code_status_at(path: &Path, binary_path: &str) -> Result<McpClientStatus> {
    if !path.exists() {
        return Ok(McpClientStatus::empty(path));
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::msg(format!("read {}: {e}", path.display())))?;
    if raw.trim().is_empty() {
        return Ok(McpClientStatus::empty(path));
    }
    let val: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?;
    let entry = val
        .get("mcpServers")
        .and_then(|servers| servers.get("runner"));
    let Some(entry) = entry else {
        return Ok(McpClientStatus::empty(path));
    };
    let command = entry
        .get("command")
        .and_then(|command| command.as_str())
        .map(ToOwned::to_owned);
    let args = json_args(entry.get("args"));
    let matches_current = command.as_deref() == Some(binary_path) && args_match_current(&args);
    Ok(McpClientStatus {
        registered: true,
        matches_current,
        command,
        args,
        config_path: path.to_string_lossy().to_string(),
        error: None,
    })
}

pub(crate) fn claude_code_write_at(path: &Path, enabled: bool, binary_path: &str) -> Result<()> {
    let mut val: serde_json::Value = if path.exists() {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::msg(format!("read {}: {e}", path.display())))?;
        if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&raw)
                .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?
        }
    } else {
        json!({})
    };

    let obj = val.as_object_mut().ok_or_else(|| {
        Error::msg(format!(
            "{} is not a JSON object at top level",
            path.display()
        ))
    })?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            Error::msg(format!(
                "{}::mcpServers is not a JSON object",
                path.display()
            ))
        })?;

    if enabled {
        servers.insert("runner".to_string(), claude_code_entry(binary_path));
    } else {
        servers.remove("runner");
    }

    let mut out = serde_json::to_string_pretty(&val)
        .map_err(|e| Error::msg(format!("serialize {}: {e}", path.display())))?;
    out.push('\n');
    std::fs::write(path, out).map_err(|e| Error::msg(format!("write {}: {e}", path.display())))?;
    Ok(())
}

fn toml_args(item: Option<&toml_edit::Item>) -> Vec<String> {
    item.and_then(|item| item.as_array())
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn codex_status_at(path: &Path, binary_path: &str) -> Result<McpClientStatus> {
    if !path.exists() {
        return Ok(McpClientStatus::empty(path));
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::msg(format!("read {}: {e}", path.display())))?;
    let doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?;
    let entry = doc
        .get("mcp_servers")
        .and_then(|item| item.as_table())
        .and_then(|servers| servers.get("runner"));
    let Some(entry) = entry else {
        return Ok(McpClientStatus::empty(path));
    };
    let entry_table = entry
        .as_table()
        .ok_or_else(|| Error::msg("mcp_servers.runner is not a table"))?;
    let command = entry_table
        .get("command")
        .and_then(|command| command.as_str())
        .map(ToOwned::to_owned);
    let args = toml_args(entry_table.get("args"));
    let matches_current = command.as_deref() == Some(binary_path) && args_match_current(&args);
    Ok(McpClientStatus {
        registered: true,
        matches_current,
        command,
        args,
        config_path: path.to_string_lossy().to_string(),
        error: None,
    })
}

pub(crate) fn codex_write_at(path: &Path, enabled: bool, binary_path: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::msg(format!("mkdir {}: {e}", parent.display())))?;
    }

    let mut doc: toml_edit::DocumentMut = if path.exists() {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::msg(format!("read {}: {e}", path.display())))?;
        raw.parse()
            .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?
    } else {
        toml_edit::DocumentMut::new()
    };

    if enabled {
        match doc.get("mcp_servers") {
            Some(item) if !item.is_table() => {
                return Err(Error::msg("mcp_servers is not a table"));
            }
            Some(_) => {}
            None => {
                doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
        }
        let servers = doc["mcp_servers"]
            .as_table_mut()
            .ok_or_else(|| Error::msg("mcp_servers is not a table"))?;
        let mut entry = toml_edit::Table::new();
        entry["command"] = toml_edit::value(binary_path);
        servers["runner"] = toml_edit::Item::Table(entry);
    } else if let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    {
        servers.remove("runner");
    }

    std::fs::write(path, doc.to_string())
        .map_err(|e| Error::msg(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[tauri::command]
pub async fn mcp_integration_status(state: State<'_, AppState>) -> Result<McpIntegrationStatus> {
    let binary_path = mcp_binary_path(&state);
    let claude_code_path = claude_code_path()?;
    let codex_path = codex_path()?;
    let claude_code = claude_code_status_at(&claude_code_path, &binary_path)
        .unwrap_or_else(|e| McpClientStatus::error(&claude_code_path, e.to_string()));
    let codex = codex_status_at(&codex_path, &binary_path)
        .unwrap_or_else(|e| McpClientStatus::error(&codex_path, e.to_string()));
    Ok(McpIntegrationStatus {
        environment: environment_label(),
        socket_path: socket_path(&state),
        binary_path,
        claude_code,
        codex,
    })
}

#[tauri::command]
pub async fn mcp_set_integration(
    state: State<'_, AppState>,
    client: String,
    enabled: bool,
) -> Result<()> {
    let binary_path = mcp_binary_path(&state);
    match Client::parse(&client)? {
        Client::ClaudeCode => claude_code_write_at(&claude_code_path()?, enabled, &binary_path),
        Client::Codex => codex_write_at(&codex_path()?, enabled, &binary_path),
    }
}

#[tauri::command]
pub async fn mcp_config_snippet(state: State<'_, AppState>) -> Result<McpConfigSnippet> {
    let runner_bin = mcp_binary_path(&state);

    let claude_code = json!({
        "mcpServers": {
            "runner": claude_code_entry(&runner_bin)
        }
    });

    let codex = format!("[mcp_servers.runner]\ncommand = \"{runner_bin}\"\n");

    Ok(McpConfigSnippet {
        claude_code: serde_json::to_string_pretty(&claude_code).unwrap_or_default(),
        codex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn claude_code_status_false_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        assert!(
            !claude_code_status_at(&path, "/test/runner")
                .unwrap()
                .registered
        );
    }

    #[test]
    fn claude_code_write_creates_runner_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");

        claude_code_write_at(&path, true, "/test/runner-mcp").unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["mcpServers"]["runner"]["type"], json!("stdio"));
        assert_eq!(
            value["mcpServers"]["runner"]["command"],
            json!("/test/runner-mcp")
        );
        assert!(value["mcpServers"]["runner"].get("args").is_none());
        let status = claude_code_status_at(&path, "/test/runner-mcp").unwrap();
        assert!(status.registered);
        assert!(status.matches_current);
        let other_status = claude_code_status_at(&path, "/other/runner-mcp").unwrap();
        assert!(other_status.registered);
        assert!(!other_status.matches_current);
    }

    #[test]
    fn claude_code_write_preserves_other_servers_and_top_level_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"github":{"command":"gh-mcp","args":[]}},"theme":"dark"}"#,
        )
        .unwrap();

        claude_code_write_at(&path, true, "/test/runner-mcp").unwrap();
        let after_enable: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            after_enable["mcpServers"]["runner"]["command"],
            json!("/test/runner-mcp")
        );
        assert_eq!(
            after_enable["mcpServers"]["github"]["command"],
            json!("gh-mcp")
        );
        assert_eq!(after_enable["theme"], json!("dark"));

        claude_code_write_at(&path, false, "/test/runner-mcp").unwrap();
        let after_disable: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(after_disable["mcpServers"].get("runner").is_none());
        assert_eq!(
            after_disable["mcpServers"]["github"]["command"],
            json!("gh-mcp")
        );
        assert_eq!(after_disable["theme"], json!("dark"));
    }

    #[test]
    fn claude_code_write_errors_on_malformed_json_without_overwriting() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, "{ not valid json").unwrap();

        let err = claude_code_write_at(&path, true, "/test/runner-mcp").unwrap_err();

        assert!(err.to_string().contains("parse"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not valid json");
    }

    #[test]
    fn codex_status_false_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".codex").join("config.toml");
        assert!(
            !codex_status_at(&path, "/test/runner-mcp")
                .unwrap()
                .registered
        );
    }

    #[test]
    fn codex_write_creates_dir_and_runner_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".codex").join("config.toml");

        codex_write_at(&path, true, "/test/runner-mcp").unwrap();

        let doc: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["runner"]["command"].as_str(),
            Some("/test/runner-mcp")
        );
        assert!(doc["mcp_servers"]["runner"].get("args").is_none());
        let status = codex_status_at(&path, "/test/runner-mcp").unwrap();
        assert!(status.registered);
        assert!(status.matches_current);
        let other_status = codex_status_at(&path, "/other/runner-mcp").unwrap();
        assert!(other_status.registered);
        assert!(!other_status.matches_current);
    }

    #[test]
    fn codex_write_preserves_other_tables_and_top_level_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "model = \"gpt-5\"\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\nargs = []\n",
        )
        .unwrap();

        codex_write_at(&path, true, "/test/runner-mcp").unwrap();
        let after: toml_edit::DocumentMut =
            std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(after["model"].as_str(), Some("gpt-5"));
        assert_eq!(
            after["mcp_servers"]["github"]["command"].as_str(),
            Some("gh-mcp")
        );
        assert_eq!(
            after["mcp_servers"]["runner"]["command"].as_str(),
            Some("/test/runner-mcp")
        );

        codex_write_at(&path, false, "/test/runner-mcp").unwrap();
        let after_disable: toml_edit::DocumentMut =
            std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert!(after_disable["mcp_servers"].get("runner").is_none());
        assert_eq!(
            after_disable["mcp_servers"]["github"]["command"].as_str(),
            Some("gh-mcp")
        );
        assert_eq!(after_disable["model"].as_str(), Some("gpt-5"));
    }

    #[test]
    fn codex_write_errors_on_malformed_toml_without_overwriting() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[unclosed-table").unwrap();

        let err = codex_write_at(&path, true, "/test/runner-mcp").unwrap_err();

        assert!(err.to_string().contains("parse"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[unclosed-table");
    }

    #[test]
    fn codex_write_errors_when_mcp_servers_is_not_table() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "mcp_servers = \"bad\"\n").unwrap();

        let err = codex_write_at(&path, true, "/test/runner-mcp").unwrap_err();

        assert!(err.to_string().contains("mcp_servers is not a table"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "mcp_servers = \"bad\"\n"
        );
    }
}

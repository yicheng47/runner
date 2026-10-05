use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpClientStatus {
    pub registered: bool,
    pub matches_current: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpServerDefinition {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerClientEntry {
    pub registered: bool,
    pub native_text: String,
    pub definition: Option<McpServerDefinition>,
    pub conflicting: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub clients: BTreeMap<McpClientId, McpServerClientEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCatalog {
    pub servers: Vec<McpServerEntry>,
}

impl McpClientStatus {
    pub fn empty() -> Self {
        Self {
            registered: false,
            matches_current: false,
            command: None,
            args: Vec::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum McpClientId {
    ClaudeCode,
    Codex,
    Trae,
    Copilot,
    Antigravity,
}
impl McpClientId {
    pub fn all() -> Vec<Self> {
        Runtime::ALL
            .into_iter()
            .filter_map(Self::for_runtime)
            .collect()
    }
    pub fn parse(raw: &str) -> Result<Self, ClientError> {
        Self::all().into_iter().find(|client| client.key() == raw).ok_or_else(|| ClientError::msg(format!("unknown MCP client: {raw:?} (expected codex, claude_code, antigravity, copilot, or trae)")))
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
            Self::Copilot => "copilot",
            Self::Trae => "trae",
        }
    }
    pub fn for_runtime(runtime: Runtime) -> Option<Self> {
        match runtime {
            Runtime::ClaudeCode => Some(Self::ClaudeCode),
            Runtime::Codex => Some(Self::Codex),
            Runtime::Antigravity => Some(Self::Antigravity),
            Runtime::Copilot => Some(Self::Copilot),
            Runtime::Trae => Some(Self::Trae),
            _ => None,
        }
    }
    pub fn runtime(self) -> Runtime {
        match self {
            Self::ClaudeCode => Runtime::ClaudeCode,
            Self::Codex => Runtime::Codex,
            Self::Antigravity => Runtime::Antigravity,
            Self::Copilot => Runtime::Copilot,
            Self::Trae => Runtime::Trae,
        }
    }
    pub fn label(self) -> &'static str {
        self.runtime().display_name()
    }
    pub fn config_file(self) -> &'static str {
        match self {
            Self::ClaudeCode => "~/.claude.json",
            Self::Codex => "~/.codex/config.toml",
            Self::Antigravity => "~/.gemini/config/mcp_config.json",
            Self::Copilot => "~/.copilot/mcp-config.json",
            Self::Trae => "~/.trae/traecli.toml",
        }
    }
    pub fn entry_key(self, name: &str) -> String {
        format!(
            "{}.{name}",
            if self.is_json() {
                "mcpServers"
            } else {
                "mcp_servers"
            }
        )
    }
    pub fn config_path(self, home: &std::path::Path) -> PathBuf {
        home.join(self.config_file().trim_start_matches("~/"))
    }
    pub fn supports_http(self) -> bool {
        self != Self::Antigravity
    }
    pub fn is_json(self) -> bool {
        !matches!(self, Self::Codex | Self::Trae)
    }
}
impl McpServerDefinition {
    pub fn from_claude(value: &serde_json::Value) -> Option<Self> {
        let kind = value.get("type").and_then(serde_json::Value::as_str);
        if !matches!(kind, None | Some("stdio" | "local" | "http")) {
            return None;
        }
        let strings = |key: &str| -> Option<BTreeMap<String, String>> {
            match value.get(key) {
                None => Some(BTreeMap::new()),
                Some(v) => v
                    .as_object()?
                    .iter()
                    .map(|(k, v)| Some((k.clone(), v.as_str()?.into())))
                    .collect(),
            }
        };
        if kind != Some("http") {
            if let Some(command) = value.get("command").and_then(serde_json::Value::as_str) {
                let args = match value.get("args") {
                    None => Vec::new(),
                    Some(v) => v
                        .as_array()?
                        .iter()
                        .map(|a| a.as_str().map(str::to_owned))
                        .collect::<Option<_>>()?,
                };
                return Some(Self::Stdio {
                    command: command.into(),
                    args,
                    env: strings("env")?,
                });
            }
        }
        if kind == Some("stdio") {
            return None;
        }
        Some(Self::Http {
            url: value.get("url")?.as_str()?.into(),
            headers: strings("headers")?,
        })
    }
    pub fn to_claude(&self) -> serde_json::Value {
        match self {
            Self::Stdio { command, args, env } => {
                serde_json::json!({"type": "stdio", "command": command, "args": args, "env": env})
            }
            Self::Http { url, headers } => {
                serde_json::json!({"type": "http", "url": url, "headers": headers})
            }
        }
    }
}

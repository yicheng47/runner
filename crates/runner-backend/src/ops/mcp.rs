use std::collections::BTreeMap;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::AppCore;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize)]
pub struct McpConfigSnippet {
    pub claude_code: String,
    pub codex: String,
    pub trae: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpIntegrationStatus {
    pub environment: String,
    pub binary_path: String,
    pub endpoint: String,
    pub claude_code: McpClientStatus,
    pub codex: McpClientStatus,
    pub trae: McpClientStatus,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum McpClientId {
    ClaudeCode,
    Codex,
    Trae,
}

impl McpClientId {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw == "claude_code" {
            return Ok(Self::ClaudeCode);
        }
        match crate::model::Runtime::parse(raw) {
            Some(crate::model::Runtime::Codex) => Ok(Self::Codex),
            Some(crate::model::Runtime::Trae) => Ok(Self::Trae),
            Some(crate::model::Runtime::ClaudeCode | crate::model::Runtime::Shell) | None => {
                Err(Error::msg(format!(
                    "unknown MCP client: {raw:?} (expected claude_code, codex, or trae)"
                )))
            }
        }
    }
}

fn home_dir() -> Result<PathBuf> {
    runner_core::app_paths::home_dir().ok_or_else(|| Error::msg("home directory is not available"))
}

fn claude_code_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude.json"))
}

pub(crate) fn codex_path() -> Result<PathBuf> {
    Ok(crate::runtime_defaults::codex_config_path(&home_dir()?))
}

fn trae_path() -> Result<PathBuf> {
    Ok(crate::runtime_defaults::trae_config_path(&home_dir()?))
}

fn mcp_binary_path(state: &AppCore) -> String {
    state
        .app_data_dir
        .join("bin")
        .join(crate::cli_install::MCP_DEST_BIN_NAME)
        .to_string_lossy()
        .to_string()
}

fn endpoint(state: &AppCore) -> String {
    runner_core::app_paths::mcp_endpoint(&state.app_data_dir, cfg!(debug_assertions)).to_string()
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

fn json_mcp_entry(binary_path: &str) -> serde_json::Value {
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
    write_entry_at(
        path,
        McpClientId::ClaudeCode,
        "runner",
        if enabled {
            Some(NativeEntry::Claude(json_mcp_entry(binary_path)))
        } else {
            None
        },
        false,
    )
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
        .and_then(|item| item.as_table_like())
        .and_then(|servers| servers.get("runner"));
    let Some(entry) = entry else {
        return Ok(McpClientStatus::empty(path));
    };
    let entry_table = entry
        .as_table_like()
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
    let entry = enabled.then(|| {
        let mut table = toml_edit::Table::new();
        table["command"] = toml_edit::value(binary_path);
        NativeEntry::Toml(table)
    });
    write_entry_at(path, McpClientId::Codex, "runner", entry, false)
}

pub fn mcp_integration_status(state: &AppCore) -> Result<McpIntegrationStatus> {
    let binary_path = mcp_binary_path(state);
    let claude_code_path = claude_code_path()?;
    let codex_path = codex_path()?;
    let trae_path = trae_path()?;
    let claude_code = claude_code_status_at(&claude_code_path, &binary_path)
        .unwrap_or_else(|e| McpClientStatus::error(&claude_code_path, e.to_string()));
    let codex = codex_status_at(&codex_path, &binary_path)
        .unwrap_or_else(|e| McpClientStatus::error(&codex_path, e.to_string()));
    let trae = codex_status_at(&trae_path, &binary_path)
        .unwrap_or_else(|e| McpClientStatus::error(&trae_path, e.to_string()));
    Ok(McpIntegrationStatus {
        environment: environment_label(),
        endpoint: endpoint(state),
        binary_path,
        claude_code,
        codex,
        trae,
    })
}

pub fn mcp_set_integration(state: &AppCore, client: &str, enabled: bool) -> Result<()> {
    let binary_path = mcp_binary_path(state);
    match McpClientId::parse(client)? {
        McpClientId::ClaudeCode => {
            claude_code_write_at(&claude_code_path()?, enabled, &binary_path)
        }
        McpClientId::Codex => codex_write_at(&codex_path()?, enabled, &binary_path),
        McpClientId::Trae => codex_write_at(&trae_path()?, enabled, &binary_path),
    }
}

pub fn mcp_config_snippet(state: &AppCore) -> Result<McpConfigSnippet> {
    let runner_bin = mcp_binary_path(state);

    let claude_code = json!({
        "mcpServers": {
            "runner": json_mcp_entry(&runner_bin)
        }
    });

    let codex = format!("[mcp_servers.runner]\ncommand = \"{runner_bin}\"\n");
    let trae = codex.clone();

    Ok(McpConfigSnippet {
        claude_code: serde_json::to_string_pretty(&claude_code).unwrap_or_default(),
        codex,
        trae,
    })
}

impl McpClientId {
    pub const ALL: [Self; 3] = [Self::ClaudeCode, Self::Codex, Self::Trae];

    pub fn key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Trae => "trae",
        }
    }

    pub fn for_runtime(runtime: crate::model::Runtime) -> Option<Self> {
        match runtime {
            crate::model::Runtime::ClaudeCode => Some(Self::ClaudeCode),
            crate::model::Runtime::Codex => Some(Self::Codex),
            crate::model::Runtime::Trae => Some(Self::Trae),
            crate::model::Runtime::Shell => None,
        }
    }

    pub fn runtime(self) -> crate::model::Runtime {
        match self {
            Self::ClaudeCode => crate::model::Runtime::ClaudeCode,
            Self::Codex => crate::model::Runtime::Codex,
            Self::Trae => crate::model::Runtime::Trae,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Trae => "TRAE CLI",
        }
    }

    pub fn config_file(self) -> &'static str {
        match self {
            Self::ClaudeCode => "~/.claude.json",
            Self::Codex => "~/.codex/config.toml",
            Self::Trae => "~/.trae/traecli.toml",
        }
    }

    pub fn entry_key(self, name: &str) -> String {
        format!(
            "{}.{name}",
            if self == Self::ClaudeCode {
                "mcpServers"
            } else {
                "mcp_servers"
            }
        )
    }

    pub fn status(self, status: &McpIntegrationStatus) -> &McpClientStatus {
        match self {
            Self::ClaudeCode => &status.claude_code,
            Self::Codex => &status.codex,
            Self::Trae => &status.trae,
        }
    }

    fn path(self) -> Result<PathBuf> {
        match self {
            Self::ClaudeCode => claude_code_path(),
            Self::Codex => codex_path(),
            Self::Trae => trae_path(),
        }
    }
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

impl McpServerDefinition {
    pub fn from_claude(value: &serde_json::Value) -> Option<Self> {
        let kind = value.get("type").and_then(serde_json::Value::as_str);
        if !matches!(kind, None | Some("stdio" | "http")) {
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

    pub fn from_toml(table: &toml_edit::Table) -> Option<Self> {
        let strings = |key: &str| -> Option<BTreeMap<String, String>> {
            match table.get(key) {
                None => Some(BTreeMap::new()),
                Some(v) => v
                    .as_table_like()?
                    .iter()
                    .map(|(k, v)| Some((k.into(), v.as_str()?.into())))
                    .collect(),
            }
        };
        if let Some(command) = table.get("command").and_then(toml_edit::Item::as_str) {
            let args = match table.get("args") {
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
        Some(Self::Http {
            url: table.get("url")?.as_str()?.into(),
            headers: strings("http_headers")?,
        })
    }

    pub fn to_claude(&self) -> serde_json::Value {
        match self {
            Self::Stdio { command, args, env } => {
                json!({"type": "stdio", "command": command, "args": args, "env": env})
            }
            Self::Http { url, headers } => json!({"type": "http", "url": url, "headers": headers}),
        }
    }

    pub fn write_toml(&self, table: &mut toml_edit::Table) {
        let map = |values: &BTreeMap<String, String>| {
            let mut result = toml_edit::InlineTable::new();
            for (key, value) in values {
                result.insert(key, value.as_str().into());
            }
            toml_edit::value(result)
        };
        match self {
            Self::Stdio { command, args, env } => {
                table.remove("url");
                table.remove("http_headers");
                table["command"] = toml_edit::value(command);
                table["args"] = toml_edit::value(args.iter().collect::<toml_edit::Array>());
                table["env"] = map(env);
            }
            Self::Http { url, headers } => {
                for key in ["command", "args", "env"] {
                    table.remove(key);
                }
                table["url"] = toml_edit::value(url);
                table["http_headers"] = map(headers);
            }
        }
    }
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
    pub runner: McpIntegrationStatus,
    pub runner_server: McpServerEntry,
    pub servers: Vec<McpServerEntry>,
}

#[derive(Clone)]
enum NativeEntry {
    Claude(serde_json::Value),
    Toml(toml_edit::Table),
}

impl NativeEntry {
    fn definition(&self) -> Option<McpServerDefinition> {
        match self {
            Self::Claude(value) => McpServerDefinition::from_claude(value),
            Self::Toml(table) => McpServerDefinition::from_toml(table),
        }
    }

    fn text(&self, name: &str) -> String {
        match self {
            Self::Claude(value) => serde_json::to_string_pretty(value).unwrap(),
            Self::Toml(table) => {
                let mut doc = toml_edit::DocumentMut::new();
                let mut parent = toml_edit::Table::new();
                parent.set_implicit(true);
                parent.insert(name, toml_edit::Item::Table(table.clone()));
                doc.insert("mcp_servers", toml_edit::Item::Table(parent));
                doc.to_string()
            }
        }
    }
}

fn read_config(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(Error::msg(format!("read {}: {e}", path.display()))),
    }
}

fn read_entries_at(path: &Path, client: McpClientId) -> Result<BTreeMap<String, NativeEntry>> {
    let raw = read_config(path)?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    if client == McpClientId::ClaudeCode {
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?;
        let object = value
            .as_object()
            .ok_or_else(|| Error::msg(format!("{} is not a JSON object", path.display())))?;
        match object.get("mcpServers") {
            None => Ok(BTreeMap::new()),
            Some(servers) => Ok(servers
                .as_object()
                .ok_or_else(|| {
                    Error::msg(format!(
                        "{}::mcpServers is not a JSON object",
                        path.display()
                    ))
                })?
                .iter()
                .map(|(name, value)| (name.clone(), NativeEntry::Claude(value.clone())))
                .collect()),
        }
    } else {
        let doc: toml_edit::DocumentMut = raw
            .parse()
            .map_err(|e| Error::msg(format!("parse {}: {e}", path.display())))?;
        let Some(servers) = doc.get("mcp_servers") else {
            return Ok(BTreeMap::new());
        };
        servers
            .as_table_like()
            .ok_or_else(|| Error::msg(format!("{}: mcp_servers is not a table", path.display())))?
            .iter()
            .map(|(name, item)| {
                let table = item.clone().into_table().map_err(|_| {
                    Error::msg(format!(
                        "{}: mcp_servers.{name} is not a table",
                        path.display()
                    ))
                })?;
                Ok((name.into(), NativeEntry::Toml(table)))
            })
            .collect()
    }
}

fn catalog_at(runner: McpIntegrationStatus, paths: &BTreeMap<McpClientId, PathBuf>) -> McpCatalog {
    let sources: BTreeMap<_, _> = paths
        .iter()
        .map(|(&client, path)| (client, read_entries_at(path, client)))
        .collect();
    let mut entries = BTreeMap::<String, McpServerEntry>::new();
    entries.insert(
        "runner".into(),
        McpServerEntry {
            name: "runner".into(),
            clients: BTreeMap::new(),
        },
    );
    for (&client, source) in &sources {
        if let Ok(servers) = source {
            for (name, native) in servers {
                entries
                    .entry(name.clone())
                    .or_insert_with(|| McpServerEntry {
                        name: name.clone(),
                        clients: BTreeMap::new(),
                    })
                    .clients
                    .insert(
                        client,
                        McpServerClientEntry {
                            registered: true,
                            native_text: native.text(name),
                            definition: native.definition(),
                            ..Default::default()
                        },
                    );
            }
        }
    }
    for entry in entries.values_mut() {
        let snapshots = entry.clients.clone();
        for (&client, source) in &sources {
            let slot = entry.clients.entry(client).or_default();
            slot.error = source.as_ref().err().map(ToString::to_string);
            slot.conflicting = slot.registered
                && snapshots.iter().any(|(&other, copy)| {
                    other != client
                        && (copy.definition != slot.definition
                            || (copy.definition.is_none() && copy.native_text != slot.native_text))
                });
        }
    }
    McpCatalog {
        runner,
        runner_server: entries.remove("runner").unwrap(),
        servers: entries.into_values().collect(),
    }
}

fn client_paths() -> Result<BTreeMap<McpClientId, PathBuf>> {
    McpClientId::ALL
        .into_iter()
        .map(|client| Ok((client, client.path()?)))
        .collect()
}

pub fn mcp_catalog(state: &AppCore) -> Result<McpCatalog> {
    Ok(catalog_at(mcp_integration_status(state)?, &client_paths()?))
}

// serde_json validates and locates values; splice only the owned member so unrelated
// JSON whitespace, ordering, and number/string spellings survive byte for byte.
struct JsonMember {
    key: String,
    key_start: usize,
    value: std::ops::Range<usize>,
    comma: Option<usize>,
}

fn json_members(raw: &str) -> Result<(Vec<JsonMember>, usize)> {
    let mut offset = raw
        .find('{')
        .ok_or_else(|| Error::msg("expected JSON object"))?
        + 1;
    let mut members = Vec::new();
    loop {
        offset += raw[offset..].len() - raw[offset..].trim_start().len();
        if raw.as_bytes()[offset] == b'}' {
            return Ok((members, offset));
        }
        let key_start = offset;
        let mut keys = serde_json::Deserializer::from_str(&raw[offset..]).into_iter::<String>();
        let key = keys
            .next()
            .unwrap()
            .map_err(|e| Error::msg(e.to_string()))?;
        offset += keys.byte_offset();
        offset += raw[offset..].find(':').unwrap() + 1;
        offset += raw[offset..].len() - raw[offset..].trim_start().len();
        let start = offset;
        let mut values =
            serde_json::Deserializer::from_str(&raw[offset..]).into_iter::<serde::de::IgnoredAny>();
        values
            .next()
            .unwrap()
            .map_err(|e| Error::msg(e.to_string()))?;
        offset += values.byte_offset();
        let end = offset;
        offset += raw[offset..].len() - raw[offset..].trim_start().len();
        let comma = (raw.as_bytes()[offset] == b',').then_some(offset);
        if comma.is_some() {
            offset += 1;
        }
        members.push(JsonMember {
            key,
            key_start,
            value: start..end,
            comma,
        });
    }
}

fn replace_json_member(raw: &str, name: &str, value: Option<&serde_json::Value>) -> Result<String> {
    let (members, close) = json_members(raw)?;
    let mut out = raw.to_owned();
    if let Some((index, member)) = members.iter().enumerate().find(|(_, m)| m.key == name) {
        if let Some(value) = value {
            out.replace_range(
                member.value.clone(),
                &serde_json::to_string_pretty(value).unwrap(),
            );
        } else {
            out.replace_range(member.key_start..member.value.end, "");
            if let Some(comma) = member.comma {
                let shifted = comma - (member.value.end - member.key_start);
                out.replace_range(shifted..shifted + 1, "");
            } else if index > 0 {
                let comma = members[index - 1].comma.unwrap();
                out.replace_range(comma..comma + 1, "");
            }
        }
    } else if let Some(value) = value {
        let text = format!(
            "{}: {}",
            serde_json::to_string(name).unwrap(),
            serde_json::to_string_pretty(value).unwrap()
        );
        out.insert_str(close, &text);
        if let Some(last) = members.last() {
            out.insert(last.value.end, ',');
        }
    }
    Ok(out)
}

fn line_range(raw: &str, span: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = raw[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let end = raw[span.end..]
        .find('\n')
        .map_or(raw.len(), |i| span.end + i + 1);
    start..end
}

fn table_ranges(raw: &str, table: &toml_edit::Table, ranges: &mut Vec<std::ops::Range<usize>>) {
    if !table.is_implicit() && !table.is_dotted() {
        if let Some(span) = table.span().filter(|s| !s.is_empty()) {
            let mut range = line_range(raw, span);
            if let Some(prefix) = table.decor().prefix().and_then(|p| p.span()) {
                range.start = range.start.min(prefix.start);
            }
            ranges.push(range);
        }
    }
    for (keys, value) in table.get_values() {
        if let (Some(key), Some(value)) = (keys.first().and_then(|k| k.span()), value.span()) {
            ranges.push(line_range(raw, key.start..value.end));
        }
    }
    for (_, item) in table.iter() {
        match item {
            toml_edit::Item::Table(child) => table_ranges(raw, child, ranges),
            toml_edit::Item::ArrayOfTables(array) => {
                for child in array.iter() {
                    table_ranges(raw, child, ranges);
                }
            }
            _ => {}
        }
    }
}

fn inline_entry_text(table: toml_edit::Table) -> String {
    let mut inline = table.into_inline_table();
    inline.fmt();
    inline.to_string()
}

// DocumentMut emits LF after every value/header. Use the parser's original spans
// for the write boundary so CRLF, an absent final newline, and interleaved tables survive.
fn splice_toml_entry(
    raw: &str,
    name: &str,
    replacement: Option<toml_edit::Table>,
) -> Result<String> {
    let parsed = toml_edit::Document::parse(raw).map_err(|e| Error::msg(e.to_string()))?;
    let parent = parsed.get("mcp_servers");
    let mut out = raw.to_owned();
    if let Some(inline) = parent.and_then(toml_edit::Item::as_inline_table) {
        let members: Vec<_> = inline
            .iter()
            .map(|(key, value)| {
                (
                    key,
                    inline.key(key).unwrap().span().unwrap(),
                    value.span().unwrap(),
                )
            })
            .collect();
        let span = inline.span().unwrap();
        if let Some((index, (_, key, value))) = members
            .iter()
            .enumerate()
            .find(|(_, (key, _, _))| *key == name)
        {
            if let Some(table) = replacement {
                out.replace_range(value.clone(), &inline_entry_text(table));
            } else {
                let mut range = key.start..value.end;
                if let Some((_, next, _)) = members.get(index + 1) {
                    if let Some(comma) = raw[value.end..next.start].find(',') {
                        range.end = value.end + comma + 1;
                    }
                } else if index > 0 {
                    let previous = &members[index - 1].2;
                    if let Some(comma) = raw[previous.end..key.start].find(',') {
                        range.start = previous.end + comma;
                    }
                } else if let Some(comma) = raw[value.end..span.end - 1].find(',') {
                    range.end = value.end + comma + 1;
                }
                out.replace_range(range, "");
            }
        } else if let Some(table) = replacement {
            let text = format!(
                "{} = {}",
                toml_edit::Key::new(name),
                inline_entry_text(table)
            );
            out.insert_str(span.end - 1, &text);
            if let Some((_, _, last)) = members.last() {
                if !raw[last.end..span.end - 1].contains(',') {
                    out.insert(last.end, ',');
                }
            }
        }
        out.parse::<toml_edit::DocumentMut>()
            .map_err(|e| Error::msg(format!("Cannot update this TOML entry in place: {e}")))?;
        return Ok(out);
    }
    let existing = parent.and_then(|p| p.get(name));
    if let Some(value) = existing.and_then(toml_edit::Item::as_inline_table) {
        if let Some(table) = replacement {
            out.replace_range(value.span().unwrap(), &inline_entry_text(table));
        } else {
            let key = parent
                .unwrap()
                .as_table()
                .unwrap()
                .key(name)
                .unwrap()
                .span()
                .unwrap();
            out.replace_range(line_range(raw, key.start..value.span().unwrap().end), "");
        }
        return Ok(out);
    }
    let mut ranges = Vec::new();
    if let Some(table) = existing.and_then(toml_edit::Item::as_table) {
        table_ranges(raw, table, &mut ranges);
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut().filter(|last| last.end >= range.start) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let append = existing
        .and_then(toml_edit::Item::as_table)
        .is_some_and(toml_edit::Table::is_dotted);
    let insertion = merged.first().map_or(raw.len(), |range| range.start);
    for range in merged.iter().rev() {
        out.replace_range(range.clone(), "");
    }
    let insertion = if append { out.len() } else { insertion };
    if let Some(mut table) = replacement {
        table.set_implicit(false);
        table.set_dotted(false);
        let mut text = NativeEntry::Toml(table).text(name);
        if insertion > 0 && !out[..insertion].ends_with('\n') {
            text.insert(0, '\n');
        }
        out.insert_str(insertion, &text);
    }
    // Reject unusual dotted/inline forms we cannot patch without altering another entry.
    out.parse::<toml_edit::DocumentMut>()
        .map_err(|e| Error::msg(format!("Cannot update this TOML entry in place: {e}")))?;
    Ok(out)
}

fn write_entry_at(
    path: &Path,
    client: McpClientId,
    name: &str,
    entry: Option<NativeEntry>,
    merge: bool,
) -> Result<()> {
    let raw = read_config(path)?;
    let existing = read_entries_at(path, client)?;
    if entry.is_none() && !existing.contains_key(name) {
        return Ok(());
    }
    let output = if client == McpClientId::ClaudeCode {
        let raw = if raw.trim().is_empty() { "{}" } else { &raw };
        let (members, _) = json_members(raw)?;
        let server_member = members.iter().find(|m| m.key == "mcpServers");
        let value = match entry {
            Some(NativeEntry::Claude(mut value)) => {
                if merge {
                    if let Some(NativeEntry::Claude(old)) = existing.get(name) {
                        let mut base = old.as_object().cloned().unwrap_or_default();
                        for key in ["type", "command", "args", "env", "url", "headers"] {
                            base.remove(key);
                        }
                        base.extend(value.as_object().unwrap().clone());
                        value = serde_json::Value::Object(base);
                    }
                }
                Some(value)
            }
            None => None,
            _ => unreachable!(),
        };
        if let Some(member) = server_member {
            let updated = replace_json_member(&raw[member.value.clone()], name, value.as_ref())?;
            let mut out = raw.to_owned();
            out.replace_range(member.value.clone(), &updated);
            out
        } else {
            replace_json_member(raw, "mcpServers", Some(&json!({ name: value.unwrap() })))?
        }
    } else {
        let replacement = match entry {
            Some(NativeEntry::Toml(mut table)) => {
                if merge {
                    if let Some(NativeEntry::Toml(old)) = existing.get(name) {
                        let definition = McpServerDefinition::from_toml(&table).unwrap();
                        table = old.clone();
                        definition.write_toml(&mut table);
                    }
                }
                Some(table)
            }
            None => None,
            _ => unreachable!(),
        };
        splice_toml_entry(&raw, name, replacement)
            .map_err(|e| Error::msg(format!("{}: {e}", path.display())))?
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::msg(format!("mkdir {}: {e}", parent.display())))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .and_then(|mut file| file.write_all(output.as_bytes()))
        .map_err(|e| Error::msg(format!("write {}: {e}", path.display())))
}

fn translated(definition: &McpServerDefinition, client: McpClientId) -> NativeEntry {
    if client == McpClientId::ClaudeCode {
        NativeEntry::Claude(definition.to_claude())
    } else {
        let mut table = toml_edit::Table::new();
        definition.write_toml(&mut table);
        NativeEntry::Toml(table)
    }
}

fn copy_at(
    paths: &BTreeMap<McpClientId, PathBuf>,
    from: McpClientId,
    to: McpClientId,
    name: &str,
) -> Result<()> {
    let source = read_entries_at(&paths[&from], from)?;
    let entry = source.get(name).ok_or_else(|| {
        Error::msg(format!(
            "{}: {name} is not registered",
            paths[&from].display()
        ))
    })?;
    let definition = entry.definition().ok_or_else(|| {
        Error::msg(format!(
            "{name} cannot be translated; add it with {}'s CLI",
            to.label()
        ))
    })?;
    write_entry_at(
        &paths[&to],
        to,
        name,
        Some(translated(&definition, to)),
        true,
    )
}

pub fn mcp_copy_server(
    _state: &AppCore,
    from: McpClientId,
    to: McpClientId,
    name: &str,
) -> Result<()> {
    check_server_name(name)?;
    copy_at(&client_paths()?, from, to, name)
}

pub fn mcp_remove_server(_state: &AppCore, client: McpClientId, name: &str) -> Result<()> {
    check_server_name(name)?;
    write_entry_at(&client.path()?, client, name, None, false)
}

fn check_server_name(name: &str) -> Result<()> {
    if name == "runner" {
        return Err(Error::msg(
            "Use Runner MCP registration to change the built-in server",
        ));
    }
    Ok(())
}

fn parse_native(client: McpClientId, name: &str, text: &str) -> Result<NativeEntry> {
    if client == McpClientId::ClaudeCode {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| Error::msg(format!("Invalid JSON: {e}")))?;
        if !value.is_object() {
            return Err(Error::msg("The MCP entry must be a JSON object"));
        }
        Ok(NativeEntry::Claude(value))
    } else {
        let mut doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e| Error::msg(format!("Invalid TOML: {e}")))?;
        if let Some(servers) = doc.get("mcp_servers") {
            let servers = servers
                .as_table_like()
                .ok_or_else(|| Error::msg("mcp_servers must be a table"))?;
            if doc.len() != 1 || servers.len() != 1 || !servers.contains_key(name) {
                return Err(Error::msg(format!("Edit only the mcp_servers.{name} table; rename servers through the agent's CLI")));
            }
            let table = servers
                .get(name)
                .unwrap()
                .clone()
                .into_table()
                .map_err(|_| Error::msg("The MCP entry must be a TOML table"))?;
            Ok(NativeEntry::Toml(table))
        } else {
            Ok(NativeEntry::Toml(std::mem::take(doc.as_table_mut())))
        }
    }
}

pub fn validate_mcp_edit(
    client: McpClientId,
    name: &str,
    text: &str,
    also_update: bool,
) -> Result<()> {
    let entry = parse_native(client, name, text)?;
    if also_update && entry.definition().is_none() {
        return Err(Error::msg("This transport cannot be translated. Turn off Also update and edit each agent's config with its own CLI."));
    }
    Ok(())
}

fn edit_at(
    paths: &BTreeMap<McpClientId, PathBuf>,
    client: McpClientId,
    name: &str,
    text: &str,
    also: &[McpClientId],
) -> Result<()> {
    validate_mcp_edit(client, name, text, !also.is_empty())?;
    let native = parse_native(client, name, text)?;
    let definition = native.definition();
    let mut errors = Vec::new();
    if let Err(e) = write_entry_at(&paths[&client], client, name, Some(native), false) {
        errors.push(e.to_string());
    }
    for &other in McpClientId::ALL
        .iter()
        .filter(|&&other| other != client && also.contains(&other))
    {
        if let Err(e) = write_entry_at(
            &paths[&other],
            other,
            name,
            Some(translated(definition.as_ref().unwrap(), other)),
            true,
        ) {
            errors.push(e.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::msg(errors.join("\n")))
    }
}

pub fn mcp_edit_server(
    _state: &AppCore,
    client: McpClientId,
    name: &str,
    native_text: &str,
    also: &[McpClientId],
) -> Result<()> {
    check_server_name(name)?;
    edit_at(&client_paths()?, client, name, native_text, also)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths(dir: &TempDir) -> BTreeMap<McpClientId, PathBuf> {
        McpClientId::ALL
            .into_iter()
            .map(|c| (c, dir.path().join(c.key())))
            .collect()
    }

    fn catalog(paths: &BTreeMap<McpClientId, PathBuf>) -> McpCatalog {
        catalog_at(
            McpIntegrationStatus {
                environment: "test".into(),
                binary_path: "/runner".into(),
                endpoint: String::new(),
                claude_code: McpClientStatus::empty(&paths[&McpClientId::ClaudeCode]),
                codex: McpClientStatus::empty(&paths[&McpClientId::Codex]),
                trae: McpClientStatus::empty(&paths[&McpClientId::Trae]),
            },
            paths,
        )
    }

    #[test]
    fn toml_writes_preserve_crlf_missing_final_newline_and_interleaved_tables() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        let prefix = "# user settings\r\nmodel = 'gpt-5'\r\n\r\n[mcp_servers] # parent\r\n";
        let middle = "\r\n[mcp_servers.other]\r\ncommand = 'other' # untouched\r\n";
        let suffix = "\r\n[ui]\r\ntheme = 'dark'";
        let raw = format!("{prefix}\r\n[mcp_servers.github]\r\ncommand = 'old'\r\nstartup_timeout_sec = 60\r\n{middle}\r\n[mcp_servers.github.env]\r\nTOKEN = 'old'\r\n{suffix}");
        std::fs::write(&paths[&Codex], &raw).unwrap();
        std::fs::write(
            &paths[&ClaudeCode],
            r#"{"mcpServers":{"github":{"command":"new","env":{"TOKEN":"new"}}}}"#,
        )
        .unwrap();
        copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
        let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert!(after.starts_with(prefix));
        assert!(after.contains(middle));
        assert!(after.ends_with(suffix));
        assert!(after.contains("startup_timeout_sec = 60"));
        let body = read_entries_at(&paths[&Codex], Codex).unwrap()["github"].text("github");
        edit_at(&paths, Codex, "github", &body.replace("new", "edited"), &[]).unwrap();
        let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert!(after.starts_with(prefix));
        assert!(after.contains(middle));
        assert!(after.ends_with(suffix));
        write_entry_at(&paths[&Codex], Codex, "github", None, false).unwrap();
        let removed = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert_eq!(removed, format!("{prefix}{middle}{suffix}"));
        codex_write_at(&paths[&Codex], true, "/runner").unwrap();
        assert!(std::fs::read_to_string(&paths[&Codex])
            .unwrap()
            .starts_with(&removed));
    }

    #[test]
    fn inline_and_dotted_toml_entries_keep_siblings_and_parent() {
        for raw in [
            "model = 'gpt'\r\nmcp_servers = { github = { command = 'old' }, other = { command = 'other' } }\r\n# trailing",
            "[mcp_servers]\r\ngithub = { command = 'old' }\r\nother = { command = 'other' }\r\n# trailing",
            "mcp_servers.github.command = 'old'\r\nmcp_servers.other.command = 'other'\r\n# trailing",
        ] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, raw).unwrap();
            let mut replacement = toml_edit::Table::new();
            replacement["command"] = toml_edit::value("new");
            write_entry_at(&path, McpClientId::Codex, "github", Some(NativeEntry::Toml(replacement)), false).unwrap();
            let updated = std::fs::read_to_string(&path).unwrap();
            assert!(updated.contains("command = 'other'"));
            assert!(updated.contains("\r\n# trailing"));
            write_entry_at(&path, McpClientId::Codex, "github", None, false).unwrap();
            let removed = std::fs::read_to_string(&path).unwrap();
            assert!(removed.contains("command = 'other'"), "input: {raw:?}\nremoved: {removed:?}");
            assert!(!removed.contains("github"));
        }
        let raw = "mcp_servers = { github = { command = 'old' } }\r\nmodel = 'gpt'";
        assert_eq!(
            splice_toml_entry(raw, "github", None).unwrap(),
            "mcp_servers = {  }\r\nmodel = 'gpt'"
        );
    }

    #[test]
    fn runner_status_reads_inline_registration_without_overwriting_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        for raw in [
            "mcp_servers = { runner = { command = '/other/runner' } }",
            "[mcp_servers]\nrunner = { command = '/other/runner' }",
        ] {
            std::fs::write(&path, raw).unwrap();
            let status = codex_status_at(&path, "/current/runner").unwrap();
            assert!(status.registered && !status.matches_current);
            assert_eq!(status.command.as_deref(), Some("/other/runner"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_only_claude_file_fails_by_path_while_codex_edit_lands() {
        use std::os::unix::fs::PermissionsExt;
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        let json = r#"{"mcpServers":{"github":{"command":"old"}}, "theme" : "dark"}"#;
        std::fs::write(&paths[&ClaudeCode], json).unwrap();
        std::fs::set_permissions(&paths[&ClaudeCode], std::fs::Permissions::from_mode(0o400))
            .unwrap();
        let result = edit_at(
            &paths,
            ClaudeCode,
            "github",
            r#"{"command":"new"}"#,
            &[Codex],
        );
        std::fs::set_permissions(&paths[&ClaudeCode], std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains(paths[&ClaudeCode].to_str().unwrap()));
        assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
        assert!(read_entries_at(&paths[&Codex], Codex).unwrap()["github"]
            .text("github")
            .contains("new"));
    }

    #[test]
    fn catalog_copies_conflicts_and_edits_without_touching_other_entries() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        let claude = "{\n  \"theme\" : \"dark\", \"mcpServers\" : {\"github\": {\"command\":\"gh-mcp\"}, \"runner\": {\"command\":\"/runner\"}}, \"n\":1e2\n}\n";
        let rest = "# model comment\nmodel = 'gpt-5'\n\n[tools]\nweb = true # stay\n\n[mcp_servers.other]\ncommand = 'other'\n\n[mcp_servers.other.env]\nTOKEN = 'secret'\n";
        std::fs::write(&paths[&ClaudeCode], claude).unwrap();
        std::fs::write(&paths[&Codex], rest).unwrap();
        let first = catalog(&paths);
        assert_eq!(
            first
                .servers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["github", "other"]
        );
        let github = &first.servers[0];
        assert!(github.clients[&ClaudeCode].registered);
        assert!(!github.clients[&Codex].registered);
        assert!(!github.clients[&Trae].registered);
        assert!(first.runner_server.clients[&ClaudeCode].registered);
        copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
        let copied = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert!(copied.starts_with(rest), "{copied}");
        assert!(copied.contains("[mcp_servers.github]"));
        assert_eq!(
            std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
            claude
        );
        let conflicting = format!("{rest}\n[mcp_servers.github]\ncommand = 'different'\nstartup_timeout_sec = 90 # keep\n");
        std::fs::write(&paths[&Codex], &conflicting).unwrap();
        let second = catalog(&paths);
        assert!(second.servers[0].clients[&ClaudeCode].conflicting);
        assert!(second.servers[0].clients[&Codex].conflicting);
        edit_at(
            &paths,
            ClaudeCode,
            "github",
            r#"{"command":"fixed","extra":42}"#,
            &[Codex],
        )
        .unwrap();
        let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert!(after.starts_with(rest));
        assert!(after.contains("startup_timeout_sec = 90 # keep\n"));
        assert!(!catalog(&paths).servers[0].clients[&Codex].conflicting);
        let after_claude = std::fs::read_to_string(&paths[&ClaudeCode]).unwrap();
        assert_eq!(
            after_claude,
            claude.replace(
                r#"{"command":"gh-mcp"}"#,
                "{\n  \"command\": \"fixed\",\n  \"extra\": 42\n}"
            )
        );
        for client in [ClaudeCode, Codex] {
            write_entry_at(&paths[&client], client, "github", None, false).unwrap();
        }
        assert_eq!(
            catalog(&paths)
                .servers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["other"]
        );
        assert!(std::fs::read_to_string(&paths[&Codex])
            .unwrap()
            .starts_with(rest));
        assert!(catalog(&paths).runner_server.clients[&ClaudeCode].registered);
    }

    #[test]
    fn copying_onto_existing_entry_keeps_unmodelled_keys_and_switches_transport() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        std::fs::write(&paths[&ClaudeCode], r#"{"mcpServers":{"github":{"type":"http","url":"https://example.test","headers":{"Authorization":"secret"}}}}"#).unwrap();
        let rest = "# untouched\nmodel = 'old'\n\n";
        std::fs::write(&paths[&Codex], format!("{rest}[mcp_servers.github]\ncommand = 'old'\nargs = ['a']\nenv = {{ KEY = 'value' }}\nstartup_timeout_sec = 60 # keep\nenabled = false\ncwd = '/tmp'\n")).unwrap();
        copy_at(&paths, ClaudeCode, Codex, "github").unwrap();
        let after = std::fs::read_to_string(&paths[&Codex]).unwrap();
        assert!(after.starts_with(rest));
        assert!(after.contains("startup_timeout_sec = 60 # keep\nenabled = false\ncwd = '/tmp'\n"));
        let entries = read_entries_at(&paths[&Codex], Codex).unwrap();
        assert_eq!(
            entries["github"].definition(),
            read_entries_at(&paths[&ClaudeCode], ClaudeCode).unwrap()["github"].definition()
        );
        assert!(!after.contains("command ="));
        assert!(!after.contains("env ="));
        copy_at(&paths, Codex, Trae, "github").unwrap();
        assert_eq!(
            read_entries_at(&paths[&Trae], Trae).unwrap()["github"].definition(),
            entries["github"].definition()
        );
    }

    #[test]
    fn invalid_and_untranslatable_edits_validate_before_any_write() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        let json = r#"{"mcpServers":{"github":{"command":"old"}}}"#;
        let toml = "[mcp_servers.github]\ncommand = 'old'\n";
        std::fs::write(&paths[&ClaudeCode], json).unwrap();
        std::fs::write(&paths[&Codex], toml).unwrap();
        for invalid in ["{", "[]", r#"{"type":"sse","url":"https://example.test"}"#] {
            assert!(edit_at(&paths, ClaudeCode, "github", invalid, &[Codex]).is_err());
            assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
            assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
        }
        for invalid in [
            "[",
            "[mcp_servers.renamed]\ncommand = 'new'",
            "[mcp_servers.github]\ncommand = 'new'\n[other]\nx = 1",
        ] {
            assert!(edit_at(&paths, Codex, "github", invalid, &[ClaudeCode]).is_err());
            assert_eq!(std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(), json);
            assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
        }
        let sse = r#"{"type":"sse","url":"https://example.test"}"#;
        edit_at(&paths, ClaudeCode, "github", sse, &[]).unwrap();
        let entry = catalog(&paths).servers.remove(0);
        assert!(entry.clients[&ClaudeCode].registered);
        assert!(entry.clients[&ClaudeCode].definition.is_none());
        assert!(copy_at(&paths, ClaudeCode, Codex, "github")
            .unwrap_err()
            .to_string()
            .contains("CLI"));
        assert_eq!(std::fs::read_to_string(&paths[&Codex]).unwrap(), toml);
    }

    #[test]
    fn malformed_client_is_reported_on_every_row_and_independent_edits_continue() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        std::fs::write(&paths[&ClaudeCode], "{ broken").unwrap();
        std::fs::write(
            &paths[&Codex],
            "[mcp_servers.github]\ncommand = 'old'\n[mcp_servers.other]\ncommand = 'other'\n",
        )
        .unwrap();
        let catalog = catalog(&paths);
        assert!(catalog.servers.iter().all(|e| e.clients[&ClaudeCode]
            .error
            .as_ref()
            .unwrap()
            .contains(paths[&ClaudeCode].to_str().unwrap())));
        assert!(catalog.runner_server.clients[&ClaudeCode].error.is_some());
        let error = edit_at(
            &paths,
            ClaudeCode,
            "github",
            r#"{"command":"new"}"#,
            &[Codex, Trae],
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains(paths[&ClaudeCode].to_str().unwrap()));
        assert_eq!(
            std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
            "{ broken"
        );
        for client in [Codex, Trae] {
            assert_eq!(
                read_entries_at(&paths[&client], client).unwrap()["github"].definition(),
                Some(McpServerDefinition::Stdio {
                    command: "new".into(),
                    args: vec![],
                    env: BTreeMap::new()
                })
            );
        }
    }

    #[test]
    fn last_removal_preserves_parent_and_every_unowned_byte() {
        use McpClientId::*;
        let dir = TempDir::new().unwrap();
        let paths = paths(&dir);
        let json = "{ \"before\":1e2, \"mcpServers\" : {  \"runner\" : {\"command\":\"old\"}  }, \"after\":\"\\u0061\" }\n";
        std::fs::write(&paths[&ClaudeCode], json).unwrap();
        claude_code_write_at(&paths[&ClaudeCode], false, "new").unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths[&ClaudeCode]).unwrap(),
            json.replace("\"runner\" : {\"command\":\"old\"}", "")
        );
        let toml = "# before\nmodel = 'gpt-5'\n\n# parent comment\n[mcp_servers] # keep parent\n\n[mcp_servers.runner]\ncommand = 'old'\n\n[after]\nflag = true\n";
        std::fs::write(&paths[&Codex], toml).unwrap();
        codex_write_at(&paths[&Codex], false, "new").unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths[&Codex]).unwrap(),
            toml.replace("\n[mcp_servers.runner]\ncommand = 'old'\n", "")
        );
    }

    #[test]
    fn json_member_splicing_handles_first_middle_last_and_escaped_names() {
        let raw = r#" { "a": {"s":"x,}[]\""}, "b\"x": [1,2], "c": 1e2 } "#;
        for (name, expected) in [
            ("a", r#" {  "b\"x": [1,2], "c": 1e2 } "#),
            ("b\"x", r#" { "a": {"s":"x,}[]\""},  "c": 1e2 } "#),
            ("c", r#" { "a": {"s":"x,}[]\""}, "b\"x": [1,2]  } "#),
        ] {
            let edited = replace_json_member(raw, name, None).unwrap();
            assert_eq!(edited, expected);
            serde_json::from_str::<serde_json::Value>(&edited).unwrap();
        }
    }

    #[test]
    fn toml_edit_roundtrips_named_table_subtables_and_extra_keys() {
        use McpClientId::*;
        let text = "[mcp_servers.\"a.b\"]\ncommand = 'mcp'\nstartup_timeout_sec = 20\n\n[mcp_servers.\"a.b\".env]\nTOKEN = 'secret'\n";
        let parsed = parse_native(Codex, "a.b", text).unwrap();
        assert_eq!(parsed.text("a.b"), text);
        assert_eq!(
            parse_native(Trae, "a.b", &parsed.text("a.b"))
                .unwrap()
                .definition(),
            parsed.definition()
        );
        assert!(
            McpServerDefinition::from_claude(&json!({"type":"ws","url":"wss://example.test"}))
                .is_none()
        );
        assert!(McpServerDefinition::from_claude(&json!({"extra": true})).is_none());
    }

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
    fn trae_write_creates_dir_and_runner_entry() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".trae").join("traecli.toml");

        codex_write_at(&path, true, "/test/runner-mcp").unwrap();

        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
        );
        let doc: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["runner"]["command"].as_str(),
            Some("/test/runner-mcp")
        );
        let status = codex_status_at(&path, "/test/runner-mcp").unwrap();
        assert!(status.registered);
        assert!(status.matches_current);
    }

    #[test]
    fn trae_write_preserves_auth_hooks_and_other_servers() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("traecli.toml");
        std::fs::write(
            &path,
            "auth_token = \"secret\"\n\n[hooks.state]\nstop = \"trusted\"\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\nargs = []\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

        codex_write_at(&path, true, "/test/runner-mcp").unwrap();
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640,
        );
        let after: toml_edit::DocumentMut =
            std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(after["auth_token"].as_str(), Some("secret"));
        assert_eq!(after["hooks"]["state"]["stop"].as_str(), Some("trusted"));
        assert_eq!(
            after["mcp_servers"]["github"]["command"].as_str(),
            Some("gh-mcp")
        );
        assert_eq!(
            after["mcp_servers"]["runner"]["command"].as_str(),
            Some("/test/runner-mcp")
        );

        codex_write_at(&path, false, "/test/runner-mcp").unwrap();
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640,
        );
        let after_disable: toml_edit::DocumentMut =
            std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert!(after_disable["mcp_servers"].get("runner").is_none());
        assert_eq!(after_disable["auth_token"].as_str(), Some("secret"));
        assert_eq!(
            after_disable["hooks"]["state"]["stop"].as_str(),
            Some("trusted")
        );
        assert_eq!(
            after_disable["mcp_servers"]["github"]["command"].as_str(),
            Some("gh-mcp")
        );
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

    #[cfg(windows)]
    #[test]
    fn agent_configs_preserve_windows_mcp_image_path() {
        let dir = tempfile::tempdir().unwrap();
        let binary = PathBuf::from(r"C:\Users\Agent User\AppData\Roaming\com.wycstudios.runner")
            .join("bin")
            .join(crate::cli_install::MCP_DEST_BIN_NAME);
        let binary = binary.to_str().unwrap();
        assert_eq!(
            binary,
            r"C:\Users\Agent User\AppData\Roaming\com.wycstudios.runner\bin\runner-mcp.exe"
        );
        let claude_path = dir.path().join(".claude.json");
        claude_code_write_at(&claude_path, true, binary).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&claude_path).unwrap()).unwrap();
        assert_eq!(json["mcpServers"]["runner"]["command"], binary);
        let codex_path = dir.path().join(".codex").join("config.toml");
        codex_write_at(&codex_path, true, binary).unwrap();
        let toml: toml_edit::DocumentMut = std::fs::read_to_string(&codex_path)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            toml["mcp_servers"]["runner"]["command"].as_str(),
            Some(binary)
        );
    }
}

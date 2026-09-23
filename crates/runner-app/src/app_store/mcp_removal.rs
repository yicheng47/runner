use std::path::Path;

use runner_backend::error::{Error, Result};
use runner_backend::ops::mcp::{self, McpClientId, McpClientStatus};

use super::AppStore;
use crate::app_settings::AppSettings;

impl AppStore {
    pub(crate) fn initialize_mcp_removal(&mut self) {
        if self.settings.mcp_registrations_removed {
            return;
        }
        let Some(home) = self.home_dir.clone() else {
            return;
        };
        let bridge = self
            .core
            .app_data_dir
            .join("bin")
            .join(runner_backend::cli_install::MCP_DEST_BIN_NAME);
        let report = remove_mcp_registrations(
            &mut self.settings,
            &home,
            || remove_registration_at(McpClientId::ClaudeCode, &home, &bridge),
            || remove_registration_at(McpClientId::Codex, &home, &bridge),
            || remove_registration_at(McpClientId::Trae, &home, &bridge),
            || remove_registration_at(McpClientId::Copilot, &home, &bridge),
        );
        for message in report.logs {
            eprintln!("{message}");
        }
        if report.completed {
            self.save_settings();
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum ClientRemoval {
    Absent,
    Removed(String),
    Preserved(String),
    Deferred(String),
}

#[derive(Debug, Default, Eq, PartialEq)]
struct McpRemovalReport {
    completed: bool,
    logs: Vec<String>,
}

fn remove_mcp_registrations<C, D, T, P>(
    settings: &mut AppSettings,
    home: &Path,
    claude_code: C,
    codex: D,
    trae: T,
    copilot: P,
) -> McpRemovalReport
where
    C: FnOnce() -> Result<ClientRemoval>,
    D: FnOnce() -> Result<ClientRemoval>,
    T: FnOnce() -> Result<ClientRemoval>,
    P: FnOnce() -> Result<ClientRemoval>,
{
    if settings.mcp_registrations_removed {
        return McpRemovalReport {
            completed: true,
            logs: Vec::new(),
        };
    }

    let mut report = McpRemovalReport::default();
    let mut failed = false;
    handle_client(
        settings,
        McpClientId::ClaudeCode,
        home,
        claude_code,
        &mut report,
        &mut failed,
    );
    handle_client(
        settings,
        McpClientId::Codex,
        home,
        codex,
        &mut report,
        &mut failed,
    );
    handle_client(
        settings,
        McpClientId::Trae,
        home,
        trae,
        &mut report,
        &mut failed,
    );
    handle_client(
        settings,
        McpClientId::Copilot,
        home,
        copilot,
        &mut report,
        &mut failed,
    );
    if !failed {
        settings.mcp_registrations_removed = true;
        report.completed = true;
    }
    report
}

fn handle_client(
    settings: &AppSettings,
    client: McpClientId,
    home: &Path,
    operation: impl FnOnce() -> Result<ClientRemoval>,
    report: &mut McpRemovalReport,
    failed: &mut bool,
) {
    if !settings.initialized_mcp_clients.contains(client.key()) {
        report.logs.push(format!(
            "Runner MCP removal left {} unchanged: {} was not initialized by Runner",
            client.config_path(home).display(),
            client.label()
        ));
        return;
    }
    match operation() {
        Ok(ClientRemoval::Absent) => {}
        Ok(ClientRemoval::Removed(message) | ClientRemoval::Preserved(message)) => {
            report.logs.push(message)
        }
        Ok(ClientRemoval::Deferred(message)) => {
            *failed = true;
            report.logs.push(message);
        }
        Err(error) => {
            *failed = true;
            report.logs.push(format!(
                "Runner MCP removal for {} failed: {error}",
                client.key()
            ));
        }
    }
}

fn remove_registration_at(
    client: McpClientId,
    home: &Path,
    bridge: &Path,
) -> Result<ClientRemoval> {
    let path = client.config_path(home);
    let bridge = bridge.to_string_lossy();
    let status = match client {
        McpClientId::ClaudeCode => mcp::claude_code_status_at(&path, &bridge),
        McpClientId::Codex | McpClientId::Trae => mcp::codex_status_at(&path, &bridge),
        McpClientId::Copilot | McpClientId::Antigravity => mcp::copilot_status_at(&path, &bridge),
    };
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Ok(ClientRemoval::Deferred(format!(
                "Runner MCP removal deferred for {}: {error}",
                path.display()
            )))
        }
    };
    if !status.registered {
        return Ok(ClientRemoval::Absent);
    }
    if !status.matches_current {
        return Ok(ClientRemoval::Preserved(format!(
            "Runner MCP removal left {} unchanged: runner points to {}",
            path.display(),
            configured_command(&status)
        )));
    }
    let removed = mcp::remove_runner_entry(client, &path, &bridge).map_err(|error| {
        Error::msg(format!(
            "write {} while removing runner: {error}",
            path.display()
        ))
    })?;
    if !removed {
        return Ok(ClientRemoval::Deferred(format!(
            "Runner MCP removal deferred for {}: runner changed before removal",
            path.display()
        )));
    }
    Ok(ClientRemoval::Removed(format!(
        "Runner MCP removal removed runner from {}",
        path.display()
    )))
}

fn configured_command(status: &McpClientStatus) -> String {
    let mut command = status
        .command
        .clone()
        .unwrap_or_else(|| "(missing command)".into());
    for arg in &status.args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn removes_only_initialized_current_registration_and_records_once() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bridge = temp
            .path()
            .join("app/bin")
            .join(runner_backend::cli_install::MCP_DEST_BIN_NAME);
        let bridge_text = bridge.to_string_lossy();
        let claude = McpClientId::ClaudeCode.config_path(&home);
        let codex = McpClientId::Codex.config_path(&home);
        let trae = McpClientId::Trae.config_path(&home);
        let copilot = McpClientId::Copilot.config_path(&home);
        for path in [&claude, &codex, &trae, &copilot] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        std::fs::write(
            &claude,
            serde_json::to_string(&serde_json::json!({
                "mcpServers": {
                    "runner": {"command": bridge_text.as_ref()},
                    "other": {"command": "other"}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            &codex,
            "[mcp_servers.runner]\ncommand = '/another/runner-mcp'\n",
        )
        .unwrap();
        std::fs::write(&trae, "[mcp_servers.other]\ncommand = 'other'\n").unwrap();
        std::fs::write(
            &copilot,
            serde_json::to_string(&serde_json::json!({
                "mcpServers": {"runner": {"command": bridge_text.as_ref()}}
            }))
            .unwrap(),
        )
        .unwrap();

        let mut settings = AppSettings {
            initialized_mcp_clients: BTreeSet::from([
                "claude_code".into(),
                "codex".into(),
                "trae".into(),
            ]),
            ..Default::default()
        };
        let report = remove_mcp_registrations(
            &mut settings,
            &home,
            || remove_registration_at(McpClientId::ClaudeCode, &home, &bridge),
            || remove_registration_at(McpClientId::Codex, &home, &bridge),
            || remove_registration_at(McpClientId::Trae, &home, &bridge),
            || remove_registration_at(McpClientId::Copilot, &home, &bridge),
        );

        assert!(report.completed);
        assert!(settings.mcp_registrations_removed);
        let claude_value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&claude).unwrap()).unwrap();
        assert!(claude_value["mcpServers"].get("runner").is_none());
        assert_eq!(claude_value["mcpServers"]["other"]["command"], "other");
        assert!(std::fs::read_to_string(&codex)
            .unwrap()
            .contains("/another/runner-mcp"));
        assert_eq!(
            std::fs::read_to_string(&trae).unwrap(),
            "[mcp_servers.other]\ncommand = 'other'\n"
        );
        let copilot_value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&copilot).unwrap()).unwrap();
        assert_eq!(
            copilot_value["mcpServers"]["runner"]["command"],
            bridge_text.as_ref()
        );
        assert_eq!(report.logs.len(), 3);
        for path in [&claude, &codex, &copilot] {
            assert!(report
                .logs
                .iter()
                .any(|line| line.contains(path.to_str().unwrap())));
        }
        assert!(report
            .logs
            .iter()
            .any(|line| line.contains("GitHub Copilot CLI was not initialized by Runner")));

        let settings_path = temp.path().join("settings.json");
        settings.save(&settings_path).unwrap();
        let mut reloaded = AppSettings::load(&settings_path).unwrap();
        let rerun = remove_mcp_registrations(
            &mut reloaded,
            &home,
            || panic!("completed removal must not inspect Claude Code"),
            || panic!("completed removal must not inspect Codex"),
            || panic!("completed removal must not inspect TRAE"),
            || panic!("completed removal must not inspect Copilot"),
        );
        assert!(rerun.completed);
        assert!(rerun.logs.is_empty());
    }

    #[test]
    fn unparseable_config_defers_completion_and_retries() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bridge = temp
            .path()
            .join("app/bin")
            .join(runner_backend::cli_install::MCP_DEST_BIN_NAME);
        let bridge_text = bridge.to_string_lossy();
        let trae = McpClientId::Trae.config_path(&home);
        std::fs::create_dir_all(trae.parent().unwrap()).unwrap();
        std::fs::write(&trae, "[broken").unwrap();
        let mut settings = AppSettings {
            initialized_mcp_clients: BTreeSet::from(["trae".into()]),
            ..Default::default()
        };

        let first = remove_mcp_registrations(
            &mut settings,
            &home,
            || panic!("uninitialized client must not run"),
            || panic!("uninitialized client must not run"),
            || remove_registration_at(McpClientId::Trae, &home, &bridge),
            || panic!("uninitialized client must not run"),
        );
        assert!(!first.completed);
        assert!(!settings.mcp_registrations_removed);
        assert_eq!(std::fs::read_to_string(&trae).unwrap(), "[broken");
        assert!(first
            .logs
            .iter()
            .any(|line| line.contains("Runner MCP removal deferred")
                && line.contains(trae.to_str().unwrap())));

        std::fs::write(
            &trae,
            format!(
                "[mcp_servers.runner]\ncommand = '{}'\n",
                bridge_text.as_ref()
            ),
        )
        .unwrap();
        let second = remove_mcp_registrations(
            &mut settings,
            &home,
            || panic!("uninitialized client must not run"),
            || panic!("uninitialized client must not run"),
            || remove_registration_at(McpClientId::Trae, &home, &bridge),
            || panic!("uninitialized client must not run"),
        );
        assert!(second.completed);
        assert!(settings.mcp_registrations_removed);
        assert!(
            !mcp::codex_status_at(&trae, &bridge_text)
                .unwrap()
                .registered
        );
    }

    #[test]
    fn mismatched_registration_is_a_final_skip() {
        let mut settings = AppSettings {
            initialized_mcp_clients: BTreeSet::from(["codex".into()]),
            ..Default::default()
        };
        let report = remove_mcp_registrations(
            &mut settings,
            Path::new("/tmp/home"),
            || panic!("uninitialized client must not run"),
            || Ok(ClientRemoval::Preserved("points elsewhere".into())),
            || panic!("uninitialized client must not run"),
            || panic!("uninitialized client must not run"),
        );

        assert!(report.completed);
        assert!(settings.mcp_registrations_removed);
        assert!(report.logs.iter().any(|line| line == "points elsewhere"));
    }

    #[test]
    fn write_failure_leaves_completion_unset_and_retries() {
        let mut settings = AppSettings {
            initialized_mcp_clients: BTreeSet::from(["codex".into()]),
            ..Default::default()
        };
        let first = remove_mcp_registrations(
            &mut settings,
            Path::new("/tmp/home"),
            || panic!("uninitialized client must not run"),
            || Err(Error::msg("write /tmp/config.toml failed")),
            || panic!("uninitialized client must not run"),
            || panic!("uninitialized client must not run"),
        );
        assert!(!first.completed);
        assert!(!settings.mcp_registrations_removed);
        assert!(first
            .logs
            .iter()
            .any(|line| line.contains("/tmp/config.toml")));

        let second = remove_mcp_registrations(
            &mut settings,
            Path::new("/tmp/home"),
            || panic!("uninitialized client must not run"),
            || Ok(ClientRemoval::Removed("removed on retry".into())),
            || panic!("uninitialized client must not run"),
            || panic!("uninitialized client must not run"),
        );
        assert!(second.completed);
        assert!(settings.mcp_registrations_removed);
        assert!(second.logs.iter().any(|line| line == "removed on retry"));
    }
}

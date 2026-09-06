use runner_backend::ops::mcp::{mcp_integration_status, mcp_set_integration, McpClientStatus};
use runner_backend::ops::runtime::runtime_catalog;

use super::AppStore;
use crate::app_settings::AppSettings;

impl AppStore {
    pub(crate) fn initialize_mcp_defaults(&mut self) {
        if ["claude_code", "codex", "trae"]
            .iter()
            .all(|client| self.settings.initialized_mcp_clients.contains(*client))
        {
            return;
        }
        let result = (|| {
            let catalog: Vec<_> = runtime_catalog(&self.core)?
                .into_iter()
                .filter(|runtime| {
                    let client = if runtime.name == "claude-code" {
                        "claude_code"
                    } else {
                        runtime.name.as_str()
                    };
                    runtime.available
                        && self
                            .settings
                            .is_agent_enabled(&runtime.name, runtime.default_enabled)
                        && !self.settings.initialized_mcp_clients.contains(client)
                })
                .collect();
            if catalog.is_empty() {
                return Ok(());
            }
            let status = mcp_integration_status(&self.core)?;
            if !std::path::Path::new(&status.binary_path).is_file() {
                return Ok(());
            }
            let mut changed = false;
            for runtime in catalog {
                let (client, client_status) = match runtime.name.as_str() {
                    "claude-code" => ("claude_code", &status.claude_code),
                    "codex" => ("codex", &status.codex),
                    "trae" => ("trae", &status.trae),
                    _ => continue,
                };
                match initialize_client(&mut self.settings, client, client_status, || {
                    mcp_set_integration(&self.core, client, true)
                }) {
                    Ok(initialized) => changed |= initialized,
                    Err(error) => eprintln!("Runner MCP default for {client} failed: {error}"),
                }
            }
            if changed {
                self.save_settings();
            }
            Ok::<_, runner_backend::error::Error>(())
        })();
        if let Err(error) = result {
            eprintln!("Runner MCP defaults failed: {error}");
        }
    }
}

fn initialize_client(
    settings: &mut AppSettings,
    client: &str,
    status: &McpClientStatus,
    enable: impl FnOnce() -> runner_backend::error::Result<()>,
) -> runner_backend::error::Result<bool> {
    if settings.initialized_mcp_clients.contains(client) || status.error.is_some() {
        return Ok(false);
    }
    // A dev or another Runner installation may already own this registration.
    if !status.registered {
        enable()?;
    }
    Ok(settings.initialized_mcp_clients.insert(client.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing_registration() -> McpClientStatus {
        McpClientStatus {
            registered: false,
            matches_current: false,
            command: None,
            args: Vec::new(),
            config_path: String::new(),
            error: None,
        }
    }

    #[test]
    fn initializes_once_and_preserves_a_later_opt_out_after_reload() {
        let mut settings = AppSettings::default();
        let status = missing_registration();
        let mut writes = 0;
        assert!(initialize_client(&mut settings, "codex", &status, || {
            writes += 1;
            Ok(())
        })
        .unwrap());
        assert_eq!(writes, 1);

        let mut reloaded =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!initialize_client(&mut reloaded, "codex", &status, || {
            panic!("must not re-enable an integration the user removed")
        })
        .unwrap());
    }

    #[test]
    fn preserves_existing_registration_and_retries_after_config_or_write_errors() {
        let mut settings = AppSettings::default();
        let mut status = missing_registration();
        status.registered = true;
        assert!(initialize_client(&mut settings, "codex", &status, || {
            panic!("must not replace another Runner registration")
        })
        .unwrap());

        status.registered = false;
        status.error = Some("invalid config".into());
        assert!(!initialize_client(&mut settings, "trae", &status, || {
            panic!("must not overwrite an unreadable config")
        })
        .unwrap());
        status.error = None;
        assert!(initialize_client(&mut settings, "trae", &status, || {
            Err(runner_backend::error::Error::msg("write failed"))
        })
        .is_err());
        assert!(!settings.initialized_mcp_clients.contains("trae"));
        assert!(initialize_client(&mut settings, "trae", &status, || Ok(())).unwrap());
    }
}

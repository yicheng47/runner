use super::helpers::*;
use super::*;
#[cfg(test)]
use crate::golden::config_home as capture_home;
#[cfg(not(test))]
use runner_core::app_paths::home_dir as capture_home;

use runner_core::protocol::runtime_metadata::trae::{PERMISSIONS, SKILL_DIRS};

pub struct Trae;
impl RuntimeAdapter for Trae {
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            ..Default::default()
        }
    }

    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::toml_defaults(&config_path(home))
    }

    fn key_capture(&self) -> KeyCapture {
        KeyCapture::RolloutScan {
            sessions_root: capture_home()
                .map(|home| home.join(".trae").join("cli").join("sessions")),
        }
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::trae::catalog()
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn model_effort_args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        model_reasoning_args(model, effort)
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        positional_first_turn(body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        subcommand_resume(prior_key)
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

#[cfg(test)]
mod tests;

pub(crate) const TRAE_CONFIG_RELATIVE_PATH: &str = ".trae/traecli.toml";
pub(crate) fn config_path(home: &Path) -> PathBuf {
    home.join(TRAE_CONFIG_RELATIVE_PATH)
}

static MCP: McpConfig = McpConfig {
    wire_name: "trae",
    serialized_name: "Trae",
    order: 2,
    config_file: "~/.trae/traecli.toml",
    format: McpFormat::Toml,
    translate: crate::ops::mcp::toml_entry,
    preserve_disabled: false,
    supports_http: true,
};

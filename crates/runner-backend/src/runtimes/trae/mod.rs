use super::catalog::*;
use super::helpers::*;
use super::*;
#[cfg(test)]
use crate::golden::config_home as capture_home;
#[cfg(not(test))]
use runner_core::app_paths::home_dir as capture_home;

static PERMISSIONS: Permissions = Permissions {
    offered: &[PermissionMode::Default, PermissionMode::Bypass],
    strip_flags: &[("--permission-mode", true)],
    equals_on_bool: false,
    variadic_flag: None,
    args: permission_args,
    matches: mode_matches,
    mission_bypass: None,
    strip_on_mission_resume: true,
};
fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::Bypass => strings(&["--permission-mode", "bypass_permissions"]),
        _ => Vec::new(),
    }
}
fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
    let pairs: &[(&str, Option<&str>)] = match mode {
        PermissionMode::Bypass => &[("--permission-mode", Some("bypass_permissions"))],
        PermissionMode::Auto => &[("--permission-mode", Some("auto"))],
        _ => &[],
    };
    !pairs.is_empty()
        && pairs
            .iter()
            .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
}
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
        Some(RuntimeCatalog {
            name: Runtime::Trae,
            display_name: Runtime::Trae.display_name(),
            command: Runtime::Trae.command().unwrap(),
            capabilities: self.capabilities(),
            native_fork: false,
            description: "",
            install_url: "",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: common_efforts(),
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
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

const SKILL_DIRS: &[&str] = &[".trae/skills"];

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
#[allow(non_upper_case_globals)]
impl crate::ops::mcp::McpClientId {
    pub const Trae: Self = Self(Runtime::Trae);
}

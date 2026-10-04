pub(crate) mod copilot_status;
pub(crate) mod copilot_trust;
pub(crate) mod skills;
use super::catalog::*;
use super::helpers::*;
use super::*;

pub(crate) fn copilot_status_args(app_data_dir: &Path) -> Vec<String> {
    if !Copilot
        .status_hooks()
        .is_some_and(|hooks| hooks.supported(cfg!(windows)))
        || !crate::runtimes::copilot::copilot_status::plugin_available(app_data_dir)
    {
        return Vec::new();
    }
    vec![
        "--plugin-dir".into(),
        crate::runtimes::copilot::copilot_status::plugin_dir(app_data_dir)
            .to_string_lossy()
            .into_owned(),
    ]
}
pub(crate) fn copilot_conversation_exists_with_home(
    key: &str,
    override_home: Option<&str>,
) -> bool {
    #[cfg(test)]
    {
        if let Some(home) = override_home {
            return copilot_conversation_exists_at(Path::new(home), key);
        }
        CONVERSATION_HOME.with_borrow(|home| {
            home.as_deref()
                .is_none_or(|home| copilot_conversation_exists_at(&home.join(".copilot"), key))
        })
    }
    #[cfg(not(test))]
    {
        crate::runtimes::copilot::copilot_trust::copilot_home(override_home)
            .is_ok_and(|home| copilot_conversation_exists_at(&home, key))
    }
}

fn copilot_conversation_exists_at(home: &Path, key: &str) -> bool {
    home.join("session-state")
        .join(key)
        .join("events.jsonl")
        .is_file()
}

static PERMISSIONS: Permissions = Permissions {
    offered: &[
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Bypass,
    ],
    strip_flags: &[
        ("--allow-tool", true),
        ("--yolo", false),
        ("--allow-all", false),
        ("--allow-all-tools", false),
        ("--allow-all-paths", false),
        ("--allow-all-urls", false),
    ],
    equals_on_bool: false,
    variadic_flag: Some("--allow-tool"),
    args: permission_args,
    matches: mode_matches,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::AcceptEdits => strings(&["--allow-tool=write"]),
        PermissionMode::Bypass => strings(&["--yolo"]),
        _ => Vec::new(),
    }
}
fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
    if mode == PermissionMode::Bypass {
        return ["--yolo", "--allow-all"]
            .iter()
            .any(|flag| flag_value_matches(args, flag, None));
    }
    let pairs: &[(&str, Option<&str>)] = match mode {
        PermissionMode::AcceptEdits => &[("--allow-tool", Some("write"))],
        PermissionMode::Bypass => &[("--yolo", None)],
        _ => &[],
    };
    !pairs.is_empty()
        && pairs
            .iter()
            .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
}
pub struct Copilot;
impl RuntimeAdapter for Copilot {
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            load: skills::load,
            toggle: Some(skills::set_enabled),
            ..Default::default()
        }
    }

    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            global_skill_toggle: true,
            ..Default::default()
        }
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::json_defaults(&settings_path(home), true)
    }

    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        copilot_home: Option<&str>,
    ) -> crate::error::Result<()> {
        if let Some(cwd) = trust_cwd(session_id, Runtime::Copilot, cwd) {
            copilot_trust::seed_project_trust(cwd, copilot_home)?;
        }
        Ok(())
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Copilot,
            display_name: Runtime::Copilot.display_name(),
            command: Runtime::Copilot.command().unwrap(),
            capabilities: self.capabilities(),
            native_fork: false,
            description: "GitHub Copilot CLI (requires a Copilot subscription)",
            install_url: "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli",
            default_enabled: true,
            models: std::iter::once(default_model_option())
                .chain(
                    [
                        "auto",
                        "claude-sonnet-5",
                        "claude-fable-5.1",
                        "claude-fable-5",
                        "claude-opus-5",
                        "claude-opus-4.8",
                        "claude-opus-4.8-fast",
                        "claude-opus-4.7",
                        "claude-sonnet-4.6",
                        "claude-haiku-4.5",
                        "gpt-5.6-sol",
                        "gpt-5.6-terra",
                        "gpt-5.6-luna",
                        "gpt-5.5",
                        "gpt-5.4",
                        "gpt-5.4-mini",
                        "gpt-5.3-codex",
                        "gpt-5-mini",
                        "mai-code-1.1-flash",
                        "mai-code-1-flash-picker",
                        "gemini-3.8-flash",
                        "gemini-3.7-flash",
                        "gemini-3.6-flash",
                        "gemini-3.5-flash",
                        "grok-4.5",
                        "kimi-k3",
                        "kimi-k2.7-code",
                    ]
                    .into_iter()
                    .map(|model| plain_option(model, model)),
                )
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: SKILL_DIRS,
    update_args: &["update"],
    npm_package: Some("@github/copilot"),
})
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn model_effort_args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        let model = trim_some(model);
        let effort = trim_some(effort);
        let mut out = Vec::new();
        if let Some(m) = model {
            out.push("--model".into());
            out.push(m.to_string());
        }
        if let Some(e) = effort {
            out.push("--effort".into());
            out.push(e.to_string());
        }
        out
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        prefixed_first_turn("-i", body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        assigned_resume(prior_key, false)
    }
    fn mission_dir_args(&self, dir: Option<&Path>) -> Vec<String> {
        add_dir(dir)
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(copilot_conversation_exists_with_home(
            key,
            ctx.role_env.get("COPILOT_HOME").map(String::as_str),
        ))
    }
    fn missing_conversation(&self) -> MissingConversation {
        MissingConversation {
            reuse_key: true,
            resume_on_launch: false,
        }
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.push("--no-auto-update".into());
        out.extend(copilot_status_args(ctx.app_data_dir));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

struct Hooks;
impl StatusHooks for Hooks {
    fn supported(&self, _windows: bool) -> bool {
        true
    }
    fn install(&self, app_data_dir: &Path) {
        if self.supported(cfg!(windows)) {
            if let Err(error) = copilot_status::install_plugin(app_data_dir) {
                log::warn!("install Copilot status plugin: {error}");
            }
        }
    }
    fn env(
        &self,
        _role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String> {
        if !(self.supported(cfg!(windows))) {
            return std::collections::BTreeMap::new();
        }
        status_env(
            copilot_status::PATH_ENV,
            copilot_status::GENERATION_ENV,
            app_data_dir,
            &spec.session_id,
        )
    }
    fn start_watcher(&self, spec: &SpawnSpec) -> Option<Box<dyn HookWatcher>> {
        if !self.supported(cfg!(windows)) {
            return None;
        }
        let path = spec.env.get(copilot_status::PATH_ENV)?;
        let generation = spec.env.get(copilot_status::GENERATION_ENV)?;
        let home =
            match copilot_trust::copilot_home(spec.env.get("COPILOT_HOME").map(String::as_str)) {
                Ok(home) => home,
                Err(error) => {
                    log::warn!(
                        "Copilot status transcript unavailable for {}: {error}",
                        spec.session_id
                    );
                    return None;
                }
            };
        match copilot_status::CopilotStatusWatcher::start(Path::new(path), generation.clone(), home)
        {
            Ok(watcher) => Some(Box::new(watcher)),
            Err(error) => {
                log::warn!(
                    "Copilot status bridge unavailable for {}: {error}",
                    spec.session_id
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;

pub(crate) const COPILOT_SETTINGS_RELATIVE_PATH: &str = ".copilot/settings.json";
pub(crate) fn settings_path(home: &Path) -> PathBuf {
    home.join(COPILOT_SETTINGS_RELATIVE_PATH)
}

const SKILL_DIRS: &[&str] = &[".copilot/skills", ".agents/skills"];

static MCP: McpConfig = McpConfig {
    wire_name: "copilot",
    serialized_name: "Copilot",
    order: 3,
    config_file: "~/.copilot/mcp-config.json",
    format: McpFormat::Json,
    translate: translate_mcp,
    preserve_disabled: false,
    supports_http: true,
};
#[allow(non_upper_case_globals)]
impl crate::ops::mcp::McpClientId {
    pub const Copilot: Self = Self(Runtime::Copilot);
}

fn translate_mcp(
    definition: &crate::ops::mcp::McpServerDefinition,
) -> crate::error::Result<crate::ops::mcp::NativeEntry> {
    let mut value = definition.to_claude();
    if matches!(
        definition,
        crate::ops::mcp::McpServerDefinition::Stdio { .. }
    ) {
        value["type"] = serde_json::json!("local");
        value["tools"] = serde_json::json!(["*"]);
    }
    Ok(crate::ops::mcp::NativeEntry::Claude(value))
}

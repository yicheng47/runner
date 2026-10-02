pub(crate) mod agy_capture;
pub(crate) mod agy_status;
pub(crate) mod agy_trust;
pub(crate) mod models;
pub(crate) mod usage;
use super::catalog::*;
use super::helpers::*;
use super::*;
pub const ANTIGRAVITY_MODELS: &[(&str, &[&str])] = &[
    ("gemini-3.8-flash", &["low", "medium", "high"]),
    ("gemini-3.7-flash", &["low", "medium", "high"]),
    ("gemini-3.6-flash", &["low", "medium", "high"]),
    ("gemini-3.1-pro", &["low", "high"]),
    ("claude-sonnet-4-6", &[]),
    ("claude-opus-4-6-thinking", &[]),
    ("gpt-oss-120b-medium", &[]),
];

pub const ANTIGRAVITY_EFFORTS: &[&str] = &["low", "medium", "high"];

pub(crate) fn antigravity_status_args(app_data_dir: &Path) -> Vec<String> {
    if !Antigravity
        .status_hooks()
        .is_some_and(|hooks| hooks.supported(cfg!(windows)))
        || !crate::runtimes::antigravity::agy_status::hooks_available(app_data_dir)
    {
        return Vec::new();
    }
    vec![
        "--add-dir".into(),
        crate::runtimes::antigravity::agy_status::hooks_dir(app_data_dir)
            .to_string_lossy()
            .into_owned(),
    ]
}
pub fn antigravity_conversation_exists(key: &str) -> bool {
    #[cfg(test)]
    {
        CONVERSATION_HOME.with_borrow(|home| {
            home.as_deref()
                .is_none_or(|home| antigravity_conversation_exists_at(home, key))
        })
    }
    #[cfg(not(test))]
    {
        runner_core::app_paths::home_dir()
            .is_none_or(|home| antigravity_conversation_exists_at(&home, key))
    }
}

fn antigravity_conversation_exists_at(home: &Path, key: &str) -> bool {
    home.join(".gemini/antigravity-cli/conversations")
        .join(format!("{key}.db"))
        .is_file()
}

static PERMISSIONS: Permissions = Permissions {
    offered: &[
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Bypass,
    ],
    strip_flags: &[
        ("--mode", true),
        ("-mode", true),
        ("--dangerously-skip-permissions", false),
        ("-dangerously-skip-permissions", false),
    ],
    equals_on_bool: true,
    variadic_flag: None,
    args: permission_args,
    matches: mode_matches,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::AcceptEdits => strings(&["--mode", "accept-edits"]),
        PermissionMode::Bypass => strings(&["--dangerously-skip-permissions"]),
        _ => Vec::new(),
    }
}
fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
    match mode {
        PermissionMode::Bypass => args
            .iter()
            .any(|arg| go_bool_flag_is_set(arg, "dangerously-skip-permissions")),
        PermissionMode::AcceptEdits => ["--mode", "-mode"]
            .iter()
            .any(|flag| flag_value_matches(args, flag, Some("accept-edits"))),
        _ => false,
    }
}
pub struct Antigravity;
impl RuntimeAdapter for Antigravity {
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn usage(&self) -> Option<&'static UsageSource> {
        Some(&USAGE)
    }

    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            ..Default::default()
        }
    }

    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            effort_needs_launch_model: true,
            ..Default::default()
        }
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::LogTail
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        _copilot_home: Option<&str>,
    ) -> crate::error::Result<()> {
        if let Some(cwd) = trust_cwd(session_id, Runtime::Antigravity, cwd) {
            agy_trust::seed_project_trust(cwd)?;
        }
        Ok(())
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Antigravity,
            display_name: Runtime::Antigravity.display_name(),
            command: Runtime::Antigravity.command().unwrap(),
            capabilities: self.capabilities(),
            native_fork: false,
            description: "Google Antigravity CLI (signs in with a Google account)",
            install_url: "https://antigravity.google/docs/cli/reference",
            default_enabled: true,
            models: std::iter::once(default_model_option())
                .chain(
                    ANTIGRAVITY_MODELS
                        .iter()
                        .map(|(model, efforts)| RuntimeCatalogOption {
                            supported_efforts: Some(
                                efforts.iter().map(|effort| (*effort).into()).collect(),
                            ),
                            ..plain_option(model, model)
                        }),
                )
                .collect(),
            efforts: std::iter::once(default_effort())
                .chain(
                    ANTIGRAVITY_EFFORTS
                        .iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: SKILL_DIRS,
            update_args: &[],
            npm_package: None,
        })
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn model_effort_args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        let model = trim_some(model);
        let effort = trim_some(effort);
        let Some(m) = model else {
            return Vec::new();
        };
        let mut out = vec!["--model".into(), m.to_string()];
        let effort = effort.map(str::to_ascii_lowercase).filter(|e| {
            ANTIGRAVITY_MODELS
                .iter()
                .any(|(id, levels)| *id == m && levels.contains(&e.as_str()))
        });
        if let Some(e) = effort {
            out.push("--effort".into());
            out.push(e);
        }
        out
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        prefixed_first_turn("-i", body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        match prior_key {
            Some(k) if is_uuid(k) => ResumePlan {
                args: vec!["--conversation".into(), k.to_string()],
                prepend: false,
                assigned_key: Some(k.to_string()),
                resuming: true,
            },
            _ => ResumePlan::fresh(),
        }
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels {
            system_prompt: false,
            resend_persona_on_fresh: true,
        }
    }
    fn mission_dir_args(&self, dir: Option<&Path>) -> Vec<String> {
        add_dir(dir)
    }
    fn conversation_exists(&self, key: &str, _ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(antigravity_conversation_exists(key))
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend([
            "--log-file".into(),
            crate::runtimes::antigravity::agy_capture::log_path(ctx.app_data_dir, ctx.session_id)
                .to_string_lossy()
                .into_owned(),
        ]);
        out.extend(antigravity_status_args(ctx.app_data_dir));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

struct Hooks;
impl StatusHooks for Hooks {
    fn supported(&self, windows: bool) -> bool {
        !windows
    }
    fn install(&self, app_data_dir: &Path) {
        if self.supported(cfg!(windows)) {
            if let Err(error) = agy_status::install_hooks(app_data_dir) {
                log::warn!("install Antigravity status hooks: {error}");
            }
        }
    }
    fn env(
        &self,
        _role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        session_id: &str,
    ) -> std::collections::BTreeMap<String, String> {
        if !(self.supported(cfg!(windows))) {
            return std::collections::BTreeMap::new();
        }
        status_env(
            agy_status::PATH_ENV,
            agy_status::GENERATION_ENV,
            app_data_dir,
            session_id,
        )
    }
    fn start_watcher(&self, spec: &SpawnSpec) -> Option<Box<dyn HookWatcher>> {
        if !self.supported(cfg!(windows)) {
            return None;
        }
        let path = spec.env.get(agy_status::PATH_ENV)?;
        let generation = spec.env.get(agy_status::GENERATION_ENV)?;
        match agy_status::AgyStatusWatcher::start(Path::new(path), generation.clone()) {
            Ok(watcher) => Some(Box::new(watcher)),
            Err(error) => {
                log::warn!(
                    "Antigravity status bridge unavailable for {}: {error}",
                    spec.session_id
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;

static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 3,
    method: "models",
    query: models::query,
    config_home: || config_home(None, ".gemini/antigravity-cli"),
};

static USAGE: UsageSource = UsageSource {
    fetch: |command, env, _denied| usage::fetch_antigravity(command, env),
};

const SKILL_DIRS: &[&str] = &[".gemini/antigravity-cli/skills", ".gemini/skills"];

static MCP: McpConfig = McpConfig {
    wire_name: "antigravity",
    serialized_name: "Antigravity",
    order: 4,
    config_file: "~/.gemini/config/mcp_config.json",
    format: McpFormat::Json,
    translate: translate_mcp,
    preserve_disabled: true,
    supports_http: false,
};
#[allow(non_upper_case_globals)]
impl crate::ops::mcp::McpClientId {
    pub const Antigravity: Self = Self(Runtime::Antigravity);
}

fn translate_mcp(
    definition: &crate::ops::mcp::McpServerDefinition,
) -> crate::error::Result<crate::ops::mcp::NativeEntry> {
    // `agy mcp add` writes args, command, disabled, env in this order and
    // omits empty args. Its HTTP entry shape is unprobed.
    let crate::ops::mcp::McpServerDefinition::Stdio { command, args, env } = definition else {
        return Err(crate::error::Error::msg(
            "Antigravity CLI's HTTP entry is not supported yet; add the server with `agy mcp add`",
        ));
    };
    let mut value = serde_json::Map::new();
    if !args.is_empty() {
        value.insert("args".into(), serde_json::json!(args));
    }
    value.insert("command".into(), serde_json::json!(command));
    value.insert("disabled".into(), serde_json::json!(false));
    if !env.is_empty() {
        value.insert("env".into(), serde_json::json!(env));
    }
    Ok(crate::ops::mcp::NativeEntry::Claude(
        serde_json::Value::Object(value),
    ))
}

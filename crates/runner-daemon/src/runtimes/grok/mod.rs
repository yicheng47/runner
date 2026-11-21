use super::helpers::*;
use super::*;
use runner_core::protocol::runtime_metadata::grok::{PERMISSIONS, SKILL_DIRS};

mod conversation;
mod models;
pub(crate) mod status;

pub struct Grok;
impl RuntimeAdapter for Grok {
    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics {
            program_status_authority: true,
            program_status_root_app: Some("grok"),
            program_status_transcript_outcomes: true,
            program_status_cancel_interrupt: true,
            ..Default::default()
        }
    }
    fn terminal_adapter(&self, _pending_turn: Option<bool>) -> Option<Box<dyn TerminalAdapter>> {
        Some(Box::new(ProgramStatusTerminal::default()))
    }
    fn starts_trusted(
        &self,
        _cwd: Option<&Path>,
        _args: &[String],
        _env: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        true
    }
    fn startup_delivery_waits_for_program_status(&self, _pending_turn: bool) -> bool {
        true
    }
    fn probe_uses_shell_env(&self) -> bool {
        true
    }
    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::grok::catalog()
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn discovery_home(&self, env: &crate::shell_path::LoginShellEnv) -> Option<PathBuf> {
        config_dir(
            &env.vars,
            runner_core::app_paths::home_dir().as_deref(),
            None,
        )
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        defaults(&home.join(".grok/config.toml"))
    }
    fn native_defaults_with_env(
        &self,
        home: &Path,
        env: &std::collections::BTreeMap<String, String>,
    ) -> crate::runtime_defaults::RuntimeDefaults {
        let mut defaults = config_dir(env, None, Some(home))
            .map(|dir| defaults(&dir.join("config.toml")))
            .unwrap_or_default();
        defaults.model = models::environment_model(env).or(defaults.model);
        defaults
    }
    fn discovered_efforts(
        &self,
        catalog: &crate::runtime_status::models::ModelCatalog,
    ) -> Option<Vec<crate::ops::runtime::RuntimeCatalogOption>> {
        let mut efforts = vec![runner_core::protocol::runtime_options::default_effort()];
        for level in catalog
            .models
            .iter()
            .filter_map(|model| model.supported_efforts.as_ref())
            .flatten()
        {
            if !efforts.iter().any(|effort| &effort.value == level) {
                efforts.push(crate::runtime_status::models::option(
                    level.clone(),
                    level.clone(),
                    None,
                ));
            }
        }
        Some(efforts)
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            ..Default::default()
        }
    }
    fn launch_role_args(&self, args: &[String]) -> Vec<String> {
        LAUNCH_OPTIONS
            .strip(args)
            .into_iter()
            .filter(|arg| !arg.starts_with("-m"))
            .collect()
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        first_turn_body(body)
            .map(|body| strings(&["--", body]))
            .unwrap_or_default()
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        let prior_key = prior_key.filter(|key| is_uuid(key));
        let key = prior_key
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        ResumePlan {
            args: strings(&[
                if prior_key.is_some() {
                    "--resume"
                } else {
                    "-s"
                },
                &key,
            ]),
            prepend: false,
            assigned_key: Some(key),
            resuming: prior_key.is_some(),
        }
    }
    fn fork_plan(&self, source_key: &str, _source_label: &str) -> Option<ForkPlan> {
        if !is_uuid(source_key) {
            return None;
        }
        let key = uuid::Uuid::new_v4().to_string();
        Some(ForkPlan::Direct(ResumePlan {
            args: strings(&["--resume", source_key, "--fork-session", "-s", &key]),
            prepend: false,
            assigned_key: Some(key),
            resuming: true,
        }))
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        conversation::exists(key, ctx)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::Hook
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels {
            system_prompt: false,
            resend_persona_on_fresh: true,
        }
    }
    fn model_effort_args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        let environment_model = models::environment_model(&Default::default());
        models::args(
            model,
            effort,
            environment_model.as_deref(),
            None,
            config_dir(&Default::default(), None, None).as_deref(),
        )
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut args = strings(&["--trust"]);
        let home = config_dir(ctx.env, ctx.cwd, None);
        let environment_model = models::environment_model(ctx.env);
        let model = models::stored_option(ctx.role_args, &["-m", "--model"]);
        let effort = models::stored_option(ctx.role_args, &["--reasoning-effort", "--effort"]);
        args.extend(models::args(
            ctx.model.or(model),
            ctx.effort.or(effort),
            environment_model.as_deref(),
            ctx.default_model,
            home.as_deref(),
        ));
        args.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        args
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn mcp_path(
        &self,
        home: &Path,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Option<PathBuf> {
        config_dir(env, None, Some(home)).map(|home| home.join("config.toml"))
    }
}

pub(super) fn config_dir(
    env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    #[cfg(test)]
    use crate::golden::config_var_os as var_os;
    #[cfg(not(test))]
    use std::env::var_os;
    let path = env
        .get("GROK_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            var_os("GROK_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| home.map(|home| home.join(".grok")))
        .or_else(|| config_home(None, ".grok"))?;
    Some(if path.is_absolute() {
        path
    } else {
        cwd.map(Path::to_path_buf)
            .or_else(|| home.map(Path::to_path_buf))
            .or_else(runner_core::app_paths::home_dir)?
            .join(path)
    })
}
fn defaults(path: &Path) -> crate::runtime_defaults::RuntimeDefaults {
    let document = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.parse::<toml_edit::DocumentMut>().ok());
    let value = |key: &str| {
        document
            .as_ref()
            .and_then(|doc| doc.get("models"))
            .and_then(|models| models.get(key))
            .and_then(toml_edit::Item::as_str)
            .and_then(|value| trim_some(Some(value)))
            .map(str::to_owned)
    };
    crate::runtime_defaults::RuntimeDefaults {
        model: value("default"),
        effort: value("default_reasoning_effort"),
    }
}
static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 5,
    method: "models",
    query: models::query,
    config_home: || config_dir(&Default::default(), None, None),
};
static MCP: McpConfig = McpConfig {
    wire_name: "grok",
    serialized_name: "Grok",
    order: 5,
    config_file: "~/.grok/config.toml",
    format: McpFormat::Toml,
    translate: models::mcp_entry,
    preserve_disabled: false,
    supports_http: true,
};
struct Hooks;
impl StatusHooks for Hooks {
    fn supported(&self, windows: bool) -> bool {
        !windows
    }
    fn install(&self, _app_data_dir: &Path) {
        #[cfg(not(windows))]
        if let Some(home) = config_dir(&Default::default(), None, None) {
            #[cfg(test)]
            if test_home().is_none() {
                return;
            }
            if let Err(error) = status::install(&home) {
                log::warn!("install Grok status hooks: {error}");
            }
        }
    }
    fn install_with_env(
        &self,
        _app_data_dir: &Path,
        env: &std::collections::BTreeMap<String, String>,
    ) {
        if !self.supported(cfg!(windows)) {
            return;
        }
        #[cfg(test)]
        if test_home().is_none() && !env.contains_key("GROK_HOME") {
            return;
        }
        if let Some(home) = config_dir(env, None, None) {
            if let Err(error) = status::install(&home) {
                log::warn!("install Grok status hooks: {error}");
            }
        }
    }
    fn start_receiver(
        &self,
        spec: &SpawnSpec,
        receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        self.supported(cfg!(windows))
            .then(|| Box::new(status::Watcher::new(receiver, spec)) as Box<dyn HookWatcher>)
    }
    fn env(
        &self,
        _role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String> {
        if !self.supported(cfg!(windows)) {
            return Default::default();
        }
        if let Some(home) = config_dir(&spec.env, spec.cwd.as_deref(), None) {
            #[cfg(test)]
            let install = test_home().is_some() || spec.env.contains_key("GROK_HOME");
            #[cfg(not(test))]
            let install = true;
            if install {
                if let Err(error) = status::install(&home) {
                    log::warn!("install Grok status hooks: {error}");
                }
            }
        }
        hook_env(app_data_dir, &spec.session_id)
    }
}
static LAUNCH_OPTIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[
        ("-m", true),
        ("--model", true),
        ("--reasoning-effort", true),
        ("--effort", true),
    ],
    equals_on_bool: false,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};

#[cfg(test)]
mod tests;

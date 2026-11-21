use super::helpers::*;
use super::*;
use runner_core::protocol::runtime_metadata::opencode::{PERMISSIONS, SKILL_DIRS};
use serde_json::{json, Value};
use std::collections::BTreeMap;

mod config;
pub(crate) mod mcp;
mod models;
pub(crate) mod status;
#[cfg(test)]
mod tests;

pub struct OpenCode;
impl RuntimeAdapter for OpenCode {
    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics {
            program_status_authority: true,
            program_status_root_app: Some("opencode"),
            program_status_transcript_outcomes: true,
            program_status_cancel_interrupt: true,
            program_status_idle_completes: true,
            program_status_suppress_child_tasks: true,
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
    fn classify_startup_error(
        &self,
        output: &str,
    ) -> Option<runner_core::protocol::status::StartupError> {
        let plain = crate::runtimes::custom_flags::strip_ansi(output);
        let rejected_standalone = plain.lines().any(line_rejects_standalone);
        let printed_v1_help = (plain.contains("Commands:") || plain.contains("Usage:"))
            && plain.contains("Options:")
            && !plain.contains("--standalone");
        if !rejected_standalone && !printed_v1_help {
            return None;
        }
        Some(runner_core::protocol::status::StartupError {
            runtime: Runtime::OpenCode,
            message: "This OpenCode executable does not support --standalone. Runner only supports OpenCode v2 (2.1.0 or newer).".into(),
            output: plain.trim().chars().take(512).collect(),
        })
    }
    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::opencode::catalog()
    }
    fn permissions(&self) -> &'static Permissions {
        &PERMISSIONS
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            manual: |_, doc| {
                doc.value("opencode/autoinvoke") == Some("false")
                    || (doc.value("opencode/autoinvoke").is_none()
                        && doc.value("disable-model-invocation") == Some("true"))
            },
            ..Default::default()
        }
    }
    fn skill_roots_with_env(
        &self,
        home: &Path,
        _codex_home: Option<&Path>,
        env: &BTreeMap<String, String>,
    ) -> Option<Vec<PathBuf>> {
        let config =
            config::config_dir(env, Some(home)).unwrap_or_else(|| home.join(".config/opencode"));
        Some(vec![
            config.join("skills"),
            home.join(".agents/skills"),
            home.join(".claude/skills"),
        ])
    }
    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn discovered_efforts(
        &self,
        catalog: &crate::runtime_status::models::ModelCatalog,
    ) -> Option<Vec<crate::ops::runtime::RuntimeCatalogOption>> {
        let mut efforts = vec![runner_core::protocol::runtime_options::default_effort()];
        for variant in catalog
            .models
            .iter()
            .filter_map(|model| model.supported_efforts.as_ref())
            .flatten()
        {
            if !efforts.iter().any(|effort| &effort.value == variant) {
                efforts.push(crate::runtime_status::models::option(
                    variant.clone(),
                    variant.clone(),
                    None,
                ));
            }
        }
        Some(efforts)
    }
    fn discovery_home(&self, env: &crate::shell_path::LoginShellEnv) -> Option<PathBuf> {
        config::config_dir(&env.vars, runner_core::app_paths::home_dir().as_deref())
    }
    fn native_defaults_with_env(
        &self,
        home: &Path,
        env: &BTreeMap<String, String>,
    ) -> crate::runtime_defaults::RuntimeDefaults {
        config::defaults(home, env)
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        config::defaults(home, &Default::default())
    }
    fn launch_role_args(&self, args: &[String]) -> Vec<String> {
        LAUNCH_OPTIONS.strip(args)
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        prefixed_first_turn("--prompt", body)
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels {
            system_prompt: false,
            resend_persona_on_fresh: true,
        }
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        let Some(key) = prior_key.filter(|key| valid_key(key)) else {
            // OpenCode owns fresh IDs; accepted arbitrary IDs can still be
            // rejected by its free-model service. Capture the native key later.
            return ResumePlan::fresh();
        };
        ResumePlan {
            args: strings(&["--session", key]),
            prepend: false,
            assigned_key: Some(key.to_owned()),
            resuming: true,
        }
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut args = strings(&["--standalone"]);
        args.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        args
    }
    fn launch_context_env(&self, ctx: &LaunchContext<'_>) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        let inherited = config::effective_env(ctx.env);
        let raw = inherited
            .get("OPENCODE_CONFIG_CONTENT")
            .map(String::as_str)
            .unwrap_or("{}");
        // Leave malformed native configuration intact so OpenCode reports it.
        if let Ok(mut document) = config::parse(raw) {
            if let Some(object) = document.as_object_mut() {
                object.insert("update".into(), json!("disable"));
                if !ctx.resuming {
                    if let Some(model) = trim_some(ctx.model) {
                        object.insert("model".into(), json!(model));
                        if let Some(variant) = trim_some(ctx.effort) {
                            if let Err(error) =
                                status::model_config(object, ctx.app_data_dir, model, variant)
                            {
                                log::warn!("configure OpenCode model variant: {error}");
                            }
                        }
                    }
                }
                env.insert("OPENCODE_CONFIG_CONTENT".into(), document.to_string());
            }
        }
        env
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        config::conversation_exists(key, ctx)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::Hook
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn mcp_json_parser(&self) -> Option<McpJsonParser> {
        Some(mcp::definition)
    }
    fn read_mcp_entries(
        &self,
        path: &Path,
    ) -> Option<crate::error::Result<BTreeMap<String, crate::ops::mcp::NativeEntry>>> {
        Some(mcp::read(path))
    }
    fn write_mcp_entry(
        &self,
        path: &Path,
        name: &str,
        entry: Option<crate::ops::mcp::NativeEntry>,
        merge: bool,
    ) -> Option<crate::error::Result<()>> {
        Some(mcp::write(path, name, entry, merge))
    }
    fn mcp_path(&self, home: &Path, env: &BTreeMap<String, String>) -> Option<PathBuf> {
        config::global_config_path(home, env)
    }
}
pub(super) fn valid_key(key: &str) -> bool {
    key.starts_with("ses_")
        && key.len() > 4
        && key.len() <= 128
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 6,
    method: "private serve --stdio; GET /api/model",
    query: models::query,
    config_home: || {
        config::config_dir(
            &Default::default(),
            runner_core::app_paths::home_dir().as_deref(),
        )
    },
};
static MCP: McpConfig = McpConfig {
    wire_name: "opencode",
    serialized_name: "OpenCode",
    order: 6,
    config_file: "~/.config/opencode/opencode.jsonc",
    format: McpFormat::Json,
    translate: mcp::entry,
    preserve_disabled: true,
    supports_http: true,
};
static LAUNCH_OPTIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[
        ("--session", true),
        ("-s", true),
        ("--continue", false),
        ("-c", false),
        ("--prompt", true),
        ("--standalone", false),
        ("--server", true),
        ("--model", true),
        ("-m", true),
        ("--variant", true),
    ],
    equals_on_bool: true,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
struct Hooks;
impl StatusHooks for Hooks {
    fn supported(&self, _windows: bool) -> bool {
        true
    }
    fn install(&self, app_data_dir: &Path) {
        if let Err(error) = status::install(app_data_dir) {
            log::warn!("install OpenCode TUI plugin: {error}");
        }
    }
    fn env(
        &self,
        _args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> BTreeMap<String, String> {
        let mut env = hook_env(app_data_dir, &spec.session_id);
        if let Err(error) = status::launch_config(app_data_dir, &spec.env).map(|config| {
            env.insert("OPENCODE_CLI_CONFIG_CONTENT".into(), config);
        }) {
            log::warn!("configure OpenCode TUI plugin: {error}");
        }
        env
    }
    fn start_receiver(
        &self,
        spec: &SpawnSpec,
        receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        Some(Box::new(status::Watcher::new(receiver, spec)))
    }
}

fn line_rejects_standalone(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "unknown arguments",
        "unknown argument",
        "unknown option",
        "unknown flag",
        "unrecognized option",
        "unrecognized argument",
        "unexpected argument",
    ];
    PHRASES.iter().any(|phrase| {
        lower.find(phrase).is_some_and(|idx| {
            let tail = lower[idx + phrase.len()..]
                .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ':');
            tail.split_whitespace().next().is_some_and(|arg| {
                arg.trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | ',' | '.'))
                    .trim_start_matches('-')
                    == "standalone"
            })
        })
    })
}

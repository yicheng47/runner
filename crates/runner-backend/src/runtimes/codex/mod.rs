pub(crate) mod codex_status;
pub(crate) mod codex_trust;
pub(crate) mod models;
pub(crate) mod skills;
mod terminal;
pub(crate) mod usage;
use super::catalog::*;
use super::helpers::*;
use super::*;
#[cfg(test)]
use crate::golden::config_home as capture_home;
#[cfg(not(test))]
use runner_core::app_paths::home_dir as capture_home;

pub(crate) fn inject_codex_hooks(args: &[String], windows: bool) -> bool {
    if !Codex
        .status_hooks()
        .is_some_and(|hooks| hooks.supported(windows))
    {
        return false;
    }
    for (index, arg) in args.iter().enumerate() {
        if arg == "--disable=hooks"
            || (arg == "--disable" && args.get(index + 1).is_some_and(|value| value == "hooks"))
        {
            return false;
        }
        let config = if arg == "-c" || arg == "--config" {
            args.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c"))
        };
        if let Some(config) = config {
            // Repeated -c keys replace the invocation layer, including quoted TOML keys.
            let config = config.strip_prefix('=').unwrap_or(config);
            let parsed = config.parse::<toml_edit::DocumentMut>().or_else(|_| {
                // Codex also accepts bare string values, e.g. -c model=gpt-5.4.
                format!(
                    "{}=true",
                    config.split_once('=').map_or(config, |(key, _)| key)
                )
                .parse::<toml_edit::DocumentMut>()
            });
            let Ok(config) = parsed else {
                return false;
            };
            if config.contains_key("hooks")
                || config
                    .get("features")
                    .and_then(|features| features.get("hooks"))
                    .and_then(toml_edit::Item::as_bool)
                    == Some(false)
            {
                return false;
            }
        }
    }
    true
}

pub(crate) fn codex_status_args(
    role_args: &[String],
    app_data_dir: &Path,
    session_id: &str,
) -> Vec<String> {
    if !inject_codex_hooks(role_args, cfg!(windows)) {
        return Vec::new();
    }
    let path = crate::session::hook_feed::status_path(app_data_dir, session_id);
    let mut args = vec![
        "--enable".into(),
        "hooks".into(),
        "--dangerously-bypass-hook-trust".into(),
    ];
    for event in crate::runtimes::codex::codex_status::EVENTS {
        let command = toml_edit::Value::from(crate::runtimes::codex::codex_status::hook_command(
            &path, event,
        ));
        args.extend([
            "-c".into(),
            format!("hooks.{event}=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"),
        ]);
    }
    args
}

static PERMISSIONS: Permissions = Permissions {
    offered: &[
        PermissionMode::Default,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ],
    strip_flags: &[("--ask-for-approval", true), ("--sandbox", true)],
    equals_on_bool: false,
    variadic_flag: None,
    args: permission_args,
    matches: mode_matches,
    mission_bypass: Some(&[
        "--ask-for-approval",
        "never",
        "--sandbox",
        "danger-full-access",
    ]),
    strip_on_mission_resume: false,
};
fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::Auto => strings(&[
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write",
        ]),
        PermissionMode::Bypass => strings(&[
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
        ]),
        _ => Vec::new(),
    }
}
fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
    let pairs: &[(&str, Option<&str>)] = match mode {
        PermissionMode::Auto => &[
            ("--ask-for-approval", Some("on-request")),
            ("--sandbox", Some("workspace-write")),
        ],
        PermissionMode::Bypass => &[
            ("--ask-for-approval", Some("never")),
            ("--sandbox", Some("workspace-write")),
        ],
        _ => &[],
    };
    !pairs.is_empty()
        && pairs
            .iter()
            .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
}
pub struct Codex;
impl RuntimeAdapter for Codex {
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn usage(&self) -> Option<&'static UsageSource> {
        Some(&USAGE)
    }

    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            load: skills::load,
            toggle: Some(skills::set_enabled),
            manual: skills::manual,
            ..Default::default()
        }
    }

    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            usage: true,
            global_skill_toggle: true,
            skill_toggle_requires_marker: true,
            codex_speed: true,
            ..Default::default()
        }
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::toml_defaults(&config_path(home))
    }

    fn terminal_adapter(&self, pending_turn: Option<bool>) -> Option<Box<dyn TerminalAdapter>> {
        pending_turn.map(|pending| {
            Box::new(terminal::CodexTerminal::new(pending)) as Box<dyn TerminalAdapter>
        })
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::RolloutScan {
            sessions_root: capture_home().map(|home| home.join(".codex").join("sessions")),
        }
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        _copilot_home: Option<&str>,
    ) -> crate::error::Result<()> {
        if let Some(cwd) = trust_cwd(session_id, Runtime::Codex, cwd) {
            codex_trust::seed_project_trust(cwd)?;
        }
        Ok(())
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        let mut codex_efforts = common_efforts();
        codex_efforts.push(option(
            "max",
            "Max",
            "Maximum reasoning depth for the hardest problems.",
        ));
        codex_efforts.push(option(
            "ultra",
            "Ultra",
            "Maximum reasoning with automatic task delegation.",
        ));
        Some(RuntimeCatalog {
            name: Runtime::Codex,
            display_name: Runtime::Codex.display_name(),
            command: Runtime::Codex.command().unwrap(),
            capabilities: self.capabilities(),
            native_fork: true,
            description: "OpenAI Codex CLI",
            install_url: "https://developers.openai.com/codex/cli",
            default_enabled: true,
            models: vec![
                default_model_option(),
                option(
                    "gpt-6-astra",
                    "gpt-6-astra",
                    "Our most capable model for complex, demanding work.",
                ),
                option(
                    "gpt-5.6-sol",
                    "gpt-5.6-sol",
                    "Reliable agentic workhorse for everyday tasks.",
                ),
                option(
                    "gpt-5.6-terra",
                    "gpt-5.6-terra",
                    "Balanced agentic coding model for everyday work.",
                ),
                option(
                    "gpt-5.6-luna",
                    "gpt-5.6-luna",
                    "Fast and affordable agentic coding model.",
                ),
                option(
                    "gpt-5.5",
                    "gpt-5.5",
                    "Frontier model for complex coding, research, and real-world work.",
                ),
                option("gpt-5.4", "gpt-5.4", "Strong model for everyday coding."),
                option(
                    "gpt-5.4-mini",
                    "gpt-5.4-mini",
                    "Small, fast, and cost-efficient model for simpler coding tasks.",
                ),
                option(
                    "gpt-5.3-codex-spark",
                    "gpt-5.3-codex-spark",
                    "Ultra-fast coding model.",
                ),
            ],
            efforts: codex_efforts,
            skills_dirs: SKILL_DIRS,
            update_args: &["update"],
            npm_package: Some("@openai/codex"),
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
    fn fork_plan(&self, source_key: &str, source_label: &str) -> Option<ForkPlan> {
        if !is_uuid(source_key) {
            return None;
        }
        let note = format!("This chat was forked from '{source_label}'.");
        Some(ForkPlan::Headless {
            args: vec![
                "exec".into(),
                "fork".into(),
                source_key.to_string(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                note,
            ],
            source_key: source_key.to_string(),
        })
    }
    fn mission_dir_args(&self, dir: Option<&Path>) -> Vec<String> {
        add_dir(dir)
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend(strings(&["-c", "check_for_update_on_startup=false"]));
        if let Some(speed) = ctx.codex_speed {
            out.extend([
                "-c".into(),
                format!("service_tier={}", speed.service_tier()),
            ]);
        }
        out.extend(codex_status_args(
            ctx.role_args,
            ctx.app_data_dir,
            ctx.session_id,
        ));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

struct Hooks;
impl StatusHooks for Hooks {
    fn supported(&self, _windows: bool) -> bool {
        true
    }
    fn tracks_pending_turn(&self) -> bool {
        true
    }
    fn env(
        &self,
        role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        session_id: &str,
    ) -> std::collections::BTreeMap<String, String> {
        if !(inject_codex_hooks(role_args, cfg!(windows))) {
            return std::collections::BTreeMap::new();
        }
        status_env(
            codex_status::PATH_ENV,
            codex_status::GENERATION_ENV,
            app_data_dir,
            session_id,
        )
    }
    fn start_watcher(&self, spec: &SpawnSpec) -> Option<Box<dyn HookWatcher>> {
        if !self.supported(cfg!(windows)) {
            return None;
        }
        let path = spec.env.get(codex_status::PATH_ENV)?;
        let generation = spec.env.get(codex_status::GENERATION_ENV)?;
        match codex_status::CodexStatusWatcher::start(Path::new(path), generation.clone()) {
            Ok(watcher) => Some(Box::new(watcher)),
            Err(error) => {
                log::warn!(
                    "Codex status bridge unavailable for {}: {error}",
                    spec.session_id
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;

pub(crate) const CODEX_CONFIG_RELATIVE_PATH: &str = ".codex/config.toml";
pub(crate) fn config_path(home: &Path) -> PathBuf {
    home.join(CODEX_CONFIG_RELATIVE_PATH)
}

static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 0,
    method: "debug models",
    query: models::query,
    config_home: || config_home(Some("CODEX_HOME"), ".codex"),
};

static USAGE: UsageSource = UsageSource {
    fetch: |command, env, _denied| usage::fetch_codex(command, env),
};

const SKILL_DIRS: &[&str] = &[".agents/skills", ".codex/skills"];

static MCP: McpConfig = McpConfig {
    wire_name: "codex",
    serialized_name: "Codex",
    order: 1,
    config_file: "~/.codex/config.toml",
    format: McpFormat::Toml,
    translate: crate::ops::mcp::toml_entry,
    preserve_disabled: false,
    supports_http: true,
};
#[allow(non_upper_case_globals)]
impl crate::ops::mcp::McpClientId {
    pub const Codex: Self = Self(Runtime::Codex);
}

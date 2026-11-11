pub(crate) mod capture;
pub(crate) mod codex_status;
pub(crate) mod codex_trust;
mod conversation;
pub(crate) mod fork;
pub(crate) mod models;
mod program_status;
pub(crate) mod skills;
mod terminal;
pub(crate) mod usage;
use super::helpers::*;
use super::*;
#[cfg(test)]
use crate::golden::config_home as capture_home;
#[cfg(test)]
use crate::golden::config_var_os as capture_var_os;
#[cfg(not(test))]
use runner_core::app_paths::home_dir as capture_home;
#[cfg(not(test))]
use std::env::var_os as capture_var_os;

fn capture_sessions_root(
    env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    codex_home(env, cwd).map(|home| home.join("sessions"))
}

fn probe_home(ctx: &ProbeContext<'_>) -> Option<PathBuf> {
    #[cfg(test)]
    if !ctx.role_env.contains_key("CODEX_HOME") && crate::runtimes::test_home().is_none() {
        return None;
    }
    let env = ctx
        .role_env
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    codex_home(&env, ctx.cwd.map(Path::new))
}

/// The Codex home a session spawned with `env` in `cwd` reads.
fn codex_home(
    env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let home = env
        .get("CODEX_HOME")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            capture_var_os("CODEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| capture_home().map(|home| home.join(".codex")))?;
    let home = if home.is_absolute() {
        home
    } else {
        cwd.map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())?
            .join(home)
    };
    Some(home)
}

/// Whether `args` move Codex to another folder (`-C`/`--cd`) or override its project
/// trust (`-c projects…`), so the spawn's own folder and config no longer decide it.
fn overrides_trust_lookup(args: &[String]) -> bool {
    let projects = |value: &str| value.trim_start().starts_with("projects");
    args.iter().enumerate().any(|(index, arg)| {
        let next = || args.get(index + 1).is_some_and(|value| projects(value));
        match arg.as_str() {
            "-C" | "--cd" => true,
            "-c" | "--config" => next(),
            _ => {
                arg.starts_with("--cd=")
                    || (arg.starts_with("-C") && !arg.starts_with("--"))
                    || arg.strip_prefix("--config=").is_some_and(projects)
                    || (arg.starts_with("-c") && !arg.starts_with("--") && projects(&arg[2..]))
            }
        }
    })
}

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
    _session_id: &str,
) -> Vec<String> {
    codex_hook_args(role_args, app_data_dir, true)
}

fn codex_hook_args(role_args: &[String], app_data_dir: &Path, status: bool) -> Vec<String> {
    if !inject_codex_hooks(role_args, cfg!(windows)) {
        return Vec::new();
    }
    use runner_core::protocol::hook;
    let mut args = vec![
        "--enable".into(),
        "hooks".into(),
        "--dangerously-bypass-hook-trust".into(),
    ];
    let executable =
        toml_edit::Value::from(hook_executable(app_data_dir).to_string_lossy().as_ref());
    args.extend([
        "-c".into(),
        format!("mcp_servers.runner_hooks={{command={executable},args=[\"hook\",\"serve\"],env_vars=[\"{}\",\"{}\",\"{}\"],required=false,enabled=true}}", hook::ENDPOINT_ENV, hook::SESSION_ENV, hook::GENERATION_ENV),
    ]);
    for event in crate::runtimes::codex::codex_status::EVENTS {
        if !status && !matches!(*event, "SessionStart" | "UserPromptSubmit") {
            continue;
        }
        if *event == "SessionEnd" {
            let command = toml_edit::Value::from(hook_report_command(
                app_data_dir,
                Runtime::Codex,
                event,
                cfg!(windows),
            ));
            args.extend([
                "-c".into(),
                format!(
                    "hooks.{event}=[{{hooks=[{{type=\"command\",command={command},timeout=1}}]}}]"
                ),
            ]);
            continue;
        }
        let mut input = toml_edit::InlineTable::new();
        for field in ["hook_event_name", "session_id", "transcript_path"] {
            input.insert(field, format!("${{{field}}}").into());
        }
        if *event == "SessionStart" {
            input.insert("source", "${source}".into());
        } else {
            input.insert("turn_id", "${turn_id}".into());
        }
        if matches!(*event, "PreCompact" | "PostCompact") {
            input.insert("trigger", "${trigger}".into());
        }
        args.extend([
            "-c".into(),
            format!("hooks.{event}=[{{hooks=[{{type=\"mcp_tool\",server=\"runner_hooks\",tool=\"report\",input={input},timeout=2}}]}}]"),
        ]);
    }
    args
}

use runner_core::protocol::runtime_metadata::codex::{PERMISSIONS, SKILL_DIRS};

pub struct Codex;
impl RuntimeAdapter for Codex {
    fn skill_roots_with_env(
        &self,
        home: &Path,
        codex_home: Option<&Path>,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> Option<Vec<PathBuf>> {
        let codex_home = codex_home?;
        Some(
            SKILL_DIRS
                .iter()
                .map(
                    |relative| match Path::new(relative).strip_prefix(".codex") {
                        Ok(suffix) => codex_home.join(suffix),
                        Err(_) => home.join(relative),
                    },
                )
                .collect(),
        )
    }
    fn classify_startup_error(
        &self,
        output: &str,
    ) -> Option<runner_core::protocol::status::StartupError> {
        let (_, cause) = output.split_once("Error loading config.toml:")?;
        let cause = cause.trim();
        let detail = cause
            .lines()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(512)
            .collect::<String>();
        let message = if cause.contains("unknown variant `mcp_tool`") {
            "This Codex does not recognize Runner's mcp_tool hook type.".into()
        } else {
            format!("Codex could not load Runner's configuration: {detail}")
        };
        Some(runner_core::protocol::status::StartupError {
            runtime: Runtime::Codex,
            message,
            output: cause.to_owned(),
        })
    }

    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics {
            startup_ready_batch: true,
            native_attention: true,
            reject_events_after_interruption: true,
            reject_completions_after_outcome: true,
            compaction: CompactionPolicy::KeepWorking,
            interrupted_turn_unavailable: true,
            ended_session_keeps_ready: true,
            ..Default::default()
        }
    }
    fn capture_prompt(
        &self,
        session_id: &str,
        first_turn: Option<String>,
    ) -> (Option<String>, Option<String>) {
        capture::mark_prompt(session_id, first_turn)
    }
    fn start_rollout_capture(&self, request: RolloutCaptureRequest) {
        capture::spawn_capture(request);
    }
    fn materialize_fork(
        &self,
        spec: &SpawnSpec,
        plan: &ForkPlan,
        timeout: Duration,
    ) -> crate::error::Result<String> {
        fork::materialize(spec, plan, timeout)
    }
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
        runner_core::protocol::runtime_metadata::codex::capabilities()
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::toml_defaults(&config_path(home))
    }

    fn terminal_adapter(&self, pending_turn: Option<bool>) -> Option<Box<dyn TerminalAdapter>> {
        pending_turn.map(|pending| {
            Box::new(terminal::CodexTerminal::new(pending)) as Box<dyn TerminalAdapter>
        })
    }
    fn startup_delivery_uses_hooks(&self, _pending_turn: bool) -> bool {
        true
    }
    /// A native fork or a resume can start without SessionStart.
    fn startup_hook_on_reopen(&self) -> bool {
        false
    }
    fn starts_trusted(
        &self,
        cwd: Option<&Path>,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        !overrides_trust_lookup(args)
            && cwd.zip(codex_home(env, cwd)).is_some_and(|(cwd, home)| {
                codex_trust::trust_decided(cwd, &home.join("config.toml"))
            })
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::RolloutScan {
            sessions_root: capture_sessions_root(&Default::default(), None),
        }
    }
    fn key_capture_for_spawn(&self, spec: &SpawnSpec) -> KeyCapture {
        KeyCapture::RolloutScan {
            sessions_root: capture_sessions_root(&spec.env, spec.cwd.as_deref()),
        }
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        _copilot_home: Option<&str>,
        _claude_config_dir: Option<&str>,
    ) -> crate::error::Result<()> {
        let hook_server = codex_trust::seed_hook_server();
        if let Some(cwd) = trust_cwd(session_id, Runtime::Codex, cwd) {
            codex_trust::seed_project_trust(cwd)?;
        }
        hook_server
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::codex::catalog()
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
    fn probe_uses_shell_env(&self) -> bool {
        true
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        conversation::conversation_exists(&probe_home(ctx)?, key)
    }
    fn recover_conversation(
        &self,
        session_id: &str,
        missing_key: &str,
        ctx: &ProbeContext<'_>,
    ) -> Option<String> {
        conversation::recover_conversation(&probe_home(ctx)?, session_id, missing_key)
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend(strings(&["-c", "check_for_update_on_startup=false"]));
        // Codex 0.161 dropped the thread title from its default terminal
        // title, and the chat name is read from it.
        out.extend(strings(&["-c", CODEX_TERMINAL_TITLE]));
        if let Some(speed) = ctx.codex_speed {
            out.extend([
                "-c".into(),
                format!("service_tier={}", speed.service_tier()),
            ]);
        }
        if ctx
            .env
            .get(REMOTE_PLUGIN_ENV)
            .is_some_and(|value| value == "false")
        {
            out.extend(strings(&["-c", "features.remote_plugin=false"]));
        }
        out.extend(if status_hooks_enabled(ctx.env) {
            codex_status_args(ctx.role_args, ctx.app_data_dir, ctx.session_id)
        } else {
            codex_hook_args(ctx.role_args, ctx.app_data_dir, false)
        });
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

struct Hooks;
impl StatusHooks for Hooks {
    fn start_receiver(
        &self,
        _spec: &SpawnSpec,
        receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        Some(Box::new(codex_status::CodexStatusWatcher::from_receiver(
            receiver,
        )))
    }
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
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String> {
        if !(inject_codex_hooks(role_args, cfg!(windows))) {
            return std::collections::BTreeMap::new();
        }
        hook_env(app_data_dir, &spec.session_id)
    }
}

#[cfg(test)]
mod tests;

pub(crate) const CODEX_CONFIG_RELATIVE_PATH: &str = ".codex/config.toml";
pub(crate) const CODEX_TERMINAL_TITLE: &str =
    r#"tui.terminal_title=["activity","thread-title","project-name"]"#;
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

#[cfg(test)]
mod startup_error_tests {
    use super::*;
    #[test]
    fn config_failure_is_classified_but_normal_output_is_not() {
        let adapter = crate::runtimes::adapter(Runtime::Codex);
        let field_error = "Error loading config.toml: unknown variant `mcp_tool`, expected one of `command`, `prompt`, `agent` in `hooks`";
        let error = adapter.classify_startup_error(field_error).unwrap();
        assert_eq!(error.runtime, Runtime::Codex);
        assert!(error.output.contains("unknown variant `mcp_tool`"));
        assert_eq!(
            error.message,
            "This Codex does not recognize Runner's mcp_tool hook type."
        );
        let future = adapter.classify_startup_error("Error loading config.toml: invalid type: integer, expected a string in hooks.command").unwrap();
        assert!(future
            .message
            .contains("invalid type: integer, expected a string in hooks.command"));
        let bounded = adapter
            .classify_startup_error(&format!(
                "Error loading config.toml: {}",
                "x".repeat(10_000)
            ))
            .unwrap();
        assert!(bounded.message.len() < 600);
        assert!(adapter
            .classify_startup_error("Interrupted by user")
            .is_none());
        assert!(crate::runtimes::adapter(Runtime::Pi)
            .classify_startup_error(field_error)
            .is_none());
    }
}

pub(crate) mod launch;
pub(crate) mod models;
pub(crate) mod onboarding;
pub(crate) mod skills;
mod trust;
pub(crate) mod usage;
/// Minimum spacing between consecutive `claude-code` PTY launches.
/// Long enough for one claude's OAuth refresh round-trip (network
/// POST to api.anthropic.com plus keychain write) to land before a
/// sibling spawn reads the same refresh token. Refresh tokens are
/// conventionally single-use, so concurrent refresh from N parallel
/// claudes causes `invalid_grant` on the losers and forces relogin
/// in those panes. See issue #171.
///
/// Conservative default at 1500ms — covers typical 100-500ms
/// round-trips with margin for slow networks. A user spawning a
/// 3-slot mission pays ~3s of wall clock for the gate (1.5s × 2
/// post-first-spawn waits); a 7-slot werewolf pays ~9s.
///
/// **First spawn through pays zero**: the gate is deadline-based,
/// not RAII-on-drop. It only sleeps when a prior claude spawned
/// within the last GRACE — single direct chats and cold-start
/// mission starts see ~0ms overhead. Scoped to claude-code only;
/// codex / other runtimes bypass.
///
/// Zeroed under `#[cfg(test)]` so existing claude-code path tests
/// don't pay the wall-clock tax. Pure-function `compute_gate_wait`
/// covers the wait-math in tests with explicit grace values.
#[cfg(not(test))]
const CLAUDE_LAUNCH_GATE_GRACE: Duration = Duration::from_millis(1500);
#[cfg(test)]
const CLAUDE_LAUNCH_GATE_GRACE: Duration = Duration::from_millis(0);

pub(crate) mod claude_status;
use super::helpers::*;
use super::*;
use std::ffi::OsString;

pub(crate) fn inject_claude_settings(role_args: &[String]) -> bool {
    !role_args
        .iter()
        .any(|arg| arg == "--settings" || arg.starts_with("--settings="))
}

pub(crate) fn claude_settings_args(
    role_args: &[String],
    app_data_dir: &Path,
    _runner_session_id: &str,
) -> Vec<String> {
    if !inject_claude_settings(role_args) {
        return Vec::new();
    }
    // Runner's terminal answers the background-colour query with the
    // live palette, so `auto` is the one theme value that never fights
    // the app: Claude Code paints light on Runner Light and dark on
    // Carbon, whatever the user's own config says.
    let mut settings = serde_json::json!({
        "tui": "fullscreen",
        "theme": "auto",
        "hooks": {},
    });
    for event in [
        "SessionStart",
        "PermissionRequest",
        "PermissionDenied",
        "PostToolUseFailure",
        "Elicitation",
        "ElicitationResult",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        if !ClaudeCode
            .status_hooks()
            .is_some_and(|hooks| hooks.supported(cfg!(windows)))
        {
            continue;
        }
        let mut entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": hook_report_command(app_data_dir, Runtime::ClaudeCode, event, false),
                "timeout": HOOK_TIMEOUT_SECS,
            }],
        });
        if event == "Notification" {
            entry["matcher"] =
                serde_json::json!("^(idle_prompt|permission_prompt|elicitation_dialog)$");
        }
        if let Some(entries) = settings["hooks"][event].as_array_mut() {
            entries.push(entry);
        } else {
            settings["hooks"][event] = serde_json::json!([entry]);
        }
    }
    // Claude Code gates `--permission-mode bypassPermissions` behind a
    // first-use consent dialog. Nobody is watching a Bypass spawn to
    // answer it (that is what Bypass means here), so acknowledge it in
    // the per-process flag settings rather than in the user's config.
    if PERMISSIONS.infer(role_args) == PermissionMode::Bypass {
        settings["skipDangerousModePermissionPrompt"] = serde_json::Value::Bool(true);
    }
    vec![
        "--settings".into(),
        serde_json::to_string(&settings).expect("Claude settings must serialize"),
    ]
}

/// Claude Code's `~/.claude/projects/<dir>` name for `cwd` on every
/// platform, as of Claude Code 2.1.300: each UTF-16 unit outside
/// `[A-Za-z0-9]` becomes `-` (so an emoji becomes `--`), and a name over
/// 200 units keeps its first 200 followed by `-` and a base-36 hash of `cwd`.
pub(crate) fn claude_code_project_dir(cwd: &str) -> String {
    const MAX_LEN: usize = 200;
    let encoded: String = cwd
        .encode_utf16()
        .map(|unit| match char::from_u32(unit.into()) {
            Some(c) if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    if encoded.len() <= MAX_LEN {
        return encoded;
    }
    let hash = cwd.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    });
    let mut value = hash.unsigned_abs();
    let mut suffix = Vec::new();
    loop {
        suffix.push(char::from_digit(value % 36, 36).expect("base-36 digit"));
        value /= 36;
        if value == 0 {
            break;
        }
    }
    let suffix: String = suffix.into_iter().rev().collect();
    format!("{}-{suffix}", &encoded[..MAX_LEN])
}

const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// The `CLAUDE_CONFIG_DIR` in a spawn environment, given its entries lowest
/// layer first. On Windows `CommandBuilder` folds the case of names and
/// `spec.env` applies in name order, so among spellings the greatest name wins.
pub(crate) fn spawned_config_dir<'a>(
    env: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Option<&'a str> {
    env.into_iter()
        .filter(|(name, _)| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(CONFIG_DIR_VAR)
            } else {
                *name == CONFIG_DIR_VAR
            }
        })
        .max_by_key(|(name, _)| *name)
        .map(|(_, value)| value.as_str())
}

/// The `CLAUDE_CONFIG_DIR` the agent process will see. `spawned` is the value
/// in the spawn environment, which replaces what the child would inherit; an
/// empty value counts as unset, even when it hides an inherited one.
pub(crate) fn agent_config_dir(spawned: Option<&str>) -> Option<OsString> {
    spawned
        .map(OsString::from)
        .or_else(inherited_config_dir)
        .filter(|dir| !dir.is_empty())
}

/// The variable in the base environment `portable-pty` gives a child, which on
/// Windows includes registry values set after `runnerd` started.
fn inherited_config_dir() -> Option<OsString> {
    #[cfg(not(test))]
    let inherited = portable_pty::CommandBuilder::new("claude")
        .get_env(CONFIG_DIR_VAR)
        .map(ToOwned::to_owned);
    // Tests must not depend on the developer's own Claude config folder (#1159).
    #[cfg(test)]
    let inherited = crate::golden::injected_config_var(CONFIG_DIR_VAR);
    inherited
}

/// The folder Claude keeps this chat's transcripts in.
fn conversation_folder(ctx: &ProbeContext<'_>) -> Option<PathBuf> {
    match agent_config_dir(spawned_config_dir(ctx.role_env)) {
        Some(dir) => Some(match ctx.cwd {
            Some(cwd) => Path::new(cwd).join(dir),
            None => dir.into(),
        }),
        None => agent_home(".claude"),
    }
}

/// True iff claude-code's conversation file for `(cwd, uuid)` exists on
/// disk, in the folder the agent launches with. Used by
/// `SessionManager::resume` to skip `--resume <uuid>` when the agent never
/// persisted a turn.
pub fn claude_code_conversation_exists(ctx: &ProbeContext<'_>, uuid: &str) -> bool {
    conversation_file_exists(
        conversation_folder(ctx).as_deref(),
        ctx.cwd,
        uuid,
        claude_code_project_dir,
    )
}

use runner_core::protocol::runtime_metadata::claude_code::{PERMISSIONS, SKILL_DIRS};

pub struct ClaudeCode;
impl RuntimeAdapter for ClaudeCode {
    fn native_permission_mode(&self, args: &[String]) -> Option<String> {
        Some(launch::permission_arg(args))
    }
    fn skill_roots_with_env(
        &self,
        _home: &Path,
        _codex_home: Option<&Path>,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Option<Vec<PathBuf>> {
        env.get(crate::claude_accounts::sharing::MAIN_FOLDER)
            .map(|folder| vec![Path::new(folder).join("skills")])
    }
    fn skill_state_with_env(
        &self,
        home: &Path,
        codex_home: Option<&Path>,
        env: &std::collections::BTreeMap<String, String>,
    ) -> SkillState {
        match env.get(crate::claude_accounts::sharing::MAIN_FOLDER) {
            Some(folder) => skills::load_folder(Path::new(folder)),
            None => skills::load(home, codex_home),
        }
    }
    fn toggle_skill_with_env(
        &self,
        home: &Path,
        path: &Path,
        enabled: bool,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Option<crate::error::Result<crate::skills::SkillCatalog>> {
        let folder = env.get(crate::claude_accounts::sharing::MAIN_FOLDER)?;
        Some((|| {
            let catalog =
                crate::skills::skill_catalog_with_env(Runtime::ClaudeCode, home, None, env)
                    .unwrap();
            skills::set_enabled_folder(
                Path::new(folder),
                crate::ops::skills::find_entry(&catalog, path)?,
                enabled,
            )?;
            Ok(
                crate::skills::skill_catalog_with_env(Runtime::ClaudeCode, home, None, env)
                    .unwrap(),
            )
        })())
    }
    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics {
            interaction_prefix: Some("claude"),
            batch_feedback: true,
            compaction: CompactionPolicy::ResumeUnfinishedTurn,
            compaction_requires_start: true,
            permission_prompt: PermissionPromptPolicy::VisibleCandidates,
            elicitation_uses_requests: true,
            idle_clears_compaction: true,
            ..Default::default()
        }
    }
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
    }
    fn mcp_path(
        &self,
        home: &Path,
        env: &std::collections::BTreeMap<String, String>,
    ) -> Option<std::path::PathBuf> {
        Some(
            env.get(crate::claude_accounts::sharing::MAIN_MCP)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".claude.json")),
        )
    }
    fn native_defaults_with_env(
        &self,
        home: &Path,
        env: &std::collections::BTreeMap<String, String>,
    ) -> crate::runtime_defaults::RuntimeDefaults {
        match env.get(crate::claude_accounts::sharing::MAIN_FOLDER) {
            Some(folder) => crate::runtime_defaults::json_defaults(
                &Path::new(folder).join("settings.json"),
                false,
            ),
            None => self.native_defaults(home),
        }
    }
    fn usage(&self) -> Option<&'static UsageSource> {
        Some(&USAGE)
    }

    fn skills(&self) -> SkillSupport {
        SkillSupport {
            roots: SKILL_DIRS,
            load: skills::load,
            toggle: Some(skills::set_enabled),
            hidden: |document| document.value("user-invocable") == Some("false"),
            ..Default::default()
        }
    }

    fn npm_dist_tag(&self) -> &'static str {
        npm_dist_tag()
    }
    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        Some(&DISCOVERY)
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        runner_core::protocol::runtime_metadata::claude_code::capabilities()
    }
    fn native_defaults(&self, home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::json_defaults(&settings_path(home), false)
    }

    fn startup_delivery_uses_hooks(&self, _pending_turn: bool) -> bool {
        true
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn launch_env(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", "1"),
            ("DISABLE_INSTALLATION_CHECKS", "1"),
        ]
    }
    fn launch_gate(&self, session_id: &str, state: &Mutex<Option<Instant>>) {
        launch::enter_gate(session_id, state, CLAUDE_LAUNCH_GATE_GRACE);
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        _copilot_home: Option<&str>,
        claude_config_dir: Option<&str>,
    ) -> crate::error::Result<()> {
        if let Some(cwd) = trust_cwd(session_id, Runtime::ClaudeCode, cwd) {
            trust::seed_project_trust(cwd, claude_config_dir)?;
        }
        Ok(())
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::Hook
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::claude_code::catalog()
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
        positional_first_turn(body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        match prior_key {
            Some(k) if is_uuid(k) => ResumePlan {
                // `--resume <uuid>` is required on resume. We tried
                // `--session-id <uuid>` previously, but claude-code
                // refuses to start with a session id it already
                // recognises as in use ("Session ID … is already in
                // use") — it treats `--session-id` as fresh-only. The
                // edge case `--resume` exposes ("session not found"
                // when the conversation file was never persisted) is
                // normally masked by spawn-time first-turn delivery. An
                // untouched direct Claude fork intentionally has no first
                // turn and can still reach this fallback. If a resume exits
                // early and the transcript is confirmed missing, the
                // forwarder wipes `agent_session_key` and the next launch
                // starts fresh.
                args: vec!["--resume".into(), k.to_string()],
                prepend: false,
                assigned_key: Some(k.to_string()),
                resuming: true,
            },
            _ => {
                // Self-assign a UUID so next time we can resume by that id.
                // claude-code's `--session-id` requires a valid UUID and
                // binds that id to the new conversation.
                let id = uuid::Uuid::new_v4().to_string();
                ResumePlan {
                    args: vec!["--session-id".into(), id.clone()],
                    prepend: false,
                    assigned_key: Some(id),
                    resuming: false,
                }
            }
        }
    }
    fn fork_plan(&self, source_key: &str, _source_label: &str) -> Option<ForkPlan> {
        if !is_uuid(source_key) {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        Some(ForkPlan::Direct(ResumePlan {
            args: vec![
                "--resume".into(),
                source_key.to_string(),
                "--fork-session".into(),
                "--session-id".into(),
                id.clone(),
            ],
            prepend: false,
            assigned_key: Some(id),
            resuming: true,
        }))
    }
    fn probe_uses_shell_env(&self) -> bool {
        true
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(claude_code_conversation_exists(ctx, key))
    }
    fn conversation_folder(&self, ctx: &ProbeContext<'_>) -> Option<PathBuf> {
        conversation_folder(ctx)
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend(claude_settings_args(
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
    fn start_receiver(
        &self,
        _spec: &SpawnSpec,
        receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        Some(Box::new(claude_status::ClaudeStatusWatcher::from_receiver(
            receiver,
        )))
    }
    fn supported(&self, _windows: bool) -> bool {
        true
    }
    fn cleanup(&self, app_data_dir: &Path) {
        if let Err(error) = claude_status::clear_leftovers(app_data_dir) {
            log::warn!("clear stale Claude status files: {error}");
        }
    }
    fn env(
        &self,
        role_args: &[String],
        _plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String> {
        if !(self.supported(cfg!(windows)) && inject_claude_settings(role_args)) {
            return std::collections::BTreeMap::new();
        }
        hook_env(app_data_dir, &spec.session_id)
    }
}

#[cfg(test)]
mod tests;

pub(crate) const CLAUDE_SETTINGS_RELATIVE_PATH: &str = ".claude/settings.json";
pub(crate) fn settings_path(home: &Path) -> PathBuf {
    home.join(CLAUDE_SETTINGS_RELATIVE_PATH)
}

static DISCOVERY: ModelDiscoverySource = ModelDiscoverySource {
    order: 1,
    method: "list_models",
    query: models::query,
    config_home: || config_home(Some("CLAUDE_CONFIG_DIR"), ".claude"),
};

fn npm_dist_tag() -> &'static str {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| runner_core::app_paths::home_dir().map(|home| home.join(".claude")));
    #[cfg(test)]
    let config_dir = crate::runtimes::test_home()
        .map(|home| home.join(".claude"))
        .or(config_dir);
    config_dir
        .and_then(|dir| claude_channel(&dir.join("settings.json")))
        .unwrap_or("latest")
}

pub(crate) fn claude_channel(settings: &std::path::Path) -> Option<&'static str> {
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(settings).ok()?).ok()?;
    match settings.get("autoUpdatesChannel")?.as_str()? {
        "stable" => Some("stable"),
        "latest" => Some("latest"),
        _ => None,
    }
}

static USAGE: UsageSource = UsageSource {
    fetch: |_command, env, denied| {
        if denied {
            Err(crate::usage::UnavailableReason::KeychainDenied)
        } else {
            usage::fetch_claude(env)
        }
    },
};

static MCP: McpConfig = McpConfig {
    wire_name: "claude_code",
    serialized_name: "ClaudeCode",
    order: 0,
    config_file: "~/.claude.json",
    format: McpFormat::Json,
    translate: crate::ops::mcp::json_entry,
    preserve_disabled: false,
    supports_http: true,
};

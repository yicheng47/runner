pub(crate) mod models;
pub(crate) mod skills;
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
                "timeout": crate::runtimes::claude_code::claude_status::HOOK_TIMEOUT_SECS,
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
#[cfg(windows)]
fn claude_code_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(not(windows))]
fn claude_code_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// True iff claude-code's conversation file for `(cwd, uuid)` exists on
/// disk. Used by `SessionManager::resume` to skip `--resume <uuid>` when
/// the agent never persisted a turn.
pub fn claude_code_conversation_exists(cwd: Option<&str>, uuid: &str) -> bool {
    conversation_file_exists(".claude", cwd, uuid, claude_code_project_dir)
}

use runner_core::protocol::runtime_metadata::claude_code::{PERMISSIONS, SKILL_DIRS};

pub struct ClaudeCode;
impl RuntimeAdapter for ClaudeCode {
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

    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        Some(&Hooks)
    }
    fn launch_env(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", "1"),
            ("DISABLE_INSTALLATION_CHECKS", "1"),
        ]
    }
    fn launch_gate(&self) -> Option<Duration> {
        Some(CLAUDE_LAUNCH_GATE_GRACE)
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
                // turn and can still reach this fallback. If a resume fails
                // (e.g. the user killed the app
                // within ~1.5s of spawn before the first turn went
                // through), the reader thread's `resume_failed`
                // heuristic wipes `agent_session_key` and the next
                // launch starts fresh.
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
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(claude_code_conversation_exists(ctx.cwd, key))
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

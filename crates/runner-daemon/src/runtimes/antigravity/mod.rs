pub use runner_core::protocol::runtime_metadata::ANTIGRAVITY_EFFORTS;
pub(crate) mod agy_capture;
pub(crate) mod agy_trust;
pub(crate) mod models;
pub(crate) mod usage;
use super::helpers::*;
use super::*;
pub use runner_core::protocol::runtime_metadata::ANTIGRAVITY_MODELS;

const LEGACY_HOOKS_DIR: &str = "antigravity-hooks";

pub(crate) fn remove_legacy_hooks_dir(app_data_dir: &Path) {
    let dir = app_data_dir.join(LEGACY_HOOKS_DIR);
    let Ok(meta) = std::fs::symlink_metadata(&dir) else {
        return;
    };
    if !meta.file_type().is_dir() {
        return;
    }
    let report = dir.join("report.sh");
    if std::fs::symlink_metadata(&report).is_ok_and(|meta| meta.file_type().is_file()) {
        let _ = std::fs::remove_file(&report);
    }
    let agents = dir.join(".agents");
    if std::fs::symlink_metadata(&agents).is_ok_and(|meta| meta.file_type().is_dir()) {
        let hooks = agents.join("hooks.json");
        if std::fs::symlink_metadata(&hooks).is_ok_and(|meta| meta.file_type().is_file()) {
            let _ = std::fs::remove_file(&hooks);
        }
        let _ = std::fs::remove_dir(&agents);
    }
    let _ = std::fs::remove_dir(&dir);
}

pub(crate) fn inject_antigravity_settings(role_args: &[String]) -> bool {
    !role_args.iter().any(|arg| {
        matches!(arg.as_str(), "--settings" | "-settings")
            || arg.starts_with("--settings=")
            || arg.starts_with("-settings=")
    })
}

pub(crate) fn antigravity_title_command(exe: &Path, windows: bool) -> String {
    #[cfg(windows)]
    let exe = ordinary_windows_path(exe);
    #[cfg(not(windows))]
    let exe = exe.to_path_buf();
    let exe = exe.to_string_lossy();
    if windows {
        format!("\"{exe}\" hook title")
    } else {
        format!("{} hook title", crate::session::launch::shell_quote(&exe))
    }
}

pub(crate) fn antigravity_settings_args(role_args: &[String], app_data_dir: &Path) -> Vec<String> {
    if !inject_antigravity_settings(role_args) {
        return Vec::new();
    }
    let command = antigravity_title_command(&hook_executable(app_data_dir), cfg!(windows));
    let settings = serde_json::json!({
        "title": {
            "type": "command",
            "command": command,
            "enabled": true,
        }
    });
    vec!["--settings".into(), settings.to_string()]
}

pub(crate) fn antigravity_summaries_db_path(
    role_args: &[String],
    role_env: &std::collections::HashMap<String, String>,
) -> Option<PathBuf> {
    if !inject_antigravity_settings(role_args) {
        return None;
    }
    let env_home = role_env
        .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    #[cfg(test)]
    let home = env_home.or_else(crate::runtimes::test_home);
    #[cfg(not(test))]
    let home = env_home.or_else(runner_core::app_paths::home_dir);
    home.map(|home| home.join(".gemini/antigravity-cli/conversation_summaries.db"))
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

use runner_core::protocol::runtime_metadata::antigravity::{PERMISSIONS, SKILL_DIRS};

pub struct Antigravity;
impl RuntimeAdapter for Antigravity {
    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics {
            program_status_authority: true,
            ..Default::default()
        }
    }
    fn prepare_capture(&self, app_data_dir: &Path, session_id: &str) {
        agy_capture::prepare_log(app_data_dir, session_id);
    }
    fn start_capture(&self, ctx: CaptureContext) {
        let summaries_db_path = antigravity_summaries_db_path(&ctx.role_args, &ctx.role_env);
        agy_capture::spawn_capture(agy_capture::CaptureRequest {
            log_path: agy_capture::log_path(&ctx.app_data_dir, &ctx.session_id),
            summaries_db_path,
            cwd: ctx.cwd,
            initial_key: ctx.initial_key,
            manager: ctx.manager,
            session_id: ctx.session_id,
            mission_id: ctx.mission_id,
            expected_row_started_at: ctx.row_started_at,
            stop: ctx.stop,
            pool: ctx.pool,
            events: ctx.events,
        });
    }
    fn cleanup_capture(&self, app_data_dir: &Path, pool: &crate::db::DbPool) {
        agy_capture::clear_orphans(app_data_dir, pool);
        remove_legacy_hooks_dir(app_data_dir);
    }
    fn remove_capture(&self, app_data_dir: &Path, session_id: &str) {
        agy_capture::remove_log(app_data_dir, session_id);
    }
    fn mcp(&self) -> Option<&'static McpConfig> {
        Some(&MCP)
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
    fn usage(&self) -> Option<&'static UsageSource> {
        Some(&USAGE)
    }
    fn capabilities(&self) -> RuntimeCapabilities {
        runner_core::protocol::runtime_metadata::antigravity::capabilities()
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::LogTail
    }
    fn seed_trust(
        &self,
        session_id: &str,
        cwd: Option<&Path>,
        _copilot_home: Option<&str>,
        _claude_config_dir: Option<&str>,
    ) -> crate::error::Result<()> {
        if let Some(cwd) = trust_cwd(session_id, Runtime::Antigravity, cwd) {
            agy_trust::seed_project_trust(cwd)?;
        }
        Ok(())
    }

    fn catalog(&self) -> Option<RuntimeCatalog> {
        runner_core::protocol::runtime_metadata::antigravity::catalog()
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
            if !ANTIGRAVITY_EFFORTS.contains(&e.as_str()) {
                return false;
            }
            if let Some((_, levels)) = ANTIGRAVITY_MODELS.iter().find(|(id, _)| *id == m) {
                levels.contains(&e.as_str())
            } else {
                runner_core::protocol::runtime_metadata::antigravity::split_leveled_model(m)
                    .is_none()
            }
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
    // agy sorts active workspaces lexicographically to choose run_command's cwd;
    // missions already use --dangerously-skip-permissions and reach the bus via `runner`.
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
        out.extend(antigravity_settings_args(ctx.role_args, ctx.app_data_dir));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
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

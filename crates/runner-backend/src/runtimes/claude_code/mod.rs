use super::catalog::*;
use super::helpers::*;
use super::*;

pub(crate) fn inject_claude_settings(runtime: Option<Runtime>, role_args: &[String]) -> bool {
    runtime == Some(Runtime::ClaudeCode)
        && !role_args
            .iter()
            .any(|arg| arg == "--settings" || arg.starts_with("--settings="))
}

pub(crate) fn claude_settings_args(
    runtime: Option<Runtime>,
    role_args: &[String],
    app_data_dir: &Path,
    runner_session_id: &str,
) -> Vec<String> {
    if !inject_claude_settings(runtime, role_args) {
        return Vec::new();
    }
    let drop_path = crate::session::claude_rekey::drop_path(app_data_dir, runner_session_id);
    let temp_path = drop_path.with_extension("json.tmp");
    let temp_path = crate::session::launch::shell_quote(&temp_path.to_string_lossy());
    let drop_path = crate::session::launch::shell_quote(&drop_path.to_string_lossy());
    let hook_command = format!("cat > {temp_path} && mv {temp_path} {drop_path}");
    // Runner's terminal answers the background-colour query with the
    // live palette, so `auto` is the one theme value that never fights
    // the app: Claude Code paints light on Runner Light and dark on
    // Carbon, whatever the user's own config says.
    let mut settings = serde_json::json!({
        "tui": "fullscreen",
        "theme": "auto",
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": hook_command,
                    "timeout": crate::session::claude_status::HOOK_TIMEOUT_SECS,
                }],
            }],
        },
    });
    let status_path = crate::session::claude_status::status_path(app_data_dir, runner_session_id);
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
        if !crate::session::claude_status::hooks_supported(runtime, cfg!(windows)) {
            continue;
        }
        let mut entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": crate::session::claude_status::hook_command(&status_path, event),
                "timeout": crate::session::claude_status::HOOK_TIMEOUT_SECS,
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

static PERMISSIONS: Permissions = Permissions {
    offered: &[
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ],
    strip_flags: &[
        ("--dangerously-skip-permissions", false),
        ("--permission-mode", true),
    ],
    equals_on_bool: false,
    variadic_flag: None,
    args: permission_args,
    matches: mode_matches,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
fn permission_args(mode: PermissionMode) -> Vec<String> {
    match mode {
        PermissionMode::AcceptEdits => strings(&["--permission-mode", "acceptEdits"]),
        PermissionMode::Auto => strings(&["--permission-mode", "auto"]),
        PermissionMode::Bypass => strings(&["--permission-mode", "bypassPermissions"]),
        _ => Vec::new(),
    }
}
fn mode_matches(args: &[String], mode: PermissionMode) -> bool {
    if mode == PermissionMode::Bypass && args.iter().any(|a| a == "--dangerously-skip-permissions")
    {
        return true;
    }
    let pairs: &[(&str, Option<&str>)] = match mode {
        PermissionMode::AcceptEdits => &[("--permission-mode", Some("acceptEdits"))],
        PermissionMode::Auto => &[("--permission-mode", Some("auto"))],
        PermissionMode::Bypass => &[("--permission-mode", Some("bypassPermissions"))],
        _ => &[],
    };
    !pairs.is_empty()
        && pairs
            .iter()
            .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
}
pub struct ClaudeCode;
impl RuntimeAdapter for ClaudeCode {
    fn catalog(&self) -> Option<RuntimeCatalog> {
        let claude_efforts = vec![
            default_effort(),
            plain_option("low", "low"),
            plain_option("medium", "medium"),
            plain_option("high", "high"),
            plain_option("xhigh", "xhigh"),
            plain_option("max", "max"),
        ];
        Some(RuntimeCatalog {
            name: Runtime::ClaudeCode,
            display_name: Runtime::ClaudeCode.display_name(),
            command: Runtime::ClaudeCode.command().unwrap(),
            native_fork: true,
            description: "Anthropic Claude Code CLI",
            install_url: "https://code.claude.com/docs/en/setup",
            default_enabled: true,
            models: vec![
                default_model_option(),
                option("fable", "fable", "Latest Claude Fable."),
                option("opus", "opus", "Latest Claude Opus."),
                option("sonnet", "sonnet", "Latest Claude Sonnet."),
                option("haiku", "haiku", "Latest Claude Haiku."),
            ],
            efforts: claude_efforts,
            skills_dirs: &[".claude/skills"],
            update_args: &["update"],
            npm_package: Some("@anthropic-ai/claude-code"),
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
            Some(Runtime::ClaudeCode),
            ctx.role_args,
            ctx.app_data_dir,
            ctx.session_id,
        ));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

#[cfg(test)]
mod tests;

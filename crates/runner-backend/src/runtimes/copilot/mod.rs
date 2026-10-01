use super::catalog::*;
use super::helpers::*;
use super::*;

pub(crate) fn copilot_status_args(runtime: Option<Runtime>, app_data_dir: &Path) -> Vec<String> {
    if runtime != Some(Runtime::Copilot)
        || !crate::session::hook_feed::hooks_supported(runtime, cfg!(windows))
        || !crate::session::copilot_status::plugin_available(app_data_dir)
    {
        return Vec::new();
    }
    vec![
        "--plugin-dir".into(),
        crate::session::copilot_status::plugin_dir(app_data_dir)
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
        crate::session::copilot_trust::copilot_home(override_home)
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
    fn catalog(&self) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Copilot,
            display_name: Runtime::Copilot.display_name(),
            command: Runtime::Copilot.command().unwrap(),
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
            skills_dirs: &[".copilot/skills", ".agents/skills"],
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
        out.extend(copilot_status_args(
            Some(Runtime::Copilot),
            ctx.app_data_dir,
        ));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

#[cfg(test)]
mod tests;

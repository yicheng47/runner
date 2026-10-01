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

pub(crate) fn antigravity_status_args(
    runtime: Option<Runtime>,
    app_data_dir: &Path,
) -> Vec<String> {
    if runtime != Some(Runtime::Antigravity)
        || !crate::session::hook_feed::hooks_supported(runtime, cfg!(windows))
        || !crate::session::agy_status::hooks_available(app_data_dir)
    {
        return Vec::new();
    }
    vec![
        "--add-dir".into(),
        crate::session::agy_status::hooks_dir(app_data_dir)
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
    fn catalog(&self) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Antigravity,
            display_name: Runtime::Antigravity.display_name(),
            command: Runtime::Antigravity.command().unwrap(),
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
            skills_dirs: &[".gemini/antigravity-cli/skills", ".gemini/skills"],
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
            crate::session::agy_capture::log_path(ctx.app_data_dir, ctx.session_id)
                .to_string_lossy()
                .into_owned(),
        ]);
        out.extend(antigravity_status_args(
            Some(Runtime::Antigravity),
            ctx.app_data_dir,
        ));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

#[cfg(test)]
mod tests;

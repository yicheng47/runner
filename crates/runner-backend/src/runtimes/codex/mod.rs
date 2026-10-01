use super::catalog::*;
use super::helpers::*;
use super::*;

pub(crate) fn inject_codex_hooks(runtime: Option<Runtime>, args: &[String], windows: bool) -> bool {
    if runtime != Some(Runtime::Codex)
        || !crate::session::hook_feed::hooks_supported(runtime, windows)
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
    runtime: Option<Runtime>,
    role_args: &[String],
    app_data_dir: &Path,
    session_id: &str,
) -> Vec<String> {
    if !inject_codex_hooks(runtime, role_args, cfg!(windows)) {
        return Vec::new();
    }
    let path = crate::session::hook_feed::status_path(app_data_dir, session_id);
    let mut args = vec![
        "--enable".into(),
        "hooks".into(),
        "--dangerously-bypass-hook-trust".into(),
    ];
    for event in crate::session::codex_status::EVENTS {
        let command =
            toml_edit::Value::from(crate::session::codex_status::hook_command(&path, event));
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
            skills_dirs: &[".agents/skills", ".codex/skills"],
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
            Some(Runtime::Codex),
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

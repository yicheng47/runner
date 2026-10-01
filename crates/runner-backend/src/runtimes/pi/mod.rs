use super::catalog::*;
use super::helpers::*;
use super::*;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub(crate) fn pi_status_args(runtime: Option<Runtime>, app_data_dir: &Path) -> Vec<String> {
    if runtime != Some(Runtime::Pi)
        || !crate::session::hook_feed::hooks_supported(runtime, cfg!(windows))
        || !crate::session::pi_status::extension_available(app_data_dir)
    {
        return Vec::new();
    }
    vec![
        "-e".into(),
        crate::session::pi_status::extension_path(app_data_dir)
            .to_string_lossy()
            .into_owned(),
    ]
}
pub(crate) fn pi_project_slug(cwd: &str) -> String {
    let cwd = cwd
        .strip_prefix('/')
        .or_else(|| cwd.strip_prefix('\\'))
        .unwrap_or(cwd);
    let encoded: String = cwd
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | ':') {
                '-'
            } else {
                character
            }
        })
        .collect();
    format!("--{encoded}--")
}

const PI_AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
const PI_SESSION_DIR_ENV: &str = "PI_CODING_AGENT_SESSION_DIR";

pub fn pi_conversation_exists(
    cwd: Option<&str>,
    key: &str,
    role_env: &HashMap<String, String>,
) -> bool {
    let Some(cwd) = cwd else {
        return true;
    };
    let session_dir = pi_effective_env(role_env, PI_SESSION_DIR_ENV);
    let agent_dir = pi_effective_env(role_env, PI_AGENT_DIR_ENV);
    #[cfg(test)]
    let home = CONVERSATION_HOME.with_borrow(|home| home.clone());
    #[cfg(not(test))]
    let home = runner_core::app_paths::home_dir();
    pi_conversation_exists_at(
        home.as_deref(),
        cwd,
        key,
        session_dir.as_deref(),
        agent_dir.as_deref(),
    )
}

fn pi_effective_env(role_env: &HashMap<String, String>, name: &str) -> Option<OsString> {
    if let Some(value) = role_env.get(name) {
        return Some(OsString::from(value));
    }
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    std::env::var_os(name)
}

fn pi_conversation_exists_at(
    home: Option<&Path>,
    cwd: &str,
    key: &str,
    session_dir: Option<&OsStr>,
    agent_dir: Option<&OsStr>,
) -> bool {
    let Some(directory) = pi_session_directory(home, cwd, session_dir, agent_dir) else {
        return true;
    };
    let suffix = format!("_{key}.jsonl");
    std::fs::read_dir(directory).ok().is_some_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.path().is_file() && entry.file_name().to_string_lossy().ends_with(&suffix)
        })
    })
}

fn pi_session_directory(
    home: Option<&Path>,
    cwd: &str,
    session_dir: Option<&OsStr>,
    agent_dir: Option<&OsStr>,
) -> Option<PathBuf> {
    let cwd = pi_resolve_path(OsStr::new(cwd), None, home)?;
    if let Some(session_dir) = session_dir.filter(|path| !path.is_empty()) {
        return pi_resolve_path(session_dir, Some(&cwd), home);
    }
    let sessions_root = if let Some(agent_dir) = agent_dir.filter(|path| !path.is_empty()) {
        pi_resolve_path(agent_dir, Some(&cwd), home)?.join("sessions")
    } else {
        home?.join(".pi").join("agent").join("sessions")
    };
    Some(sessions_root.join(pi_project_slug(&cwd.to_string_lossy())))
}

fn pi_resolve_path(path: &OsStr, cwd: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(path);
    let expanded = if path == Path::new("~") {
        home?.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        home?.join(rest)
    } else {
        path.to_path_buf()
    };
    if expanded.is_absolute() {
        Some(expanded)
    } else {
        Some(
            cwd.map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())?
                .join(expanded),
        )
    }
}

static PERMISSIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[],
    equals_on_bool: false,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
pub struct Pi;
impl RuntimeAdapter for Pi {
    fn catalog(&self) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog {
            name: Runtime::Pi,
            display_name: Runtime::Pi.display_name(),
            command: Runtime::Pi.command().unwrap(),
            native_fork: true,
            description: "pi coding agent (bring your own model provider)",
            install_url: "https://github.com/earendil-works/pi",
            default_enabled: true,
            models: vec![default_model_option()],
            efforts: std::iter::once(default_effort())
                .chain(
                    ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(|effort| plain_option(effort, effort)),
                )
                .collect(),
            skills_dirs: &[".pi/agent/skills", ".agents/skills"],
            update_args: &["update"],
            npm_package: Some("@earendil-works/pi-coding-agent"),
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
            out.push("--thinking".into());
            out.push(e.to_ascii_lowercase());
        }
        out
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        prefixed_first_turn("--", body)
    }
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan {
        assigned_resume(prior_key, true)
    }
    fn fork_plan(&self, source_key: &str, _source_label: &str) -> Option<ForkPlan> {
        if !is_uuid(source_key) {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        Some(ForkPlan::Direct(ResumePlan {
            args: vec![
                "--fork".into(),
                source_key.to_string(),
                "--session-id".into(),
                id.clone(),
            ],
            prepend: true,
            assigned_key: Some(id),
            resuming: true,
        }))
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels {
            system_prompt: true,
            resend_persona_on_fresh: true,
        }
    }
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext<'_>) -> Option<bool> {
        Some(pi_conversation_exists(ctx.cwd, key, ctx.role_env))
    }
    fn missing_conversation(&self) -> MissingConversation {
        MissingConversation {
            reuse_key: true,
            resume_on_launch: true,
        }
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        let mut out = Vec::new();
        if ctx.mission {
            out.push("--approve".into());
        }
        out.extend(self.model_effort_args(ctx.model, ctx.effort));
        out.extend(pi_status_args(Some(Runtime::Pi), ctx.app_data_dir));
        out.extend(self.prompt_channels().system_prompt_args(ctx.system_prompt));
        out.extend(self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn }));
        out
    }
}

#[cfg(test)]
mod tests;

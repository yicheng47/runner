use super::*;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub struct Permissions {
    pub offered: &'static [PermissionMode],
    pub strip_flags: &'static [(&'static str, bool)],
    pub equals_on_bool: bool,
    pub variadic_flag: Option<&'static str>,
    pub args: fn(PermissionMode) -> Vec<String>,
    pub matches: fn(&[String], PermissionMode) -> bool,
    pub mission_bypass: Option<&'static [&'static str]>,
    pub strip_on_mission_resume: bool,
}
impl Permissions {
    pub fn mode_args(&self, mode: PermissionMode) -> Vec<String> {
        (self.args)(mode)
    }
    pub fn strip(&self, args: &[String]) -> Vec<String> {
        let keys = self.strip_flags;
        // Go's flag package also takes `=value` on a boolean flag.
        let equals_on_bool = self.equals_on_bool;
        if keys.is_empty() {
            return args.to_vec();
        }
        let mut out = Vec::with_capacity(args.len());
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if self.variadic_flag == Some(arg.as_str()) {
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    i += 1;
                }
                continue;
            }
            // Exact-match `--flag` form. For takes_value flags we also
            // skip the next token if present (it's the value).
            if let Some(&(_, takes_value)) = keys.iter().find(|(name, _)| name == arg) {
                i += if takes_value && i + 1 < args.len() {
                    2
                } else {
                    1
                };
                continue;
            }
            // `--flag=value` form: strip the whole token in one go.
            if keys.iter().any(|(name, takes_value)| {
                (*takes_value || equals_on_bool) && arg.starts_with(&format!("{name}="))
            }) {
                i += 1;
                continue;
            }
            out.push(arg.clone());
            i += 1;
        }
        out
    }
    pub fn apply(&self, args: &[String], mode: PermissionMode) -> Vec<String> {
        let mut out = self.strip(args);
        out.extend(self.mode_args(mode));
        out
    }
    pub fn mission_args(&self, mode: MissionPermissionMode) -> Option<Vec<String>> {
        match mode {
            MissionPermissionMode::RoleDefault => None,
            MissionPermissionMode::Auto => Some(self.mode_args(PermissionMode::Auto)),
            MissionPermissionMode::Bypass => Some(
                self.mission_bypass
                    .map(strings)
                    .unwrap_or_else(|| self.mode_args(PermissionMode::Bypass)),
            ),
        }
    }
    pub fn apply_mission(&self, args: &[String], mode: MissionPermissionMode) -> Vec<String> {
        match self.mission_args(mode) {
            None => args.to_vec(),
            Some(extra) => {
                let mut out = self.strip(args);
                out.extend(extra);
                out
            }
        }
    }
    pub fn infer(&self, args: &[String]) -> PermissionMode {
        [
            PermissionMode::Bypass,
            PermissionMode::Auto,
            PermissionMode::AcceptEdits,
        ]
        .into_iter()
        .find(|mode| (self.matches)(args, *mode))
        .unwrap_or(PermissionMode::Default)
    }
}
pub(super) fn go_bool_flag_is_set(arg: &str, name: &str) -> bool {
    let Some(rest) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')) else {
        return false;
    };
    match rest.strip_prefix(name) {
        Some("") => true,
        Some(value) => value
            .strip_prefix('=')
            .is_some_and(|value| matches!(value, "1" | "t" | "T" | "true" | "TRUE" | "True")),
        None => false,
    }
}
pub(super) fn flag_value_matches(args: &[String], flag: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return args.iter().any(|a| a == flag);
    };
    let equals_token = format!("{flag}={expected}");
    for (i, arg) in args.iter().enumerate() {
        if arg == &equals_token {
            return true;
        }
        if arg == flag {
            if let Some(next) = args.get(i + 1) {
                if next == expected {
                    return true;
                }
            }
        }
    }
    false
}
pub(super) fn is_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}
pub(super) fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).into()).collect()
}
pub(super) fn trim_some(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}
pub(super) fn first_turn_body(body: Option<&str>) -> Option<&str> {
    let body = body.filter(|s| !s.trim().is_empty())?;
    debug_assert!(body.len() <= FIRST_TURN_ARGV_MAX_BYTES,
        "first-turn argv body exceeds {FIRST_TURN_ARGV_MAX_BYTES} bytes (got {}) — persistence-layer validation should have caught this", body.len());
    (body.len() <= FIRST_TURN_ARGV_MAX_BYTES).then_some(body)
}
pub(super) fn positional_first_turn(body: Option<&str>) -> Vec<String> {
    first_turn_body(body)
        .map(|body| vec![body.into()])
        .unwrap_or_default()
}
pub(super) fn prefixed_first_turn(prefix: &str, body: Option<&str>) -> Vec<String> {
    first_turn_body(body)
        .map(|body| strings(&[prefix, body]))
        .unwrap_or_default()
}
pub(super) fn add_dir(dir: Option<&Path>) -> Vec<String> {
    dir.map(|dir| vec!["--add-dir".into(), dir.to_string_lossy().into_owned()])
        .unwrap_or_default()
}
pub(super) fn model_reasoning_args(model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    let model = trim_some(model);
    let effort = trim_some(effort);
    let mut out = Vec::new();
    if let Some(m) = model {
        out.push("--model".into());
        out.push(m.to_string());
    }
    if let Some(e) = effort {
        // No dedicated flag; reuse the config-override path.
        // Lowercase: codex's TOML enum is case-sensitive
        // (rejects "High" with "unknown variant").
        out.push("-c".into());
        out.push(format!("model_reasoning_effort={}", e.to_ascii_lowercase()));
    }
    out
}
pub(super) fn subcommand_resume(prior_key: Option<&str>) -> ResumePlan {
    match prior_key {
        Some(k) if is_uuid(k) => ResumePlan {
            // `codex resume <uuid>` is a subcommand prefix. The caller
            // places these args ahead of any user-supplied args.
            args: vec!["resume".into(), k.to_string()],
            prepend: true,
            assigned_key: Some(k.to_string()),
            resuming: true,
        },
        _ => ResumePlan::fresh(),
    }
}
pub(super) fn assigned_resume(prior_key: Option<&str>, prepend: bool) -> ResumePlan {
    let prior_key = prior_key.filter(|key| is_uuid(key));
    let id = prior_key
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    ResumePlan {
        args: vec!["--session-id".into(), id.clone()],
        prepend,
        assigned_key: Some(id),
        resuming: prior_key.is_some(),
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static CONVERSATION_HOME: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_conversation_home<T>(home: &Path, run: impl FnOnce() -> T) -> T {
    let prior = CONVERSATION_HOME.with_borrow_mut(|value| value.replace(home.to_owned()));
    let result = run();
    CONVERSATION_HOME.with_borrow_mut(|value| *value = prior);
    result
}

/// The fake home `with_conversation_home` installed, for adapters that edit
/// an agent's own config at spawn and must stay out of the real one in tests.
#[cfg(test)]
pub(crate) fn test_home() -> Option<PathBuf> {
    CONVERSATION_HOME.with_borrow(|home| home.clone())
}

/// Check an agent conversation path using that CLI's project-directory encoder.
pub(super) fn conversation_file_exists(
    agent_dir: &str,
    cwd: Option<&str>,
    uuid: &str,
    encode_project_dir: fn(&str) -> String,
) -> bool {
    // `cfg(test)` short-circuits the filesystem check so unit tests
    // for the resume flow don't have to fake out the agent project
    // directory. The encoders are exercised directly below.
    #[cfg(test)]
    {
        CONVERSATION_HOME.with_borrow(|home| match (home.as_deref(), cwd) {
            (Some(home), Some(cwd)) => {
                conversation_file_exists_at(home, agent_dir, cwd, uuid, encode_project_dir)
            }
            _ => true,
        })
    }
    #[cfg(not(test))]
    {
        let Some(cwd) = cwd else {
            // No cwd → claude-code falls back to the parent's, which we
            // can't reproduce here. Be permissive: let `--resume` try and
            // surface its own error rather than masking it.
            return true;
        };
        let Some(home) = runner_core::app_paths::home_dir() else {
            return true;
        };
        conversation_file_exists_at(&home, agent_dir, cwd, uuid, encode_project_dir)
    }
}

pub(super) fn conversation_file_exists_at(
    home: &Path,
    agent_dir: &str,
    cwd: &str,
    uuid: &str,
    encode_project_dir: fn(&str) -> String,
) -> bool {
    home.join(agent_dir)
        .join("projects")
        .join(encode_project_dir(cwd))
        .join(format!("{uuid}.jsonl"))
        .exists()
}

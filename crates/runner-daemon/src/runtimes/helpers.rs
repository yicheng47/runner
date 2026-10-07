use super::*;
use std::io::Write;
use std::path::{Path, PathBuf};

pub use runner_core::protocol::runtime_metadata::Permissions;

pub(super) fn is_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}
pub use runner_core::protocol::runtime_metadata::strings;
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
    let projects = home.join(agent_dir).join("projects");
    let exists = |cwd: &str| {
        projects
            .join(encode_project_dir(cwd))
            .join(format!("{uuid}.jsonl"))
            .exists()
    };
    exists(cwd)
        || std::fs::canonicalize(cwd).ok().is_some_and(|canonical| {
            #[cfg(windows)]
            let canonical = ordinary_windows_path(&canonical);
            exists(&canonical.to_string_lossy())
        })
}

#[cfg(windows)]
pub(crate) fn ordinary_windows_path(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let root = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => PathBuf::from(r"\\").join(server).join(share),
        _ => return path.to_path_buf(),
    };
    root.join(components.as_path())
}

pub(crate) fn resolve_config_write_path(config_path: &Path) -> crate::error::Result<PathBuf> {
    match std::fs::symlink_metadata(config_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::canonicalize(config_path)
            .map_err(|e| {
                crate::error::Error::msg(format!("realpath {}: {e}", config_path.display()))
            }),
        Ok(_) => Ok(config_path.to_path_buf()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(config_path.to_path_buf()),
        Err(e) => Err(crate::error::Error::msg(format!(
            "metadata {}: {e}",
            config_path.display()
        ))),
    }
}

pub(crate) fn write_config_atomically(path: &Path, contents: &[u8]) -> crate::error::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        crate::error::Error::msg(format!("config path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|e| crate::error::Error::msg(format!("mkdir {}: {e}", parent.display())))?;
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
        crate::error::Error::msg(format!("create temp file in {}: {e}", parent.display()))
    })?;
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions).map_err(|e| {
            crate::error::Error::msg(format!("set temp permissions for {}: {e}", path.display()))
        })?;
    }
    temp.write_all(contents).map_err(|e| {
        crate::error::Error::msg(format!("write temp config for {}: {e}", path.display()))
    })?;
    temp.as_file().sync_all().map_err(|e| {
        crate::error::Error::msg(format!("sync temp config for {}: {e}", path.display()))
    })?;
    temp.persist(path).map_err(|e| {
        crate::error::Error::msg(format!("persist {}: {}", path.display(), e.error))
    })?;
    Ok(())
}

pub(super) fn trust_cwd<'a>(
    session_id: &str,
    runtime: Runtime,
    cwd: Option<&'a Path>,
) -> Option<&'a Path> {
    if cwd.is_none() {
        log::debug!(
            "skipping {:?} project trust seed without cwd: session={session_id}",
            Some(runtime)
        );
    }
    cwd
}

pub(super) fn hook_executable(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join("bin")
        .join(runner_core::cli_install::AGENT_DEST_BIN_NAME)
}

pub(super) fn hook_env(
    app_data_dir: &Path,
    session_id: &str,
) -> std::collections::BTreeMap<String, String> {
    use runner_core::protocol::hook;
    std::collections::BTreeMap::from([
        (hook::SESSION_ENV.into(), session_id.into()),
        (
            hook::GENERATION_ENV.into(),
            uuid::Uuid::new_v4().to_string(),
        ),
        (
            hook::EXECUTABLE_ENV.into(),
            hook_executable(app_data_dir).to_string_lossy().into_owned(),
        ),
    ])
}

pub(super) fn hook_report_command(
    app_data_dir: &Path,
    runtime: Runtime,
    event: &str,
    powershell: bool,
) -> String {
    let executable = hook_executable(app_data_dir)
        .to_string_lossy()
        .replace('\\', "/");
    if powershell {
        format!(
            "try{{& '{}' hook report --runtime {} --event '{event}'}}catch{{}};exit 0",
            executable.replace('\'', "''"),
            runtime.key()
        )
    } else {
        let fallback = if runtime == Runtime::Antigravity {
            if event == "PreInvocation" {
                "{ if [ -n \"$RUNNER_ANTIGRAVITY_WORKSPACE_CONTEXT\" ]; then printf '%s\\n' \"$RUNNER_ANTIGRAVITY_WORKSPACE_CONTEXT\"; else printf '{}\\n'; fi; }"
            } else {
                "printf '{}\\n'"
            }
        } else {
            ":"
        };
        format!(
            "{} hook report --runtime {} --event {event} 2>/dev/null || {fallback}",
            crate::session::launch::shell_quote(&executable),
            runtime.key()
        )
    }
}

pub(super) fn config_home(env_name: Option<&str>, relative: &str) -> Option<PathBuf> {
    #[cfg(test)]
    use crate::golden::{config_home as home_dir, config_var_os as var_os};
    #[cfg(not(test))]
    use {runner_core::app_paths::home_dir, std::env::var_os};
    env_name
        .and_then(var_os)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(relative)))
}

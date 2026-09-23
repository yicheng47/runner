//! Login-shell environment discovery for GUI-launched child PTYs.
//!
//! Finder/launchd starts Runner with a stripped environment. Discovery
//! captures the small set of shell-owned values that agent CLIs need,
//! while callers retain the last successful snapshot when a later probe
//! fails or times out.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[cfg(any(windows, test))]
use crate::cli_install::{RegistryPathValue, RegistryValueKind};

#[cfg(unix)]
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const POLL_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(unix)]
const STDOUT_DRAIN_GRACE: Duration = Duration::from_millis(500);

#[cfg(any(unix, test))]
const CAPTURED_VARS: &[&str] = &[
    "PATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginShellEnv {
    pub path: Option<String>,
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOutcome {
    Ok,
    WindowsRegistry,
    WindowsRegistryError,
    Timeout,
    SpawnError,
    EmptyCapture,
    NoShell,
}

impl DiscoveryOutcome {
    pub fn is_success(self) -> bool {
        matches!(self, Self::Ok | Self::WindowsRegistry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    pub shell: Option<String>,
    pub outcome: DiscoveryOutcome,
    pub duration_ms: u64,
    pub env: LoginShellEnv,
}

#[derive(Debug, Clone)]
pub struct DiscoveryState {
    pub checking: bool,
    pub result: Option<DiscoveryResult>,
    pub seeded_shell: Option<String>,
    pub last_known_good_captured_at: Option<String>,
    pub(crate) models: crate::runtime_status::models::ModelDiscovery,
    pub(crate) versions: crate::runtime_status::versions::VersionDiscovery,
}

impl DiscoveryState {
    pub fn startup(
        seeded_shell: Option<String>,
        last_known_good_captured_at: Option<String>,
    ) -> Self {
        Self {
            checking: true,
            result: None,
            seeded_shell,
            last_known_good_captured_at,
            models: Default::default(),
            versions: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn pending() -> Self {
        Self::startup(None, None)
    }
}

#[cfg(unix)]
pub fn resolve_login_shell_env() -> DiscoveryResult {
    let started = Instant::now();
    let shell = configured_shell(std::env::var("SHELL").ok());
    let Some(shell) = shell else {
        return finish_discovery(
            None,
            DiscoveryOutcome::NoShell,
            started,
            LoginShellEnv::default(),
        );
    };
    resolve_shell_env(&shell, RESOLVE_TIMEOUT, started)
}

pub(crate) fn configured_shell(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn shell_login_args(shell: &str) -> Vec<String> {
    #[cfg(not(unix))]
    let _ = shell;
    #[cfg(unix)]
    if shell_probe_args(shell).is_some() {
        return vec!["-l".to_string()];
    }
    Vec::new()
}

#[cfg(unix)]
fn resolve_shell_env(shell: &str, timeout: Duration, started: Instant) -> DiscoveryResult {
    let Some(probe_args) = shell_probe_args(shell) else {
        return finish_discovery(
            Some(shell.to_string()),
            DiscoveryOutcome::SpawnError,
            started,
            LoginShellEnv::default(),
        );
    };

    let mut inner = String::new();
    for var in CAPTURED_VARS {
        inner.push_str(&format!(
            "printf '%s' '__RUNNER_KV_{var}_BEGIN__'; printenv {var} 2>/dev/null; printf '%s\\n' '__RUNNER_KV_{var}_END__'; "
        ));
    }

    let mut child = match Command::new(shell)
        .args(probe_args)
        .arg(&inner)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            return finish_discovery(
                Some(shell.to_string()),
                DiscoveryOutcome::SpawnError,
                started,
                LoginShellEnv::default(),
            );
        }
    };

    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return finish_discovery(
                Some(shell.to_string()),
                DiscoveryOutcome::SpawnError,
                started,
                LoginShellEnv::default(),
            );
        }
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return finish_discovery(
                    Some(shell.to_string()),
                    DiscoveryOutcome::Timeout,
                    started,
                    LoginShellEnv::default(),
                );
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return finish_discovery(
                    Some(shell.to_string()),
                    DiscoveryOutcome::SpawnError,
                    started,
                    LoginShellEnv::default(),
                );
            }
        }
    };

    if !status.success() {
        return finish_discovery(
            Some(shell.to_string()),
            DiscoveryOutcome::SpawnError,
            started,
            LoginShellEnv::default(),
        );
    }

    let stdout_bytes = match rx.recv_timeout(STDOUT_DRAIN_GRACE) {
        Ok(bytes) => bytes,
        Err(_) => {
            return finish_discovery(
                Some(shell.to_string()),
                DiscoveryOutcome::EmptyCapture,
                started,
                LoginShellEnv::default(),
            );
        }
    };
    let env = parse_login_shell_env(&String::from_utf8_lossy(&stdout_bytes));
    let outcome = capture_outcome(&env);
    finish_discovery(Some(shell.to_string()), outcome, started, env)
}

#[cfg(unix)]
fn capture_outcome(env: &LoginShellEnv) -> DiscoveryOutcome {
    if env.path.is_none() && env.vars.is_empty() {
        DiscoveryOutcome::EmptyCapture
    } else {
        DiscoveryOutcome::Ok
    }
}

#[cfg(unix)]
fn shell_probe_args(shell: &str) -> Option<&'static [&'static str]> {
    match std::path::Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
    {
        Some("bash" | "zsh" | "sh" | "dash" | "ksh") => Some(&["-ilc"]),
        Some("fish") => Some(&["-i", "-l", "-c"]),
        _ => None,
    }
}

#[cfg(windows)]
pub fn resolve_login_shell_env() -> DiscoveryResult {
    use crate::cli_install::{SystemUserPathRegistry, UserPathRegistry as _};

    let started = Instant::now();
    let registry = SystemUserPathRegistry;
    let (outcome, env) =
        registry_discovery(registry.read_machine_path(), registry.read_path(), |name| {
            std::env::var(name).ok()
        });
    finish_discovery(None, outcome, started, env)
}

#[cfg(all(not(unix), not(windows)))]
pub fn resolve_login_shell_env() -> DiscoveryResult {
    finish_discovery(
        None,
        DiscoveryOutcome::NoShell,
        Instant::now(),
        LoginShellEnv::default(),
    )
}

#[cfg(any(windows, test))]
fn registry_discovery(
    machine: crate::error::Result<Option<RegistryPathValue>>,
    user: crate::error::Result<Option<RegistryPathValue>>,
    environment: impl FnMut(&str) -> Option<String>,
) -> (DiscoveryOutcome, LoginShellEnv) {
    let machine = match machine {
        Ok(value) => {
            if value.is_none() {
                log::warn!("runtime discovery: Windows machine PATH is missing from the registry");
            }
            Some(value)
        }
        Err(error) => {
            log::warn!("runtime discovery: Windows machine PATH read failed: {error}");
            None
        }
    };
    let user = match user {
        Ok(value) => {
            if value.is_none() {
                log::warn!("runtime discovery: Windows user PATH is missing from the registry");
            }
            Some(value)
        }
        Err(error) => {
            log::warn!("runtime discovery: Windows user PATH read failed: {error}");
            None
        }
    };
    if machine.is_none() && user.is_none() {
        return (
            DiscoveryOutcome::WindowsRegistryError,
            LoginShellEnv::default(),
        );
    }
    (
        DiscoveryOutcome::WindowsRegistry,
        registry_login_shell_env(machine.flatten(), user.flatten(), environment),
    )
}

#[cfg(any(windows, test))]
fn registry_login_shell_env(
    machine: Option<RegistryPathValue>,
    user: Option<RegistryPathValue>,
    mut environment: impl FnMut(&str) -> Option<String>,
) -> LoginShellEnv {
    let mut entries = Vec::<String>::new();
    for value in [machine, user].into_iter().flatten() {
        let value = match value.kind {
            RegistryValueKind::String => value.value,
            RegistryValueKind::ExpandString => {
                expand_windows_environment(&value.value, &mut environment)
            }
        };
        for entry in value.split(';').filter(|entry| !entry.is_empty()) {
            if !entries
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(entry))
            {
                entries.push(entry.to_string());
            }
        }
    }
    LoginShellEnv {
        path: (!entries.is_empty()).then(|| entries.join(";")),
        vars: BTreeMap::new(),
    }
}

#[cfg(any(windows, test))]
fn expand_windows_environment(
    value: &str,
    environment: &mut impl FnMut(&str) -> Option<String>,
) -> String {
    let mut expanded = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        expanded.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push_str(&remaining[start..]);
            return expanded;
        };
        let name = &after_start[..end];
        if let Some(replacement) = environment(name) {
            expanded.push_str(&replacement);
        } else {
            expanded.push('%');
            expanded.push_str(name);
            expanded.push('%');
        }
        remaining = &after_start[end + 1..];
    }
    expanded.push_str(remaining);
    expanded
}

fn finish_discovery(
    shell: Option<String>,
    outcome: DiscoveryOutcome,
    started: Instant,
    env: LoginShellEnv,
) -> DiscoveryResult {
    let duration_ms = started.elapsed().as_millis() as u64;
    log::info!(
        "runtime discovery: shell={} duration_ms={} outcome={:?}",
        shell.as_deref().unwrap_or("<none>"),
        duration_ms,
        outcome,
    );
    DiscoveryResult {
        shell,
        outcome,
        duration_ms,
        env,
    }
}

#[cfg(any(unix, test))]
fn parse_login_shell_env(stdout: &str) -> LoginShellEnv {
    let mut env = LoginShellEnv::default();
    for var in CAPTURED_VARS {
        let begin = format!("__RUNNER_KV_{var}_BEGIN__");
        let end = format!("__RUNNER_KV_{var}_END__");
        let Some(begin_idx) = stdout.rfind(&begin) else {
            continue;
        };
        let after_begin = &stdout[begin_idx + begin.len()..];
        let Some(end_idx) = after_begin.find(&end) else {
            continue;
        };
        let value = after_begin[..end_idx].trim();
        if value.is_empty() {
            continue;
        }
        if *var == "PATH" {
            env.path = Some(value.to_string());
        } else {
            env.vars.insert(var.to_string(), value.to_string());
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(name: &str, value: &str) -> String {
        format!("__RUNNER_KV_{name}_BEGIN__{value}__RUNNER_KV_{name}_END__\n")
    }

    #[test]
    fn parses_path_and_proxy_values_ignoring_rc_banner() {
        let mut stdout = String::from("Welcome to zsh!\nnvm: using node v20\n");
        stdout.push_str(&block("PATH", "/opt/homebrew/bin:/usr/bin:/bin"));
        stdout.push_str(&block("HTTPS_PROXY", "http://127.0.0.1:7890"));
        stdout.push_str(&block("https_proxy", "http://127.0.0.1:7890"));
        stdout.push_str(&block("NO_PROXY", "localhost,127.0.0.1,*.byted.org"));
        stdout.push_str(&block("HTTP_PROXY", ""));
        let parsed = parse_login_shell_env(&stdout);
        assert_eq!(
            parsed.path.as_deref(),
            Some("/opt/homebrew/bin:/usr/bin:/bin")
        );
        assert_eq!(
            parsed.vars.get("HTTPS_PROXY").map(String::as_str),
            Some("http://127.0.0.1:7890"),
        );
        assert_eq!(
            parsed.vars.get("https_proxy").map(String::as_str),
            Some("http://127.0.0.1:7890"),
        );
        assert_eq!(
            parsed.vars.get("NO_PROXY").map(String::as_str),
            Some("localhost,127.0.0.1,*.byted.org"),
        );
        assert!(!parsed.vars.contains_key("HTTP_PROXY"));
    }

    #[test]
    fn banner_marker_does_not_shadow_real_block() {
        let mut stdout = String::from("echo: __RUNNER_KV_PATH_BEGIN__ (banner)\n");
        stdout.push_str(&block("PATH", "/usr/bin:/bin"));
        assert_eq!(
            parse_login_shell_env(&stdout).path.as_deref(),
            Some("/usr/bin:/bin")
        );
    }

    #[test]
    fn missing_or_empty_blocks_return_default() {
        for stdout in [
            "just a banner",
            "__RUNNER_KV_PATH_BEGIN__only",
            "",
            &block("PATH", ""),
        ] {
            assert_eq!(parse_login_shell_env(stdout), LoginShellEnv::default());
        }
    }

    #[test]
    fn registry_discovery_expands_values_orders_machine_first_and_dedupes() {
        let machine = RegistryPathValue {
            value: r"%SystemRoot%\System32;C:\Tools".into(),
            kind: RegistryValueKind::ExpandString,
        };
        let user = RegistryPathValue {
            value: r"c:\tools;%Unexpanded%\bin;C:\Users\Jason\bin".into(),
            kind: RegistryValueKind::String,
        };
        let (outcome, env) = registry_discovery(Ok(Some(machine)), Ok(Some(user)), |name| {
            name.eq_ignore_ascii_case("SystemRoot")
                .then(|| r"C:\Windows".to_string())
        });
        assert_eq!(outcome, DiscoveryOutcome::WindowsRegistry);
        assert_eq!(
            env.path.as_deref(),
            Some(r"C:\Windows\System32;C:\Tools;%Unexpanded%\bin;C:\Users\Jason\bin")
        );
        assert!(env.vars.is_empty());
    }

    #[test]
    fn absent_registry_paths_are_a_successful_empty_refresh() {
        let (outcome, env) = registry_discovery(Ok(None), Ok(None), |_| None);
        assert_eq!(outcome, DiscoveryOutcome::WindowsRegistry);
        assert_eq!(env, LoginShellEnv::default());
    }

    #[test]
    fn registry_discovery_keeps_user_path_when_machine_read_fails() {
        let user = RegistryPathValue {
            value: r"C:\Users\Jason\bin".into(),
            kind: RegistryValueKind::String,
        };
        let (outcome, env) = registry_discovery(
            Err(crate::error::Error::msg("machine denied")),
            Ok(Some(user)),
            |_| None,
        );
        assert_eq!(outcome, DiscoveryOutcome::WindowsRegistry);
        assert_eq!(env.path.as_deref(), Some(r"C:\Users\Jason\bin"));
    }

    #[test]
    fn registry_discovery_keeps_machine_path_when_user_read_fails() {
        let machine = RegistryPathValue {
            value: r"C:\Windows\System32".into(),
            kind: RegistryValueKind::String,
        };
        let (outcome, env) = registry_discovery(
            Ok(Some(machine)),
            Err(crate::error::Error::msg("user denied")),
            |_| None,
        );
        assert_eq!(outcome, DiscoveryOutcome::WindowsRegistry);
        assert_eq!(env.path.as_deref(), Some(r"C:\Windows\System32"));
    }

    #[test]
    fn registry_discovery_fails_only_when_both_reads_fail() {
        let (outcome, env) = registry_discovery(
            Err(crate::error::Error::msg("machine denied")),
            Err(crate::error::Error::msg("user denied")),
            |_| None,
        );
        assert_eq!(outcome, DiscoveryOutcome::WindowsRegistryError);
        assert_eq!(env, LoginShellEnv::default());
    }

    #[test]
    fn empty_registry_values_produce_no_path() {
        assert_eq!(
            registry_discovery(
                Ok(Some(RegistryPathValue {
                    value: ";;".into(),
                    kind: RegistryValueKind::String,
                })),
                Ok(None),
                |_| None,
            ),
            (DiscoveryOutcome::WindowsRegistry, LoginShellEnv::default())
        );
    }

    #[cfg(unix)]
    #[test]
    fn supported_shells_use_their_startup_semantics() {
        assert_eq!(shell_probe_args("/bin/zsh"), Some(&["-ilc"][..]));
        assert_eq!(shell_probe_args("/bin/bash"), Some(&["-ilc"][..]));
        assert_eq!(
            shell_probe_args("/opt/homebrew/bin/fish"),
            Some(&["-i", "-l", "-c"][..])
        );
        assert_eq!(shell_probe_args("/bin/tcsh"), None);
        assert_eq!(shell_login_args("/bin/zsh"), ["-l"]);
        assert_eq!(shell_login_args("/opt/homebrew/bin/fish"), ["-l"]);
        assert!(shell_login_args("/usr/local/bin/elvish").is_empty());
        assert_eq!(configured_shell(None), None);
        assert_eq!(configured_shell(Some("  ".into())), None);
        assert_eq!(
            configured_shell(Some(" /bin/zsh ".into())).as_deref(),
            Some("/bin/zsh")
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalid_shell_maps_to_spawn_error() {
        let result = resolve_shell_env(
            "/definitely/missing/runner-shell",
            Duration::from_millis(20),
            Instant::now(),
        );
        assert_eq!(result.outcome, DiscoveryOutcome::SpawnError);
    }

    #[cfg(unix)]
    #[test]
    fn slow_shell_maps_to_timeout_and_empty_capture_is_typed() {
        use std::os::unix::fs::PermissionsExt;

        let slow_dir = tempfile::tempdir().unwrap();
        let slow_shell = slow_dir.path().join("zsh");
        std::fs::write(&slow_shell, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        let mut permissions = std::fs::metadata(&slow_shell).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&slow_shell, permissions).unwrap();
        let result = resolve_shell_env(
            slow_shell.to_str().unwrap(),
            Duration::from_millis(20),
            Instant::now(),
        );
        assert_eq!(result.outcome, DiscoveryOutcome::Timeout);
        assert_eq!(
            capture_outcome(&LoginShellEnv::default()),
            DiscoveryOutcome::EmptyCapture
        );
    }
}

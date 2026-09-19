// Install Runner's bundled CLI sidecars under `$APPDATA/runner/bin/`.
// Child PTYs get `runner` on PATH for mission coordination, while MCP
// clients launch `runner-mcp` directly from their config files.
//
// Naming. The source-side agent binary remains `runner-agent-cli`; this
// installer renames it to `runner` in app data so spawned PTYs get the
// intended user-facing command without colliding with another `runner`
// artifact in a shared target directory. The GPUI binary is `Runner`. The
// MCP proxy is a separate `runner-mcp` binary and is installed as-is.
//
// Source resolution. Development builds and release packaging leave
// `runner-agent-cli` and `runner-mcp` next to the `Runner` executable;
// `locate_source` resolves them from that directory by name.
//
// Skip-if-current optimization. Compare (size, mtime) — if the source
// file's mtime is `<=` the destination's AND sizes match, skip the
// copy. Hash-compare would be slower without buying anything for the
// "rebuilt-CLI mtime moves forward" case.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use runner_core::command_install::{self, PathStyle};
pub use runner_core::command_install::{
    runner_command_name, RunnerCommandState, RunnerCommandStatus,
};

/// Source-side agent CLI artifact. Installed into app data as `runner`.
const AGENT_SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-agent-cli.exe"
} else {
    "runner-agent-cli"
};

/// Source-side MCP proxy artifact. Installed into app data as `runner-mcp`.
const MCP_SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

/// Name of the agent CLI we drop into `$APPDATA/runner/bin/`. Must match what
/// `SessionManager::spawn` puts on PATH — arch §5.3 Layer 2 has the
/// CLI being invoked as bare `runner` from inside spawned PTYs.
const AGENT_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner.exe"
} else {
    "runner"
};

/// Name of the MCP proxy binary registered with Claude Code, Codex, TRAE, and GitHub Copilot CLI.
pub const MCP_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

// Called from the app's `boot_core` on every launch, before any session can
// spawn or MCP config is written. Mission shims, spawned PATHs, and MCP
// configs all consume the destinations.
pub fn install_runner_cli(app_data_dir: &Path) -> Result<()> {
    install_binary(app_data_dir, AGENT_SOURCE_BIN_NAME, AGENT_DEST_BIN_NAME)
}

pub fn install_mcp_cli(app_data_dir: &Path) -> Result<()> {
    install_binary(app_data_dir, MCP_SOURCE_BIN_NAME, MCP_DEST_BIN_NAME)
}

fn install_binary(app_data_dir: &Path, source_name: &str, dest_name: &str) -> Result<()> {
    let Some(source) = locate_source(source_name)? else {
        log::warn!(
            "bundled CLI sidecar ({source_name}) not found next to current_exe; \
             skipping install of {dest_name}. Build the CLI sidecars and \
             relaunch."
        );
        return Ok(());
    };
    let dest_dir = app_data_dir.join("bin");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(dest_name);

    if up_to_date(&source, &dest)? {
        return Ok(());
    }

    // Copy via tempfile + rename to keep the swap atomic — a half-written
    // file would crash the next process that runs this sidecar.
    let tmp = tempfile::NamedTempFile::new_in(&dest_dir)?;
    std::fs::copy(&source, tmp.path())?;
    tmp.persist(&dest).map_err(|e| Error::Io(e.error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }
    Ok(())
}

/// Drop a per-(mission,slot) `runner` shim into
/// `$APPDATA/missions/<mission_id>/shims/<handle>/bin/runner` that
/// hardcodes the slot's `RUNNER_*` env vars and `exec`s the real
/// bundled CLI. PATH inside the spawned PTY prepends this dir, so
/// `runner …` resolves to the shim regardless of what shell context
/// the agent CLI's tool-call subprocess runs under. Without this,
/// claude-code's Bash tool spawns a non-login shell that doesn't
/// inherit the PTY's env, and the bundled CLI exits with "missing
/// required env var".
///
/// Each call rewrites the shim atomically (tempfile + rename) so
/// resume can refresh the values without leaving a half-written
/// file an agent could crash on. The path is keyed by mission_id +
/// handle (not session_id) because session_id rotates on every
/// resume, while the env vars don't — the shim is reusable across
/// resumes of the same slot.
pub fn install_session_runner_shim(
    app_data_dir: &Path,
    crew_id: &str,
    mission_id: &str,
    handle: &str,
    event_log: &Path,
    mission_cwd: Option<&str>,
) -> Result<PathBuf> {
    let shim_dir = app_data_dir
        .join("missions")
        .join(mission_id)
        .join("shims")
        .join(handle)
        .join("bin");
    std::fs::create_dir_all(&shim_dir)?;
    let shim_path = shim_dir.join("runner");
    let real_runner = app_data_dir.join("bin").join(AGENT_DEST_BIN_NAME);

    let event_log_str = event_log.to_string_lossy();
    #[cfg(windows)]
    let real_runner = PathBuf::from(real_runner.to_string_lossy().replace('\\', "/"));
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script
        .push_str("# Auto-generated session shim. See cli_install::install_session_runner_shim.\n");
    script.push_str(&format!("export RUNNER_CREW_ID='{}'\n", sh_escape(crew_id)));
    script.push_str(&format!(
        "export RUNNER_MISSION_ID='{}'\n",
        sh_escape(mission_id)
    ));
    script.push_str(&format!("export RUNNER_HANDLE='{}'\n", sh_escape(handle)));
    script.push_str(&format!(
        "export RUNNER_EVENT_LOG='{}'\n",
        sh_escape(&event_log_str)
    ));
    if let Some(cwd) = mission_cwd {
        script.push_str(&format!("export MISSION_CWD='{}'\n", sh_escape(cwd)));
    }
    script.push_str(&format!(
        "exec '{}' \"$@\"\n",
        sh_escape(&real_runner.to_string_lossy())
    ));

    let tmp = tempfile::NamedTempFile::new_in(&shim_dir)?;
    std::fs::write(tmp.path(), script.as_bytes())?;
    tmp.persist(&shim_path).map_err(|e| Error::Io(e.error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&shim_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&shim_path, perms)?;
    }
    #[cfg(windows)]
    {
        let cmd_script = windows_cmd_shim(
            &app_data_dir.join("bin").join(AGENT_DEST_BIN_NAME),
            crew_id,
            mission_id,
            handle,
            &event_log_str,
            mission_cwd,
        )?;
        let tmp = tempfile::NamedTempFile::new_in(&shim_dir)?;
        std::fs::write(tmp.path(), cmd_script.as_bytes())?;
        tmp.persist(shim_dir.join("runner.cmd"))
            .map_err(|e| Error::Io(e.error))?;
    }
    Ok(shim_dir)
}

#[cfg(windows)]
fn windows_cmd_shim(
    real_runner: &Path,
    crew_id: &str,
    mission_id: &str,
    handle: &str,
    event_log: &str,
    mission_cwd: Option<&str>,
) -> Result<String> {
    let real_runner = real_runner.to_string_lossy();
    let mut script = String::from("@echo off\r\nsetlocal\r\n");
    for (name, value) in [
        ("RUNNER_CREW_ID", Some(crew_id)),
        ("RUNNER_MISSION_ID", Some(mission_id)),
        ("RUNNER_HANDLE", Some(handle)),
        ("RUNNER_EVENT_LOG", Some(event_log)),
        ("MISSION_CWD", mission_cwd),
    ] {
        if let Some(value) = value {
            if value.contains('"') {
                return Err(Error::msg(format!("{name} contains a quote")));
            }
            let value = value.replace('%', "%%");
            script.push_str(&format!("set \"{name}={value}\"\r\n"));
        }
    }
    if real_runner.contains('"') {
        return Err(Error::msg("runner path contains a quote"));
    }
    let real_runner = real_runner.replace('%', "%%");
    script.push_str(&format!("\"{real_runner}\" %*\r\n"));
    Ok(script)
}

/// Escape a string for inside single-quoted POSIX shell. Single
/// quotes can't contain themselves; the canonical workaround is to
/// close the quote, emit `'\''`, and reopen.
fn sh_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn locate_source(source_name: &str) -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::msg("current_exe has no parent"))?;
    let candidate = dir.join(source_name);
    // The app executable is `Runner`, so the equality guard only protects
    // future renames from copying the running executable over itself; the
    // candidate must also exist.
    if candidate.exists() && candidate != exe {
        return Ok(Some(candidate));
    }
    Ok(None)
}

fn up_to_date(source: &Path, dest: &Path) -> Result<bool> {
    let Ok(dst_meta) = std::fs::metadata(dest) else {
        return Ok(false);
    };
    let src_meta = std::fs::metadata(source)?;
    if src_meta.len() != dst_meta.len() {
        return Ok(false);
    }
    let src_mtime = src_meta.modified().ok();
    let dst_mtime = dst_meta.modified().ok();
    match (src_mtime, dst_mtime) {
        (Some(s), Some(d)) => Ok(s <= d),
        _ => Ok(false),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPlatform {
    Unix,
    Windows,
}

#[derive(Clone, Debug)]
pub struct CommandInstallInputs {
    pub home: PathBuf,
    pub login_path: String,
    pub system_path: String,
    pub sidecar: PathBuf,
    pub local_bin: PathBuf,
    pub system_bin: PathBuf,
    pub system_bin_writable: bool,
    pub debug: bool,
    pub platform: CommandPlatform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryValueKind {
    String,
    ExpandString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryPathValue {
    pub value: String,
    pub kind: RegistryValueKind,
}

pub trait UserPathRegistry {
    fn read_path(&self) -> Result<Option<RegistryPathValue>>;
    fn write_path(&mut self, value: &RegistryPathValue) -> Result<()>;
    fn broadcast_environment_change(&mut self) -> Result<()>;
}

pub struct NoUserPathRegistry;

impl UserPathRegistry for NoUserPathRegistry {
    fn read_path(&self) -> Result<Option<RegistryPathValue>> {
        Ok(None)
    }

    fn write_path(&mut self, _value: &RegistryPathValue) -> Result<()> {
        Err(Error::msg("Windows user PATH registry is unavailable"))
    }

    fn broadcast_environment_change(&mut self) -> Result<()> {
        Err(Error::msg("Windows environment broadcast is unavailable"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscalationOutcome {
    Completed,
    Cancelled,
}

pub trait CommandEscalation {
    fn run(&mut self, apple_script: &str) -> Result<EscalationOutcome>;
}

#[cfg(target_os = "macos")]
pub struct OsascriptEscalation;

#[cfg(target_os = "macos")]
impl CommandEscalation for OsascriptEscalation {
    fn run(&mut self, apple_script: &str) -> Result<EscalationOutcome> {
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", apple_script])
            .output()?;
        if output.status.success() {
            return Ok(EscalationOutcome::Completed);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("-128") {
            return Ok(EscalationOutcome::Cancelled);
        }
        Err(Error::msg(format!(
            "error running osascript: {}",
            stderr.trim()
        )))
    }
}

pub struct NoEscalation;

impl CommandEscalation for NoEscalation {
    fn run(&mut self, _apple_script: &str) -> Result<EscalationOutcome> {
        Err(Error::msg("administrator escalation is unavailable"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandActionOutcome {
    Installed(PathBuf),
    AlreadyInstalled(PathBuf),
    Removed(PathBuf),
    NotInstalled,
    Foreign(PathBuf),
    NoTarget,
    Cancelled,
    Unsupported,
}

pub fn command_status(
    inputs: &CommandInstallInputs,
    registry: &dyn UserPathRegistry,
) -> Result<RunnerCommandStatus> {
    match inputs.platform {
        CommandPlatform::Unix => Ok(command_install::inspect_unix_command(
            &inputs.sidecar,
            &inputs.login_path,
            &inputs.local_bin,
            &inputs.system_bin,
            inputs.debug,
        )),
        CommandPlatform::Windows => {
            let user_path = registry
                .read_path()?
                .map(|value| value.value)
                .unwrap_or_default();
            let sidecar_dir = windows_sidecar_dir(&inputs.sidecar)?;
            Ok(command_install::inspect_windows_command(
                &sidecar_dir,
                &user_path,
                &inputs.system_path,
                inputs.debug,
            ))
        }
    }
}

pub fn default_command_target(
    inputs: &CommandInstallInputs,
    registry: &dyn UserPathRegistry,
) -> Result<Option<PathBuf>> {
    match inputs.platform {
        CommandPlatform::Windows => {
            if inputs.debug {
                Ok(None)
            } else {
                let _ = registry.read_path()?;
                Ok(Some(windows_sidecar_dir(&inputs.sidecar)?))
            }
        }
        CommandPlatform::Unix => {
            let entries = command_install::path_entries(&inputs.login_path, PathStyle::Unix);
            if entries.iter().any(|entry| {
                command_install::path_entry_matches(entry, &inputs.local_bin, PathStyle::Unix)
            }) {
                return Ok(Some(inputs.local_bin.clone()));
            }
            if inputs.system_bin_writable
                && entries.iter().any(|entry| {
                    command_install::path_entry_matches(entry, &inputs.system_bin, PathStyle::Unix)
                })
            {
                return Ok(Some(inputs.system_bin.clone()));
            }
            Ok(None)
        }
    }
}

pub fn directory_writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

pub fn install_command_default(
    inputs: &CommandInstallInputs,
    registry: &mut dyn UserPathRegistry,
    escalation: &mut dyn CommandEscalation,
) -> Result<CommandActionOutcome> {
    match inputs.platform {
        CommandPlatform::Windows => install_windows(inputs, registry),
        CommandPlatform::Unix => {
            let status = command_status(inputs, registry)?;
            if let Some(outcome) = existing_install_outcome(&status) {
                return Ok(outcome);
            }
            let Some(target) = default_command_target(inputs, registry)? else {
                return Ok(CommandActionOutcome::NoTarget);
            };
            install_unix(inputs, &target, false, escalation)
        }
    }
}

pub fn install_command_explicit(
    inputs: &CommandInstallInputs,
    registry: &mut dyn UserPathRegistry,
    escalation: &mut dyn CommandEscalation,
    force_escalated: bool,
) -> Result<CommandActionOutcome> {
    match inputs.platform {
        CommandPlatform::Windows => install_windows(inputs, registry),
        CommandPlatform::Unix => {
            let status = command_status(inputs, registry)?;
            if let Some(outcome) = existing_install_outcome(&status) {
                return Ok(outcome);
            }
            let target = if force_escalated {
                inputs.system_bin.clone()
            } else {
                default_command_target(inputs, registry)?
                    .unwrap_or_else(|| inputs.system_bin.clone())
            };
            install_unix(inputs, &target, true, escalation)
        }
    }
}

pub fn explicit_command_target_path(
    inputs: &CommandInstallInputs,
    registry: &dyn UserPathRegistry,
    force_escalated: bool,
) -> Result<PathBuf> {
    match inputs.platform {
        CommandPlatform::Windows => windows_sidecar_dir(&inputs.sidecar),
        CommandPlatform::Unix => {
            let target = if force_escalated {
                inputs.system_bin.clone()
            } else {
                default_command_target(inputs, registry)?
                    .unwrap_or_else(|| inputs.system_bin.clone())
            };
            Ok(target.join(command_install::runner_command_name(inputs.debug)))
        }
    }
}

pub fn uninstall_command(
    inputs: &CommandInstallInputs,
    registry: &mut dyn UserPathRegistry,
    escalation: &mut dyn CommandEscalation,
) -> Result<CommandActionOutcome> {
    match inputs.platform {
        CommandPlatform::Windows => uninstall_windows(inputs, registry),
        CommandPlatform::Unix => {
            let status = command_status(inputs, registry)?;
            match status.state {
                RunnerCommandState::Installed | RunnerCommandState::Shadowed => {
                    let path = status.path.expect("installed command has a path");
                    match std::fs::remove_file(&path) {
                        Ok(()) => Ok(CommandActionOutcome::Removed(path)),
                        Err(_) => match escalation.run(&escalated_remove_script(&path))? {
                            EscalationOutcome::Completed => Ok(CommandActionOutcome::Removed(path)),
                            EscalationOutcome::Cancelled => Ok(CommandActionOutcome::Cancelled),
                        },
                    }
                }
                RunnerCommandState::Foreign => Ok(CommandActionOutcome::Foreign(
                    status.path.expect("foreign command has a path"),
                )),
                RunnerCommandState::NotInstalled => Ok(CommandActionOutcome::NotInstalled),
                RunnerCommandState::Unsupported => Ok(CommandActionOutcome::Unsupported),
            }
        }
    }
}

pub fn force_escalated_command_install() -> bool {
    #[cfg(debug_assertions)]
    {
        force_escalated_setting(
            true,
            std::env::var("RUNNER_COMMAND_INSTALL_FORCE_ESCALATED")
                .ok()
                .as_deref(),
        )
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

fn force_escalated_setting(debug: bool, value: Option<&str>) -> bool {
    debug && value == Some("1")
}

pub fn escalated_link_script(sidecar: &Path, link: &Path) -> String {
    let parent = link.parent().unwrap_or(Path::new("/usr/local/bin"));
    let shell = format!(
        "mkdir -p {} && ln -sf {} {}",
        shell_quote(parent),
        shell_quote(sidecar),
        shell_quote(link),
    );
    format!(
        "do shell script \"{}\" with administrator privileges",
        apple_script_escape(&shell)
    )
}

fn escalated_remove_script(link: &Path) -> String {
    let shell = format!("rm -f {}", shell_quote(link));
    format!(
        "do shell script \"{}\" with administrator privileges",
        apple_script_escape(&shell)
    )
}

fn existing_install_outcome(status: &RunnerCommandStatus) -> Option<CommandActionOutcome> {
    match status.state {
        RunnerCommandState::Installed | RunnerCommandState::Shadowed => {
            Some(CommandActionOutcome::AlreadyInstalled(
                status.path.clone().expect("installed command has a path"),
            ))
        }
        RunnerCommandState::Foreign => Some(CommandActionOutcome::Foreign(
            status.path.clone().expect("foreign command has a path"),
        )),
        RunnerCommandState::Unsupported => Some(CommandActionOutcome::Unsupported),
        RunnerCommandState::NotInstalled => None,
    }
}

fn install_unix(
    inputs: &CommandInstallInputs,
    target_dir: &Path,
    allow_escalation: bool,
    escalation: &mut dyn CommandEscalation,
) -> Result<CommandActionOutcome> {
    let link = target_dir.join(command_install::runner_command_name(inputs.debug));
    if std::fs::symlink_metadata(&link).is_ok() {
        return if command_install::command_link_is_owned(&link, &inputs.sidecar) {
            Ok(CommandActionOutcome::AlreadyInstalled(link))
        } else {
            Ok(CommandActionOutcome::Foreign(link))
        };
    }
    let plain = std::fs::create_dir_all(target_dir).and_then(|()| {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&inputs.sidecar, &link)
        }
        #[cfg(not(unix))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unix symlinks are unavailable",
            ))
        }
    });
    match plain {
        Ok(()) => Ok(CommandActionOutcome::Installed(link)),
        Err(error) if !allow_escalation => Err(error.into()),
        Err(_) => match escalation.run(&escalated_link_script(&inputs.sidecar, &link))? {
            EscalationOutcome::Completed => Ok(CommandActionOutcome::Installed(link)),
            EscalationOutcome::Cancelled => Ok(CommandActionOutcome::Cancelled),
        },
    }
}

fn install_windows(
    inputs: &CommandInstallInputs,
    registry: &mut dyn UserPathRegistry,
) -> Result<CommandActionOutcome> {
    if inputs.debug {
        return Ok(CommandActionOutcome::Unsupported);
    }
    let sidecar_dir = windows_sidecar_dir(&inputs.sidecar)?;
    let mut current = registry.read_path()?.unwrap_or(RegistryPathValue {
        value: String::new(),
        kind: RegistryValueKind::String,
    });
    if command_install::path_entries(&current.value, PathStyle::Windows)
        .iter()
        .any(|entry| command_install::path_entry_matches(entry, &sidecar_dir, PathStyle::Windows))
    {
        return Ok(CommandActionOutcome::AlreadyInstalled(sidecar_dir));
    }
    if !current.value.is_empty() && !current.value.ends_with(';') {
        current.value.push(';');
    }
    current.value.push_str(&sidecar_dir.to_string_lossy());
    registry.write_path(&current)?;
    registry.broadcast_environment_change()?;
    Ok(CommandActionOutcome::Installed(sidecar_dir))
}

fn uninstall_windows(
    inputs: &CommandInstallInputs,
    registry: &mut dyn UserPathRegistry,
) -> Result<CommandActionOutcome> {
    if inputs.debug {
        return Ok(CommandActionOutcome::Unsupported);
    }
    let sidecar_dir = windows_sidecar_dir(&inputs.sidecar)?;
    let Some(mut current) = registry.read_path()? else {
        return Ok(CommandActionOutcome::NotInstalled);
    };
    let entries = current.value.split(';').collect::<Vec<_>>();
    let kept = entries
        .iter()
        .copied()
        .filter(|entry| {
            !command_install::path_entry_matches(Path::new(entry), &sidecar_dir, PathStyle::Windows)
        })
        .collect::<Vec<_>>();
    if kept.len() == entries.len() {
        return Ok(CommandActionOutcome::NotInstalled);
    }
    current.value = kept.join(";");
    registry.write_path(&current)?;
    registry.broadcast_environment_change()?;
    Ok(CommandActionOutcome::Removed(sidecar_dir))
}

fn windows_sidecar_dir(sidecar: &Path) -> Result<PathBuf> {
    let value = sidecar.to_string_lossy();
    let index = value
        .rfind(['\\', '/'])
        .ok_or_else(|| Error::msg("Runner sidecar has no parent directory"))?;
    Ok(PathBuf::from(&value[..index]))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", sh_escape(&path.to_string_lossy()))
}

fn apple_script_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(windows)]
pub struct SystemUserPathRegistry;

#[cfg(windows)]
impl UserPathRegistry for SystemUserPathRegistry {
    fn read_path(&self) -> Result<Option<RegistryPathValue>> {
        windows_registry::read_user_path()
    }

    fn write_path(&mut self, value: &RegistryPathValue) -> Result<()> {
        windows_registry::write_user_path(value)
    }

    fn broadcast_environment_change(&mut self) -> Result<()> {
        windows_registry::broadcast_environment_change()
    }
}

#[cfg(windows)]
mod windows_registry {
    use super::*;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    pub(super) fn read_user_path() -> Result<Option<RegistryPathValue>> {
        let key = open_environment(KEY_QUERY_VALUE)?;
        let name = wide("Path");
        let mut kind = 0;
        let mut bytes = 0;
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            unsafe { RegCloseKey(key) };
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            let result = check(status, "read HKCU\\Environment Path size");
            unsafe { RegCloseKey(key) };
            result?;
        }
        let mut data = vec![0u16; bytes as usize / 2];
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        unsafe { RegCloseKey(key) };
        check(status, "read HKCU\\Environment Path")?;
        while data.last() == Some(&0) {
            data.pop();
        }
        let kind = match kind {
            REG_SZ => RegistryValueKind::String,
            REG_EXPAND_SZ => RegistryValueKind::ExpandString,
            other => return Err(Error::msg(format!("unsupported user Path type {other}"))),
        };
        Ok(Some(RegistryPathValue {
            value: String::from_utf16_lossy(&data),
            kind,
        }))
    }

    pub(super) fn write_user_path(value: &RegistryPathValue) -> Result<()> {
        let key = open_environment(KEY_SET_VALUE)?;
        let name = wide("Path");
        let data = wide(&value.value);
        let kind = match value.kind {
            RegistryValueKind::String => REG_SZ,
            RegistryValueKind::ExpandString => REG_EXPAND_SZ,
        };
        let status = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                kind,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        unsafe { RegCloseKey(key) };
        check(status, "write HKCU\\Environment Path")
    }

    pub(super) fn broadcast_environment_change() -> Result<()> {
        let environment = wide("Environment");
        let mut result = 0;
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            )
        };
        if sent == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    fn open_environment(access: u32) -> Result<HKEY> {
        let subkey = wide("Environment");
        let mut key = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, access, &mut key) };
        check(status, "open HKCU\\Environment")?;
        Ok(key)
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(status: u32, operation: &str) -> Result<()> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(Error::msg(format!("{operation} failed with code {status}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[derive(Default)]
    struct FakeRegistry {
        value: Option<RegistryPathValue>,
        writes: Vec<RegistryPathValue>,
        broadcasts: usize,
    }

    impl UserPathRegistry for FakeRegistry {
        fn read_path(&self) -> Result<Option<RegistryPathValue>> {
            Ok(self.value.clone())
        }

        fn write_path(&mut self, value: &RegistryPathValue) -> Result<()> {
            self.value = Some(value.clone());
            self.writes.push(value.clone());
            Ok(())
        }

        fn broadcast_environment_change(&mut self) -> Result<()> {
            self.broadcasts += 1;
            Ok(())
        }
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    #[derive(Default)]
    struct FakeEscalation {
        scripts: Vec<String>,
        outcome: Option<EscalationOutcome>,
    }

    impl CommandEscalation for FakeEscalation {
        fn run(&mut self, apple_script: &str) -> Result<EscalationOutcome> {
            self.scripts.push(apple_script.to_owned());
            Ok(self.outcome.unwrap_or(EscalationOutcome::Completed))
        }
    }

    fn unix_inputs(temp: &Path, login_path: String) -> CommandInstallInputs {
        CommandInstallInputs {
            home: temp.join("home"),
            login_path,
            system_path: String::new(),
            sidecar: temp.join("Application Support/runner/bin/runner"),
            local_bin: temp.join("home/.local/bin"),
            system_bin: temp.join("usr/local/bin"),
            system_bin_writable: true,
            debug: false,
            platform: CommandPlatform::Unix,
        }
    }

    fn windows_inputs(temp: &Path, debug: bool) -> CommandInstallInputs {
        CommandInstallInputs {
            home: temp.join("home"),
            login_path: String::new(),
            system_path: String::new(),
            sidecar: PathBuf::from(r"C:\Program Files\Runner\bin\runner.exe"),
            local_bin: PathBuf::new(),
            system_bin: PathBuf::new(),
            system_bin_writable: false,
            debug,
            platform: CommandPlatform::Windows,
        }
    }

    #[test]
    fn default_target_prefers_local_then_writable_system_and_requires_path_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let registry = FakeRegistry::default();
        let local = root.join("home/.local/bin");
        let system = root.join("usr/local/bin");

        let inputs = unix_inputs(root, format!("{}:/usr/bin", local.display()));
        assert_eq!(
            default_command_target(&inputs, &registry).unwrap(),
            Some(local.clone())
        );

        let inputs = unix_inputs(root, format!("{}/:/usr/bin", system.display()));
        assert_eq!(
            default_command_target(&inputs, &registry).unwrap(),
            Some(system.clone())
        );

        let mut inputs = unix_inputs(root, system.display().to_string());
        inputs.system_bin_writable = false;
        assert_eq!(default_command_target(&inputs, &registry).unwrap(), None);

        let inputs = unix_inputs(root, "/usr/bin:/bin".into());
        assert_eq!(default_command_target(&inputs, &registry).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn unix_install_is_idempotent_reports_foreign_and_uninstalls_only_owned_link() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("home/.local/bin");
        let inputs = unix_inputs(temp.path(), local.display().to_string());
        fs::create_dir_all(inputs.sidecar.parent().unwrap()).unwrap();
        fs::write(&inputs.sidecar, "sidecar").unwrap();
        let mut registry = FakeRegistry::default();
        let mut escalation = FakeEscalation::default();

        assert_eq!(
            install_command_default(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Installed(local.join("runner"))
        );
        assert_eq!(fs::read_link(local.join("runner")).unwrap(), inputs.sidecar);
        assert_eq!(
            install_command_default(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::AlreadyInstalled(local.join("runner"))
        );
        assert!(escalation.scripts.is_empty());
        assert_eq!(
            uninstall_command(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Removed(local.join("runner"))
        );
        assert!(!local.join("runner").exists());

        fs::write(local.join("runner"), "foreign").unwrap();
        assert_eq!(
            install_command_default(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Foreign(local.join("runner"))
        );
        assert_eq!(fs::read_to_string(local.join("runner")).unwrap(), "foreign");
        assert!(matches!(
            uninstall_command(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Foreign(_)
        ));

        fs::remove_file(local.join("runner")).unwrap();
        symlink(temp.path().join("somewhere-else"), local.join("runner")).unwrap();
        assert!(matches!(
            install_command_default(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Foreign(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_status_reports_shadowing_executable() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        let temp = tempfile::tempdir().unwrap();
        let shadow_dir = temp.path().join("shadow");
        let local = temp.path().join("home/.local/bin");
        let inputs = unix_inputs(
            temp.path(),
            format!("{}:{}", shadow_dir.display(), local.display()),
        );
        fs::create_dir_all(&shadow_dir).unwrap();
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(inputs.sidecar.parent().unwrap()).unwrap();
        fs::write(&inputs.sidecar, "sidecar").unwrap();
        symlink(&inputs.sidecar, local.join("runner")).unwrap();
        let shadow = shadow_dir.join("runner");
        fs::write(&shadow, "shadow").unwrap();
        fs::set_permissions(&shadow, fs::Permissions::from_mode(0o755)).unwrap();

        let status = command_status(&inputs, &FakeRegistry::default()).unwrap();
        assert_eq!(status.state, RunnerCommandState::Shadowed);
        assert_eq!(status.shadowed_by, Some(shadow));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_install_escalates_once_with_two_layers_of_quoting_and_cancel_is_normal() {
        let temp = tempfile::tempdir().unwrap();
        let mut inputs = unix_inputs(temp.path(), "/usr/bin".into());
        inputs.sidecar = temp.path().join("Application Support/Runner's bin/runner");
        let blocked = temp.path().join("blocked");
        fs::write(&blocked, "not a directory").unwrap();
        inputs.system_bin = blocked.join("usr/local/bin");
        let mut registry = FakeRegistry::default();
        let mut escalation = FakeEscalation::default();

        assert!(matches!(
            install_command_explicit(&inputs, &mut registry, &mut escalation, false).unwrap(),
            CommandActionOutcome::Installed(_)
        ));
        assert_eq!(escalation.scripts.len(), 1);
        assert_eq!(
            escalation.scripts[0],
            escalated_link_script(&inputs.sidecar, &inputs.system_bin.join("runner"))
        );
        assert!(escalation.scripts[0].contains("Application Support"));
        assert!(escalation.scripts[0].contains("Runner'\\\\''s bin"));

        escalation.outcome = Some(EscalationOutcome::Cancelled);
        assert_eq!(
            install_command_explicit(&inputs, &mut registry, &mut escalation, false).unwrap(),
            CommandActionOutcome::Cancelled
        );
        assert_eq!(escalation.scripts.len(), 2);
    }

    #[test]
    fn escalated_link_script_is_exact_for_spaces_and_a_single_quote() {
        assert_eq!(
            escalated_link_script(
                Path::new("/Users/jason/Library/Application Support/Runner's bin/runner"),
                Path::new("/usr/local/bin/runner"),
            ),
            r#"do shell script "mkdir -p '/usr/local/bin' && ln -sf '/Users/jason/Library/Application Support/Runner'\\''s bin/runner' '/usr/local/bin/runner'" with administrator privileges"#,
        );
    }

    #[test]
    fn windows_path_edit_adds_once_removes_only_runner_and_preserves_type() {
        let temp = tempfile::tempdir().unwrap();
        let inputs = windows_inputs(temp.path(), false);
        let mut registry = FakeRegistry {
            value: Some(RegistryPathValue {
                value: r"C:\Tools;C:\Windows".into(),
                kind: RegistryValueKind::ExpandString,
            }),
            ..FakeRegistry::default()
        };

        assert!(matches!(
            install_command_default(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::Installed(_)
        ));
        assert_eq!(registry.writes.len(), 1);
        assert_eq!(registry.writes[0].kind, RegistryValueKind::ExpandString);
        assert_eq!(
            registry.writes[0].value,
            r"C:\Tools;C:\Windows;C:\Program Files\Runner\bin"
        );
        assert_eq!(registry.broadcasts, 1);

        assert!(matches!(
            install_command_default(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::AlreadyInstalled(_)
        ));
        assert_eq!(registry.writes.len(), 1);

        registry.value.as_mut().unwrap().value =
            r"C:\Tools;c:\PROGRAM FILES\runner\BIN\;C:\Windows".into();
        assert!(matches!(
            install_command_default(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::AlreadyInstalled(_)
        ));
        assert_eq!(registry.writes.len(), 1);

        assert!(matches!(
            uninstall_command(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::Removed(_)
        ));
        assert_eq!(
            registry.writes.last().unwrap().value,
            r"C:\Tools;C:\Windows"
        );
        assert_eq!(
            registry.writes.last().unwrap().kind,
            RegistryValueKind::ExpandString
        );
        assert_eq!(registry.broadcasts, 2);
    }

    #[test]
    fn windows_debug_build_is_unsupported_and_edits_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let inputs = windows_inputs(temp.path(), true);
        let mut registry = FakeRegistry::default();
        assert_eq!(
            install_command_default(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::Unsupported
        );
        assert_eq!(
            uninstall_command(&inputs, &mut registry, &mut NoEscalation).unwrap(),
            CommandActionOutcome::Unsupported
        );
        assert!(registry.writes.is_empty());
        assert_eq!(registry.broadcasts, 0);
    }

    #[test]
    fn forced_escalation_setting_is_ignored_for_release_builds() {
        assert!(force_escalated_setting(true, Some("1")));
        assert!(!force_escalated_setting(false, Some("1")));
        assert!(!force_escalated_setting(true, Some("0")));
    }

    #[test]
    fn install_copies_source_to_dest_and_renames() {
        // Stage a fake source binary next to a fake current_exe and
        // assert install_runner_cli puts it at $APPDATA/bin/runner with
        // executable permissions on Unix.
        let workspace = tempfile::tempdir().unwrap();
        let exe_dir = workspace.path().join("target/debug");
        fs::create_dir_all(&exe_dir).unwrap();

        // Fake the CLI artifact next to the (would-be) current_exe.
        let source = exe_dir.join(AGENT_SOURCE_BIN_NAME);
        {
            let mut f = fs::File::create(&source).unwrap();
            writeln!(f, "#!/bin/sh\necho fake").unwrap();
        }
        // Note: this test exercises the copy logic indirectly. We call
        // through the public install fn against an `app_data_dir` that
        // is just a tempdir; locate_source uses `current_exe()`, which
        // for `cargo test` returns the test binary itself, not our
        // fake — so we'd skip with "not found". To make the test
        // meaningful, we exercise the up_to_date and copy helpers
        // directly instead. install_runner_cli's prod path is covered
        // manually until end-to-end packaging tests land.
        let app_data = tempfile::tempdir().unwrap();
        let bin_dir = app_data.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let dest = bin_dir.join(AGENT_DEST_BIN_NAME);

        // First copy: dest doesn't exist, must be replaced.
        assert!(!up_to_date(&source, &dest).unwrap());
        let tmp = tempfile::NamedTempFile::new_in(&bin_dir).unwrap();
        std::fs::copy(&source, tmp.path()).unwrap();
        tmp.persist(&dest).unwrap();
        assert!(dest.exists());
        assert_eq!(
            fs::metadata(&source).unwrap().len(),
            fs::metadata(&dest).unwrap().len()
        );

        // Second copy: dest now matches by size+mtime, should skip.
        assert!(up_to_date(&source, &dest).unwrap());
    }

    #[test]
    fn shim_dir_includes_mission_id_so_concurrent_missions_dont_collide() {
        // Regression guard for #55: when the per-crew "at most one live
        // mission" cap was lifted, two missions on the same crew can
        // run side by side. They share `crew_id` and (when the same
        // slot template is on both rosters) `slot_handle`, so the
        // shim's path key MUST also include `mission_id` to keep the
        // two RUNNER_* env exports separate. Two installs differing
        // only in `mission_id` must produce different dirs and
        // different baked env values.
        let app_data = tempfile::tempdir().unwrap();
        let event_log_a = app_data.path().join("missions/m-a/events.jsonl");
        let event_log_b = app_data.path().join("missions/m-b/events.jsonl");
        // The shim writer needs the source bin (for the `exec` line).
        // Stage a fake bundled CLI so the install has something to
        // point at — content is irrelevant; the shim just embeds the
        // path.
        let bin_dir = app_data.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join(AGENT_DEST_BIN_NAME), "#!/bin/sh\nexit 0\n").unwrap();

        let dir_a = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "m-a",
            "architect",
            &event_log_a,
            None,
        )
        .unwrap();
        let dir_b = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "m-b",
            "architect",
            &event_log_b,
            None,
        )
        .unwrap();

        assert_ne!(
            dir_a, dir_b,
            "shim dirs for two missions on the same crew + slot must differ",
        );
        #[cfg(unix)]
        assert!(
            dir_a.to_string_lossy().contains("/m-a/"),
            "dir_a must include mission_id m-a: {dir_a:?}",
        );
        #[cfg(windows)]
        assert!(
            dir_a.components().any(|part| part.as_os_str() == "m-a"),
            "dir_a must include mission_id m-a: {dir_a:?}",
        );
        #[cfg(unix)]
        assert!(
            dir_b.to_string_lossy().contains("/m-b/"),
            "dir_b must include mission_id m-b: {dir_b:?}",
        );
        #[cfg(windows)]
        assert!(
            dir_b.components().any(|part| part.as_os_str() == "m-b"),
            "dir_b must include mission_id m-b: {dir_b:?}",
        );

        // The baked RUNNER_MISSION_ID export must match the dir's
        // mission_id, not leak across — without this guarantee a slot
        // running in mission m-a could attribute events to m-b.
        let script_a = std::fs::read_to_string(dir_a.join("runner")).unwrap();
        let script_b = std::fs::read_to_string(dir_b.join("runner")).unwrap();
        assert!(
            script_a.contains("export RUNNER_MISSION_ID='m-a'"),
            "shim_a must export the m-a mission id: {script_a}",
        );
        assert!(
            script_b.contains("export RUNNER_MISSION_ID='m-b'"),
            "shim_b must export the m-b mission id: {script_b}",
        );
    }

    #[test]
    fn session_shim_has_exact_shell_contents() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        let shim_dir = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "mission-1",
            "coder",
            &event_log,
            Some("a'b"),
        )
        .unwrap();
        let runner_path = app_data.path().join("bin").join(AGENT_DEST_BIN_NAME);
        let runner_path = runner_path.to_string_lossy();
        #[cfg(windows)]
        let runner_path = runner_path.replace('\\', "/");
        assert_eq!(
            fs::read_to_string(shim_dir.join("runner")).unwrap(),
            format!(
                "#!/bin/sh\n# Auto-generated session shim. See cli_install::install_session_runner_shim.\nexport RUNNER_CREW_ID='crew-1'\nexport RUNNER_MISSION_ID='mission-1'\nexport RUNNER_HANDLE='coder'\nexport RUNNER_EVENT_LOG='{}'\nexport MISSION_CWD='a'\\''b'\nexec '{}' \"$@\"\n",
                event_log.display(), runner_path,
            ),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(shim_dir.join("runner"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            assert!(!shim_dir.join("runner.cmd").exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn session_shim_has_exact_cmd_contents_and_refreshes_both_files() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        for cwd in [Some(r"C:\Agent Tools\repo"), None] {
            let shim_dir = install_session_runner_shim(
                app_data.path(),
                "crew-1",
                "mission-1",
                "coder",
                &event_log,
                cwd,
            )
            .unwrap();
            let cwd_line = cwd
                .map(|cwd| format!("set \"MISSION_CWD={cwd}\"\r\n"))
                .unwrap_or_default();
            assert_eq!(
                fs::read_to_string(shim_dir.join("runner.cmd")).unwrap(),
                format!(
                    "@echo off\r\nsetlocal\r\nset \"RUNNER_CREW_ID=crew-1\"\r\nset \"RUNNER_MISSION_ID=mission-1\"\r\nset \"RUNNER_HANDLE=coder\"\r\nset \"RUNNER_EVENT_LOG={}\"\r\n{cwd_line}\"{}\" %*\r\n",
                    event_log.display(), app_data.path().join("bin").join(AGENT_DEST_BIN_NAME).display(),
                ),
            );
            let shell = fs::read_to_string(shim_dir.join("runner")).unwrap();
            assert_eq!(shell.contains("export MISSION_CWD="), cwd.is_some());
        }
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_rejects_quotes_in_every_value() {
        let bad = "bad\"value";
        for index in 0..6 {
            let mut values = ["runner.exe", "crew", "mission", "coder", "events", "cwd"];
            values[index] = bad;
            assert!(
                windows_cmd_shim(
                    Path::new(values[0]),
                    values[1],
                    values[2],
                    values[3],
                    values[4],
                    Some(values[5]),
                )
                .is_err(),
                "accepted {bad:?} at index {index}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_escapes_percent_in_every_value_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let runner = dir.path().join("agent%helper.cmd");
        fs::write(&runner, "@echo off\r\necho %RUNNER_CREW_ID%\r\necho %RUNNER_MISSION_ID%\r\necho %RUNNER_HANDLE%\r\necho %RUNNER_EVENT_LOG%\r\necho %MISSION_CWD%\r\n").unwrap();
        let value = r"C:\a%b\repo";
        let script = windows_cmd_shim(&runner, value, value, value, value, Some(value)).unwrap();
        for name in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
            "MISSION_CWD",
        ] {
            assert!(script.contains(&format!("set \"{name}=C:\\a%%b\\repo\"\r\n")));
        }
        assert!(script.contains("agent%%helper.cmd\" %*"));
        let shim = dir.path().join("runner.cmd");
        fs::write(&shim, script).unwrap();
        let output = std::process::Command::new(&shim).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{value}\r\n").repeat(5)
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_error_preserves_the_shell_shim() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        let error = install_session_runner_shim(
            app_data.path(),
            "crew",
            "mission",
            "coder",
            &event_log,
            Some("bad\"cwd"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("quote"));
        let shim_dir = app_data.path().join("missions/mission/shims/coder/bin");
        let script = fs::read_to_string(shim_dir.join("runner")).unwrap();
        assert!(script.contains("export MISSION_CWD='bad\"cwd'\n"));
        assert!(!shim_dir.join("runner.cmd").exists());
    }
}

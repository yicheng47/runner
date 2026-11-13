mod command;
mod env;
mod help;
mod msg;
mod output;
mod remote;
mod roster;
mod signal;

use clap::Parser;

#[derive(Parser)]
#[command(name = "runnerd")]
struct DaemonArgs {
    #[arg(long)]
    app_data_dir: Option<std::path::PathBuf>,
    #[arg(long)]
    log_dir: Option<std::path::PathBuf>,
    #[arg(long)]
    home_dir: Option<std::path::PathBuf>,
    #[arg(long)]
    endpoint: Option<std::path::PathBuf>,
    #[arg(long)]
    mcp_endpoint: Option<std::path::PathBuf>,
    #[arg(long, hide = true)]
    isolated: bool,
}

fn daemon(args: DaemonArgs) -> Result<(), Box<dyn std::error::Error>> {
    use runner_core::{app_paths, daemon_process::NativePaths};
    if args.isolated
        && (args.app_data_dir.is_none()
            || args.log_dir.is_none()
            || args.home_dir.is_none()
            || args.endpoint.is_none()
            || args.mcp_endpoint.is_none())
    {
        return Err("isolated runnerd requires every root and endpoint".into());
    }
    let paths = match (args.app_data_dir, args.log_dir, args.home_dir) {
        (Some(app_data_dir), Some(log_dir), home_dir) => NativePaths {
            app_data_dir,
            log_dir,
            home_dir,
        },
        (None, None, None) => NativePaths::resolve()?,
        _ => return Err("runnerd requires app-data-dir and log-dir together".into()),
    };
    if args.isolated {
        let home = paths.home_dir.as_ref().unwrap();
        // main dispatches runnerd before logging, boot workers or Tokio can start threads.
        unsafe {
            std::env::set_var("HOME", home);
            std::env::set_var("USERPROFILE", home);
            std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
            std::env::set_var("CODEX_HOME", home.join(".codex"));
        }
    }
    let config = runner_daemon::daemon::server::Config {
        endpoint: args
            .endpoint
            .map(app_paths::IpcEndpoint)
            .unwrap_or_else(|| {
                app_paths::daemon_endpoint(&paths.app_data_dir, cfg!(debug_assertions))
            }),
        mcp_endpoint: args
            .mcp_endpoint
            .map(app_paths::IpcEndpoint)
            .unwrap_or_else(|| {
                app_paths::mcp_endpoint(&paths.app_data_dir, cfg!(debug_assertions))
            }),
        paths,
        isolated: args.isolated,
    };
    // The same single-threaded startup continues through environment cleanup in run.
    unsafe { runner_daemon::daemon::server::run(config) }?;
    Ok(())
}

fn forward_session_shim(exe: &std::path::Path) -> Option<i32> {
    use runner_core::cli_install::{SessionShimConfig, SESSION_SHIM_CONFIG_NAME};

    let path = exe.parent()?.join(SESSION_SHIM_CONFIG_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("runner: failed to read {}: {error}", path.display());
            return Some(2);
        }
    };
    let config: SessionShimConfig = match serde_json::from_slice(&bytes) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("runner: invalid {}: {error}", path.display());
            return Some(2);
        }
    };
    let mut command = std::process::Command::new(&config.runner);
    command.args(std::env::args_os().skip(1));
    for (name, value) in &config.env {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    match command.status() {
        Ok(status) => Some(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "runner: failed to start {}: {error}",
                config.runner.display()
            );
            Some(2)
        }
    }
}

/// A login shell's startup files can put another build's `runner` ahead of the
/// session's own on PATH (#1149), so a `runner` that is not the session's CLI
/// runs that one instead, in either direction between builds. The target, like
/// the slot shim, starts without the variable, so an invocation hands off at
/// most once. Hook reports address their daemon by endpoint and stay put. A
/// caller that means another build's CLI, as the development skill does,
/// clears the variable for that command.
fn session_cli(
    args: &[std::ffi::OsString],
    exe: Option<&std::path::Path>,
    target: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    let invoked = std::path::Path::new(args.first()?);
    if !invoked
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("runner"))
        || args.get(1).is_some_and(|command| command == "hook")
    {
        return None;
    }
    let target = std::path::PathBuf::from(target?);
    if !target.is_absolute() {
        return None;
    }
    let canonical = target.canonicalize().ok()?;
    (exe.and_then(|exe| exe.canonicalize().ok()) != Some(canonical)).then_some(target)
}

/// Returns only when the target could not be started.
fn hand_off(target: &std::path::Path, args: &[std::ffi::OsString]) -> Option<i32> {
    let mut command = std::process::Command::new(target);
    command
        .args(args.iter().skip(1))
        .env_remove(runner_core::protocol::SESSION_CLI_ENV);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let _ = command.exec();
        None
    }
    #[cfg(windows)]
    {
        command
            .status()
            .ok()
            .map(|status| status.code().unwrap_or(1))
    }
}

fn main() {
    let exe = std::env::current_exe().ok();
    if exe
        .as_ref()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().eq_ignore_ascii_case("runnerd"))
        })
        .unwrap_or(false)
    {
        if let Err(error) = daemon(DaemonArgs::parse()) {
            eprintln!("runnerd: {error}");
            std::process::exit(1);
        }
        return;
    }
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(code) = session_cli(
        &args,
        exe.as_deref(),
        std::env::var_os(runner_core::protocol::SESSION_CLI_ENV),
    )
    .and_then(|target| hand_off(&target, &args))
    {
        std::process::exit(code);
    }
    if let Some(code) = exe.as_deref().and_then(forward_session_shim) {
        std::process::exit(code);
    }
    std::process::exit(command::run(command::Cli::parse()));
}

#[cfg(test)]
mod tests {
    use super::session_cli;
    use std::ffi::OsString;
    use std::path::Path;

    fn executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn invocation(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_runner_by_name_or_path_hands_off_to_another_session_cli() {
        let root = tempfile::tempdir().unwrap();
        let running = root.path().join("installed/runner");
        let session = root.path().join("dev/runner");
        executable(&running);
        executable(&session);
        let target = || Some(session.clone().into_os_string());
        let decide = |args: &[&str], target| session_cli(&invocation(args), Some(&running), target);

        for arg0 in [
            "runner",
            "runner.exe",
            "./runner",
            running.to_str().unwrap(),
        ] {
            assert_eq!(
                decide(&[arg0, "mission", "list"], target()),
                Some(session.clone()),
                "{arg0}"
            );
        }
        assert_eq!(decide(&["runner"], None), None);
        for arg0 in ["runner-dev", "runner-agent-cli", "/build/runner-agent-cli"] {
            assert_eq!(decide(&[arg0, "mission", "list"], target()), None, "{arg0}");
        }
        for subcommand in ["report", "title"] {
            assert_eq!(
                decide(&["runner", "hook", subcommand], target()),
                None,
                "hook commands keep their CLI"
            );
        }
        let missing = root.path().join("gone/runner").into_os_string();
        assert_eq!(decide(&["runner"], Some(missing)), None);
        assert_eq!(decide(&["runner"], Some("runner".into())), None);
        assert_eq!(decide(&["runner"], Some("dev/runner".into())), None);
        assert_eq!(
            session_cli(&invocation(&["runner"]), None, target()),
            Some(session.clone()),
            "an unknown running executable still runs the session's CLI",
        );
    }

    #[test]
    fn the_session_cli_itself_does_not_hand_off() {
        let root = tempfile::tempdir().unwrap();
        let session = root.path().join("dev/runner");
        executable(&session);
        let args = invocation(&["runner"]);
        let target = Some(session.clone().into_os_string());
        assert_eq!(session_cli(&args, Some(&session), target.clone()), None);
        #[cfg(unix)]
        {
            let link = root.path().join("local/bin/runner");
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&session, &link).unwrap();
            assert_eq!(
                session_cli(&args, Some(&link), target.clone()),
                None,
                "a command link to the session's CLI is that CLI",
            );
            assert_eq!(
                session_cli(&args, Some(&session), Some(link.into_os_string())),
                None,
                "a target reached through a link is the running CLI",
            );
        }
    }

    #[test]
    fn a_development_cli_obeys_an_installed_session_unless_the_caller_clears_it() {
        let root = tempfile::tempdir().unwrap();
        let dev_sidecar = root.path().join("com.wycstudios.runner-dev/bin/runner");
        let installed_shim = root
            .path()
            .join("com.wycstudios.runner/missions/m/shims/coder/bin/runner");
        executable(&dev_sidecar);
        executable(&installed_shim);
        let args = invocation(&["runner", "mission", "start"]);
        assert_eq!(
            session_cli(
                &args,
                Some(&dev_sidecar),
                Some(installed_shim.clone().into_os_string())
            ),
            Some(installed_shim),
        );
        assert_eq!(
            session_cli(&args, Some(&dev_sidecar), Some(OsString::new())),
            None
        );
    }
}

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
        std::env::set_var("HOME", home);
        std::env::set_var("USERPROFILE", home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("CODEX_HOME", home.join(".codex"));
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
    runner_daemon::daemon::server::run(config)?;
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
    if let Some(code) = exe.as_deref().and_then(forward_session_shim) {
        std::process::exit(code);
    }
    std::process::exit(command::run(command::Cli::parse()));
}

mod command;
mod env;
mod help;
mod msg;
mod output;
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

fn main() {
    if std::env::current_exe()
        .ok()
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
    std::process::exit(command::run(command::Cli::parse()));
}

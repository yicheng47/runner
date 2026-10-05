use crate::{
    cli_install, db, event_bus, events, mcp, ops, repo, runtime_status, session, shell_path,
    windows, AppCore,
};
use anyhow::{Context as _, Result};
pub use runner_core::daemon_process::NativePaths;
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub struct NativeMcpServer {
    core: AppCore,
    _runtime: tokio::runtime::Runtime,
}

impl NativeMcpServer {
    pub fn start(core: &AppCore) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("runner-ipc")
            .enable_all()
            .build()
            .context("create native MCP runtime")?;
        core.mcp
            .start(
                &crate::app_paths::mcp_endpoint(&core.app_data_dir, cfg!(debug_assertions)),
                core.clone(),
                runtime.handle(),
            )
            .context("start native MCP listener")?;
        Ok(Self {
            core: core.clone(),
            _runtime: runtime,
        })
    }
}

impl Drop for NativeMcpServer {
    fn drop(&mut self) {
        self.core.mcp.stop();
    }
}

pub fn boot_core(
    paths: &NativePaths,
    model_runtimes: Vec<crate::model::Runtime>,
) -> Result<AppCore> {
    boot(paths, model_runtimes, true)
}

pub fn boot(
    paths: &NativePaths,
    model_runtimes: Vec<crate::model::Runtime>,
    background: bool,
) -> Result<AppCore> {
    std::fs::create_dir_all(&paths.app_data_dir)
        .with_context(|| format!("create {}", paths.app_data_dir.display()))?;
    // Mission shims exec `$APPDATA/bin/runner`, so the CLI must be in place
    // before any session spawns. Best-effort: a copy failure is reported and
    // the app keeps running (#480).
    if background {
        if let Err(error) = cli_install::install_runner_cli(&paths.app_data_dir) {
            eprintln!("Runner bundled agent CLI install failed: {error}");
        }
        if let Err(error) = cli_install::remove_stale_mcp_cli(&paths.app_data_dir) {
            eprintln!("Runner stale MCP bridge cleanup failed: {error}");
        }
    }
    let pool = Arc::new(
        db::open_pool(&paths.app_data_dir.join("runner.db")).context("open Runner database")?,
    );
    let login_shell_lkg = match db::login_shell_env_lkg(&pool) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("runtime discovery LKG read failed: {error}");
            None
        }
    };
    let runtime_shell_env = Arc::new(RwLock::new(
        login_shell_lkg
            .as_ref()
            .map(|snapshot| snapshot.env.clone())
            .unwrap_or_default(),
    ));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(
        login_shell_lkg
            .as_ref()
            .map(|snapshot| snapshot.shell.clone())
            .filter(|shell| !shell.is_empty()),
        login_shell_lkg
            .as_ref()
            .map(|snapshot| snapshot.captured_at.clone()),
    )));
    let runtime: Arc<dyn session::runtime::SessionRuntime> =
        Arc::new(session::pty_runtime::PtyRuntime::new());
    let sessions = session::SessionManager::new(
        Arc::clone(&runtime_shell_env),
        Arc::clone(&runtime_discovery),
        runtime,
    );
    let window_registry = Arc::new(windows::WindowRegistry::new());
    let event_channel = events::EventChannel::new();

    let core = AppCore {
        db: Arc::clone(&pool),
        app_data_dir: paths.app_data_dir.clone(),
        sessions,
        runtime_shell_env: Arc::clone(&runtime_shell_env),
        runtime_discovery: Arc::clone(&runtime_discovery),
        usage: Arc::new(crate::usage::UsageService::default()),
        buses: event_bus::BusRegistry::new(),
        routers: crate::router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: window_registry,
        events: event_channel.clone(),
        session_event_observer: Default::default(),
        app_version: runner_core::version::display_version(),
    };

    if let Err(error) = core.sessions.start_runtime_watchers(
        &core.app_data_dir,
        Arc::clone(&core.db),
        Arc::new(core.session_events()),
    ) {
        eprintln!("Runner session watcher startup failed: {error}");
    }

    super::block_on(ops::mission::mount_all_running_mission_routers(&core));
    session::pty_runtime::cleanup_stale_running_rows_on_startup(&pool)
        .context("clean up stale PTY sessions")?;
    match pool.get() {
        Ok(conn) => match repo::node::clear_unread_on_startup(&conn, chrono::Utc::now()) {
            Ok(cleared) if cleared > 0 => {
                eprintln!(
                    "Runner startup cleanup: cleared {cleared} stale unread tab completion(s)"
                );
            }
            Ok(_) => {}
            Err(error) => eprintln!("Runner tab unread startup cleanup failed: {error}"),
        },
        Err(error) => eprintln!("Runner tab unread startup cleanup failed: {error}"),
    }
    session::pty_runtime::cleanup_orphan_processes_on_startup(&pool)
        .context("clean up orphan PTY processes")?;
    core.usage.set_enabled(model_runtimes.clone());
    if background {
        runtime_status::start_background_discovery(
            event_channel,
            Arc::clone(&pool),
            runtime_shell_env,
            runtime_discovery,
            false,
            model_runtimes,
        );
        #[cfg(not(test))]
        core.usage.start_scheduler(core.clone());
    }
    Ok(core)
}

pub fn stop_running_sessions_on_quit(core: &AppCore) -> Result<()> {
    let ids = {
        let mut conn = core.db.get().context("get database connection")?;
        repo::session::mark_running_for_resume_on_launch(&mut conn)
            .context("stamp sessions for resume on launch")?
    };
    let n = ids.len();
    let started = Instant::now();
    let result = core.sessions.kill_many(&ids);
    if n > 0 {
        let elapsed = started.elapsed();
        log::info!("quit teardown: stopped {n} sessions in {elapsed:?}");
    }
    result.map_err(|error| anyhow::anyhow!("failed to stop sessions on quit: {error}"))
}

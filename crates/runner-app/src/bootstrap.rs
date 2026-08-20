use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{bail, Context as _, Result};
use runner_backend::{
    db, event_bus, events, mcp, repo, runtime_status, session, shell_path, windows, AppCore,
};

pub const APP_IDENTIFIER: &str = "com.wycstudios.runner";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePaths {
    pub app_data_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl NativePaths {
    pub fn new(app_data_dir: PathBuf, log_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            log_dir,
        }
    }
}

pub fn native_paths() -> Result<NativePaths> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(paths_for_home(Path::new(&home), cfg!(debug_assertions)))
}

fn paths_for_home(home: &Path, debug: bool) -> NativePaths {
    let segment = if debug {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    };
    NativePaths {
        app_data_dir: home
            .join("Library")
            .join("Application Support")
            .join(&segment),
        log_dir: home.join("Library").join("Logs").join(segment),
    }
}

pub fn boot_core(paths: &NativePaths) -> Result<AppCore> {
    std::fs::create_dir_all(&paths.app_data_dir)
        .with_context(|| format!("create {}", paths.app_data_dir.display()))?;
    std::fs::create_dir_all(&paths.log_dir)
        .with_context(|| format!("create {}", paths.log_dir.display()))?;

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
            .map(|snapshot| snapshot.shell.clone()),
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
    window_registry.register("main");
    let event_channel = events::EventChannel::new();

    let core = AppCore {
        db: Arc::clone(&pool),
        app_data_dir: paths.app_data_dir.clone(),
        sessions,
        runtime_shell_env: Arc::clone(&runtime_shell_env),
        runtime_discovery: Arc::clone(&runtime_discovery),
        buses: event_bus::BusRegistry::new(),
        routers: runner_backend::router::RouterRegistry::new(),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: window_registry,
        events: event_channel.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };

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
    runtime_status::start_background_discovery(
        event_channel,
        Arc::clone(&pool),
        runtime_shell_env,
        runtime_discovery,
    );
    Ok(core)
}

pub fn stop_running_sessions_on_quit(core: &AppCore) -> Result<()> {
    let ids = {
        let mut conn = core.db.get().context("get database connection")?;
        repo::session::mark_running_for_resume_on_launch(&mut conn)
            .context("stamp sessions for resume on launch")?
    };
    let mut failures = Vec::new();
    for id in ids {
        if let Err(error) = core.sessions.kill(&id) {
            failures.push(format!("{id}: {error}"));
        }
    }
    if !failures.is_empty() {
        bail!("failed to stop sessions on quit: {}", failures.join("; "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_match_tauri_bundle_convention() {
        let release = paths_for_home(Path::new("/Users/tester"), false);
        assert_eq!(
            release.app_data_dir,
            Path::new("/Users/tester/Library/Application Support/com.wycstudios.runner")
        );
        assert_eq!(
            release.log_dir,
            Path::new("/Users/tester/Library/Logs/com.wycstudios.runner")
        );

        let debug = paths_for_home(Path::new("/Users/tester"), true);
        assert!(debug.app_data_dir.ends_with("com.wycstudios.runner-dev"));
        assert!(debug.log_dir.ends_with("com.wycstudios.runner-dev"));
    }

    #[test]
    fn quit_preparation_stamps_running_sessions_and_requeues_claims() {
        let temp = tempfile::tempdir().unwrap();
        let pool = Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap());
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES
                    ('r1', 'alpha', 'Alpha', 'shell', '/bin/cat',
                     '[]', '2026-08-18T00:00:00Z', '2026-08-18T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions
                    (id, runner_id, status, started_at, resume_on_launch)
                 VALUES
                    ('s1', 'r1', 'running', '2026-08-18T00:00:00Z', 0),
                    ('claimed', 'r1', 'stopped', '2026-08-18T00:00:01Z', 2)",
                [],
            )
            .unwrap();
        }
        let runtime: Arc<dyn session::runtime::SessionRuntime> =
            Arc::new(session::pty_runtime::PtyRuntime::new());
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::clone(&pool),
            app_data_dir: PathBuf::new(),
            sessions: session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                runtime,
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: event_bus::BusRegistry::new(),
            routers: runner_backend::router::RouterRegistry::new(),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            app_version: "0.0.0-test".into(),
        };

        stop_running_sessions_on_quit(&core).unwrap();

        let conn = pool.get().unwrap();
        for id in ["s1", "claimed"] {
            let stamp: i64 = conn
                .query_row(
                    "SELECT resume_on_launch FROM sessions WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stamp, 1, "{id} must be pending for the next launch");
        }
    }
}

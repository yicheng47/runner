use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use runner_app::{db, event_bus, events, mcp, session, shell_path, windows, AppCore};

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
    let login_shell_env = shell_path::resolve_login_shell_env().env;
    let runtime: Arc<dyn session::runtime::SessionRuntime> =
        Arc::new(session::pty_runtime::PtyRuntime::new());
    let sessions = session::SessionManager::new(login_shell_env, runtime);
    let window_registry = Arc::new(windows::WindowRegistry::new());
    window_registry.register("main");

    let core = AppCore {
        db: Arc::clone(&pool),
        app_data_dir: paths.app_data_dir.clone(),
        sessions,
        buses: event_bus::BusRegistry::new(),
        routers: runner_app::router::RouterRegistry::new(),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: window_registry,
        events: events::EventChannel::new(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    session::pty_runtime::cleanup_stale_running_rows_on_startup(&pool)
        .context("clean up stale PTY sessions")?;
    Ok(core)
}

pub fn stop_running_direct_sessions(core: &AppCore) -> Result<()> {
    let ids = {
        let conn = core.db.get().context("get database connection")?;
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions
             WHERE status = 'running' AND mission_id IS NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        ids
    };
    for id in ids {
        core.sessions
            .kill(&id)
            .with_context(|| format!("stop direct session {id}"))?;
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
}

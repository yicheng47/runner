use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use runner_backend::{
    db, event_bus, events, mcp, ops, repo, runtime_status, session, shell_path, windows, AppCore,
};

pub const APP_IDENTIFIER: &str = "com.wycstudios.runner";
pub const AUTO_RESUME_STAGGER_MS: u64 = 300;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AutoResumeReport {
    pub resumed: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePaths {
    pub app_data_dir: PathBuf,
    pub log_dir: PathBuf,
}

pub struct NativeMcpServer {
    core: AppCore,
    _runtime: tokio::runtime::Runtime,
}

impl NativeMcpServer {
    pub fn start(core: &AppCore) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("runner-mcp")
            .enable_all()
            .build()
            .context("create native MCP runtime")?;
        core.mcp
            .start(
                &core.app_data_dir.join("mcp.sock"),
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
    let event_channel = events::EventChannel::new();

    let core = AppCore {
        db: Arc::clone(&pool),
        app_data_dir: paths.app_data_dir.clone(),
        sessions,
        runtime_shell_env: Arc::clone(&runtime_shell_env),
        runtime_discovery: Arc::clone(&runtime_discovery),
        buses: event_bus::BusRegistry::new(),
        routers: runner_backend::router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: window_registry,
        events: event_channel.clone(),
        session_event_observer: Default::default(),
        app_version: crate::version::display_version(),
    };

    if let Err(error) = core.sessions.start_claude_session_key_watcher(
        &core.app_data_dir,
        Arc::clone(&core.db),
        Arc::new(core.session_events()),
    ) {
        eprintln!("Runner Claude session-key watcher startup failed: {error}");
    }

    futures::executor::block_on(ops::mission::mount_all_running_mission_routers(&core));
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

pub fn consume_resume_on_launch(
    core: &AppCore,
    enabled: bool,
    dims_for: impl Fn(&str) -> Option<(u16, u16)>,
) -> Result<AutoResumeReport> {
    consume_launch_claims(
        enabled,
        || {
            let conn = core.db.get().context("get launch-resume connection")?;
            repo::session::clear_chat_resume_on_launch(&conn)
                .context("clear chat launch-resume claims")?;
            Ok(())
        },
        || {
            let mut conn = core.db.get().context("get launch-resume connection")?;
            repo::session::take_resume_on_launch(&mut conn).context("take launch-resume claim")
        },
        |session_id| {
            let dims = dims_for(session_id);
            ops::session::session_resume_on_launch(
                core,
                session_id,
                dims.map(|size| size.0),
                dims.map(|size| size.1),
            )
            .map(drop)
            .map_err(|error| error.to_string())
        },
        || std::thread::sleep(Duration::from_millis(AUTO_RESUME_STAGGER_MS)),
    )
}

fn consume_launch_claims(
    enabled: bool,
    mut clear: impl FnMut() -> Result<()>,
    mut take: impl FnMut() -> Result<Option<repo::session::ResumeOnLaunchClaim>>,
    mut resume: impl FnMut(&str) -> std::result::Result<(), String>,
    mut wait: impl FnMut(),
) -> Result<AutoResumeReport> {
    if !enabled {
        clear()?;
    }

    let mut report = AutoResumeReport::default();
    let mut attempted_chat = false;
    while let Some(claim) = take()? {
        if attempted_chat && !claim.shell {
            wait();
        }
        attempted_chat |= !claim.shell;
        match resume(&claim.session_id) {
            Ok(()) => report.resumed.push(claim.session_id),
            Err(error) => report.errors.push(format!("{}: {error}", claim.session_id)),
        }
    }
    Ok(report)
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
    use std::cell::{Cell, RefCell};

    fn launch_claim(session_id: &str, shell: bool) -> repo::session::ResumeOnLaunchClaim {
        repo::session::ResumeOnLaunchClaim {
            session_id: session_id.to_owned(),
            shell,
        }
    }

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
    fn native_mcp_server_binds_and_removes_the_app_data_socket() {
        let temp = tempfile::tempdir().unwrap();
        let paths = NativePaths::new(temp.path().join("data"), temp.path().join("logs"));
        let core = boot_core(&paths).unwrap();
        let socket_path = paths.app_data_dir.join("mcp.sock");

        let server = NativeMcpServer::start(&core).unwrap();
        assert_eq!(
            core.mcp.socket_path().as_deref(),
            Some(socket_path.as_path())
        );
        assert!(socket_path.exists());
        server._runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
        });

        drop(server);
        assert!(!socket_path.exists());
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
            mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
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

    #[test]
    fn launch_claim_consumer_clears_gated_chats_but_drains_shells_when_disabled() {
        let cleared = Cell::new(false);
        let claims = RefCell::new(vec![None, Some(launch_claim("shell-session", true))]);
        let resumed = RefCell::new(Vec::new());
        let report = consume_launch_claims(
            false,
            || {
                cleared.set(true);
                Ok(())
            },
            || Ok(claims.borrow_mut().pop().unwrap()),
            |session_id| {
                resumed.borrow_mut().push(session_id.to_owned());
                Ok(())
            },
            || {},
        )
        .unwrap();

        assert_eq!(report.resumed, ["shell-session"]);
        assert!(report.errors.is_empty());
        assert!(cleared.get());
        assert_eq!(&*resumed.borrow(), &["shell-session"]);
    }

    #[test]
    fn launch_claim_consumer_skips_shell_stagger_and_continues_after_failure() {
        let claims = RefCell::new(vec![
            None,
            Some(launch_claim("session-b", false)),
            Some(launch_claim("shell-session", true)),
            Some(launch_claim("session-a", false)),
        ]);
        let attempts = RefCell::new(Vec::new());
        let trace = RefCell::new(Vec::new());
        let report = consume_launch_claims(
            true,
            || panic!("enabled launch must not clear claims"),
            || Ok(claims.borrow_mut().pop().unwrap()),
            |session_id| {
                attempts.borrow_mut().push(session_id.to_owned());
                trace.borrow_mut().push(format!("resume:{session_id}"));
                if session_id == "session-a" {
                    Err("rejected key".into())
                } else {
                    Ok(())
                }
            },
            || trace.borrow_mut().push("wait".into()),
        )
        .unwrap();

        assert_eq!(
            &*attempts.borrow(),
            &["session-a", "shell-session", "session-b"]
        );
        assert_eq!(
            &*trace.borrow(),
            &[
                "resume:session-a",
                "resume:shell-session",
                "wait",
                "resume:session-b"
            ]
        );
        assert_eq!(report.resumed, ["shell-session", "session-b"]);
        assert_eq!(report.errors, ["session-a: rejected key"]);
    }
}

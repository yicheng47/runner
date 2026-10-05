use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows, AppCore};
use runner_core::protocol::DaemonClient;

pub(crate) fn core(
    db: Arc<db::DbPool>,
    app_data_dir: PathBuf,
    sessions: Arc<session::SessionManager>,
    runtime_shell_env: Arc<RwLock<shell_path::LoginShellEnv>>,
    runtime_discovery: Arc<RwLock<shell_path::DiscoveryState>>,
) -> AppCore {
    AppCore {
        db,
        app_data_dir,
        sessions,
        runtime_shell_env,
        runtime_discovery,
        usage: Arc::new(runner_backend::usage::UsageService::default()),
        buses: event_bus::BusRegistry::new(),
        routers: router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        mcp: Arc::new(mcp::McpHandle::new()),
        windows: Arc::new(windows::WindowRegistry::new()),
        events: events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    }
}

#[allow(dead_code)]
pub(crate) fn client(core: &AppCore) -> DaemonClient {
    runner_backend::daemon::InProcessTransport::client(core.clone())
}

#[allow(dead_code)]
pub(crate) fn isolated_client(root: &std::path::Path) -> DaemonClient {
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    client(&core(
        Arc::new(db::open_pool(&root.join("client.db")).unwrap()),
        root.to_owned(),
        session::SessionManager::new(
            env.clone(),
            discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        env,
        discovery,
    ))
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use runner_core::protocol::DaemonClient;
use runner_daemon::{db, event_bus, events, router, session, shell_path, windows, AppCore};

pub(crate) fn core(
    db: Arc<db::DbPool>,
    app_data_dir: PathBuf,
    sessions: Arc<session::SessionManager>,
    runtime_shell_env: Arc<RwLock<shell_path::LoginShellEnv>>,
    runtime_discovery: Arc<RwLock<shell_path::DiscoveryState>>,
) -> AppCore {
    let events = events::EventChannel::new();
    AppCore {
        files: Arc::new(runner_daemon::files::FilesService::new(
            runtime_shell_env.clone(),
            events.clone(),
        )),
        remote: Default::default(),
        db,
        app_data_dir,
        sessions,
        runtime_shell_env,
        runtime_discovery,
        usage: Arc::new(runner_daemon::usage::UsageService::without_fetch()),
        claude_accounts: Arc::new(
            runner_daemon::claude_accounts::AccountService::without_authentication(),
        ),
        buses: event_bus::BusRegistry::new(),
        routers: router::RouterRegistry::new(),
        mission_roster_locks: Default::default(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        windows: Arc::new(windows::WindowRegistry::new()),
        events,
        session_event_observer: Default::default(),
        reports: Default::default(),
        app_version: "0.0.0-test".into(),
    }
}

/// GPUI drops globals after entities, so by then the store's tasks are
/// cancelled and queued. `#[gpui::test]` drains that queue; a plain `#[test]`
/// app never does, and each queued task would hold its core's SQLite pool open
/// for the rest of the run.
struct DrainTasksOnDrop(gpui::TestDispatcher);

impl gpui::Global for DrainTasksOnDrop {}

impl Drop for DrainTasksOnDrop {
    fn drop(&mut self) {
        self.0.drain_tasks();
    }
}

pub(crate) fn drain_tasks_when_app_drops(cx: &mut gpui::App) {
    if cx.has_global::<DrainTasksOnDrop>() {
        return;
    }
    if let Some(dispatcher) = cx.background_executor().dispatcher().as_test() {
        let dispatcher = dispatcher.clone();
        cx.set_global(DrainTasksOnDrop(dispatcher));
    }
}

#[allow(dead_code)]
pub(crate) fn client(core: &AppCore) -> DaemonClient {
    runner_daemon::daemon::InProcessTransport::client(core.clone())
}

#[allow(dead_code)]
pub(crate) fn isolated_client(root: &std::path::Path) -> DaemonClient {
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    client(&core(
        Arc::new(db::open_fixture_pool(&root.join("client.db")).unwrap()),
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

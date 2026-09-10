use std::path::PathBuf;
use std::sync::Arc;

use crate::db;
use crate::event_bus::BusRegistry;
use crate::events::EventChannel;
use crate::mcp::McpHandle;
use crate::router::RouterRegistry;
use crate::session::runtime::{
    OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
    SpawnSpec,
};
use crate::session::SessionManager;
use crate::shell_path::LoginShellEnv;
use crate::windows::WindowRegistry;
use crate::AppCore;

struct InertRuntime;

impl SessionRuntime for InertRuntime {
    fn spawn(&self, _spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }

    fn stop(&self, _session: &RuntimeSession) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }

    fn send_bytes(&self, _session: &RuntimeSession, _bytes: &[u8]) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }

    fn send_key(&self, _session: &RuntimeSession, _key: &str) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }

    fn resize(&self, _session: &RuntimeSession, _cols: u16, _rows: u16) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }

    fn status(&self, _session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Err(RuntimeError::Msg("unused test runtime".into()))
    }
}

pub(crate) fn test_core_in(app_data_dir: PathBuf) -> AppCore {
    let runtime_shell_env = Arc::new(std::sync::RwLock::new(LoginShellEnv::default()));
    let runtime_discovery = Arc::new(std::sync::RwLock::new(
        crate::shell_path::DiscoveryState::startup(None, None),
    ));
    AppCore {
        db: Arc::new(db::open_in_memory().unwrap()),
        app_data_dir,
        sessions: SessionManager::new(
            Arc::clone(&runtime_shell_env),
            Arc::clone(&runtime_discovery),
            Arc::new(InertRuntime),
        ),
        runtime_shell_env,
        runtime_discovery,
        buses: BusRegistry::new(),
        routers: RouterRegistry::new(),
        mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
        mcp: Arc::new(McpHandle::new()),
        windows: Arc::new(WindowRegistry::new()),
        events: EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    }
}

pub(crate) fn test_core() -> AppCore {
    test_core_in(PathBuf::new())
}

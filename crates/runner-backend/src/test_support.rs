use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::Connection;

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
        usage: Arc::new(crate::usage::UsageService::default()),
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

pub(crate) fn insert_test_role(
    conn: &Connection,
    id: &str,
    handle: &str,
    runtime: &str,
    command: &str,
) {
    let now = Utc::now();
    crate::repo::role::insert(
        conn,
        &crate::repo::role::RoleRow {
            id: id.into(),
            handle: handle.into(),
            display_name: handle.into(),
            runtime: runtime.into(),
            command: command.into(),
            args_json: Some(Vec::new()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(Default::default()),
            model: None,
            effort: None,
            created_at: now,
            updated_at: now,
        },
    )
    .unwrap();
}

pub(crate) fn insert_test_slot(
    conn: &Connection,
    id: &str,
    crew_id: &str,
    role_id: &str,
    slot_handle: &str,
    position: i64,
    lead: bool,
) {
    crate::repo::slot::insert(
        conn,
        &crate::repo::slot::SlotRow {
            id: id.into(),
            crew_id: crew_id.into(),
            role_id: role_id.into(),
            slot_handle: slot_handle.into(),
            position,
            lead,
            runtime_override: None,
            model_override: None,
            effort_override: None,
            added_at: Utc::now(),
        },
    )
    .unwrap();
}

/// Turn foreign-key enforcement off on `conn`, so a delete test proves the
/// code does its dependent work itself instead of leaning on the schema.
pub(crate) fn foreign_keys_off(conn: &Connection) {
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    assert_eq!(
        row_count(conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        0
    );
}

pub(crate) fn row_count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

pub(crate) fn test_session_row(
    id: &str,
    status: crate::model::SessionStatus,
) -> crate::repo::session::SessionRowDb {
    let mut row = crate::repo::session::SessionRowDb::new_running(id.into());
    row.status = status;
    row.started_at = Some(Utc::now());
    row
}

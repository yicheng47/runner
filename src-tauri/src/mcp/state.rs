use std::path::PathBuf;
use std::sync::Arc;

use tauri::AppHandle;

use crate::{
    db::DbPool, event_bus::BusRegistry, mcp::McpHandle, router::RouterRegistry,
    session::SessionManager, AppState,
};

#[derive(Clone)]
pub(crate) struct McpState {
    pub db: Arc<DbPool>,
    pub app_data_dir: PathBuf,
    pub sessions: Arc<SessionManager>,
    pub buses: Arc<BusRegistry>,
    pub routers: Arc<RouterRegistry>,
    pub mcp: Arc<McpHandle>,
    pub app_handle: AppHandle,
}

impl McpState {
    pub(crate) fn app_state(&self) -> AppState {
        AppState {
            db: Arc::clone(&self.db),
            app_data_dir: self.app_data_dir.clone(),
            sessions: Arc::clone(&self.sessions),
            buses: Arc::clone(&self.buses),
            routers: Arc::clone(&self.routers),
            mcp: Arc::clone(&self.mcp),
        }
    }
}

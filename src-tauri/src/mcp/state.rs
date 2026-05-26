use std::sync::Arc;

use tauri::AppHandle;

use crate::db::DbPool;

#[derive(Clone)]
#[allow(dead_code)] // fields read by Phase 2 tool methods
pub(crate) struct McpState {
    pub db: Arc<DbPool>,
    pub app_handle: AppHandle,
}

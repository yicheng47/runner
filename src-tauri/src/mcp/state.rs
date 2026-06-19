use std::sync::Arc;

use tauri::AppHandle;

use crate::{db::DbPool, session::SessionManager};

#[derive(Clone)]
pub(crate) struct McpState {
    pub db: Arc<DbPool>,
    pub sessions: Arc<SessionManager>,
    pub app_handle: AppHandle,
}

use chrono::Utc;
use serde::Deserialize;
use tauri::{Emitter, Manager, State};

use crate::error::{Error, Result};
use crate::repo;
use crate::repo::tab::TabRow;
use crate::session::manager::SessionActivityState;
use crate::windows::Subject;
use crate::AppState;

const ATTENTION_CHANGED_EVENT: &str = "chat/tab-attention-changed";

#[derive(Debug, Deserialize)]
pub struct TabUpsertInput {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub position: i64,
    pub layout: String,
}

#[derive(Debug, Deserialize)]
pub struct TabImportInput {
    pub name: String,
    pub position: i64,
    pub layout: String,
}

fn validate_layout(layout: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(layout)
        .map(|_| ())
        .map_err(|e| Error::msg(format!("invalid tab layout: {e}")))
}

#[tauri::command]
pub fn tab_list(state: State<'_, AppState>) -> Result<Vec<TabRow>> {
    let mut conn = state.db.get()?;
    Ok(repo::tab::list_with_active_sessions(&mut conn)?)
}

#[tauri::command]
pub fn tab_upsert(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    input: TabUpsertInput,
) -> Result<TabRow> {
    validate_layout(&input.layout)?;
    ulid::Ulid::from_string(&input.id)
        .map_err(|_| Error::msg(format!("invalid tab id: {}", input.id)))?;
    let mut conn = state.db.get()?;
    let existing = repo::tab::get(&conn, &input.id)?;
    let row = TabRow {
        id: input.id,
        folder_id: input.folder_id,
        name: input.name.trim().to_owned(),
        position: input.position,
        layout: input.layout,
        created_at: existing
            .as_ref()
            .map(|row| row.created_at.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
        last_completed_at: existing
            .as_ref()
            .and_then(|row| row.last_completed_at.clone()),
        last_viewed_at: existing.and_then(|row| row.last_viewed_at),
    };
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::tab::upsert_move_not_copy(&tx, &row)?;
    tx.commit()?;
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(row)
}

#[tauri::command]
pub fn tab_delete(state: State<'_, AppState>, app: tauri::AppHandle, id: String) -> Result<()> {
    let conn = state.db.get()?;
    repo::tab::delete(&conn, &id)?;
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn tab_move_to_folder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    folder_id: Option<String>,
) -> Result<TabRow> {
    let conn = state.db.get()?;
    if let Some(folder_id) = folder_id.as_deref() {
        if repo::folder::get(&conn, folder_id)?.is_none() {
            return Err(Error::msg(format!("folder not found: {folder_id}")));
        }
    }
    if repo::tab::move_to_folder(&conn, &id, folder_id.as_deref())? == 0 {
        return Err(Error::msg(format!("tab not found: {id}")));
    }
    let row = repo::tab::get(&conn, &id)?.ok_or_else(|| Error::msg("tab disappeared"))?;
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(row)
}

#[tauri::command]
pub fn tab_reorder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    folder_id: Option<String>,
    ordered_ids: Vec<String>,
) -> Result<Vec<TabRow>> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(folder_id) = folder_id.as_deref() {
        if repo::folder::get(&tx, folder_id)?.is_none() {
            return Err(Error::msg(format!("folder not found: {folder_id}")));
        }
    }
    repo::tab::move_and_reorder(&tx, &id, folder_id.as_deref(), &ordered_ids)
        .map_err(|error| Error::msg(format!("reorder tab: {error}")))?;
    let rows = repo::tab::list(&tx)?;
    tx.commit()?;
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(rows)
}

#[tauri::command]
pub fn tab_mark_viewed(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    member_ids: Vec<String>,
) -> Result<TabRow> {
    mark_tab_viewed_for_window(&state, &app, window.label(), &id, member_ids)
}

fn mark_tab_viewed_for_window<R: tauri::Runtime>(
    state: &AppState,
    app: &tauri::AppHandle<R>,
    window_label: &str,
    id: &str,
    member_ids: Vec<String>,
) -> Result<TabRow> {
    state.windows.mark_focused(window_label);
    state.windows.set_subjects(
        window_label,
        member_ids.into_iter().map(Subject::DirectChat).collect(),
    );
    let conn = state.db.get()?;
    let row = repo::tab::mark_viewed(&conn, id, Utc::now())?
        .ok_or_else(|| Error::msg(format!("tab not found: {id}")))?;
    let _ = app.emit(ATTENTION_CHANGED_EVENT, serde_json::json!({ "tab_id": id }));
    crate::broadcast_focus_map(app);
    Ok(row)
}

pub(crate) fn record_session_completion<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: &str,
) -> Result<()> {
    let Some(state) = app.try_state::<AppState>() else {
        return Ok(());
    };
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::tab::ensure_active_sessions(&tx)?;
    let Some(tab) = repo::tab::find_for_session(&tx, session_id)? else {
        tx.commit()?;
        return Ok(());
    };
    let member_ids = repo::tab::session_ids(&tab);
    let activity = state.sessions.activity_snapshot();
    if member_ids
        .iter()
        .any(|id| activity.get(id) == Some(&SessionActivityState::Busy))
    {
        tx.commit()?;
        return Ok(());
    }
    let viewed = state.windows.any_focused_displaying(&member_ids);
    let row = repo::tab::record_completion(&tx, &tab.id, viewed, Utc::now())?;
    tx.commit()?;
    if row.is_some() {
        let _ = app.emit(
            ATTENTION_CHANGED_EVENT,
            serde_json::json!({ "tab_id": tab.id }),
        );
    }
    Ok(())
}

pub(crate) fn mark_direct_sessions_viewed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    session_ids: &[String],
) -> Result<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::tab::ensure_active_sessions(&tx)?;
    let mut tab_ids = Vec::new();
    for session_id in session_ids {
        if let Some(tab) = repo::tab::find_for_session(&tx, session_id)? {
            if !tab_ids.contains(&tab.id) {
                tab_ids.push(tab.id);
            }
        }
    }
    let now = Utc::now();
    for tab_id in &tab_ids {
        repo::tab::mark_viewed(&tx, tab_id, now)?;
    }
    tx.commit()?;
    if !tab_ids.is_empty() {
        let _ = app.emit(
            ATTENTION_CHANGED_EVENT,
            serde_json::json!({ "tab_ids": tab_ids }),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn tab_import_once(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    tabs: Vec<TabImportInput>,
) -> Result<Vec<TabRow>> {
    let mut conn = state.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing: i64 = tx.query_row("SELECT COUNT(*) FROM tabs", [], |row| row.get(0))?;
    if existing == 0 {
        for tab in tabs {
            validate_layout(&tab.layout)?;
            let row = repo::tab::create(&tx, None, tab.name.trim(), tab.position, &tab.layout)?;
            repo::tab::upsert_move_not_copy(&tx, &row)?;
        }
    }
    repo::tab::ensure_active_sessions(&tx)?;
    tx.commit()?;
    let rows = repo::tab::list(&conn)?;
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use chrono::{DateTime, FixedOffset, Utc};
    use tauri::{Listener, Manager};

    use super::*;
    use crate::db;
    use crate::event_bus::BusRegistry;
    use crate::mcp::McpHandle;
    use crate::router::RouterRegistry;
    use crate::session::manager::TauriSessionEvents;
    use crate::session::runtime::{
        OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
        SpawnSpec,
    };
    use crate::session::SessionManager;
    use crate::shell_path::LoginShellEnv;
    use crate::windows::WindowRegistry;

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

    fn test_app() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(AppState {
            db: Arc::new(db::open_in_memory().unwrap()),
            app_data_dir: PathBuf::new(),
            sessions: SessionManager::new(LoginShellEnv::default(), Arc::new(InertRuntime)),
            buses: BusRegistry::new(),
            routers: RouterRegistry::new(),
            mcp: Arc::new(McpHandle::new()),
            windows: Arc::new(WindowRegistry::new()),
        });
        app
    }

    fn create_tab(state: &AppState, session_ids: &[&str]) -> TabRow {
        let layout = serde_json::json!({
            "preset": if session_ids.len() == 1 { "single" } else { "cols-2" },
            "slots": session_ids,
            "sizes": {},
        });
        let conn = state.db.get().unwrap();
        repo::tab::create(&conn, None, "chat", 0, &layout.to_string()).unwrap()
    }

    fn attention_events<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Arc<AtomicUsize> {
        let count = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&count);
        app.listen(ATTENTION_CHANGED_EVENT, move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        count
    }

    fn parsed(value: Option<&str>) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value.expect("timestamp")).unwrap()
    }

    #[test]
    fn final_idle_in_focused_window_completes_and_views_tab() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        state.windows.mark_focused("main");
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_completed_at, row.last_viewed_at);
        assert!(row.last_completed_at.is_some());
    }

    #[test]
    fn final_idle_in_background_marks_tab_unread() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_some());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn idle_member_does_not_complete_tab_while_peer_is_busy() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a", "b"]);
        let events = TauriSessionEvents(app.handle().clone());

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("b", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn activation_and_focus_return_advance_viewed_and_emit_invalidation() {
        let app = test_app();
        let state = app.state::<AppState>();
        let tab = create_tab(&state, &["a"]);
        let observed = attention_events(app.handle());
        let first_completion = Utc::now();
        repo::tab::record_completion(&state.db.get().unwrap(), &tab.id, false, first_completion)
            .unwrap();

        let activated = mark_tab_viewed_for_window(
            &state,
            app.handle(),
            "main",
            &tab.id,
            vec!["a".to_string()],
        )
        .unwrap();
        assert!(
            parsed(activated.last_viewed_at.as_deref())
                >= parsed(activated.last_completed_at.as_deref())
        );
        assert_eq!(
            state.windows.focused_direct_sessions("main"),
            ["a".to_string()]
        );
        assert_eq!(observed.load(Ordering::SeqCst), 1);

        state.windows.mark_blurred("main");
        let second_completion = first_completion + chrono::Duration::seconds(1);
        repo::tab::record_completion(&state.db.get().unwrap(), &tab.id, false, second_completion)
            .unwrap();
        state.windows.mark_focused("main");
        let visible = state.windows.focused_direct_sessions("main");
        mark_direct_sessions_viewed(app.handle(), &state, &visible).unwrap();

        let focused = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(
            parsed(focused.last_viewed_at.as_deref())
                >= parsed(focused.last_completed_at.as_deref())
        );
        assert_eq!(observed.load(Ordering::SeqCst), 2);
    }
}

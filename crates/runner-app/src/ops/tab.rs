use chrono::Utc;
use serde::Deserialize;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::events::EventChannel;
use crate::repo;
use crate::repo::tab::TabRow;
use crate::session::manager::SessionActivityState;
use crate::session::SessionManager;
use crate::windows::{Subject, WindowRegistry};
use crate::AppCore;

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

pub fn tab_list(state: &AppCore) -> Result<Vec<TabRow>> {
    let mut conn = state.db.get()?;
    Ok(repo::tab::list_with_active_sessions(&mut conn)?)
}

pub fn tab_upsert(state: &AppCore, input: TabUpsertInput) -> Result<TabRow> {
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
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(row)
}

pub fn tab_delete(state: &AppCore, id: &str) -> Result<()> {
    let conn = state.db.get()?;
    repo::tab::delete(&conn, id)?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(())
}

pub fn tab_move_to_folder(state: &AppCore, id: &str, folder_id: Option<String>) -> Result<TabRow> {
    let conn = state.db.get()?;
    if let Some(folder_id) = folder_id.as_deref() {
        if repo::folder::get(&conn, folder_id)?.is_none() {
            return Err(Error::msg(format!("folder not found: {folder_id}")));
        }
    }
    if repo::tab::move_to_folder(&conn, id, folder_id.as_deref())? == 0 {
        return Err(Error::msg(format!("tab not found: {id}")));
    }
    let row = repo::tab::get(&conn, id)?.ok_or_else(|| Error::msg("tab disappeared"))?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(row)
}

pub fn tab_reorder(
    state: &AppCore,
    id: &str,
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
    repo::tab::move_and_reorder(&tx, id, folder_id.as_deref(), &ordered_ids)
        .map_err(|error| Error::msg(format!("reorder tab: {error}")))?;
    let rows = repo::tab::list(&tx)?;
    tx.commit()?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(rows)
}

/// Body of the `tab_mark_viewed` command. The window label comes from the
/// invoking webview (resolved by the Tauri wrapper), not trusted from the
/// caller.
pub fn mark_tab_viewed(
    state: &AppCore,
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
    state.events.emit(
        ATTENTION_CHANGED_EVENT,
        &serde_json::json!({ "tab_id": id }),
    );
    state.broadcast_focus_map();
    Ok(row)
}

/// Takes the state pieces individually (not `&AppCore`) because the main
/// caller is `CoreSessionEvents::status`, which holds the session manager
/// only weakly to avoid an Arc cycle.
pub(crate) fn record_session_completion(
    db: &DbPool,
    sessions: &SessionManager,
    windows: &WindowRegistry,
    events: &EventChannel,
    session_id: &str,
) -> Result<()> {
    let mut conn = db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    repo::tab::ensure_active_sessions(&tx)?;
    let Some(tab) = repo::tab::find_for_session(&tx, session_id)? else {
        tx.commit()?;
        return Ok(());
    };
    let member_ids = repo::tab::session_ids(&tab);
    let activity = sessions.activity_snapshot();
    if member_ids
        .iter()
        .any(|id| activity.get(id) == Some(&SessionActivityState::Busy))
    {
        tx.commit()?;
        return Ok(());
    }
    if !sessions.take_completion_armed(&member_ids) {
        tx.commit()?;
        return Ok(());
    }
    let viewed = windows.any_focused_displaying(&member_ids);
    let row = repo::tab::record_completion(&tx, &tab.id, viewed, Utc::now())?;
    tx.commit()?;
    if row.is_some() {
        events.emit(
            ATTENTION_CHANGED_EVENT,
            &serde_json::json!({ "tab_id": tab.id }),
        );
    }
    Ok(())
}

pub fn mark_direct_sessions_viewed(state: &AppCore, session_ids: &[String]) -> Result<()> {
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
        state.events.emit(
            ATTENTION_CHANGED_EVENT,
            &serde_json::json!({ "tab_ids": tab_ids }),
        );
    }
    Ok(())
}

pub fn tab_import_once(state: &AppCore, tabs: Vec<TabImportInput>) -> Result<Vec<TabRow>> {
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
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use chrono::{DateTime, FixedOffset, Utc};
    use tokio::sync::broadcast;

    use super::*;
    use crate::db;
    use crate::event_bus::BusRegistry;
    use crate::events::AppEvent;
    use crate::mcp::McpHandle;
    use crate::router::RouterRegistry;
    use crate::session::runtime::{
        OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
        SpawnSpec,
    };
    use crate::shell_path::LoginShellEnv;

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

    fn test_core() -> AppCore {
        AppCore {
            db: Arc::new(db::open_in_memory().unwrap()),
            app_data_dir: PathBuf::new(),
            sessions: SessionManager::new(LoginShellEnv::default(), Arc::new(InertRuntime)),
            buses: BusRegistry::new(),
            routers: RouterRegistry::new(),
            mcp: Arc::new(McpHandle::new()),
            windows: Arc::new(WindowRegistry::new()),
            events: EventChannel::new(),
            app_version: "0.0.0-test".into(),
        }
    }

    fn create_tab(state: &AppCore, session_ids: &[&str]) -> TabRow {
        let layout = serde_json::json!({
            "preset": if session_ids.len() == 1 { "single" } else { "cols-2" },
            "slots": session_ids,
            "sizes": {},
        });
        let conn = state.db.get().unwrap();
        repo::tab::create(&conn, None, "chat", 0, &layout.to_string()).unwrap()
    }

    /// Count `chat/tab-attention-changed` events delivered since the
    /// receiver subscribed. Every emit in these tests happens on the
    /// calling thread, so draining afterwards observes them all.
    fn drain_attention_count(rx: &mut broadcast::Receiver<AppEvent>) -> usize {
        let mut count = 0;
        while let Ok(ev) = rx.try_recv() {
            if ev.name == ATTENTION_CHANGED_EVENT {
                count += 1;
            }
        }
        count
    }

    fn parsed(value: Option<&str>) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value.expect("timestamp")).unwrap()
    }

    #[test]
    fn armed_final_idle_in_focused_window_completes_and_views_tab() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        state.windows.mark_focused("main");
        let events = state.session_events();

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
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
    fn armed_final_idle_in_background_marks_tab_unread() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        state.windows.register("main");
        state
            .windows
            .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
        let events = state.session_events();

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
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
    fn spontaneous_settle_does_not_complete_tab_or_emit_invalidation() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let events = state.session_events();

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());
        assert_eq!(drain_attention_count(&mut rx), 0);
    }

    #[test]
    fn armed_member_waits_for_busy_peer_before_completing_tab() {
        let state = test_core();
        let tab = create_tab(&state, &["a", "b"]);
        let events = state.session_events();

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("b", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_none());
        assert!(row.last_viewed_at.is_none());

        state
            .sessions
            .publish_direct_activity("b", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(row.last_completed_at.is_some());
        assert!(row.last_viewed_at.is_none());
    }

    #[test]
    fn completion_arm_is_consumed_after_recording() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let events = state.session_events();

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state.sessions.arm_completion("a");
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);
        let first = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap()
            .last_completed_at
            .expect("armed settle should record completion");

        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Busy, "test", &events);
        state
            .sessions
            .publish_direct_activity("a", SessionActivityState::Idle, "test", &events);

        let row = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_completed_at.as_deref(), Some(first.as_str()));
        assert_eq!(drain_attention_count(&mut rx), 1);
    }

    #[test]
    fn activation_and_focus_return_advance_viewed_and_emit_invalidation() {
        let state = test_core();
        let tab = create_tab(&state, &["a"]);
        let mut rx = state.events.subscribe();
        let first_completion = Utc::now();
        repo::tab::record_completion(&state.db.get().unwrap(), &tab.id, false, first_completion)
            .unwrap();

        let activated = mark_tab_viewed(&state, "main", &tab.id, vec!["a".to_string()]).unwrap();
        assert!(
            parsed(activated.last_viewed_at.as_deref())
                >= parsed(activated.last_completed_at.as_deref())
        );
        assert_eq!(
            state.windows.focused_direct_sessions("main"),
            ["a".to_string()]
        );
        assert_eq!(drain_attention_count(&mut rx), 1);

        state.windows.mark_blurred("main");
        let second_completion = first_completion + chrono::Duration::seconds(1);
        repo::tab::record_completion(&state.db.get().unwrap(), &tab.id, false, second_completion)
            .unwrap();
        state.windows.mark_focused("main");
        let visible = state.windows.focused_direct_sessions("main");
        mark_direct_sessions_viewed(&state, &visible).unwrap();

        let focused = repo::tab::get(&state.db.get().unwrap(), &tab.id)
            .unwrap()
            .unwrap();
        assert!(
            parsed(focused.last_viewed_at.as_deref())
                >= parsed(focused.last_completed_at.as_deref())
        );
        assert_eq!(drain_attention_count(&mut rx), 1);
    }
}

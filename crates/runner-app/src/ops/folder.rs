use crate::error::{Error, Result};
use crate::repo;
use crate::AppCore;

use crate::repo::folder::FolderRow;

fn clean_name(name: String) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::msg("folder name cannot be empty"));
    }
    Ok(name.to_owned())
}

pub fn folder_list(state: &AppCore) -> Result<Vec<FolderRow>> {
    let conn = state.db.get()?;
    Ok(repo::folder::list(&conn)?)
}

pub fn folder_create(state: &AppCore, name: String) -> Result<FolderRow> {
    let conn = state.db.get()?;
    let row = repo::folder::create(&conn, &clean_name(name)?)?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(row)
}

pub fn folder_rename(state: &AppCore, id: String, name: String) -> Result<FolderRow> {
    let conn = state.db.get()?;
    if repo::folder::rename(&conn, &id, &clean_name(name)?)? == 0 {
        return Err(Error::msg(format!("folder not found: {id}")));
    }
    let row = repo::folder::get(&conn, &id)?.ok_or_else(|| Error::msg("folder disappeared"))?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(row)
}

pub fn folder_reorder(state: &AppCore, ordered_ids: Vec<String>) -> Result<Vec<FolderRow>> {
    let conn = state.db.get()?;
    repo::folder::reorder(&conn, &ordered_ids)?;
    let rows = repo::folder::list(&conn)?;
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(rows)
}

pub async fn folder_delete(state: &AppCore, id: String) -> Result<()> {
    let member_rows = {
        let conn = state.db.get()?;
        if repo::folder::get(&conn, &id)?.is_none() {
            return Err(Error::msg(format!("folder not found: {id}")));
        }
        repo::tab::list(&conn)?
            .into_iter()
            .filter(|tab| tab.folder_id.as_deref() == Some(id.as_str()))
            .flat_map(|tab| repo::tab::session_ids(&tab))
            .collect::<Vec<_>>()
    };

    for session_id in &member_rows {
        let running = {
            let conn = state.db.get()?;
            repo::session::get_row(&conn, session_id)?
                .is_some_and(|row| row.status == crate::model::SessionStatus::Running)
        };
        if running {
            state
                .sessions
                .kill(session_id)
                .map_err(|e| Error::msg(format!("stop session {session_id}: {e}")))?;
        }
    }

    let archived_ids = {
        let mut conn = state.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let archived_ids = repo::tab::delete_folder_tabs_and_archive(&tx, &id)?;
        if repo::folder::delete_after_tabs(&tx, &id)? == 0 {
            return Err(Error::msg(format!("folder not found: {id}")));
        }
        tx.commit()?;
        archived_ids
    };
    for session_id in &archived_ids {
        state.sessions.purge_session_buffers(session_id);
        state.events.emit(
            "session/archived",
            &serde_json::json!({ "session_id": session_id }),
        );
    }
    state
        .events
        .emit("chat/layout-changed", &serde_json::json!({}));
    Ok(())
}

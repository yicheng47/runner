use chrono::Utc;
use serde::Deserialize;
use tauri::{Emitter, State};

use crate::error::{Error, Result};
use crate::repo;
use crate::repo::tab::TabRow;
use crate::AppState;

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
    let created_at = repo::tab::get(&conn, &input.id)?
        .map(|row| row.created_at)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let row = TabRow {
        id: input.id,
        folder_id: input.folder_id,
        name: input.name.trim().to_owned(),
        position: input.position,
        layout: input.layout,
        created_at,
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

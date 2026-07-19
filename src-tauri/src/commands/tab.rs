use tauri::State;

use runner_app::error::Result;
use runner_app::ops::tab::{self, TabImportInput, TabUpsertInput};
use runner_app::repo::tab::TabRow;

use crate::AppState;

#[tauri::command]
pub fn tab_list(state: State<'_, AppState>) -> Result<Vec<TabRow>> {
    tab::tab_list(&state)
}

#[tauri::command]
pub fn tab_upsert(state: State<'_, AppState>, input: TabUpsertInput) -> Result<TabRow> {
    tab::tab_upsert(&state, input)
}

#[tauri::command]
pub fn tab_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    tab::tab_delete(&state, &id)
}

#[tauri::command]
pub fn tab_move_to_folder(
    state: State<'_, AppState>,
    id: String,
    folder_id: Option<String>,
) -> Result<TabRow> {
    tab::tab_move_to_folder(&state, &id, folder_id)
}

#[tauri::command]
pub fn tab_reorder(
    state: State<'_, AppState>,
    id: String,
    folder_id: Option<String>,
    ordered_ids: Vec<String>,
) -> Result<Vec<TabRow>> {
    tab::tab_reorder(&state, &id, folder_id, ordered_ids)
}

#[tauri::command]
pub fn tab_mark_viewed(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    member_ids: Vec<String>,
) -> Result<TabRow> {
    tab::mark_tab_viewed(&state, window.label(), &id, member_ids)
}

#[tauri::command]
pub fn tab_import_once(
    state: State<'_, AppState>,
    tabs: Vec<TabImportInput>,
) -> Result<Vec<TabRow>> {
    tab::tab_import_once(&state, tabs)
}

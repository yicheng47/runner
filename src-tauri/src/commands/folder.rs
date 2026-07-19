use tauri::State;

use runner_app::error::Result;
use runner_app::ops::folder;
use runner_app::repo::folder::FolderRow;

use crate::AppState;

#[tauri::command]
pub fn folder_list(state: State<'_, AppState>) -> Result<Vec<FolderRow>> {
    folder::folder_list(&state)
}

#[tauri::command]
pub fn folder_create(state: State<'_, AppState>, name: String) -> Result<FolderRow> {
    folder::folder_create(&state, name)
}

#[tauri::command]
pub fn folder_rename(state: State<'_, AppState>, id: String, name: String) -> Result<FolderRow> {
    folder::folder_rename(&state, id, name)
}

#[tauri::command]
pub fn folder_reorder(
    state: State<'_, AppState>,
    ordered_ids: Vec<String>,
) -> Result<Vec<FolderRow>> {
    folder::folder_reorder(&state, ordered_ids)
}

#[tauri::command]
pub async fn folder_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    folder::folder_delete(&state, id).await
}

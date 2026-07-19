use tauri::State;

use runner_app::error::Result;
use runner_app::ops::project;
use runner_app::repo::project::ProjectRow;

use crate::AppState;

#[tauri::command]
pub fn project_list(state: State<'_, AppState>) -> Result<Vec<ProjectRow>> {
    project::project_list(&state)
}

#[tauri::command]
pub fn project_create(state: State<'_, AppState>, name: String, cwd: String) -> Result<ProjectRow> {
    project::project_create(&state, name, cwd)
}

#[tauri::command]
pub fn project_rename(state: State<'_, AppState>, id: String, name: String) -> Result<ProjectRow> {
    project::project_rename(&state, id, name)
}

#[tauri::command]
pub fn project_set_cwd(state: State<'_, AppState>, id: String, cwd: String) -> Result<ProjectRow> {
    project::project_set_cwd(&state, id, cwd)
}

#[tauri::command]
pub fn project_reorder(
    state: State<'_, AppState>,
    ordered_ids: Vec<String>,
) -> Result<Vec<ProjectRow>> {
    project::project_reorder(&state, ordered_ids)
}

#[tauri::command]
pub fn project_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    project::project_delete(&state, id)
}

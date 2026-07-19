use tauri::State;

use runner_app::error::Result;
use runner_app::model::Runner;
use runner_app::ops::runner::{
    self, CreateRunnerInput, RunnerActivity, RunnerWithActivity, UpdateRunnerInput,
};

use crate::AppState;

#[tauri::command]
pub async fn runner_list(state: State<'_, AppState>) -> Result<Vec<Runner>> {
    runner::runner_list(&state)
}

#[tauri::command]
pub async fn runner_list_with_activity(
    state: State<'_, AppState>,
) -> Result<Vec<RunnerWithActivity>> {
    runner::runner_list_with_activity(&state)
}

#[tauri::command]
pub async fn runner_get(state: State<'_, AppState>, id: String) -> Result<Runner> {
    runner::runner_get(&state, &id)
}

#[tauri::command]
pub async fn runner_get_by_handle(state: State<'_, AppState>, handle: String) -> Result<Runner> {
    runner::runner_get_by_handle(&state, &handle)
}

#[tauri::command]
pub async fn runner_create(state: State<'_, AppState>, input: CreateRunnerInput) -> Result<Runner> {
    runner::runner_create(&state, input)
}

#[tauri::command]
pub async fn runner_update(
    state: State<'_, AppState>,
    id: String,
    input: UpdateRunnerInput,
) -> Result<Runner> {
    runner::runner_update(&state, &id, input)
}

#[tauri::command]
pub async fn runner_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    runner::runner_delete(&state, &id)
}

#[tauri::command]
pub async fn runner_activity(state: State<'_, AppState>, id: String) -> Result<RunnerActivity> {
    runner::runner_activity(&state, &id)
}

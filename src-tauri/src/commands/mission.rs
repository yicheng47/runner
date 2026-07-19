use tauri::State;

use runner_app::error::Result;
use runner_app::model::Mission;
use runner_app::ops::mission::{
    self, MissionSummary, PostHumanSignalInput, StartMissionInput, StartMissionOutput,
};

use crate::AppState;

#[tauri::command]
pub async fn mission_start(
    state: State<'_, AppState>,
    input: StartMissionInput,
    initial_cols: Option<u16>,
    initial_rows: Option<u16>,
) -> Result<StartMissionOutput> {
    let initial_size = initial_cols
        .zip(initial_rows)
        .filter(|(cols, rows)| *cols > 0 && *rows > 0);
    mission::mission_start_impl_with_size(&state, input, initial_size).await
}

#[tauri::command]
pub async fn mission_attach(state: State<'_, AppState>, mission_id: String) -> Result<Mission> {
    mission::mission_attach(&state, &mission_id).await
}

#[tauri::command]
pub async fn mission_stop(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission::mission_stop_impl(&state, id).await
}

#[tauri::command]
pub async fn mission_archive(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission::mission_archive_impl(&state, id).await
}

#[tauri::command]
pub async fn mission_unarchive(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission::mission_unarchive(&state, id).await
}

#[tauri::command]
pub async fn mission_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    mission::mission_delete(&state, &id)
}

#[tauri::command]
pub async fn mission_reset(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission::mission_reset_impl(&state, id).await
}

#[tauri::command]
pub async fn mission_pin(state: State<'_, AppState>, id: String, pinned: bool) -> Result<Mission> {
    mission::mission_pin_impl(&state, id, pinned).await
}

#[tauri::command]
pub async fn mission_rename(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<Mission> {
    mission::mission_rename_impl(&state, id, title).await
}

#[tauri::command]
pub async fn mission_set_project(
    state: State<'_, AppState>,
    id: String,
    project_id: Option<String>,
) -> Result<Mission> {
    mission::mission_set_project(&state, &id, project_id)
}

#[tauri::command]
pub async fn mission_list(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<Mission>> {
    mission::mission_list(&state, crew_id)
}

#[tauri::command]
pub async fn mission_list_archived(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<Mission>> {
    mission::mission_list_archived(&state, crew_id)
}

#[tauri::command]
pub async fn mission_list_summary(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<MissionSummary>> {
    mission::mission_list_summary_impl(&state, crew_id).await
}

#[tauri::command]
pub async fn mission_get(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission::mission_get(&state, &id)
}

#[tauri::command]
pub async fn mission_events_replay(
    state: State<'_, AppState>,
    mission_id: String,
) -> Result<Vec<runner_core::model::Event>> {
    mission::mission_events_replay(&state, &mission_id)
}

#[tauri::command]
pub async fn mission_post_human_signal(
    state: State<'_, AppState>,
    input: PostHumanSignalInput,
) -> Result<runner_core::model::Event> {
    mission::mission_post_human_signal_impl(&state, input).await
}

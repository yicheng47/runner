use std::collections::BTreeMap;

use tauri::State;

use runner_app::error::Result;
use runner_app::ops::session::{self, DirectSessionEntry, SessionRow};
use runner_app::session::manager::{OutputEvent, SessionActivityState, SpawnedSession};

use crate::AppState;

#[tauri::command]
pub async fn session_list(
    state: State<'_, AppState>,
    mission_id: String,
) -> Result<Vec<SessionRow>> {
    session::session_list(&state, &mission_id)
}

#[tauri::command]
pub async fn session_inject_stdin(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<()> {
    session::session_inject_stdin(&state, &session_id, &text)
}

#[tauri::command]
pub async fn session_kill(state: State<'_, AppState>, session_id: String) -> Result<()> {
    session::session_kill(&state, &session_id)
}

#[tauri::command]
pub fn session_activity_snapshot(
    state: State<'_, AppState>,
) -> BTreeMap<String, SessionActivityState> {
    session::session_activity_snapshot(&state)
}

#[tauri::command]
pub async fn session_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<()> {
    session::session_resize(&state, &session_id, cols, rows)
}

#[tauri::command]
pub async fn session_output_snapshot(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<OutputEvent>> {
    session::session_output_snapshot(&state, &session_id)
}

#[tauri::command]
pub async fn session_replay_watermark(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<u64> {
    session::session_replay_watermark(&state, &session_id)
}

#[tauri::command]
pub async fn session_paste_image(bytes: Vec<u8>, mime_type: String) -> Result<()> {
    session::session_paste_image(bytes, &mime_type)
}

#[tauri::command]
pub async fn session_list_recent_direct(
    state: State<'_, AppState>,
) -> Result<Vec<DirectSessionEntry>> {
    session::session_list_recent_direct(&state)
}

#[tauri::command]
pub async fn session_get(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<DirectSessionEntry>> {
    session::session_get(&state, &session_id)
}

#[tauri::command]
pub async fn session_archive(state: State<'_, AppState>, session_id: String) -> Result<()> {
    session::session_archive(&state, &session_id)
}

#[tauri::command]
pub async fn session_unarchive(state: State<'_, AppState>, session_id: String) -> Result<()> {
    session::session_unarchive(&state, &session_id)
}

#[tauri::command]
pub async fn session_delete(state: State<'_, AppState>, session_id: String) -> Result<()> {
    session::session_delete(&state, &session_id)
}

#[tauri::command]
pub async fn session_list_archived(state: State<'_, AppState>) -> Result<Vec<DirectSessionEntry>> {
    session::session_list_archived(&state)
}

#[tauri::command]
pub async fn session_rename(
    state: State<'_, AppState>,
    session_id: String,
    title: Option<String>,
) -> Result<()> {
    session::session_rename(&state, &session_id, title)
}

#[tauri::command]
pub async fn session_pin(
    state: State<'_, AppState>,
    session_id: String,
    pinned: bool,
) -> Result<()> {
    session::session_pin(&state, &session_id, pinned)
}

#[tauri::command]
pub async fn session_resume(
    state: State<'_, AppState>,
    session_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    session::session_resume(&state, &session_id, cols, rows)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn session_start_direct(
    state: State<'_, AppState>,
    runner_id: String,
    runtime: Option<String>,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    session::session_start_direct(&state, runner_id, runtime, project_id, cwd, cols, rows)
}

#[tauri::command]
pub async fn session_start_runtime(
    state: State<'_, AppState>,
    runtime: String,
    project_id: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SpawnedSession> {
    session::session_start_runtime(&state, &runtime, project_id, cwd, cols, rows)
}

#[tauri::command]
pub async fn session_set_project(
    state: State<'_, AppState>,
    session_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<()> {
    session::session_set_project(&state, session_ids, project_id)
}

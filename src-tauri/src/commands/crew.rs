use tauri::State;

use runner_app::error::Result;
use runner_app::model::Crew;
use runner_app::ops::crew::{self, CreateCrewInput, CrewListItem, UpdateCrewInput};

use crate::AppState;

#[tauri::command]
pub async fn crew_list(state: State<'_, AppState>) -> Result<Vec<CrewListItem>> {
    crew::crew_list(&state)
}

#[tauri::command]
pub async fn crew_get(state: State<'_, AppState>, id: String) -> Result<Crew> {
    crew::crew_get(&state, &id)
}

#[tauri::command]
pub async fn crew_create(state: State<'_, AppState>, input: CreateCrewInput) -> Result<Crew> {
    crew::crew_create(&state, input)
}

#[tauri::command]
pub async fn crew_update(
    state: State<'_, AppState>,
    id: String,
    input: UpdateCrewInput,
) -> Result<Crew> {
    crew::crew_update(&state, &id, input)
}

#[tauri::command]
pub async fn crew_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    crew::crew_delete(&state, &id)
}

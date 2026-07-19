use tauri::State;

use runner_app::error::Result;
use runner_app::model::SlotWithRunner;
use runner_app::ops::slot::{self, CreateSlotInput, CrewMembership, UpdateSlotInput};

use crate::AppState;

#[tauri::command]
pub async fn slot_list(state: State<'_, AppState>, crew_id: String) -> Result<Vec<SlotWithRunner>> {
    slot::slot_list(&state, &crew_id)
}

#[tauri::command]
pub async fn runner_crews_list(
    state: State<'_, AppState>,
    runner_id: String,
) -> Result<Vec<CrewMembership>> {
    slot::runner_crews_list(&state, &runner_id)
}

#[tauri::command]
pub async fn slot_create(
    state: State<'_, AppState>,
    input: CreateSlotInput,
) -> Result<SlotWithRunner> {
    slot::slot_create(&state, input)
}

#[tauri::command]
pub async fn slot_update(
    state: State<'_, AppState>,
    slot_id: String,
    input: UpdateSlotInput,
) -> Result<SlotWithRunner> {
    slot::slot_update(&state, &slot_id, input)
}

#[tauri::command]
pub async fn slot_delete(state: State<'_, AppState>, slot_id: String) -> Result<()> {
    slot::slot_delete(&state, &slot_id)
}

#[tauri::command]
pub async fn slot_set_lead(state: State<'_, AppState>, slot_id: String) -> Result<SlotWithRunner> {
    slot::slot_set_lead(&state, &slot_id)
}

#[tauri::command]
pub async fn slot_reorder(
    state: State<'_, AppState>,
    crew_id: String,
    ordered_slot_ids: Vec<String>,
) -> Result<Vec<SlotWithRunner>> {
    slot::slot_reorder(&state, &crew_id, ordered_slot_ids)
}

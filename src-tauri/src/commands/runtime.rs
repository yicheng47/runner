use runner_app::ops::runtime::{self, RuntimeDefinition};

#[tauri::command]
pub async fn runtime_list() -> Vec<RuntimeDefinition> {
    runtime::runtime_list()
}

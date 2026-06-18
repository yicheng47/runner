use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDefinition {
    pub name: String,
    pub display_name: String,
    pub command: String,
}

#[tauri::command]
pub async fn runtime_list() -> Vec<RuntimeDefinition> {
    crate::router::runtime::runtime_definitions()
        .iter()
        .map(|runtime| RuntimeDefinition {
            name: runtime.name.to_string(),
            display_name: runtime.display_name.to_string(),
            command: runtime.command.to_string(),
        })
        .collect()
}

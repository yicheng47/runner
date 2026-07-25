use serde::Serialize;
use tauri::{Emitter, State};

use crate::error::{Error, Result};
use crate::runtime_status::{OverrideValidationError, RuntimeStatusResponse};
use crate::AppState;

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

#[tauri::command]
pub async fn runtime_status_list(state: State<'_, AppState>) -> Result<RuntimeStatusResponse> {
    crate::runtime_status::status_list(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    )
}

#[tauri::command]
pub async fn runtime_set_override(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    runtime: String,
    path: String,
) -> std::result::Result<RuntimeStatusResponse, OverrideValidationError> {
    let path = path.trim();
    if crate::router::runtime::runtime_definition(&runtime).is_none() {
        return Err(OverrideValidationError {
            code: "unknown_runtime".into(),
            message: format!("Unknown runtime: {runtime}."),
        });
    }
    if path.is_empty() {
        crate::db::set_runtime_override(&state.db, &runtime, None).map_err(persistence_error)?;
    } else {
        crate::runtime_status::validate_override(&runtime, path)?;
        crate::db::set_runtime_override(&state.db, &runtime, Some(path))
            .map_err(persistence_error)?;
        log::info!("runtime override saved: runtime={} path={}", runtime, path);
    }
    app.emit("runtime/changed", ())
        .map_err(|error| persistence_error(Error::msg(error.to_string())))?;
    crate::runtime_status::status_list(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    )
    .map_err(persistence_error)
}

#[tauri::command]
pub async fn runtime_clear_override(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    runtime: String,
) -> Result<RuntimeStatusResponse> {
    if crate::router::runtime::runtime_definition(&runtime).is_none() {
        return Err(Error::msg(format!("unknown runtime: {runtime}")));
    }
    crate::db::set_runtime_override(&state.db, &runtime, None)?;
    log::info!("runtime override cleared: runtime={runtime}");
    app.emit("runtime/changed", ())
        .map_err(|error| Error::msg(format!("emit runtime/changed: {error}")))?;
    crate::runtime_status::status_list(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    )
}

#[tauri::command]
pub async fn runtime_refresh(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<RuntimeStatusResponse> {
    crate::runtime_status::refresh_background_discovery(
        app,
        state.db.clone(),
        state.runtime_shell_env.clone(),
        state.runtime_discovery.clone(),
    )?;
    crate::runtime_status::status_list(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
    )
}

fn persistence_error(error: Error) -> OverrideValidationError {
    OverrideValidationError {
        code: "persistence_failed".into(),
        message: error.to_string(),
    }
}

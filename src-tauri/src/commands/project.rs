use tauri::{Emitter, State};

use crate::error::{Error, Result};
use crate::repo;
use crate::repo::project::ProjectRow;
use crate::AppState;

fn clean_value(value: String, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::msg(format!("project {label} cannot be empty")));
    }
    Ok(value.to_owned())
}

fn emit_changed(app: &tauri::AppHandle) {
    let _ = app.emit("project/changed", serde_json::json!({}));
}

#[tauri::command]
pub fn project_list(state: State<'_, AppState>) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    Ok(repo::project::list(&conn)?)
}

#[tauri::command]
pub fn project_create(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    name: String,
    cwd: String,
) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    let row = repo::project::create(
        &conn,
        &clean_value(name, "name")?,
        &clean_value(cwd, "cwd")?,
    )?;
    emit_changed(&app);
    Ok(row)
}

#[tauri::command]
pub fn project_rename(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::rename(&conn, &id, &clean_value(name, "name")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(&app);
    Ok(row)
}

#[tauri::command]
pub fn project_set_cwd(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    cwd: String,
) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::set_cwd(&conn, &id, &clean_value(cwd, "cwd")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(&app);
    Ok(row)
}

#[tauri::command]
pub fn project_set_collapsed(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    collapsed: bool,
) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::set_collapsed(&conn, &id, collapsed)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(&app);
    Ok(row)
}

#[tauri::command]
pub fn project_reorder(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    ordered_ids: Vec<String>,
) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    repo::project::reorder(&conn, &ordered_ids)?;
    let rows = repo::project::list(&conn)?;
    emit_changed(&app);
    Ok(rows)
}

#[tauri::command]
pub fn project_delete(state: State<'_, AppState>, app: tauri::AppHandle, id: String) -> Result<()> {
    let conn = state.db.get()?;
    if repo::project::delete(&conn, &id)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    emit_changed(&app);
    let _ = app.emit("mission/changed", serde_json::json!({}));
    let _ = app.emit("session/updated", serde_json::json!({}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::clean_value;

    #[test]
    fn clean_value_trims_and_rejects_blank() {
        assert_eq!(clean_value("  Runner  ".into(), "name").unwrap(), "Runner");
        assert!(clean_value("  ".into(), "cwd").is_err());
    }
}

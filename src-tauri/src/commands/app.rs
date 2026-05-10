use tauri::{AppHandle, Manager};

use crate::error::Error;

/// Called by the frontend after React has mounted and painted its first frame.
/// Shows the main window — the window starts hidden so the user sees the dock
/// bounce → fully-rendered window instead of a blank webview.
#[tauri::command]
pub fn app_ready(app: AppHandle) -> crate::error::Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| Error::msg("main window not found"))?;
    window.show().map_err(|e| Error::msg(e.to_string()))?;
    window.set_focus().map_err(|e| Error::msg(e.to_string()))?;
    Ok(())
}

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

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

/// Reveal the app log directory in the system file browser.
///
/// On macOS this is `~/Library/Logs/com.wycstudios.runner/`, where
/// `tauri-plugin-log` writes `runner.log` (+ rotations). Surfaced via
/// the Help → "Reveal logs in Finder" menu and the Settings →
/// Diagnostics pane; both routes call this same command.
#[tauri::command]
pub fn runner_logs_reveal(app: AppHandle) -> crate::error::Result<()> {
    reveal_logs_dir(&app)
}

/// Shared backend for both menu and Settings-pane entry points.
pub(crate) fn reveal_logs_dir(app: &AppHandle) -> crate::error::Result<()> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| Error::msg(format!("resolve log dir: {e}")))?;
    // `tauri-plugin-log` lazily creates the dir on first write. If
    // the user clicks "Reveal" before any log line has been emitted
    // (race-y but possible during very early startup), create it
    // ourselves so the opener has something to point at.
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| Error::msg(format!("create log dir: {e}")))?;
    }
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| Error::msg(format!("open log dir: {e}")))?;
    Ok(())
}

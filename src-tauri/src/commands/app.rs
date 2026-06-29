use tauri::AppHandle;
// `Manager` is only needed by the macOS-only `show_main_window`
// (`get_webview_window`); importing it unconditionally trips `unused_imports`
// under `-D warnings` on other platforms.
#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

use crate::error::Error;
use crate::log_dir_for_build;

/// Called by the frontend after React has mounted and painted its first frame.
/// Shows the **calling** window — every window (main + secondaries) starts
/// hidden so the user sees the dock bounce → fully-rendered window instead of
/// a blank webview. Resolving the window from the invoking webview (rather
/// than hard-coding `main`) is what lets secondary windows reveal themselves.
#[tauri::command]
pub fn app_ready(window: tauri::WebviewWindow) -> crate::error::Result<()> {
    window.show().map_err(|e| Error::msg(e.to_string()))?;
    window.set_focus().map_err(|e| Error::msg(e.to_string()))?;
    Ok(())
}

/// Show + focus the `main` window. Used only by the macOS `Reopen` path
/// (dock-icon click with no visible windows); `app_ready` shows the calling
/// window directly, so this is dead code on other platforms.
#[cfg(target_os = "macos")]
pub(crate) fn show_main_window(app: &AppHandle) -> crate::error::Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| Error::msg("main window not found"))?;
    window.show().map_err(|e| Error::msg(e.to_string()))?;
    window.set_focus().map_err(|e| Error::msg(e.to_string()))?;
    Ok(())
}

/// Reveal the app log directory in the system file browser.
///
/// On macOS this is `~/Library/Logs/com.wycstudios.runner/` for
/// release builds and `…/com.wycstudios.runner-dev/` for `tauri dev`
/// builds — `tauri-plugin-log` writes `runner.log` (+ rotations)
/// there. Surfaced via the Help → "Reveal logs in Finder" menu and
/// the Settings → Diagnostics pane; both routes call this same
/// command.
#[tauri::command]
pub fn runner_logs_reveal(app: AppHandle) -> crate::error::Result<()> {
    reveal_logs_dir(&app)
}

/// Shared backend for both menu and Settings-pane entry points.
pub(crate) fn reveal_logs_dir(app: &AppHandle) -> crate::error::Result<()> {
    // Must mirror the dev/prod-split path the plugin-log builder is
    // wired with in `lib.rs`. `app.path().app_log_dir()` ignores the
    // `-dev` segment and would point at the prod dir in dev builds —
    // the same drift that made the user file a bug just now.
    let dir = log_dir_for_build();
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

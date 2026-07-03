// Window lifecycle commands (impl 0018, spec 12).
//
// Additional webview windows are built Rust-side via `WebviewWindowBuilder`,
// so no JS-side `core:webview:allow-create-webview-window` permission is
// needed — the frontend only ever *invokes* these commands. Each new window
// mounts the same React bundle; an optional initial route rides as a URL hash
// fragment that a tiny frontend bootstrap consumes on mount.

use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::error::{Error, Result};
use crate::windows::{Subject, WindowEntry};
use crate::{broadcast_focus_map, AppState};

/// Shared window-building path used by both the `window_open` command and the
/// `File → New Window` menu handler. Kept out of the command wrapper so
/// `on_menu_event` can call the same code via `app.state::<AppState>()`.
pub fn open_window(
    app: &AppHandle,
    state: &AppState,
    initial_route: Option<String>,
    position: Option<(i32, i32)>,
) -> Result<String> {
    let label = format!("window-{}", ulid::Ulid::new());

    // BrowserRouter can't resolve a deep path (`/missions/<id>`) through
    // Tauri's asset protocol in release builds, so the initial route rides as
    // a hash fragment that `InitialRouteBootstrap` navigates to on mount.
    let url = match initial_route {
        Some(route) if !route.is_empty() => {
            let route = if route.starts_with('/') {
                route
            } else {
                format!("/{route}")
            };
            WebviewUrl::App(format!("index.html#{route}").into())
        }
        _ => WebviewUrl::App("index.html".into()),
    };

    // Mirror `main`'s chrome (tauri.conf.json) so secondary windows are
    // visually identical. Starts hidden to avoid the white flash; the
    // generalized `app_ready` shows the calling window after first paint.
    // `mut` is only consumed by the macOS chrome block below — other
    // platforms have no extra builder methods, hence the conditional allow.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = WebviewWindowBuilder::new(app, &label, url)
        .title("Runner")
        .inner_size(1440.0, 900.0)
        .accept_first_mouse(true)
        .visible(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(16.0, 22.0));
    }

    let window = builder.build().map_err(|e| Error::msg(e.to_string()))?;

    // Position: an explicit hint wins; otherwise cascade off the
    // most-recently-focused existing window so a fresh window doesn't land
    // exactly on top of the one it was spawned from.
    if let Some((x, y)) = position {
        let _ = window.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    } else if let Some(reference) = cascade_reference(app, state, &label) {
        if let Ok(pos) = reference.outer_position() {
            let _ = window.set_position(tauri::PhysicalPosition::new(pos.x + 32, pos.y + 32));
        }
    }

    state.windows.register(&label);
    broadcast_focus_map(app);
    Ok(label)
}

/// Anchor window for the cascade offset: the most-recently-focused window that
/// already exists (excluding the one being built).
fn cascade_reference(
    app: &AppHandle,
    state: &AppState,
    new_label: &str,
) -> Option<tauri::WebviewWindow> {
    let label = state
        .windows
        .snapshot()
        .into_iter()
        .filter(|e| e.label != new_label)
        .max_by(|a, b| a.focused_at.cmp(&b.focused_at))
        .map(|e| e.label)?;
    app.get_webview_window(&label)
}

/// Open a new webview window. Returns the new window's label. Thin wrapper
/// around `open_window` so the menu handler and the command share one path.
#[tauri::command]
pub fn window_open(
    app: AppHandle,
    state: State<AppState>,
    initial_route: Option<String>,
    position: Option<(i32, i32)>,
) -> Result<String> {
    open_window(&app, &state, initial_route, position)
}

/// Bring another window to the front — the overlay's "Focus that window"
/// action. `core:window:allow-set-focus` is already granted.
#[tauri::command]
pub fn window_focus_other(app: AppHandle, label: String) -> Result<()> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| Error::msg(format!("window {label} not found")))?;
    window.set_focus().map_err(|e| Error::msg(e.to_string()))?;
    Ok(())
}

/// The frontend reports the subjects it currently shows on every route or
/// pane-layout change — one for a single-pane surface, up to three when the
/// direct-chat surface is split (impl 0020). Label is resolved from the
/// invoking webview, not trusted from the caller.
#[tauri::command]
pub fn window_report_subjects(
    window: tauri::WebviewWindow,
    state: State<AppState>,
    app: AppHandle,
    subjects: Vec<Subject>,
) -> Result<()> {
    state.windows.set_subjects(window.label(), subjects);
    broadcast_focus_map(&app);
    Ok(())
}

/// Snapshot of the focus map, so a freshly-mounted window can hydrate without
/// waiting for the next broadcast.
#[tauri::command]
pub fn window_list_subjects(state: State<AppState>) -> Result<Vec<WindowEntry>> {
    Ok(state.windows.snapshot())
}

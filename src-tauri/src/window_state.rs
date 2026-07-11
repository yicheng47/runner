// Main-window geometry persistence (impl 0027), hand-rolled after
// pivoting off `tauri-plugin-window-state`.
//
// Why not the plugin: it stores PHYSICAL coordinates and gates restore
// on a physical-space monitor-intersection check. On multi-monitor
// macOS the physical space is fractured (tauri#7890 — monitor sizes
// unscaled, monitor offsets scaled, secondary-screen window positions
// inconsistent with the primary), so the check fails for any window on
// a secondary display and restore silently degrades to OS-default
// placement. macOS's native global space is LOGICAL points, which is
// consistent across mixed-scale monitors and round-trips cleanly
// through tao's set_position — so everything here is stored and
// applied logical.
//
// Semantics match what the plugin was configured for: size + position
// + maximized, main window only, restored while the window is still
// hidden (before `app_ready` reveals it, so no flash and no jump).
// The state file lives in `app_data_dir`, which already carries the
// dev/prod `-dev` split — no filename games needed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewWindow};

const STATE_FILENAME: &str = "window-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Outer frame origin and inner size, in logical points. Always the
    /// NORMAL (un-maximized) frame — a maximized quit keeps the last
    /// normal geometry here so un-maximize after relaunch lands on a
    /// real frame instead of a monitor-sized one.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

/// Live snapshot, refreshed on every main-window Moved/Resized and read
/// back at save time. Needed because `RunEvent::Exit` can fire after
/// the window is gone, and because a maximized window's live frame is
/// the monitor's — the last normal geometry only exists here. Main
/// window only, so a module singleton is enough.
static LAST: Mutex<Option<WindowState>> = Mutex::new(None);

fn state_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(STATE_FILENAME)
}

fn read(app_data_dir: &Path) -> Option<WindowState> {
    let raw = std::fs::read_to_string(state_path(app_data_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_atomic(app_data_dir: &Path, state: &WindowState) -> std::io::Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(app_data_dir)?;
    serde_json::to_writer_pretty(&mut tmp, state)?;
    tmp.flush()?;
    tmp.persist(state_path(app_data_dir))?;
    Ok(())
}

/// Current snapshot from the live window: normal geometry when the
/// window is normal; the cached normal geometry with the flag flipped
/// when maximized (the live frame would be the monitor's).
fn snapshot<R: Runtime>(window: &WebviewWindow<R>) -> Option<WindowState> {
    let maximized = window.is_maximized().ok()?;
    if maximized {
        let last = *LAST.lock().unwrap();
        return last.map(|s| WindowState {
            maximized: true,
            ..s
        });
    }
    let scale = window.scale_factor().ok()?;
    let position = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    Some(WindowState {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized: false,
    })
}

/// Moved/Resized hook. Cheap enough to run per drag tick; the plugin
/// maintained the same event-driven cache.
pub fn note<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(s) = snapshot(&window) {
        *LAST.lock().unwrap() = Some(s);
    }
}

/// Persist the current state. Called from the main window's
/// hide-on-close (checkpoint — also covers crash resilience, the open
/// question impl 0027 deferred) and from `RunEvent::Exit` (final).
/// Falls back to the cache when the window is already gone, and to the
/// existing file when there was never anything to observe.
pub fn save<R: Runtime>(app: &AppHandle<R>, app_data_dir: &Path) {
    note(app);
    let state = (*LAST.lock().unwrap()).or_else(|| read(app_data_dir));
    let Some(state) = state else {
        return;
    };
    if let Err(e) = write_atomic(app_data_dir, &state) {
        log::warn!("window-state: save failed: {e}");
    }
}

/// Apply persisted geometry during `setup`, while the main window is
/// still hidden — the `app_ready` reveal then happens directly at the
/// restored frame. Off-screen recovery: position is applied only when
/// the saved frame overlaps some monitor's logical rect; otherwise the
/// OS keeps placement and only the size restores (monitor unplugged
/// since last quit).
pub fn restore<R: Runtime>(window: &WebviewWindow<R>, app_data_dir: &Path) {
    let Some(state) = read(app_data_dir) else {
        // Fresh install: seed the cache from the config-default frame
        // anyway. `snapshot`'s maximized branch needs a cached normal
        // frame, so without this a maximize-before-any-move quit on
        // first launch would have nothing to persist.
        *LAST.lock().unwrap() = snapshot(window);
        return;
    };
    let _ = window.set_size(LogicalSize::new(state.width, state.height));
    let monitors: Vec<Rect> = window
        .available_monitors()
        .map(|monitors| {
            monitors
                .iter()
                .map(|m| {
                    // work_area is computed from NSScreen.visibleFrame
                    // (AppKit's consistent logical space) scaled by
                    // this monitor's own factor, so dividing by that
                    // factor recovers true logical points.
                    // Monitor::size() does NOT round-trip like this —
                    // it derives from CGDisplay pixel APIs, the
                    // fractured space of tauri#7890 this module exists
                    // to avoid.
                    let sf = m.scale_factor();
                    let pos = m.work_area().position.to_logical::<f64>(sf);
                    let size = m.work_area().size.to_logical::<f64>(sf);
                    Rect {
                        x: pos.x,
                        y: pos.y,
                        width: size.width,
                        height: size.height,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if overlaps_any(
        &Rect {
            x: state.x,
            y: state.y,
            width: state.width,
            height: state.height,
        },
        &monitors,
    ) {
        let _ = window.set_position(LogicalPosition::new(state.x, state.y));
    }
    if state.maximized {
        let _ = window.maximize();
    }
    // Seed the cache so a launch-then-quit without any move/resize
    // still writes back the same normal geometry.
    *LAST.lock().unwrap() = Some(state);
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn overlaps_any(frame: &Rect, monitors: &[Rect]) -> bool {
    monitors.iter().any(|m| {
        frame.x < m.x + m.width
            && frame.x + frame.width > m.x
            && frame.y < m.y + m.height
            && frame.y + frame.height > m.y
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn state_file_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let state = WindowState {
            x: -1080.0,
            y: 374.0,
            width: 1080.0,
            height: 1117.0,
            maximized: true,
        };
        write_atomic(tmp.path(), &state).unwrap();
        assert_eq!(read(tmp.path()), Some(state));
    }

    #[test]
    fn read_tolerates_missing_and_garbage_files() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read(tmp.path()), None);
        std::fs::write(state_path(tmp.path()), "not json").unwrap();
        assert_eq!(read(tmp.path()), None);
    }

    #[test]
    fn overlap_accepts_secondary_monitor_negative_coords() {
        // Three-screen layout: portrait left at x=-1080, primary at 0,
        // right at 2560 — logical points. A frame on the left screen
        // (the exact case the plugin's physical-space check failed)
        // must count as on-screen.
        let monitors = [
            rect(-1080.0, 0.0, 1080.0, 1920.0),
            rect(0.0, 0.0, 2560.0, 1440.0),
            rect(2560.0, 0.0, 2560.0, 1440.0),
        ];
        assert!(overlaps_any(
            &rect(-1080.0, 374.0, 1080.0, 1117.0),
            &monitors
        ));
    }

    #[test]
    fn overlap_rejects_frame_just_beyond_scaled_monitor_edge() {
        // Guards the doubled-size mistake the review caught: monitor
        // rects must be true logical (2560 wide for a 2x display, not
        // 5120 backing pixels). A frame parked just past the right
        // edge sits inside the doubled rect but outside the real one —
        // it must be rejected or an unplugged-display frame would
        // restore off-screen.
        let monitors = [rect(0.0, 0.0, 2560.0, 1440.0)];
        assert!(!overlaps_any(
            &rect(2600.0, 100.0, 1080.0, 800.0),
            &monitors
        ));
        assert!(overlaps_any(&rect(2500.0, 100.0, 1080.0, 800.0), &monitors));
    }

    #[test]
    fn overlap_rejects_frame_on_unplugged_monitor() {
        // Same saved frame, but the left monitor is gone: no overlap,
        // so restore keeps size and lets the OS place the window.
        let monitors = [rect(0.0, 0.0, 2560.0, 1440.0)];
        assert!(!overlaps_any(
            &rect(-1080.0, 374.0, 1080.0, 1117.0),
            &monitors
        ));
        // Partial overlap (frame straddling a live edge) still counts.
        assert!(overlaps_any(&rect(-200.0, 100.0, 1080.0, 800.0), &monitors));
    }
}

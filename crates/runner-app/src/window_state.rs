use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui::{point, px, size, App, Bounds, Pixels, Window, WindowBounds};
use serde::{Deserialize, Serialize};

const STATE_FILENAME: &str = "window-state.json";
const LAYOUT_FILENAME: &str = "window-layout.json";
const WINDOW_WIDTH_MIN: f64 = 640.;
const WINDOW_HEIGHT_MIN: f64 = 480.;
pub(crate) const MAX_SECONDARY_WINDOWS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowState {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SecondaryWindowState {
    pub(crate) frame: WindowState,
    pub(crate) route: Option<String>,
    #[serde(default)]
    pub(crate) focused_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct MainWindowState {
    #[serde(default)]
    pub(crate) route: Option<String>,
    #[serde(default)]
    pub(crate) focused_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowLayout {
    pub(crate) main_open: bool,
    #[serde(default)]
    pub(crate) main_window: MainWindowState,
    pub(crate) secondary_windows: Vec<SecondaryWindowState>,
}

impl Default for WindowLayout {
    fn default() -> Self {
        Self {
            main_open: true,
            main_window: MainWindowState::default(),
            secondary_windows: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct LayoutRead {
    pub(crate) layout: WindowLayout,
    pub(crate) warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RestoredWindowState {
    Main(MainWindowState),
    Secondary(SecondaryWindowState),
}

impl RestoredWindowState {
    pub(crate) fn focused_at(&self) -> i64 {
        match self {
            Self::Main(state) => state.focused_at,
            Self::Secondary(state) => state.focused_at,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

pub(crate) fn state_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(STATE_FILENAME)
}

pub(crate) fn layout_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LAYOUT_FILENAME)
}

pub(crate) fn read(app_data_dir: &Path) -> Option<WindowState> {
    let raw = std::fs::read_to_string(state_path(app_data_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("window state path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file in {}", parent.display()))?;
    serde_json::to_writer_pretty(&mut tmp, value).context("serialize window state")?;
    tmp.flush().context("flush window state")?;
    tmp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(crate) fn save(app_data_dir: &Path, state: WindowState) -> Result<()> {
    write_atomic(&state_path(app_data_dir), &state)
}

pub(crate) fn read_layout(app_data_dir: &Path) -> LayoutRead {
    let path = layout_path(app_data_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LayoutRead {
                layout: WindowLayout::default(),
                warnings: vec![format!(
                    "{} is missing; using the default layout",
                    path.display()
                )],
            };
        }
        Err(error) => {
            return LayoutRead {
                layout: WindowLayout::default(),
                warnings: vec![format!(
                    "read {}: {error}; using the default layout",
                    path.display()
                )],
            };
        }
    };
    let value = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value,
        Err(error) => {
            return LayoutRead {
                layout: WindowLayout::default(),
                warnings: vec![format!(
                    "parse {}: {error}; using the default layout",
                    path.display()
                )],
            };
        }
    };
    if value.is_array() {
        return LayoutRead {
            layout: WindowLayout::default(),
            warnings: vec![format!(
                "{} uses the legacy bare-array shape; using the default layout",
                path.display()
            )],
        };
    }
    let mut layout = match serde_json::from_value::<WindowLayout>(value) {
        Ok(layout) => layout,
        Err(error) => {
            return LayoutRead {
                layout: WindowLayout::default(),
                warnings: vec![format!(
                    "parse {}: {error}; using the default layout",
                    path.display()
                )],
            };
        }
    };
    let mut warnings = Vec::new();
    if layout.secondary_windows.len() > MAX_SECONDARY_WINDOWS {
        let dropped = layout.secondary_windows.len() - MAX_SECONDARY_WINDOWS;
        layout
            .secondary_windows
            .sort_by_key(|window| window.focused_at);
        layout.secondary_windows.drain(..dropped);
        warnings.push(format!(
            "{} contains too many secondary windows; dropped the {dropped} least recently focused",
            path.display()
        ));
    }
    if !layout.main_open && layout.secondary_windows.is_empty() {
        layout = WindowLayout::default();
        warnings.push(format!(
            "{} describes no open windows; using the default layout",
            path.display()
        ));
    }
    LayoutRead { layout, warnings }
}

pub(crate) fn save_layout(app_data_dir: &Path, layout: &WindowLayout) -> Result<()> {
    write_atomic(&layout_path(app_data_dir), layout)
}

pub(crate) fn restore_order(layout: WindowLayout) -> Vec<RestoredWindowState> {
    let mut windows = layout
        .secondary_windows
        .into_iter()
        .map(RestoredWindowState::Secondary)
        .collect::<Vec<_>>();
    if layout.main_open {
        windows.push(RestoredWindowState::Main(layout.main_window));
    }
    windows.sort_by_key(RestoredWindowState::focused_at);
    windows
}

pub(crate) fn load_and_migrate(
    app_data_dir: &Path,
    settings_path: &Path,
    default_frame: Rect,
) -> Result<Option<WindowState>> {
    let existing = read(app_data_dir);
    let legacy_size = retire_legacy_size(settings_path)?;
    if existing.is_some() {
        return Ok(existing);
    }
    let Some((width, height)) = legacy_size else {
        return Ok(None);
    };
    let state = WindowState {
        x: default_frame.x + (default_frame.width - width) / 2.,
        y: default_frame.y + (default_frame.height - height) / 2.,
        width,
        height,
        maximized: false,
    };
    save(app_data_dir, state)?;
    Ok(Some(state))
}

fn retire_legacy_size(path: &Path) -> Result<Option<(f64, f64)>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let object = value
        .as_object_mut()
        .with_context(|| format!("{} must contain a JSON object", path.display()))?;
    let width = object
        .get("windowWidth")
        .and_then(serde_json::Value::as_f64);
    let height = object
        .get("windowHeight")
        .and_then(serde_json::Value::as_f64);
    let had_legacy =
        object.remove("windowWidth").is_some() | object.remove("windowHeight").is_some();
    if had_legacy {
        write_atomic(path, &value)?;
    }
    Ok(width.zip(height).filter(|(width, height)| {
        width.is_finite() && height.is_finite() && *width > 0. && *height > 0.
    }))
}

pub(crate) fn restored_bounds(
    state: WindowState,
    displays: &[Rect],
    fallback: Bounds<Pixels>,
) -> WindowBounds {
    let mut saved = Rect {
        x: state.x,
        y: state.y,
        width: normalize_dimension(
            state.width,
            f32::from(fallback.size.width) as f64,
            WINDOW_WIDTH_MIN,
        ),
        height: normalize_dimension(
            state.height,
            f32::from(fallback.size.height) as f64,
            WINDOW_HEIGHT_MIN,
        ),
    };
    let display = displays
        .iter()
        .find(|display| overlaps(&saved, display))
        .or_else(|| displays.first());
    if let Some(display) = display {
        saved.width = saved.width.min(display.width);
        saved.height = saved.height.min(display.height);
    }
    let origin = if overlaps_any(&saved, displays) {
        point(px(state.x as f32), px(state.y as f32))
    } else {
        fallback.origin
    };
    let bounds = Bounds::new(
        origin,
        size(px(saved.width as f32), px(saved.height as f32)),
    );
    if state.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

fn normalize_dimension(value: f64, fallback: f64, minimum: f64) -> f64 {
    if value.is_finite() && value > 0. {
        value.max(minimum)
    } else {
        fallback
    }
}

pub(crate) fn snapshot(window: &Window, previous: Option<WindowState>) -> WindowState {
    #[cfg(target_os = "macos")]
    if let Some((bounds, content_size)) = native_window_frame(window) {
        return snapshot_from_bounds(
            window.window_bounds(),
            content_size,
            window.is_maximized(),
            previous,
            Some(bounds),
        );
    }
    snapshot_from_bounds(
        window.window_bounds(),
        window.viewport_size(),
        window.is_maximized(),
        previous,
        None,
    )
}

fn snapshot_from_bounds(
    window_bounds: WindowBounds,
    content_size: gpui::Size<Pixels>,
    maximized: bool,
    previous: Option<WindowState>,
    outer_bounds: Option<Bounds<Pixels>>,
) -> WindowState {
    let (bounds, content_size, maximized) = match window_bounds {
        WindowBounds::Fullscreen(_) => {
            if let Some(previous) = previous {
                return WindowState {
                    maximized: false,
                    ..previous
                };
            }
            (window_bounds.get_bounds(), content_size, false)
        }
        WindowBounds::Maximized(_) | WindowBounds::Windowed(_) if maximized => {
            if let Some(previous) = previous {
                return WindowState {
                    maximized: true,
                    ..previous
                };
            }
            (window_bounds.get_bounds(), content_size, true)
        }
        WindowBounds::Maximized(bounds) => (bounds, content_size, true),
        WindowBounds::Windowed(bounds) => (bounds, content_size, false),
    };
    let bounds = outer_bounds.unwrap_or(bounds);
    WindowState {
        x: f32::from(bounds.origin.x) as f64,
        y: f32::from(bounds.origin.y) as f64,
        width: f32::from(content_size.width) as f64,
        height: f32::from(content_size.height) as f64,
        maximized,
    }
}

pub(crate) fn display_rects(cx: &App) -> Vec<Rect> {
    #[cfg(target_os = "macos")]
    if let Some(rects) = native_display_rects() {
        return rects;
    }
    cx.displays()
        .into_iter()
        .map(|display| {
            let bounds = display.visible_bounds();
            Rect {
                x: f32::from(bounds.origin.x) as f64,
                y: f32::from(bounds.origin.y) as f64,
                width: f32::from(bounds.size.width) as f64,
                height: f32::from(bounds.size.height) as f64,
            }
        })
        .collect()
}

pub(crate) fn outer_origin(window: &Window) -> gpui::Point<Pixels> {
    #[cfg(target_os = "macos")]
    if let Some((bounds, _)) = native_window_frame(window) {
        return bounds.origin;
    }
    window.window_bounds().get_bounds().origin
}

#[cfg(target_os = "macos")]
fn native_window_frame(window: &Window) -> Option<(Bounds<Pixels>, gpui::Size<Pixels>)> {
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSView, NSWindow};
    use raw_window_handle::RawWindowHandle;

    let handle = raw_window_handle::HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }?;
    let native = view.window()?;
    let frame = native.frame();
    let content = NSWindow::contentRectForFrameRect(&native, frame);
    let mtm = MainThreadMarker::new()?;
    let primary_height = NSScreen::screens(mtm).objectAtIndex(0).frame().size.height;
    Some((
        Bounds::new(
            point(
                px(frame.origin.x as f32),
                px((primary_height - frame.origin.y - frame.size.height) as f32),
            ),
            size(px(frame.size.width as f32), px(frame.size.height as f32)),
        ),
        size(
            px(content.size.width as f32),
            px(content.size.height as f32),
        ),
    ))
}

#[cfg(target_os = "macos")]
fn native_display_rects() -> Option<Vec<Rect>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let primary_height = screens.objectAtIndex(0).frame().size.height;
    Some(
        (0..screens.count())
            .map(|index| {
                let frame = screens.objectAtIndex(index).visibleFrame();
                Rect {
                    x: frame.origin.x,
                    y: primary_height - frame.origin.y - frame.size.height,
                    width: frame.size.width,
                    height: frame.size.height,
                }
            })
            .collect(),
    )
}

fn overlaps_any(frame: &Rect, displays: &[Rect]) -> bool {
    displays.iter().any(|display| overlaps(frame, display))
}

fn overlaps(frame: &Rect, display: &Rect) -> bool {
    frame.x < display.x + display.width
        && frame.x + frame.width > display.x
        && frame.y < display.y + display.height
        && frame.y + frame.height > display.y
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

    fn secondary(index: usize, focused_at: i64) -> SecondaryWindowState {
        SecondaryWindowState {
            frame: WindowState {
                x: index as f64 * 32.,
                y: index as f64 * 32.,
                width: 1200.,
                height: 800.,
                maximized: false,
            },
            route: Some(format!("/missions/mission-{index}")),
            focused_at,
        }
    }

    #[test]
    fn state_file_matches_the_tauri_shape_and_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let state = WindowState {
            x: -1080.,
            y: 374.,
            width: 1080.,
            height: 1117.,
            maximized: true,
        };
        save(temp.path(), state).unwrap();
        assert_eq!(read(temp.path()), Some(state));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(state_path(temp.path())).unwrap())
                .unwrap();
        assert_eq!(
            json.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["x", "y", "width", "height", "maximized"]
        );
    }

    #[test]
    fn secondary_layout_round_trips_without_changing_main_state() {
        let temp = tempfile::tempdir().unwrap();
        let main = WindowState {
            x: 10.,
            y: 20.,
            width: 1440.,
            height: 900.,
            maximized: false,
        };
        let secondary = SecondaryWindowState {
            frame: WindowState {
                x: 42.,
                y: 52.,
                width: 1200.,
                height: 800.,
                maximized: true,
            },
            route: Some("/missions/mission-1".into()),
            focused_at: 20,
        };

        let missing = read_layout(temp.path());
        assert_eq!(missing.layout, WindowLayout::default());
        assert_eq!(missing.warnings.len(), 1);
        save(temp.path(), main).unwrap();
        let layout = WindowLayout {
            main_open: false,
            main_window: MainWindowState::default(),
            secondary_windows: vec![secondary],
        };
        save_layout(temp.path(), &layout).unwrap();

        assert_eq!(read(temp.path()), Some(main));
        assert_eq!(read_layout(temp.path()).layout, layout);
    }

    #[test]
    fn malformed_legacy_and_empty_layouts_all_recover_to_main() {
        let temp = tempfile::tempdir().unwrap();
        for raw in [
            "not json",
            "[]",
            r#"{"main_open":false,"secondary_windows":[]}"#,
        ] {
            std::fs::write(layout_path(temp.path()), raw).unwrap();
            let read = read_layout(temp.path());
            assert_eq!(read.layout, WindowLayout::default());
            assert_eq!(read.warnings.len(), 1);
        }
    }

    #[test]
    fn older_object_layout_defaults_new_focus_fields() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            layout_path(temp.path()),
            r#"{"main_open":true,"secondary_windows":[{"frame":{"x":32.0,"y":32.0,"width":1200.0,"height":800.0,"maximized":false},"route":"/runners"}]}"#,
        )
        .unwrap();
        let read = read_layout(temp.path());
        assert!(read.warnings.is_empty());
        assert_eq!(read.layout.main_window, MainWindowState::default());
        assert_eq!(read.layout.secondary_windows[0].focused_at, 0);
    }

    #[test]
    fn restore_caps_secondary_windows_and_orders_every_window_by_focus() {
        let temp = tempfile::tempdir().unwrap();
        let oversized = WindowLayout {
            main_open: true,
            main_window: MainWindowState {
                route: Some("/chats/main".into()),
                focused_at: 30,
            },
            secondary_windows: (0..MAX_SECONDARY_WINDOWS + 2)
                .map(|index| secondary(index, index as i64 + 10))
                .collect(),
        };
        save_layout(temp.path(), &oversized).unwrap();
        let read = read_layout(temp.path());
        assert_eq!(read.layout.secondary_windows.len(), MAX_SECONDARY_WINDOWS);
        assert_eq!(read.layout.secondary_windows[0].focused_at, 12);
        assert_eq!(read.layout.secondary_windows.last().unwrap().focused_at, 19);
        assert_eq!(read.warnings.len(), 1);

        let ordered = restore_order(WindowLayout {
            main_open: true,
            main_window: MainWindowState {
                route: Some("/chats/main".into()),
                focused_at: 30,
            },
            secondary_windows: vec![secondary(1, 20), secondary(0, 10)],
        });
        assert_eq!(
            ordered
                .iter()
                .map(RestoredWindowState::focused_at)
                .collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
        assert!(matches!(ordered.last(), Some(RestoredWindowState::Main(_))));
    }

    #[test]
    fn migration_moves_legacy_size_and_retires_settings_keys() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().join("ui-settings.json");
        std::fs::write(
            &settings,
            r#"{"appZoom":1.2,"windowWidth":1200.0,"windowHeight":700.0}"#,
        )
        .unwrap();
        let state = load_and_migrate(temp.path(), &settings, rect(0., 0., 2560., 1440.))
            .unwrap()
            .unwrap();
        assert_eq!(
            state,
            WindowState {
                x: 680.,
                y: 370.,
                width: 1200.,
                height: 700.,
                maximized: false,
            }
        );
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        assert_eq!(settings, serde_json::json!({ "appZoom": 1.2 }));
    }

    #[test]
    fn existing_state_wins_while_legacy_keys_are_retired() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().join("ui-settings.json");
        std::fs::write(&settings, r#"{"windowWidth":1200.0,"windowHeight":700.0}"#).unwrap();
        let existing = WindowState {
            x: 10.,
            y: 20.,
            width: 900.,
            height: 600.,
            maximized: true,
        };
        save(temp.path(), existing).unwrap();
        assert_eq!(
            load_and_migrate(temp.path(), &settings, rect(0., 0., 1., 1.)).unwrap(),
            Some(existing)
        );
        assert_eq!(std::fs::read_to_string(settings).unwrap(), "{}");
    }

    #[test]
    fn overlap_accepts_negative_coordinates_and_partial_frames() {
        let displays = [rect(-1080., 0., 1080., 1920.), rect(0., 0., 2560., 1440.)];
        assert!(overlaps_any(&rect(-1080., 374., 1080., 1117.), &displays));
        assert!(overlaps_any(&rect(-200., 100., 1080., 800.), &displays));
    }

    #[test]
    fn offscreen_restore_keeps_size_but_uses_fallback_position() {
        let state = WindowState {
            x: 2600.,
            y: 100.,
            width: 1080.,
            height: 800.,
            maximized: false,
        };
        let fallback = Bounds::new(point(px(100.), px(50.)), size(px(1440.), px(900.)));
        let restored = restored_bounds(state, &[rect(0., 0., 2560., 1440.)], fallback);
        assert_eq!(
            restored,
            WindowBounds::Windowed(Bounds::new(
                point(px(100.), px(50.)),
                size(px(1080.), px(800.))
            ))
        );
    }

    #[test]
    fn invalid_small_and_oversized_frames_restore_to_usable_bounds() {
        let fallback = Bounds::new(point(px(100.), px(50.)), size(px(1440.), px(900.)));
        let display = [rect(0., 0., 1280., 720.)];
        let zero = restored_bounds(
            WindowState {
                x: 20.,
                y: 20.,
                width: 0.,
                height: -1.,
                maximized: false,
            },
            &display,
            fallback,
        );
        assert_eq!(zero.get_bounds().size, size(px(1280.), px(720.)));

        let small = restored_bounds(
            WindowState {
                x: 20.,
                y: 20.,
                width: 1.,
                height: 2.,
                maximized: false,
            },
            &display,
            fallback,
        );
        assert_eq!(small.get_bounds().size, size(px(640.), px(480.)));

        let huge = restored_bounds(
            WindowState {
                x: 20.,
                y: 20.,
                width: 5000.,
                height: 4000.,
                maximized: false,
            },
            &display,
            fallback,
        );
        assert_eq!(huge.get_bounds().size, size(px(1280.), px(720.)));
    }

    #[test]
    fn maximized_and_fullscreen_snapshots_keep_the_normal_frame() {
        let normal = WindowState {
            x: 100.,
            y: 50.,
            width: 1200.,
            height: 700.,
            maximized: false,
        };
        let screen = Bounds::new(point(px(0.), px(0.)), size(px(2560.), px(1440.)));
        assert_eq!(
            snapshot_from_bounds(
                WindowBounds::Windowed(screen),
                size(px(2560.), px(1400.)),
                true,
                Some(normal),
                None,
            ),
            WindowState {
                maximized: true,
                ..normal
            }
        );
        let restore = Bounds::new(
            point(px(normal.x as f32), px(normal.y as f32)),
            size(px(normal.width as f32), px(normal.height as f32)),
        );
        assert_eq!(
            snapshot_from_bounds(
                WindowBounds::Fullscreen(restore),
                size(px(2560.), px(1400.)),
                false,
                Some(normal),
                None,
            ),
            normal
        );
    }

    #[test]
    fn normal_snapshot_combines_outer_position_with_inner_size() {
        let outer = Bounds::new(point(px(-1080.), px(374.)), size(px(1100.), px(1160.)));
        assert_eq!(
            snapshot_from_bounds(
                WindowBounds::Windowed(outer),
                size(px(1080.), px(1117.)),
                false,
                None,
                Some(outer),
            ),
            WindowState {
                x: -1080.,
                y: 374.,
                width: 1080.,
                height: 1117.,
                maximized: false,
            }
        );
    }
}

use crate::*;

#[derive(Clone)]
pub(crate) struct LiveWindowState {
    pub(crate) label: String,
    pub(crate) frame: window_state::WindowState,
    pub(crate) route: Option<String>,
    pub(crate) focused_at: i64,
}

#[derive(Default)]
pub(crate) struct WindowLayoutCheckpoint {
    pub(crate) last_layout: Option<window_state::WindowLayout>,
    pub(crate) last_main_frame: Option<window_state::WindowState>,
}

impl Global for WindowLayoutCheckpoint {}

pub(crate) fn collect_window_layout(
    cx: &mut App,
) -> (window_state::WindowLayout, Vec<LiveWindowState>) {
    let focused_at = global_app_store(cx)
        .read(cx)
        .window_entries
        .iter()
        .map(|entry| (entry.label.clone(), entry.focused_at.timestamp_micros()))
        .collect::<HashMap<_, _>>();
    let mut windows = cx
        .windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<NativeRoot>())
        .filter_map(|handle| {
            handle
                .update(cx, |this, window, _| {
                    if this.closing {
                        return None;
                    }
                    this.note_window_state(window);
                    Some(LiveWindowState {
                        label: this.window_label.clone(),
                        frame: this.window_state,
                        route: this.persisted_window_route(),
                        focused_at: focused_at
                            .get(&this.window_label)
                            .copied()
                            .unwrap_or_default(),
                    })
                })
                .ok()
                .flatten()
        })
        .collect::<Vec<_>>();
    windows.sort_by(|left, right| {
        left.focused_at
            .cmp(&right.focused_at)
            .then_with(|| left.label.cmp(&right.label))
    });

    let mut layout = window_state::WindowLayout {
        main_open: false,
        main_window: window_state::MainWindowState::default(),
        secondary_windows: Vec::new(),
    };
    for window in &windows {
        if window.label == "main" {
            layout.main_open = true;
            layout.main_window = window_state::MainWindowState {
                route: window.route.clone(),
                focused_at: window.focused_at,
            };
        } else {
            layout
                .secondary_windows
                .push(window_state::SecondaryWindowState {
                    frame: window.frame,
                    route: window.route.clone(),
                    focused_at: window.focused_at,
                });
        }
    }
    (layout, windows)
}

pub(crate) fn format_window_layout_details(windows: &[LiveWindowState]) -> String {
    let details = windows
        .iter()
        .map(|window| {
            format!(
                "{} {} {:.0},{:.0},{:.0},{:.0} {}",
                window.label,
                window.route.as_deref().unwrap_or("none"),
                window.frame.x,
                window.frame.y,
                window.frame.width,
                window.frame.height,
                window.focused_at,
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("[{details}]")
}

pub(crate) fn checkpoint_window_layout(cx: &mut App) {
    save_window_layout_checkpoint(cx, false);
}

#[cfg(not(test))]
pub(crate) fn checkpoint_window_layout_on_quit(cx: &mut App) {
    save_window_layout_checkpoint(cx, true);
}

pub(crate) fn save_window_layout_checkpoint(cx: &mut App, force: bool) {
    let app_data_dir = global_app_store(cx).read(cx).app_data_dir.clone();
    let (layout, windows) = collect_window_layout(cx);
    let main_frame = windows
        .iter()
        .find(|window| window.label == "main")
        .map(|window| window.frame);
    let checkpoint = cx.global::<WindowLayoutCheckpoint>();
    let save_main = force || checkpoint.last_main_frame != main_frame;
    let save_layout = force || checkpoint.last_layout.as_ref() != Some(&layout);

    if save_main {
        if let Some(main_frame) = main_frame {
            match window_state::save(&app_data_dir, main_frame) {
                Ok(()) => {
                    cx.global_mut::<WindowLayoutCheckpoint>().last_main_frame = Some(main_frame);
                }
                Err(error) => eprintln!("Runner window-state save failed: {error:#}"),
            }
        }
    }
    if !save_layout {
        return;
    }
    match window_state::save_layout(&app_data_dir, &layout) {
        Ok(()) => {
            cx.global_mut::<WindowLayoutCheckpoint>().last_layout = Some(layout.clone());
            eprintln!(
                "Runner window-layout: saved main_open={} secondaries={} {}",
                layout.main_open,
                layout.secondary_windows.len(),
                format_window_layout_details(&windows),
            );
        }
        Err(error) => eprintln!("Runner window-layout save failed: {error:#}"),
    }
}

pub(crate) fn checkpoint_window_layout_deferred(cx: &mut Context<NativeRoot>) {
    cx.defer(checkpoint_window_layout);
}

#[cfg(not(test))]
pub(crate) fn save_window_settings(cx: &mut App) {
    for handle in cx.windows() {
        if let Some(window) = handle.downcast::<NativeRoot>() {
            let _ = window.update(cx, |this, _, cx| this.save_settings(cx));
        }
    }
}

use crate::surfaces::panes::{MIN_SPLIT_PANE_HEIGHT, MIN_SPLIT_PANE_WIDTH};
#[cfg(test)]
use crate::surfaces::{pane_close_behavior, PaneCloseBehavior};
use crate::*;

// Fixed sidebar content with the computer picker measured 364.5 px at 1×.
const MIN_WINDOW_HEIGHT: f32 = 400.;

fn opening_window_minimum(settings: &AppSettings) -> gpui::Size<gpui::Pixels> {
    size(
        px((settings.sidebar_width + 6. + MIN_SPLIT_PANE_WIDTH + 8.) * settings.app_zoom),
        px(MIN_WINDOW_HEIGHT.max(8. + 40. + MIN_SPLIT_PANE_HEIGHT + 8.) * settings.app_zoom),
    )
}

pub(crate) fn close_tab(this: &mut NativeRoot, window: &mut Window, cx: &mut Context<NativeRoot>) {
    if this.launcher_page_visible() {
        this.close_launcher_page(window, cx);
        return;
    }
    if this.remote_client.is_active() {
        this.close_remote_front_page(window, cx);
        return;
    }
    let drawer_focused = this.route == AppRoute::Chat
        && this.tabs.active().is_some_and(PaneLayout::drawer_open)
        && this.drawer_focus.contains_focused(window, cx);
    let mission_drawer_focused = matches!(this.route, AppRoute::Mission(_))
        && this.mission_workspace.read(cx).drawer_focused(window, cx);
    let leaves = (this.route == AppRoute::Chat)
        .then(|| this.tabs.active())
        .flatten()
        .map(|layout| layout.root.leaves().len())
        .unwrap_or_default();
    match close_target(&this.route, leaves, drawer_focused, mission_drawer_focused) {
        CloseTarget::ChatDrawer => this.hide_terminal_drawer(window, cx),
        CloseTarget::MissionDrawer => {
            this.mission_workspace
                .update(cx, |workspace, workspace_cx| {
                    workspace.hide_terminal_drawer(window, workspace_cx)
                });
        }
        CloseTarget::Pane => {
            let Some((pane_id, session_id)) = this.tabs.active().map(|layout| {
                let pane_id = layout.focused_pane_id.clone();
                let session_id = layout
                    .root
                    .leaves()
                    .into_iter()
                    .find(|leaf| leaf.id == pane_id)
                    .and_then(|leaf| leaf.session_id.clone());
                (pane_id, session_id)
            }) else {
                return;
            };
            let behavior = pane_close_behavior(
                session_id
                    .as_deref()
                    .and_then(|session_id| this.session_entry(session_id, cx))
                    .map(|entry| entry.agent_runtime.as_str()),
            );
            match (behavior, session_id) {
                (PaneCloseBehavior::CloseTerminal, Some(session_id)) => {
                    this.request_close_terminal_pane(&pane_id, &session_id, window, cx);
                }
                (PaneCloseBehavior::ArchiveChat, Some(session_id)) => {
                    this.request_close_chat_pane(&pane_id, &session_id, window, cx);
                }
                (PaneCloseBehavior::LayoutOnly, _) => this.close_pane(&pane_id, window, cx),
                (PaneCloseBehavior::CloseTerminal | PaneCloseBehavior::ArchiveChat, None) => {}
            }
        }
        CloseTarget::Tab => this.request_close_single_pane_tab(window, cx),
        CloseTarget::None => {}
    }
}

#[cfg(not(test))]
pub(crate) fn close_window(
    this: &mut NativeRoot,
    window: &mut Window,
    cx: &mut Context<NativeRoot>,
) {
    #[cfg(any(windows, target_os = "linux"))]
    if cx.windows().len() == 1 {
        this.request_quit(window, cx);
        return;
    }
    this.prepare_window_close(window, cx);
    window.remove_window();
}

pub(crate) fn handle_reopen(cx: &mut App) {
    if cx.try_global::<GlobalAppStore>().is_none() {
        // Startup is still running and will open and activate the windows itself.
        return;
    }
    if !window_label_is_open(cx, "main") {
        if let Err(error) = open_runner_window("main".into(), None, None, cx) {
            eprintln!("Runner main-window reopen failed: {error:#}");
        }
    }
    cx.activate(true);
}

pub(crate) fn open_new_runner_window(
    initial_route: Option<String>,
    cx: &mut App,
) -> Result<String> {
    let label = if !window_label_is_open(cx, "main") {
        "main".into()
    } else {
        runner_core::protocol::window::allocate_label()
    };
    open_runner_window(label, initial_route, None, cx)
}

pub(crate) fn open_runner_window(
    label: String,
    initial_route: Option<String>,
    restored_window_state: Option<window_state::WindowState>,
    cx: &mut App,
) -> Result<String> {
    let app_store = global_app_store(cx);
    let log_dir = cx.global::<GlobalNativePaths>().0.log_dir.clone();
    let core = app_store.read(cx).client.clone();
    let default_size = size(px(1440.), px(900.));
    #[cfg(not(target_os = "linux"))]
    let fallback = Bounds::centered(None, default_size, cx);
    #[cfg(target_os = "linux")]
    let fallback = platform_ui::initial_window_bounds(default_size, None, cx);
    let (bounds, initial_window_state) = if label == "main" {
        let displays = window_state::display_rects(cx);
        let default_rect = displays.first().copied().unwrap_or(window_state::Rect {
            x: f32::from(fallback.origin.x) as f64,
            y: f32::from(fallback.origin.y) as f64,
            width: f32::from(fallback.size.width) as f64,
            height: f32::from(fallback.size.height) as f64,
        });
        let restored = match window_state::load_and_migrate(
            &global_app_store(cx).read(cx).app_data_dir,
            &settings_path(&global_app_store(cx).read(cx).app_data_dir),
            default_rect,
        ) {
            Ok(state) => state,
            Err(error) => {
                eprintln!("Runner window-state restore failed: {error:#}");
                None
            }
        };
        (
            restored
                .map(|state| window_state::restored_bounds(state, &displays, fallback))
                .unwrap_or(WindowBounds::Windowed(fallback)),
            restored,
        )
    } else if let Some(state) = restored_window_state {
        let displays = window_state::display_rects(cx);
        (
            window_state::restored_bounds(state, &displays, fallback),
            Some(state),
        )
    } else {
        let origin = runner_core::protocol::window::cascade_reference(
            &core.window_snapshot().unwrap_or_default(),
            &label,
        )
        .and_then(|reference| window_origin_for_label(cx, &reference))
        .map(|origin| origin + point(px(32.), px(32.)))
        .unwrap_or(fallback.origin);
        #[cfg(target_os = "linux")]
        let bounds = platform_ui::initial_window_bounds(default_size, Some(origin), cx);
        #[cfg(not(target_os = "linux"))]
        let bounds = Bounds::new(origin, default_size);
        (WindowBounds::Windowed(bounds), None)
    };
    core.window_register(&label)?;
    let open_label = label.clone();
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            #[cfg(target_os = "macos")]
            app_owns_titlebar_drag: true,
            window_min_size: Some(opening_window_minimum(&app_store.read(cx).settings)),
            #[cfg(target_os = "linux")]
            app_id: Some(platform_ui::DESKTOP_APP_ID.into()),
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            window_background: runner_app::appearance::effective(
                app_store.read(cx).settings.window_material,
                cx,
            )
            .background(),
            #[cfg(target_os = "macos")]
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: true,
                // The zoom-aware AppKit frame path owns this position. Giving GPUI a second
                // position makes its resize callback race the frame update and visibly jump.
                traffic_light_position: None,
            }),
            #[cfg(any(windows, target_os = "linux"))]
            titlebar: Some(TitlebarOptions {
                title: Some("Runner".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        },
        |window, cx| {
            let root = cx.new(|cx| {
                NativeRoot::new(
                    open_label.clone(),
                    log_dir.clone(),
                    initial_route.clone(),
                    initial_window_state,
                    app_store.clone(),
                    window,
                    cx,
                )
            });
            let weak = root.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                #[cfg(any(windows, target_os = "linux"))]
                if cx.windows().len() == 1 {
                    let _ = weak.update(cx, |this, cx| this.request_quit(window, cx));
                    return false;
                }
                let _ = weak.update(cx, |this, cx| {
                    this.prepare_window_close(window, cx);
                });
                platform_ui::finish_window_close(window)
            });
            root
        },
    );
    match result {
        Ok(_) => {
            checkpoint_window_layout(cx);
            Ok(label)
        }
        Err(error) => {
            let _ = core.unregister(&label);
            Err(error)
        }
    }
}

pub(crate) fn window_label_is_open(cx: &App, label: &str) -> bool {
    cx.windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<NativeRoot>())
        .any(|handle| handle.read(cx).is_ok_and(|root| root.window_label == label))
}

pub(crate) fn window_origin_for_label(
    cx: &mut App,
    label: &str,
) -> Option<gpui::Point<gpui::Pixels>> {
    cx.windows().into_iter().find_map(|handle| {
        let handle = handle.downcast::<NativeRoot>()?;
        if !handle.read(cx).is_ok_and(|root| root.window_label == label) {
            return None;
        }
        handle
            .update(cx, |_, window, _| window_state::outer_origin(window))
            .ok()
    })
}

pub(crate) fn focus_other_window(label: &str, cx: &mut App) {
    let target = cx.windows().into_iter().find_map(|handle| {
        let typed = handle.downcast::<NativeRoot>()?;
        typed
            .read(cx)
            .is_ok_and(|root| root.window_label == label)
            .then_some(typed)
    });
    if let Some(target) = target {
        let _ = target.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
    }
}

#[cfg(test)]
mod minimum_tests {
    use super::*;

    #[test]
    fn opening_floor_uses_saved_sidebar_width_and_zoom() {
        let mut settings = AppSettings {
            sidebar_width: 240.,
            app_zoom: 1.,
            ..Default::default()
        };
        assert_eq!(opening_window_minimum(&settings), size(px(494.), px(400.)));
        settings.sidebar_width = 320.;
        settings.app_zoom = 1.5;
        assert_eq!(opening_window_minimum(&settings), size(px(861.), px(600.)));
    }
}

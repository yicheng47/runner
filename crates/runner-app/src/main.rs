mod app_settings;
mod app_shell;
mod assets;
mod mac_chrome;
mod terminal_element;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, point, prelude::*, px, relative, rems, size, AnyElement, App, Application,
    Bounds, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, KeyBinding, KeyDownEvent,
    Menu, MenuItem, MouseButton, OsAction, ScrollDelta, ScrollHandle, ScrollWheelEvent,
    SharedString, Subscription, SystemMenuType, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use runner_app::bootstrap::{boot_core, native_paths, stop_running_sessions_on_quit, NativePaths};
use runner_app::pane_layout::{
    PaneLayout, PaneLeaf, PaneNode, PresetKind, SplitOrientation, TabSet,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::{
    Button, ButtonSize, IconButton, IconButtonSize, Scrollbar, SessionControl, SessionControlKind,
};
use runner_app::{theme, Copy, Cut, Paste, SelectAll};
use runner_backend::model::{Runner, SessionStatus};
use runner_backend::ops::session::DirectSessionEntry;
use runner_backend::AppCore;
use runner_terminal::terminal::{TerminalBridge, TerminalSession};

use app_settings::{settings_path, AppSettings};
use app_shell::AppRoute;
use assets::Assets;
use terminal_element::TerminalElement;
use toast::ToastHost;

actions!(
    runner_app_ui,
    [
        CloseWindow,
        Hide,
        HideOthers,
        Maximize,
        Minimize,
        NewTab,
        OpenSettings,
        Quit,
        ShowAll,
        ToggleFullscreen,
        ToggleSidebar,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);

mod chat;
mod panes;
mod sidebar;
mod start_chat;
mod toast;

use panes::pane_fractions;
use sidebar::session_label;
use start_chat::StartChatModal;

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;
const WORKSPACE_HEADER_HEIGHT: f32 = 44.;
const PANE_HEADER_HEIGHT: f32 = 34.;

struct AttachedChat {
    terminal: Arc<TerminalSession>,
    terminal_scrollbar: Entity<Scrollbar>,
    terminal_input: Entity<TerminalInput>,
    _terminal_input_subscription: Subscription,
    _terminal_focus_subscription: Subscription,
    terminal_focus: FocusHandle,
    scroll_accumulator: f32,
}

#[derive(Clone)]
struct SplitResizeDrag {
    split_id: String,
    orientation: SplitOrientation,
}

impl Render for SplitResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

struct NativeRoot {
    core: AppCore,
    bridge: Arc<TerminalBridge>,
    sessions: Vec<DirectSessionEntry>,
    runners: Vec<Runner>,
    tabs: TabSet,
    attached: HashMap<String, AttachedChat>,
    root_focus: FocusHandle,
    sidebar_scroll: ScrollHandle,
    sidebar_scrollbar: Entity<Scrollbar>,
    waker: Arc<dyn Fn() + Send + Sync>,
    start_chat_modal: Option<StartChatModal>,
    last_focused_runner_id: Option<String>,
    layout_picker_open: bool,
    split_sizes_dirty: bool,
    error: Option<String>,
    settings: AppSettings,
    settings_path: PathBuf,
    route: AppRoute,
    sidebar_preview_open: bool,
    titlebar_drag_armed: bool,
    toasts: ToastHost,
    _appearance_subscription: Option<Subscription>,
    _bounds_subscription: Option<Subscription>,
}

impl NativeRoot {
    fn new(
        core: AppCore,
        settings_path: PathBuf,
        settings: AppSettings,
        settings_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let bridge =
            TerminalBridge::new(core.clone(), Arc::clone(&waker)).expect("start event bridge");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let (runtime_event_tx, mut runtime_event_rx) = futures::channel::mpsc::unbounded::<()>();
        let mut app_events = core.events.subscribe();
        cx.background_spawn(async move {
            loop {
                match app_events.recv().await {
                    Ok(event) if event.name == "runtime/changed" => {
                        if runtime_event_tx.unbounded_send(()).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn(async move |weak, cx| {
            while runtime_event_rx.next().await.is_some() {
                while runtime_event_rx.try_recv().is_ok() {}
                if weak
                    .update(cx, |this, cx| this.refresh_start_chat_runtimes(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let mut errors: Vec<_> = settings_error.into_iter().collect();
        window.set_rem_size(px(16. * settings.app_zoom));
        let sessions = match runner_backend::ops::session::session_list_recent_direct(&core) {
            Ok(sessions) => sessions,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let runners = match runner_backend::ops::runner::runner_list(&core) {
            Ok(runners) => runners,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let tabs = match runner_backend::ops::node::node_list(&core)
            .map_err(anyhow::Error::from)
            .and_then(|rows| TabSet::from_rows(&rows))
        {
            Ok(tabs) => tabs,
            Err(error) => {
                errors.push(error.to_string());
                TabSet::default()
            }
        };

        let root_focus = cx.focus_handle();
        let sidebar_scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let sidebar_scrollbar = cx.new(|_| Scrollbar::app(sidebar_scroll.clone(), scroll_owner));
        let last_focused_runner_id = tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .and_then(|session_id| sessions.iter().find(|entry| entry.session_id == session_id))
            .and_then(|entry| entry.runner_id.clone());
        let mut root = Self {
            core,
            bridge,
            sessions,
            runners,
            tabs,
            attached: HashMap::new(),
            root_focus,
            sidebar_scroll,
            sidebar_scrollbar,
            waker,
            start_chat_modal: None,
            last_focused_runner_id,
            layout_picker_open: false,
            split_sizes_dirty: false,
            error: (!errors.is_empty()).then(|| errors.join("\n")),
            settings,
            settings_path,
            route: AppRoute::Chat,
            sidebar_preview_open: false,
            titlebar_drag_armed: false,
            toasts: ToastHost::default(),
            _appearance_subscription: None,
            _bounds_subscription: None,
        };
        root._appearance_subscription = Some(cx.observe_window_appearance(window, |_, _, cx| {
            cx.notify();
        }));
        root._bounds_subscription = Some(cx.observe_window_bounds(window, |this, window, cx| {
            mac_chrome::sync_traffic_lights(window, this.settings.app_zoom);
            cx.notify();
        }));
        mac_chrome::sync_traffic_lights(window, root.settings.app_zoom);
        if let Err(error) = root.ensure_active_tab_attached(window, cx) {
            root.error = Some(error.to_string());
        }
        root.focus_active_terminal(window);
        root
    }

    fn refresh_sessions(&mut self) {
        match runner_backend::ops::session::session_list_recent_direct(&self.core) {
            Ok(sessions) => self.sessions = sessions,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reload_tabs(&mut self) -> Result<()> {
        let rows = runner_backend::ops::node::node_list(&self.core)?;
        self.tabs.replace_rows(&rows)
    }

    fn session_entry(&self, session_id: &str) -> Option<&DirectSessionEntry> {
        self.sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
    }
}

impl Render for NativeRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_app_shell(window, cx)
    }
}

fn run() -> Result<()> {
    let paths = native_paths()?;
    let core = boot_core(&paths)?;
    print_startup_paths(&paths);
    let shutdown_core = core.clone();
    let ui_settings_path = settings_path(&paths.app_data_dir);

    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let quit_core = core.clone();
            cx.on_action(move |_: &Quit, cx| {
                if let Err(error) = stop_running_sessions_on_quit(&quit_core) {
                    eprintln!("Runner quit session teardown failed: {error:#}");
                }
                cx.quit();
            });
            cx.on_action(|_: &Hide, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
            cx.on_action(|_: &Minimize, cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.minimize_window());
                }
            });
            cx.on_action(|_: &Maximize, cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.zoom_window());
                }
            });
            cx.on_action(|_: &CloseWindow, cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.remove_window());
                }
            });
            let close_core = core.clone();
            cx.on_window_closed(move |cx| {
                if cx.windows().is_empty() {
                    if let Err(error) = stop_running_sessions_on_quit(&close_core) {
                        eprintln!("Runner quit session teardown failed: {error:#}");
                    }
                    cx.quit();
                }
            })
            .detach();
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-h", Hide, None),
                KeyBinding::new("cmd-alt-h", HideOthers, None),
                KeyBinding::new("cmd-m", Minimize, None),
                KeyBinding::new("cmd-w", CloseWindow, None),
                KeyBinding::new("cmd-t", NewTab, None),
                KeyBinding::new("cmd-s", ToggleSidebar, None),
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("cmd-=", ZoomIn, None),
                KeyBinding::new("cmd-shift-=", ZoomIn, None),
                KeyBinding::new("cmd--", ZoomOut, None),
                KeyBinding::new("cmd-0", ZoomReset, None),
                KeyBinding::new("ctrl-cmd-f", ToggleFullscreen, None),
                KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: "Runner".into(),
                    items: vec![
                        MenuItem::os_submenu("Services", SystemMenuType::Services),
                        MenuItem::separator(),
                        MenuItem::action("Hide Runner", Hide),
                        MenuItem::action("Hide Others", HideOthers),
                        MenuItem::action("Show All", ShowAll),
                        MenuItem::separator(),
                        MenuItem::action("Quit Runner", Quit),
                    ],
                },
                Menu {
                    name: "Edit".into(),
                    items: vec![
                        MenuItem::os_action("Cut", Cut, OsAction::Cut),
                        MenuItem::os_action("Copy", Copy, OsAction::Copy),
                        MenuItem::os_action("Paste", Paste, OsAction::Paste),
                        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                    ],
                },
                Menu {
                    name: "View".into(),
                    items: vec![MenuItem::action("Enter Full Screen", ToggleFullscreen)],
                },
                Menu {
                    name: "Window".into(),
                    items: vec![
                        MenuItem::action("Minimize", Minimize),
                        MenuItem::action("Maximize", Maximize),
                        MenuItem::separator(),
                        MenuItem::action("Close Window", CloseWindow),
                    ],
                },
            ]);

            open_runner_window(core.clone(), ui_settings_path.clone(), cx)
                .expect("open Runner window");
            cx.activate(true);
        });

    stop_running_sessions_on_quit(&shutdown_core)?;
    Ok(())
}

fn open_runner_window(core: AppCore, settings_path: PathBuf, cx: &mut App) -> Result<()> {
    let (settings, settings_error) = match AppSettings::load(&settings_path) {
        Ok(settings) => (settings, None),
        Err(error) => (AppSettings::default(), Some(error.to_string())),
    };
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: true,
                // The zoom-aware AppKit frame path owns this position. Giving GPUI a second
                // position makes its resize callback race the frame update and visibly jump.
                traffic_light_position: None,
            }),
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                NativeRoot::new(
                    core.clone(),
                    settings_path.clone(),
                    settings.clone(),
                    settings_error.clone(),
                    window,
                    cx,
                )
            })
        },
    )?;
    Ok(())
}

fn print_startup_paths(paths: &NativePaths) {
    eprintln!(
        "Runner: database={} logs={}",
        paths.app_data_dir.join("runner.db").display(),
        paths.log_dir.display()
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Runner failed: {error:#}");
        std::process::exit(1);
    }
}

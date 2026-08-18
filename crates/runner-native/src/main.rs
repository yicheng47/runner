mod terminal_element;
mod theme;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, prelude::*, px, relative, size, AnyElement, App, Application, Bounds, Context,
    CursorStyle, DragMoveEvent, Entity, FocusHandle, KeyBinding, KeyDownEvent, Menu, MenuItem,
    MouseButton, ScrollDelta, ScrollWheelEvent, SharedString, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowOptions,
};
use runner_app::model::{Runner, SessionStatus};
use runner_app::ops::session::DirectSessionEntry;
use runner_app::AppCore;
use runner_native::bootstrap::{
    boot_core, native_paths, stop_running_sessions_on_quit, NativePaths,
};
use runner_native::pane_layout::{
    PaneLayout, PaneLeaf, PaneNode, PresetKind, SplitOrientation, TabSet,
};
use runner_native::terminal_ime::TerminalInput;
use runner_terminal::terminal::{TerminalBridge, TerminalSession};

use terminal_element::TerminalElement;

actions!(runner_native_ui, [Quit, TermPaste, NewTab]);

mod chat;
mod panes;
mod sidebar;

use panes::pane_fractions;
use sidebar::session_label;

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;
const SIDEBAR_WIDTH: f32 = 248.;
const WORKSPACE_HEADER_HEIGHT: f32 = 42.;
const PANE_HEADER_HEIGHT: f32 = 34.;

struct AttachedChat {
    terminal: Arc<TerminalSession>,
    terminal_input: Entity<TerminalInput>,
    _terminal_input_subscription: Subscription,
    _terminal_focus_subscription: Subscription,
    terminal_focus: FocusHandle,
    scroll_accumulator: f32,
}

#[derive(Clone)]
enum NewChatTarget {
    NewTab,
    Pane { tab_id: String, pane_id: String },
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
    waker: Arc<dyn Fn() + Send + Sync>,
    new_chat_target: Option<NewChatTarget>,
    layout_picker_open: bool,
    split_sizes_dirty: bool,
    error: Option<String>,
}

impl NativeRoot {
    fn new(core: AppCore, window: &mut Window, cx: &mut Context<Self>) -> Self {
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

        let mut errors = Vec::new();
        let sessions = match runner_app::ops::session::session_list_recent_direct(&core) {
            Ok(sessions) => sessions,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let runners = match runner_app::ops::runner::runner_list(&core) {
            Ok(runners) => runners,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let tabs = match runner_app::ops::node::node_list(&core)
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
        let mut root = Self {
            core,
            bridge,
            sessions,
            runners,
            tabs,
            attached: HashMap::new(),
            root_focus,
            waker,
            new_chat_target: None,
            layout_picker_open: false,
            split_sizes_dirty: false,
            error: (!errors.is_empty()).then(|| errors.join("\n")),
        };
        if let Err(error) = root.ensure_active_tab_attached(window, cx) {
            root.error = Some(error.to_string());
        }
        root.focus_active_terminal(window);
        root
    }

    fn refresh_sessions(&mut self) {
        match runner_app::ops::session::session_list_recent_direct(&self.core) {
            Ok(sessions) => self.sessions = sessions,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reload_tabs(&mut self) -> Result<()> {
        let rows = runner_app::ops::node::node_list(&self.core)?;
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
        if self.bridge.take_session_refresh() {
            self.refresh_sessions();
        }

        let sidebar = self.render_sidebar(cx);
        let workspace = self.render_active_tab(window, cx);
        div()
            .size_full()
            .flex()
            .track_focus(&self.root_focus)
            .bg(theme::bg())
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(workspace)
                    .children(self.error.as_ref().map(|error| {
                        div()
                            .flex_none()
                            .px_3()
                            .py_2()
                            .bg(gpui::rgb(0x3b1d2b))
                            .text_sm()
                            .text_color(gpui::rgb(0xf7768e))
                            .child(SharedString::from(error.clone()))
                    })),
            )
            .on_action(cx.listener(Self::begin_new_tab))
    }
}

fn run() -> Result<()> {
    let paths = native_paths()?;
    let core = boot_core(&paths)?;
    print_startup_paths(&paths);
    let shutdown_core = core.clone();

    Application::new().run(move |cx: &mut App| {
        let quit_core = core.clone();
        cx.on_action(move |_: &Quit, cx| {
            if let Err(error) = stop_running_sessions_on_quit(&quit_core) {
                eprintln!("Runner Native quit session teardown failed: {error:#}");
            }
            cx.quit();
        });
        let close_core = core.clone();
        cx.on_window_closed(move |cx| {
            if cx.windows().is_empty() {
                if let Err(error) = stop_running_sessions_on_quit(&close_core) {
                    eprintln!("Runner Native quit session teardown failed: {error:#}");
                }
                cx.quit();
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-v", TermPaste, Some("Terminal")),
        ]);
        cx.set_menus(vec![Menu {
            name: "Runner Native".into(),
            items: vec![
                MenuItem::action("New Chat", NewTab),
                MenuItem::action("Quit", Quit),
            ],
        }]);

        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Runner Native".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| NativeRoot::new(core.clone(), window, cx)),
        )
        .expect("open Runner Native window");
        cx.activate(true);
    });

    stop_running_sessions_on_quit(&shutdown_core)?;
    Ok(())
}

fn print_startup_paths(paths: &NativePaths) {
    eprintln!(
        "Runner Native: database={} logs={}",
        paths.app_data_dir.join("runner.db").display(),
        paths.log_dir.display()
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Runner Native failed: {error:#}");
        std::process::exit(1);
    }
}

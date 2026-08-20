mod app_settings;
mod app_shell;
mod assets;
mod chat_lifecycle;
mod list_controls;
mod mac_chrome;
mod mission_composer;
mod mission_feed;
mod mission_markdown;
mod settings_page;
mod sidebar_logic;
mod terminal_element;
mod terminal_glyphs;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, point, prelude::*, px, relative, rems, size, AnyElement, App, Application,
    Bounds, ClipboardEntry, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, KeyBinding,
    KeyDownEvent, Menu, MenuItem, MouseButton, OsAction, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Subscription, SystemMenuType, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use runner_app::bootstrap::{boot_core, native_paths, stop_running_sessions_on_quit, NativePaths};
use runner_app::pane_layout::{
    PaneLayout, PaneLeaf, PaneNode, PresetKind, SplitOrientation, TabSet,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::{
    Button, ButtonSize, ContextMenu, CopyValueButton, IconButton, IconButtonSize,
    MenuItem as UiMenuItem, PopoverMenu, Scrollbar, SessionControl, SessionControlKind,
};
use runner_app::{theme, Copy, Cut, Paste, SelectAll};
use runner_backend::model::{Runner, SessionStatus};
use runner_backend::ops::mission::MissionSummary;
use runner_backend::ops::session::DirectSessionEntry;
use runner_backend::repo::node::NodeRow;
use runner_backend::repo::project::ProjectRow;
use runner_backend::session::manager::SessionActivityState;
use runner_backend::AppCore;
use runner_terminal::terminal::{TerminalBridge, TerminalSession};

use app_settings::{settings_path, AppSettings};
use app_shell::AppRoute;
use assets::{
    Assets, INTER_FONT, MESLO_FONT_BOLD, MESLO_FONT_BOLD_ITALIC, MESLO_FONT_ITALIC,
    MESLO_FONT_REGULAR,
};
use terminal_element::TerminalElement;
use toast::ToastHost;

actions!(
    runner_app_ui,
    [
        CloseWindow,
        ClosePane,
        FocusNextPane,
        FocusPreviousPane,
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
mod crews;
mod mission_workspace;
mod panes;
mod runners;
mod sidebar;
mod start_chat;
mod start_mission;
mod toast;

use crews::CrewSurfaces;
use mission_workspace::MissionWorkspaceState;
use panes::{adjacent_pane_index, pane_fractions};
use runners::RunnerSurfaces;
use settings_page::SettingsState;
use sidebar::{session_label, ProjectModal, SidebarRefreshKind, SidebarRename};
use sidebar_logic::DropTarget;
use start_chat::StartChatModal;
use start_mission::StartMissionModalState;

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

struct SidebarVisibilityTransition {
    start: f32,
    target: f32,
    started_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntityRefreshKind {
    None,
    Runners,
    All,
}

impl EntityRefreshKind {
    fn for_event(event: &runner_backend::events::AppEvent) -> Self {
        match event.name {
            "runner/activity" => Self::Runners,
            "runner/changed" | "crew/changed" | "slot/changed" => Self::All,
            _ => Self::None,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self == Self::None || self == other {
            return other;
        }
        if other == Self::None {
            return self;
        }
        Self::All
    }

    fn runners(self) -> bool {
        matches!(self, Self::Runners | Self::All)
    }

    fn crews(self) -> bool {
        self == Self::All
    }
}

#[derive(Clone, Copy)]
struct ChatTransition {
    kind: chat_lifecycle::TransitionKind,
    started_at: Instant,
    baseline_seq: u64,
    generation: u64,
}

#[derive(Clone)]
enum ChatMenuAction {
    TogglePin { session_id: String, pinned: bool },
    RenameSession { session_id: String, current: String },
    RenameTab { tab_id: String, current: String },
    Archive(Vec<String>),
}

#[derive(Clone)]
enum ChatRenameTarget {
    Session {
        session_id: String,
        original: String,
    },
    Tab {
        tab_id: String,
        original: String,
    },
}

struct ChatRenameModal {
    target: ChatRenameTarget,
    input: Entity<runner_app::ui::TextField>,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    submitting: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct ChatPanelResizeDrag;

impl Render for ChatPanelResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

impl SidebarVisibilityTransition {
    fn new(visible: bool) -> Self {
        let value = if visible { 1. } else { 0. };
        Self {
            start: value,
            target: value,
            started_at: None,
        }
    }

    fn animate_to(&mut self, target: f32, now: Instant, duration: Duration) -> (f32, bool) {
        let current = self.value_at(now, duration);
        if self.target != target {
            self.start = current;
            self.target = target;
            self.started_at = Some(now);
        }
        let Some(started_at) = self.started_at else {
            return (self.target, false);
        };
        if now.saturating_duration_since(started_at) >= duration {
            self.start = self.target;
            self.started_at = None;
            return (self.target, false);
        }
        (self.value_at(now, duration), true)
    }

    fn value_at(&self, now: Instant, duration: Duration) -> f32 {
        let Some(started_at) = self.started_at else {
            return self.target;
        };
        let delta = (now.saturating_duration_since(started_at).as_secs_f32()
            / duration.as_secs_f32())
        .clamp(0., 1.);
        self.start + (self.target - self.start) * gpui::ease_in_out(delta)
    }
}

struct NativeRoot {
    core: AppCore,
    bridge: Arc<TerminalBridge>,
    sessions: Vec<DirectSessionEntry>,
    runners: Vec<Runner>,
    nodes: Vec<NodeRow>,
    projects: Vec<ProjectRow>,
    missions: Vec<MissionSummary>,
    session_activity: BTreeMap<String, SessionActivityState>,
    tabs: TabSet,
    attached: HashMap<String, AttachedChat>,
    root_focus: FocusHandle,
    layout_picker_focus: FocusHandle,
    sidebar_scroll: ScrollHandle,
    sidebar_scrollbar: Entity<Scrollbar>,
    waker: Arc<dyn Fn() + Send + Sync>,
    start_chat_modal: Option<StartChatModal>,
    start_mission_modal: Option<StartMissionModalState>,
    sidebar_create_menu: Entity<PopoverMenu>,
    sidebar_context_menu: Option<Entity<ContextMenu>>,
    sidebar_rename: Option<SidebarRename>,
    project_modal: Option<ProjectModal>,
    project_delete_confirm: Option<String>,
    project_delete_busy: bool,
    sidebar_archiving_sessions: HashSet<String>,
    sidebar_archiving_missions: HashSet<String>,
    stopping_sessions: HashSet<String>,
    chat_transitions: HashMap<String, ChatTransition>,
    next_chat_transition_generation: u64,
    session_exit_codes: HashMap<String, Option<i32>>,
    chat_error: Option<String>,
    chat_warning: Option<String>,
    active_chat_detail: Option<DirectSessionEntry>,
    session_key_copy: Entity<CopyValueButton>,
    chat_action_menu: Entity<PopoverMenu>,
    chat_menu_actions: Vec<ChatMenuAction>,
    pane_action_menus: HashMap<String, Entity<PopoverMenu>>,
    chat_rename_modal: Option<ChatRenameModal>,
    active_project_id: Option<String>,
    sidebar_dragged_id: Option<String>,
    sidebar_drop_target: Option<DropTarget>,
    sidebar_drop_marker: Option<String>,
    last_focused_runner_id: Option<String>,
    layout_picker_open: bool,
    split_sizes_dirty: bool,
    error: Option<String>,
    settings: AppSettings,
    settings_path: PathBuf,
    settings_page: SettingsState,
    route: AppRoute,
    settings_return_route: AppRoute,
    sidebar_visibility: SidebarVisibilityTransition,
    chat_panel_visibility: SidebarVisibilityTransition,
    sidebar_preview_open: bool,
    sidebar_preview_peeking: bool,
    titlebar_drag_armed: bool,
    window_size_save_generation: u64,
    toasts: ToastHost,
    runner_surfaces: RunnerSurfaces,
    crew_surfaces: CrewSurfaces,
    mission_workspace: MissionWorkspaceState,
    _appearance_subscription: Option<Subscription>,
    _activation_subscription: Option<Subscription>,
    _bounds_subscription: Option<Subscription>,
    _sidebar_rename_focus_subscription: Option<Subscription>,
    _project_cwd_subscription: Option<Subscription>,
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
                if weak
                    .update(cx, |this, cx| {
                        if this.route.terminal_visible() {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (chat_event_tx, mut chat_event_rx) =
            futures::channel::mpsc::unbounded::<runner_backend::events::AppEvent>();
        let mut chat_events = core.events.subscribe();
        cx.background_spawn(async move {
            loop {
                match chat_events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name,
                            "session/exit" | "session/updated" | "session/warning"
                        ) =>
                    {
                        if chat_event_tx.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while let Some(event) = chat_event_rx.next().await {
                if weak
                    .update_in(cx, |this, window, cx| {
                        this.handle_chat_lifecycle_event(event, window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (mission_event_tx, mut mission_event_rx) =
            futures::channel::mpsc::unbounded::<runner_backend::events::AppEvent>();
        let mut mission_events = core.events.subscribe();
        cx.background_spawn(async move {
            loop {
                match mission_events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name,
                            "event/appended"
                                | "mission/changed"
                                | "router/delivery-blocked"
                                | "session/exit"
                                | "session/updated"
                                | "session/warning"
                                | "session/input-error"
                                | "window_focus_map"
                        ) =>
                    {
                        if mission_event_tx.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if mission_event_tx
                            .unbounded_send(runner_backend::events::AppEvent {
                                name: "mission/resync",
                                payload: serde_json::Value::Null,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |weak, cx| {
            while let Some(event) = mission_event_rx.next().await {
                if weak
                    .update_in(cx, |this, window, cx| {
                        this.handle_mission_workspace_event(event, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (sidebar_event_tx, mut sidebar_event_rx) =
            futures::channel::mpsc::unbounded::<(SidebarRefreshKind, EntityRefreshKind)>();
        let mut sidebar_events = core.events.subscribe();
        cx.background_spawn(async move {
            loop {
                let refresh = match sidebar_events.recv().await {
                    Ok(event) => SidebarRefreshKind::for_event(&event)
                        .map(|sidebar| (sidebar, EntityRefreshKind::for_event(&event))),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        Some((SidebarRefreshKind::All, EntityRefreshKind::All))
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if refresh.is_some_and(|refresh| sidebar_event_tx.unbounded_send(refresh).is_err())
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |weak, cx| {
            while let Some((mut sidebar_refresh, mut entity_refresh)) =
                sidebar_event_rx.next().await
            {
                while let Ok(next) = sidebar_event_rx.try_recv() {
                    sidebar_refresh = sidebar_refresh.merge(next.0);
                    entity_refresh = entity_refresh.merge(next.1);
                }
                if weak
                    .update(cx, |this, cx| {
                        this.refresh_sidebar(sidebar_refresh, cx);
                        match this.route.clone() {
                            AppRoute::Runners if entity_refresh.runners() => {
                                this.load_runner_page(cx)
                            }
                            AppRoute::RunnerDetail(handle) if entity_refresh.runners() => {
                                this.load_runner_detail(handle, cx)
                            }
                            AppRoute::Crews if entity_refresh.crews() => this.load_crew_page(cx),
                            AppRoute::CrewEditor(crew_id) if entity_refresh.crews() => {
                                this.load_crew_editor(crew_id, cx)
                            }
                            AppRoute::Chat
                            | AppRoute::Runners
                            | AppRoute::RunnerDetail(_)
                            | AppRoute::Crews
                            | AppRoute::CrewEditor(_)
                            | AppRoute::Mission(_)
                            | AppRoute::Settings => {}
                        }
                    })
                    .is_err()
                {
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
                    .update(cx, |this, cx| {
                        this.refresh_start_chat_runtimes(cx);
                        this.refresh_runner_form_runtimes(cx);
                    })
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
        let nodes = match runner_backend::ops::node::node_list(&core) {
            Ok(nodes) => nodes,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let tabs = match TabSet::from_rows(&nodes) {
            Ok(tabs) => tabs,
            Err(error) => {
                errors.push(error.to_string());
                TabSet::default()
            }
        };
        let projects = match runner_backend::ops::project::project_list(&core) {
            Ok(projects) => projects,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let missions = match futures::executor::block_on(
            runner_backend::ops::mission::mission_list_summary_impl(&core, None),
        ) {
            Ok(missions) => missions,
            Err(error) => {
                errors.push(error.to_string());
                Vec::new()
            }
        };
        let session_activity = runner_backend::ops::session::session_activity_snapshot(&core);

        let root_focus = cx.focus_handle();
        let layout_picker_focus = cx.focus_handle();
        let sidebar_scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let sidebar_scrollbar = cx.new(|_| Scrollbar::app(sidebar_scroll.clone(), scroll_owner));
        let active_chat_detail = tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .and_then(|session_id| {
                runner_backend::ops::session::session_get(&core, session_id)
                    .ok()
                    .flatten()
            });
        let initial_session_key = active_chat_detail
            .as_ref()
            .and_then(|entry| entry.agent_session_key.clone());
        let session_key_copy = cx.new(|copy_cx| {
            CopyValueButton::new(
                copy_cx.focus_handle(),
                initial_session_key,
                "Copy session_key",
            )
        });
        let chat_root = cx.entity();
        let chat_action_menu = cx.new(move |menu_cx| {
            let action_root = chat_root.clone();
            PopoverMenu::new(
                "chat-actions",
                menu_cx.focus_handle(),
                Vec::new(),
                Rc::new(move |index, window, cx| {
                    action_root.update(cx, |this, cx| {
                        this.handle_chat_menu_action(index, window, cx);
                    });
                }),
                menu_cx,
            )
            .min_width(px(160.))
            .trigger_size(IconButtonSize::Md)
            .trigger_icon("more-horizontal.svg")
            .trigger_tooltip("Chat actions")
        });
        let create_root = cx.entity();
        let sidebar_create_menu = cx.new(move |menu_cx| {
            let action_root = create_root.clone();
            PopoverMenu::new(
                "sidebar-create",
                menu_cx.focus_handle(),
                vec![
                    UiMenuItem::new("New chat").icon("message-square-plus.svg"),
                    UiMenuItem::new("New mission").icon("flag.svg"),
                ],
                Rc::new(move |index, window, cx| {
                    action_root.update(cx, |this, cx| {
                        this.handle_sidebar_create_action(index, window, cx);
                    });
                }),
                menu_cx,
            )
            .min_width(px(160.))
            .trigger_size(IconButtonSize::Sm)
            .trigger_icon("plus.svg")
            .without_trigger_tooltip()
        });
        let last_focused_runner_id = tabs
            .active()
            .and_then(PaneLayout::focused_session_id)
            .and_then(|session_id| sessions.iter().find(|entry| entry.session_id == session_id))
            .and_then(|entry| entry.runner_id.clone());
        let active_project_id = tabs.active_tab_id().and_then(|tab_id| {
            let tab = nodes.iter().find(|node| node.id == tab_id)?;
            let parent_id = tab.parent_id.as_deref()?;
            nodes
                .iter()
                .find(|node| {
                    node.id == parent_id
                        && node.node_type == runner_backend::repo::node::NodeType::Project
                })
                .and_then(|node| node.ref_id.clone())
        });
        let sidebar_visibility = SidebarVisibilityTransition::new(!settings.sidebar_collapsed);
        let chat_panel_visibility = SidebarVisibilityTransition::new(settings.chat_panel_open);
        let runner_surfaces = RunnerSurfaces::new(cx.entity(), cx);
        let crew_surfaces = CrewSurfaces::new(cx.entity(), cx);
        let mission_workspace = MissionWorkspaceState::new(cx.entity(), cx);
        let settings_page = SettingsState::new(cx.entity(), &settings, cx);
        let mut root = Self {
            core,
            bridge,
            sessions,
            runners,
            nodes,
            projects,
            missions,
            session_activity,
            tabs,
            attached: HashMap::new(),
            root_focus,
            layout_picker_focus,
            sidebar_scroll,
            sidebar_scrollbar,
            waker,
            start_chat_modal: None,
            start_mission_modal: None,
            sidebar_create_menu,
            sidebar_context_menu: None,
            sidebar_rename: None,
            project_modal: None,
            project_delete_confirm: None,
            project_delete_busy: false,
            sidebar_archiving_sessions: HashSet::new(),
            sidebar_archiving_missions: HashSet::new(),
            stopping_sessions: HashSet::new(),
            chat_transitions: HashMap::new(),
            next_chat_transition_generation: 0,
            session_exit_codes: HashMap::new(),
            chat_error: None,
            chat_warning: None,
            active_chat_detail,
            session_key_copy,
            chat_action_menu,
            chat_menu_actions: Vec::new(),
            pane_action_menus: HashMap::new(),
            chat_rename_modal: None,
            active_project_id,
            sidebar_dragged_id: None,
            sidebar_drop_target: None,
            sidebar_drop_marker: None,
            last_focused_runner_id,
            layout_picker_open: false,
            split_sizes_dirty: false,
            error: (!errors.is_empty()).then(|| errors.join("\n")),
            settings,
            settings_path,
            settings_page,
            route: AppRoute::Chat,
            settings_return_route: AppRoute::Chat,
            sidebar_visibility,
            chat_panel_visibility,
            sidebar_preview_open: false,
            sidebar_preview_peeking: false,
            titlebar_drag_armed: false,
            window_size_save_generation: 0,
            toasts: ToastHost::default(),
            runner_surfaces,
            crew_surfaces,
            mission_workspace,
            _appearance_subscription: None,
            _activation_subscription: None,
            _bounds_subscription: None,
            _sidebar_rename_focus_subscription: None,
            _project_cwd_subscription: None,
        };
        root._appearance_subscription = Some(cx.observe_window_appearance(window, |_, _, cx| {
            cx.notify();
        }));
        root._bounds_subscription = Some(cx.observe_window_bounds(window, |this, window, cx| {
            mac_chrome::sync_traffic_lights(window, this.settings.app_zoom);
            this.schedule_window_size_save(window, cx);
            cx.notify();
        }));
        root._activation_subscription =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                this.sync_sidebar_window_activation(window, cx)
            }));
        mac_chrome::sync_traffic_lights(window, root.settings.app_zoom);
        if let Err(error) = root.ensure_active_tab_attached(window, cx) {
            root.error = Some(error.to_string());
        }
        root.focus_active_terminal(window);
        root.sync_sidebar_window_activation(window, cx);
        root.start_launch_auto_resume(window, cx);
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
        self.tabs.replace_rows(&rows)?;
        self.nodes = rows;
        self.sync_active_project_from_active_tab();
        self.prune_sidebar_collapse_state();
        Ok(())
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

#[cfg(target_os = "macos")]
fn install_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().expect("Runner must start on the main thread");
    let data = NSData::with_bytes(include_bytes!("../../../assets/icon.png"));
    let image = NSImage::initWithData(NSImage::alloc(), &data).expect("invalid app icon");
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
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
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(INTER_FONT),
                    Cow::Borrowed(MESLO_FONT_REGULAR),
                    Cow::Borrowed(MESLO_FONT_BOLD),
                    Cow::Borrowed(MESLO_FONT_ITALIC),
                    Cow::Borrowed(MESLO_FONT_BOLD_ITALIC),
                ])
                .expect("bundled fonts must be valid");

            #[cfg(target_os = "macos")]
            install_app_icon();

            // Installed here, rather than in `boot_core`, so AppKit
            // registration happens on GPUI's process main thread.
            #[cfg(target_os = "macos")]
            runner_backend::wake::install(&core.events);

            let quit_core = core.clone();
            cx.on_action(move |_: &Quit, cx| {
                if let Some(window) = cx
                    .active_window()
                    .and_then(|window| window.downcast::<NativeRoot>())
                {
                    let _ = window.update(cx, |this, _, _| this.save_settings());
                }
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
                KeyBinding::new("cmd-w", ClosePane, None),
                KeyBinding::new("cmd-[", FocusPreviousPane, None),
                KeyBinding::new("cmd-]", FocusNextPane, None),
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
    let (window_width, window_height) = cx
        .primary_display()
        .map(|display| {
            let display_size = display.visible_bounds().size;
            app_settings::clamp_window_size_to_display(
                settings.window_width,
                settings.window_height,
                f32::from(display_size.width),
                f32::from(display_size.height),
            )
        })
        .unwrap_or((settings.window_width, settings.window_height));
    let bounds = Bounds::centered(None, size(px(window_width), px(window_height)), cx);
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
            let root = cx.new(|cx| {
                NativeRoot::new(
                    core.clone(),
                    settings_path.clone(),
                    settings.clone(),
                    settings_error.clone(),
                    window,
                    cx,
                )
            });
            let weak = root.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                let _ = weak.update(cx, |this, _| {
                    this.save_window_size(window);
                    this.save_settings();
                });
                true
            });
            root
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

#[cfg(test)]
mod native_root_tests {
    use super::*;

    #[test]
    fn terminal_repaints_only_when_the_chat_surface_is_visible() {
        assert!(AppRoute::Chat.terminal_visible());
        for route in [
            AppRoute::Runners,
            AppRoute::RunnerDetail("runner".into()),
            AppRoute::Crews,
            AppRoute::CrewEditor("crew".into()),
            AppRoute::Settings,
        ] {
            assert!(!route.terminal_visible());
        }
    }

    #[test]
    fn entity_refresh_events_match_runner_and_crew_dependencies() {
        let event = |name| runner_backend::events::AppEvent {
            name,
            payload: serde_json::Value::Null,
        };

        assert_eq!(
            EntityRefreshKind::for_event(&event("runner/activity")),
            EntityRefreshKind::Runners
        );
        for name in ["runner/changed", "crew/changed", "slot/changed"] {
            assert_eq!(
                EntityRefreshKind::for_event(&event(name)),
                EntityRefreshKind::All
            );
        }
        assert_eq!(
            EntityRefreshKind::for_event(&event("session/status")),
            EntityRefreshKind::None
        );
    }

    #[test]
    fn entity_refresh_merge_covers_every_pair() {
        use EntityRefreshKind::{All, None, Runners};

        for (left, right, expected) in [
            (None, None, None),
            (None, Runners, Runners),
            (None, All, All),
            (Runners, None, Runners),
            (Runners, Runners, Runners),
            (Runners, All, All),
            (All, None, All),
            (All, Runners, All),
            (All, All, All),
        ] {
            assert_eq!(left.merge(right), expected, "{left:?} + {right:?}");
        }
    }

    #[test]
    fn collapse_transition_eases_from_open_to_closed() {
        let start = Instant::now();
        let duration = Duration::from_millis(200);
        let mut transition = SidebarVisibilityTransition::new(true);

        assert_eq!(transition.animate_to(0., start, duration), (1., true));
        assert_eq!(
            transition.animate_to(0., start + Duration::from_millis(100), duration),
            (0.5, true)
        );
        assert_eq!(
            transition.animate_to(0., start + Duration::from_millis(200), duration),
            (0., false)
        );
    }

    #[test]
    fn reversing_transition_continues_from_the_current_width() {
        let start = Instant::now();
        let duration = Duration::from_millis(200);
        let midpoint = start + Duration::from_millis(100);
        let mut transition = SidebarVisibilityTransition::new(true);

        transition.animate_to(0., start, duration);
        assert_eq!(transition.animate_to(1., midpoint, duration), (0.5, true));
        assert_eq!(
            transition.animate_to(1., midpoint + Duration::from_millis(100), duration),
            (0.75, true)
        );
    }
}

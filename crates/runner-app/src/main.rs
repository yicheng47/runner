mod app_settings;
mod app_store;
mod assets;
mod keymap;
mod list_controls;
mod mac_chrome;
mod surfaces;
mod terminal;
mod window_state;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, point, prelude::*, px, relative, rems, size, AnyElement, App, Application,
    Bounds, ClipboardItem, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, Global,
    KeyDownEvent, Menu, MenuItem, MouseButton, OsAction, QuitMode, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Subscription, SystemMenuType, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use runner_app::bootstrap::{
    boot_core, native_paths, stop_running_sessions_on_quit, NativeMcpServer, NativePaths,
};
use runner_app::pane_layout::{
    PaneLayout, PaneLeaf, PaneNode, PresetKind, SplitOrientation, TabSet,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::{
    Button, ButtonSize, ContextMenu, CopyValueButton, DuplicateSubjectKind,
    DuplicateSubjectOverlay, IconButton, IconButtonSize, MenuItem as UiMenuItem, PopoverMenu,
    Scrollbar, SessionControl, SessionControlKind, Tooltip,
};
use runner_app::{theme, Copy, Cut, Paste, SelectAll};
use runner_backend::model::SessionStatus;
use runner_backend::ops::session::DirectSessionEntry;
use runner_backend::session::manager::SessionActivityState;
use runner_backend::AppCore;
use runner_terminal::terminal::TerminalSession;

use app_settings::{settings_path, AppSettings};
use app_store::{global_app_store, AppStore, GlobalAppStore, StoreRefreshKind, StoreRevisions};
use assets::{
    Assets, INTER_FONT, MESLO_FONT_BOLD, MESLO_FONT_BOLD_ITALIC, MESLO_FONT_ITALIC,
    MESLO_FONT_REGULAR,
};
use terminal::{TerminalElement, TerminalInteraction};
use toast::ToastHost;

actions!(
    runner_app_ui,
    [
        CloseWindowOrPane,
        CommandPalette,
        FocusNextPane,
        FocusPreviousPane,
        Hide,
        HideOthers,
        Maximize,
        Minimize,
        MissionTabNext,
        MissionTabPrevious,
        NavigateNextPage,
        NavigatePreviousPage,
        NewTab,
        NewWindow,
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

mod toast;

use surfaces::{
    AppRoute, CommandPaletteState, CrewSurfaces, MissionWorkspace, ProjectModal, RunnerSurfaces,
    SettingsPane, SettingsState, Sidebar, StartChatModal, StartMissionModalState,
};

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;
const WORKSPACE_HEADER_HEIGHT: f32 = 44.;
const PANE_HEADER_HEIGHT: f32 = 34.;
const WINDOW_STATE_SAVE_DELAY_MS: u64 = 300;

struct AttachedChat {
    terminal: Arc<TerminalSession>,
    terminal_interaction: Entity<TerminalInteraction>,
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

#[derive(Clone, Copy)]
struct ChatTransition {
    kind: surfaces::chat_lifecycle::TransitionKind,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseTarget {
    Pane,
    Window,
}

fn close_target(route: &AppRoute, leaves: usize) -> CloseTarget {
    if *route == AppRoute::Chat && leaves > 1 {
        CloseTarget::Pane
    } else {
        CloseTarget::Window
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuntimeLocation {
    Chat(String),
    Mission(String),
}

const RUNTIME_NAVIGATION_HISTORY_LIMIT: usize = 64;

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
    window_label: String,
    closing: bool,
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    tabs: TabSet,
    attached: HashMap<String, AttachedChat>,
    root_focus: FocusHandle,
    chat_focus: FocusHandle,
    layout_picker_focus: FocusHandle,
    sidebar: Entity<Sidebar>,
    start_chat_modal: Option<StartChatModal>,
    start_mission_modal: Option<StartMissionModalState>,
    project_modal: Option<ProjectModal>,
    project_delete_confirm: Option<String>,
    project_delete_busy: bool,
    stopping_sessions: HashSet<String>,
    chat_transitions: HashMap<String, ChatTransition>,
    next_chat_transition_generation: u64,
    session_exit_codes: HashMap<String, Option<i32>>,
    chat_error: Option<String>,
    chat_warning: Option<String>,
    active_chat_detail: Option<DirectSessionEntry>,
    archived_chat_detail: Option<DirectSessionEntry>,
    session_key_copy: Entity<CopyValueButton>,
    archived_session_key_copy: Entity<CopyValueButton>,
    chat_action_menu: Entity<PopoverMenu>,
    chat_menu_actions: Vec<ChatMenuAction>,
    pane_action_menus: HashMap<String, Entity<PopoverMenu>>,
    chat_rename_modal: Option<ChatRenameModal>,
    last_focused_runner_id: Option<String>,
    layout_picker_open: bool,
    split_sizes_dirty: bool,
    chat_secondaries: HashMap<String, String>,
    dismissed_duplicate_chats: HashSet<String>,
    error: Option<String>,
    settings_page: SettingsState,
    route: AppRoute,
    settings_return_route: AppRoute,
    runtime_navigation_history: Vec<RuntimeLocation>,
    runtime_navigation_index: Option<usize>,
    sidebar_collapsed: bool,
    sidebar_visibility: SidebarVisibilityTransition,
    chat_panel_visibility: SidebarVisibilityTransition,
    command_palette: Entity<CommandPaletteState>,
    sidebar_preview_open: bool,
    sidebar_preview_peeking: bool,
    titlebar_drag_armed: bool,
    window_state: window_state::WindowState,
    window_state_save_generation: u64,
    toasts: ToastHost,
    runner_surfaces: RunnerSurfaces,
    crew_surfaces: CrewSurfaces,
    mission_workspace: Entity<MissionWorkspace>,
    _appearance_subscription: Option<Subscription>,
    _activation_subscription: Option<Subscription>,
    _bounds_subscription: Option<Subscription>,
    _project_cwd_subscription: Option<Subscription>,
    _store_subscription: Subscription,
}

impl NativeRoot {
    fn new(
        window_label: String,
        initial_route_path: Option<String>,
        initial_window_state: Option<window_state::WindowState>,
        app_store: Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let core = app_store.read(cx).core.clone();
        let settings = app_store.read(cx).settings.clone();
        let sessions = app_store.read(cx).sessions.clone();
        let nodes = app_store.read(cx).nodes.clone();
        let store_revisions = app_store.read(cx).revisions;
        let mut errors: Vec<_> = app_store.read(cx).error.clone().into_iter().collect();

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
                        this.refresh_agents_pane(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        window.set_rem_size(px(16. * settings.app_zoom));
        let mut tabs = match TabSet::from_rows(&nodes) {
            Ok(tabs) => tabs,
            Err(error) => {
                errors.push(error.to_string());
                TabSet::default()
            }
        };
        let route_path = initial_route_path
            .as_deref()
            .unwrap_or_default()
            .trim_matches('/');
        let initial_route = if let Some(mission_id) = route_path
            .strip_prefix("missions/")
            .filter(|id| !id.is_empty())
        {
            AppRoute::Mission(mission_id.to_owned())
        } else if let Some(session_id) = route_path
            .strip_prefix("chats/")
            .filter(|id| !id.is_empty())
        {
            if tabs.activate_session(session_id) {
                AppRoute::Chat
            } else {
                AppRoute::Runners
            }
        } else if let Some(handle) = route_path
            .strip_prefix("runners/")
            .filter(|handle| !handle.is_empty())
        {
            AppRoute::RunnerDetail(handle.to_owned())
        } else if let Some(crew_id) = route_path
            .strip_prefix("crews/")
            .filter(|crew_id| !crew_id.is_empty())
        {
            AppRoute::CrewEditor(crew_id.to_owned())
        } else if route_path == "chats" {
            AppRoute::Chat
        } else if route_path == "runners" {
            AppRoute::Runners
        } else if route_path == "crews" {
            AppRoute::Crews
        } else if route_path == "settings" {
            AppRoute::Settings
        } else if window_label == "main" {
            AppRoute::Chat
        } else {
            AppRoute::Runners
        };

        let root_focus = cx.focus_handle();
        let chat_focus = cx.focus_handle();
        let layout_picker_focus = cx.focus_handle();
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
        let archived_session_key_copy = cx
            .new(|copy_cx| CopyValueButton::new(copy_cx.focus_handle(), None, "Copy session_key"));
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
        let sidebar_collapsed = settings.sidebar_collapsed;
        let sidebar_visibility = SidebarVisibilityTransition::new(!sidebar_collapsed);
        let chat_panel_visibility = SidebarVisibilityTransition::new(settings.chat_panel_open);
        let runner_surfaces = RunnerSurfaces::new(cx.entity(), cx);
        let crew_surfaces = CrewSurfaces::new(cx.entity(), cx);
        let sidebar_shell = cx.entity().downgrade();
        let sidebar_store = app_store.clone();
        let sidebar = cx.new(move |sidebar_cx| {
            Sidebar::new(sidebar_shell, sidebar_store, active_project_id, sidebar_cx)
        });
        let mission_shell = cx.entity().downgrade();
        let mission_store = app_store.clone();
        let mission_window_label = window_label.clone();
        let mission_workspace = cx.new(|workspace_cx| {
            MissionWorkspace::new(
                mission_window_label,
                mission_shell,
                mission_store,
                window,
                workspace_cx,
            )
        });
        let palette_shell = cx.entity().downgrade();
        let palette_store = app_store.clone();
        let command_palette = cx.new(move |palette_cx| {
            CommandPaletteState::new(palette_shell, palette_store, palette_cx)
        });
        let settings_page = SettingsState::new(cx.entity(), &settings, cx);
        let runtime_navigation_history = match &initial_route {
            AppRoute::Chat => active_chat_detail
                .as_ref()
                .map(|session| vec![RuntimeLocation::Chat(session.session_id.clone())])
                .unwrap_or_default(),
            AppRoute::Mission(mission_id) => vec![RuntimeLocation::Mission(mission_id.clone())],
            _ => Vec::new(),
        };
        let runtime_navigation_index = (!runtime_navigation_history.is_empty()).then_some(0);
        let mut root = Self {
            window_label: window_label.clone(),
            closing: false,
            app_store: app_store.clone(),
            store_revisions,
            tabs,
            attached: HashMap::new(),
            root_focus,
            chat_focus,
            layout_picker_focus,
            sidebar,
            start_chat_modal: None,
            start_mission_modal: None,
            project_modal: None,
            project_delete_confirm: None,
            project_delete_busy: false,
            stopping_sessions: HashSet::new(),
            chat_transitions: HashMap::new(),
            next_chat_transition_generation: 0,
            session_exit_codes: HashMap::new(),
            chat_error: None,
            chat_warning: None,
            active_chat_detail,
            archived_chat_detail: None,
            session_key_copy,
            archived_session_key_copy,
            chat_action_menu,
            chat_menu_actions: Vec::new(),
            pane_action_menus: HashMap::new(),
            chat_rename_modal: None,
            last_focused_runner_id,
            layout_picker_open: false,
            split_sizes_dirty: false,
            chat_secondaries: HashMap::new(),
            dismissed_duplicate_chats: HashSet::new(),
            error: (!errors.is_empty()).then(|| errors.join("\n")),
            settings_page,
            route: initial_route.clone(),
            settings_return_route: initial_route,
            runtime_navigation_history,
            runtime_navigation_index,
            sidebar_collapsed,
            sidebar_visibility,
            chat_panel_visibility,
            command_palette,
            sidebar_preview_open: false,
            sidebar_preview_peeking: false,
            titlebar_drag_armed: false,
            window_state: initial_window_state
                .unwrap_or_else(|| window_state::snapshot(window, None)),
            window_state_save_generation: 0,
            toasts: ToastHost::default(),
            runner_surfaces,
            crew_surfaces,
            mission_workspace,
            _appearance_subscription: None,
            _activation_subscription: None,
            _bounds_subscription: None,
            _project_cwd_subscription: None,
            _store_subscription: cx
                .observe(&app_store, |this, _, cx| this.handle_app_store_update(cx)),
        };
        root._appearance_subscription = Some(cx.observe_window_appearance(window, |_, _, cx| {
            cx.notify();
        }));
        root._bounds_subscription = Some(cx.observe_window_bounds(window, |this, window, cx| {
            mac_chrome::sync_traffic_lights(window, this.settings(cx).app_zoom);
            this.schedule_window_state_checkpoint(window, cx);
            cx.notify();
        }));
        root._activation_subscription =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                this.sync_sidebar_window_activation(window, cx)
            }));
        mac_chrome::sync_traffic_lights(window, root.settings(cx).app_zoom);
        match root.route.clone() {
            AppRoute::Mission(mission_id) => root.open_mission(mission_id, window, cx),
            AppRoute::Runners => {
                root.load_runner_page(cx);
                window.focus(&root.root_focus);
            }
            AppRoute::RunnerDetail(handle) => {
                root.load_runner_detail(handle, cx);
                window.focus(&root.root_focus);
            }
            AppRoute::Crews => {
                root.load_crew_page(cx);
                window.focus(&root.root_focus);
            }
            AppRoute::CrewEditor(crew_id) => {
                root.load_crew_editor(crew_id, cx);
                window.focus(&root.root_focus);
            }
            AppRoute::Settings => {
                root.enter_settings_pane(SettingsPane::General, window, cx);
                window.focus(&root.root_focus);
            }
            AppRoute::Chat | AppRoute::ArchivedChat => {}
        }
        root.sync_window_activation(window, cx);
        if root.route == AppRoute::Chat {
            if let Err(error) = root.ensure_active_tab_attached(window, cx) {
                root.error = Some(error.to_string());
            }
            root.focus_active_terminal(window, cx);
        }
        root.start_launch_auto_resume(window, cx);
        root.start_focus_map_listener(window, cx);
        root
    }

    fn core<'a>(&self, cx: &'a App) -> &'a AppCore {
        &self.app_store.read(cx).core
    }

    fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }

    fn refresh_sessions(&self, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh_sessions(store_cx));
    }

    fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    fn reload_tabs(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.app_store
            .update(cx, |store, store_cx| store.refresh_nodes(store_cx))?;
        self.apply_tab_rows(cx)
    }

    fn apply_tab_rows(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.tabs.replace_rows(&self.app_store.read(cx).nodes)?;
        self.sync_active_project_from_active_tab(cx);
        self.prune_sidebar_collapse_state(cx);
        Ok(())
    }

    fn session_entry<'a>(&self, session_id: &str, cx: &'a App) -> Option<&'a DirectSessionEntry> {
        self.app_store
            .read(cx)
            .sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
    }

    fn handle_app_store_update(&mut self, cx: &mut Context<Self>) {
        let revisions = self.app_store.read(cx).revisions;
        let previous = self.store_revisions;
        self.store_revisions = revisions;
        let reactions = revisions.reactions_since(previous);

        if reactions.sync_error {
            self.error = self.app_store.read(cx).error.clone();
        }
        if reactions.reload_tabs {
            if let Err(error) = self.apply_tab_rows(cx) {
                self.error = Some(error.to_string());
            }
        }
        if reactions.prune_sidebar {
            self.prune_sidebar_collapse_state(cx);
        }
        if reactions.prune_window_state {
            self.prune_store_dependent_window_state(cx);
        }
        if reactions.reload_runner_surfaces {
            match self.route.clone() {
                AppRoute::Runners => self.load_runner_page(cx),
                AppRoute::RunnerDetail(handle) => self.load_runner_detail(handle, cx),
                _ => {}
            }
        }
        if reactions.reload_crew_surfaces {
            match self.route.clone() {
                AppRoute::Crews => self.load_crew_page(cx),
                AppRoute::CrewEditor(crew_id) => self.load_crew_editor(crew_id, cx),
                _ => {}
            }
        }
        if reactions.apply_terminal_settings {
            self.apply_terminal_settings(cx);
        }

        if reactions.notify_shell() || (reactions.terminal_wake && self.route.terminal_visible()) {
            cx.notify();
        }
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

#[derive(Clone)]
struct LiveWindowState {
    label: String,
    frame: window_state::WindowState,
    route: Option<String>,
    focused_at: i64,
}

#[derive(Default)]
struct WindowLayoutCheckpoint {
    last_layout: Option<window_state::WindowLayout>,
    last_main_frame: Option<window_state::WindowState>,
}

impl Global for WindowLayoutCheckpoint {}

fn collect_window_layout(cx: &mut App) -> (window_state::WindowLayout, Vec<LiveWindowState>) {
    let core = global_app_store(cx).read(cx).core.clone();
    let focused_at = core
        .windows
        .snapshot()
        .into_iter()
        .map(|entry| (entry.label, entry.focused_at.timestamp_micros()))
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

fn format_window_layout_details(windows: &[LiveWindowState]) -> String {
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

fn checkpoint_window_layout(cx: &mut App) {
    save_window_layout_checkpoint(cx, false);
}

fn checkpoint_window_layout_on_quit(cx: &mut App) {
    save_window_layout_checkpoint(cx, true);
}

fn save_window_layout_checkpoint(cx: &mut App, force: bool) {
    let core = global_app_store(cx).read(cx).core.clone();
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
            match window_state::save(&core.app_data_dir, main_frame) {
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
    match window_state::save_layout(&core.app_data_dir, &layout) {
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

fn checkpoint_window_layout_deferred(cx: &mut Context<NativeRoot>) {
    cx.defer(checkpoint_window_layout);
}

fn save_window_settings(cx: &mut App) {
    for handle in cx.windows() {
        if let Some(window) = handle.downcast::<NativeRoot>() {
            let _ = window.update(cx, |this, _, cx| this.save_settings(cx));
        }
    }
}

fn run() -> Result<()> {
    let paths = native_paths()?;
    let core = boot_core(&paths)?;
    let mcp_server = match NativeMcpServer::start(&core) {
        Ok(server) => Some(server),
        Err(error) => {
            eprintln!("Runner MCP listener failed to start: {error:#}");
            None
        }
    };
    print_startup_paths(&paths);
    let shutdown_core = core.clone();
    let ui_settings_path = settings_path(&paths.app_data_dir);

    let application = Application::new()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::Explicit);
    application.on_reopen(|cx| {
        if !window_label_is_open(cx, "main") {
            if let Err(error) = open_runner_window("main".into(), None, None, cx) {
                eprintln!("Runner main-window reopen failed: {error:#}");
            }
        }
        cx.activate(true);
    });
    application.run(move |cx: &mut App| {
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

        let (settings, settings_error) = match AppSettings::load(&ui_settings_path) {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(error.to_string())),
        };
        let app_store = cx.new(|cx| {
            AppStore::new(
                core.clone(),
                ui_settings_path.clone(),
                settings,
                settings_error,
                cx,
            )
        });
        let keymap_overrides = app_store.read(cx).settings.keymap_overrides.clone();
        keymap::install_bindings(cx, &keymap_overrides, false);
        cx.set_global(GlobalAppStore(app_store));
        cx.set_global(WindowLayoutCheckpoint::default());

        // App::shutdown clears its windows before polling the returned future, so the
        // checkpoint must run in this callback body rather than inside the future.
        let quit_core = core.clone();
        cx.on_app_quit(move |cx| {
            save_window_settings(cx);
            checkpoint_window_layout_on_quit(cx);
            if let Err(error) = stop_running_sessions_on_quit(&quit_core) {
                eprintln!("Runner quit session teardown failed: {error:#}");
            }
            std::future::ready(())
        })
        .detach();

        cx.on_action(|_: &Quit, cx| cx.quit());
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
        cx.on_action(|_: &CloseWindowOrPane, cx| {
            if let Some(window) = cx
                .active_window()
                .and_then(|window| window.downcast::<NativeRoot>())
            {
                let _ = window.update(cx, |this, window, cx| {
                    let leaves = (this.route == AppRoute::Chat)
                        .then(|| this.tabs.active())
                        .flatten()
                        .map(|layout| layout.root.leaves().len())
                        .unwrap_or_default();
                    match close_target(&this.route, leaves) {
                        CloseTarget::Pane => {
                            eprintln!(
                                "Runner cmd-w: route={:?} leaves={leaves} -> pane",
                                this.route
                            );
                            if let Some(pane_id) = this
                                .tabs
                                .active()
                                .map(|layout| layout.focused_pane_id.clone())
                            {
                                this.close_pane(&pane_id, window, cx);
                            }
                        }
                        CloseTarget::Window => {
                            eprintln!(
                                "Runner cmd-w: route={:?} leaves={leaves} -> window",
                                this.route
                            );
                            this.prepare_window_close(window, cx);
                            window.remove_window();
                        }
                    }
                });
            }
        });
        cx.on_action(|_: &NewWindow, cx| {
            cx.defer(|cx| {
                if let Err(error) = open_new_runner_window(None, cx) {
                    eprintln!("Runner new window failed: {error:#}");
                }
                cx.activate(true);
            });
        });
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
                name: "File".into(),
                items: vec![MenuItem::action("New Window", NewWindow)],
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
                    MenuItem::action("Close Window", CloseWindowOrPane),
                ],
            },
        ]);

        let restored_layout = window_state::read_layout(&core.app_data_dir);
        for warning in &restored_layout.warnings {
            eprintln!("Runner window-layout: restore fallback: {warning}");
        }
        let mut restored_labels = Vec::new();
        for restored in window_state::restore_order(restored_layout.layout) {
            let result = match restored {
                window_state::RestoredWindowState::Main(state) => {
                    open_runner_window("main".into(), state.route, None, cx)
                }
                window_state::RestoredWindowState::Secondary(state) => open_runner_window(
                    runner_backend::ops::window::allocate_label(),
                    state.route,
                    Some(state.frame),
                    cx,
                ),
            };
            match result {
                Ok(label) => restored_labels.push(label),
                Err(error) => eprintln!("Runner window-layout restore failed: {error:#}"),
            }
        }
        if restored_labels.is_empty() {
            restored_labels.push(
                open_runner_window("main".into(), None, None, cx)
                    .expect("open fallback Runner window"),
            );
        }
        if let Some(label) = restored_labels.last() {
            focus_other_window(label, cx);
        }
        let (_, restored_windows) = collect_window_layout(cx);
        eprintln!(
            "Runner window-layout: restored count={} {}",
            restored_windows.len(),
            format_window_layout_details(&restored_windows),
        );
        cx.activate(true);
    });

    let shutdown_result = stop_running_sessions_on_quit(&shutdown_core);
    drop(mcp_server);
    shutdown_result
}

fn open_new_runner_window(initial_route: Option<String>, cx: &mut App) -> Result<String> {
    let label = if !window_label_is_open(cx, "main") {
        "main".into()
    } else {
        runner_backend::ops::window::allocate_label()
    };
    open_runner_window(label, initial_route, None, cx)
}

fn open_runner_window(
    label: String,
    initial_route: Option<String>,
    restored_window_state: Option<window_state::WindowState>,
    cx: &mut App,
) -> Result<String> {
    let app_store = global_app_store(cx);
    let core = app_store.read(cx).core.clone();
    let default_size = size(px(1440.), px(900.));
    let fallback = Bounds::centered(None, default_size, cx);
    let (bounds, initial_window_state) = if label == "main" {
        let displays = window_state::display_rects(cx);
        let default_rect = displays.first().copied().unwrap_or(window_state::Rect {
            x: f32::from(fallback.origin.x) as f64,
            y: f32::from(fallback.origin.y) as f64,
            width: f32::from(fallback.size.width) as f64,
            height: f32::from(fallback.size.height) as f64,
        });
        let restored = match window_state::load_and_migrate(
            &core.app_data_dir,
            &settings_path(&core.app_data_dir),
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
        let origin =
            runner_backend::ops::window::cascade_reference(&core.windows.snapshot(), &label)
                .and_then(|reference| window_origin_for_label(cx, &reference))
                .map(|origin| origin + point(px(32.), px(32.)))
                .unwrap_or(fallback.origin);
        (
            WindowBounds::Windowed(Bounds::new(origin, default_size)),
            None,
        )
    };
    core.windows.register(&label);
    core.broadcast_focus_map();
    let open_label = label.clone();
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
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
                    open_label.clone(),
                    initial_route.clone(),
                    initial_window_state,
                    app_store.clone(),
                    window,
                    cx,
                )
            });
            let weak = root.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.prepare_window_close(window, cx);
                });
                true
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
            runner_backend::ops::window::unregister(&core, &label);
            Err(error)
        }
    }
}

fn window_label_is_open(cx: &App, label: &str) -> bool {
    global_app_store(cx)
        .read(cx)
        .core
        .windows
        .snapshot()
        .iter()
        .any(|entry| entry.label == label)
}

fn window_origin_for_label(cx: &mut App, label: &str) -> Option<gpui::Point<gpui::Pixels>> {
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

fn focus_other_window(label: &str, cx: &mut App) {
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
    fn cmd_w_closes_only_a_split_chat_pane() {
        assert_eq!(close_target(&AppRoute::Chat, 2), CloseTarget::Pane);
        assert_eq!(close_target(&AppRoute::Chat, 1), CloseTarget::Window);
        assert_eq!(close_target(&AppRoute::Runners, 0), CloseTarget::Window);
        assert_eq!(close_target(&AppRoute::Crews, 0), CloseTarget::Window);
        assert_eq!(close_target(&AppRoute::Settings, 0), CloseTarget::Window);
    }

    #[test]
    fn terminal_repaints_only_when_the_chat_surface_is_visible() {
        assert!(AppRoute::Chat.terminal_visible());
        for route in [
            AppRoute::Runners,
            AppRoute::RunnerDetail("runner".into()),
            AppRoute::Crews,
            AppRoute::CrewEditor("crew".into()),
            AppRoute::Mission("mission".into()),
            AppRoute::ArchivedChat,
            AppRoute::Settings,
        ] {
            assert!(!route.terminal_visible());
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

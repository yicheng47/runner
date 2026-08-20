use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, svg, AnyElement, App, Bounds, CursorStyle, DragMoveEvent, Entity,
    FontWeight, KeyDownEvent, MouseButton, Pixels, ScrollDelta, ScrollWheelEvent, SharedString,
    Window,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, CopyValueButton, Field, IconButton, IconButtonSize,
    MenuItem as UiMenuItem, Modal, OverlayWidth, PopoverMenu, RunnerAvatar, RunnerPresence,
    SessionControl, SessionControlKind, SessionControlVariant, SessionOverlay, SessionOverlayKind,
    TextField, Tooltip,
};
use runner_backend::model::{
    Crew, Event, EventKind, Mission, MissionStatus, SessionStatus, SlotWithRunner,
};
use runner_backend::ops::session::SessionRow;
use runner_backend::windows::Subject;
use runner_terminal::terminal::{TerminalSession, UserInputMode};

use super::*;
use crate::mission_composer::{
    key_down as composer_key_down, mention_options, select_target as select_composer_target,
    update_draft as update_composer_draft, ComposerPost, ComposerState,
    RosterEntry as ComposerRosterEntry,
};
use crate::mission_feed::{
    group_feed_blocks, is_human_authored, message_target, message_text, project_asks, FeedBlock,
};

const WORKSPACE_TABS_HEIGHT: f32 = 38.;
const WORKSPACE_TAB_WIDTH: f32 = 136.;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MissionTab {
    Feed,
    Session(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MissionRailView {
    #[default]
    Runners,
    Meta,
}

impl MissionRailView {
    fn from_setting(value: &str) -> Self {
        if value == "meta" {
            Self::Meta
        } else {
            Self::Runners
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Self::Runners => "runners",
            Self::Meta => "meta",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissionTransitionKind {
    Starting,
    Resuming,
}

#[derive(Clone, Copy)]
struct MissionTransition {
    kind: MissionTransitionKind,
    started_at: Instant,
    baseline_seq: u64,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotOverlayState {
    Archiving,
    Resuming,
    Starting,
    Stopped,
    None,
}

fn resolve_slot_overlay(
    archiving: bool,
    transition: Option<MissionTransitionKind>,
    status: SessionStatus,
) -> SlotOverlayState {
    if archiving {
        SlotOverlayState::Archiving
    } else if transition == Some(MissionTransitionKind::Resuming) {
        SlotOverlayState::Resuming
    } else if transition == Some(MissionTransitionKind::Starting) {
        SlotOverlayState::Starting
    } else if status != SessionStatus::Running {
        SlotOverlayState::Stopped
    } else {
        SlotOverlayState::None
    }
}

fn is_concurrent_resume_error(error: &str) -> bool {
    [
        "is already being resumed",
        "is already running — attach instead",
    ]
    .iter()
    .any(|fragment| error.contains(fragment))
}

#[derive(Clone)]
enum MissionMenuAction {
    Pin,
    Rename,
    Reset,
    Archive,
}

pub(crate) struct MissionRenameModal {
    mission_id: String,
    original: String,
    input: Entity<TextField>,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    submitting: bool,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct DeliveryBlocked {
    unread_count: usize,
}

pub(crate) struct MissionWorkspaceState {
    pub mission_id: Option<String>,
    generation: u64,
    refresh_generation: u64,
    event_resync_generation: u64,
    mission: Option<Mission>,
    crew: Option<Crew>,
    sessions: Vec<SessionRow>,
    events: Vec<Event>,
    runner_statuses: BTreeMap<String, SessionActivityState>,
    goal: Option<String>,
    feed_blocks: Vec<FeedBlock>,
    askers_by_question: HashMap<String, String>,
    resolved_asks: HashMap<String, String>,
    pending_ask_choices: HashMap<String, String>,
    expanded_signal_payloads: HashSet<String>,
    submitting_asks: HashSet<String>,
    feed_scroll: ScrollHandle,
    feed_was_near_bottom: bool,
    feed_has_new_messages: bool,
    roster: Vec<SlotWithRunner>,
    composer: ComposerState,
    composer_input: Entity<TextField>,
    composer_posting: bool,
    composer_anchor: Option<Bounds<Pixels>>,
    _composer_subscription: Subscription,
    loading: bool,
    loading_overlay_visible: bool,
    error: Option<String>,
    warning: Option<String>,
    active_tab: MissionTab,
    open_tabs: Vec<String>,
    delivery_blocked: HashMap<String, DeliveryBlocked>,
    transitions: HashMap<String, MissionTransition>,
    next_transition_generation: u64,
    stopping: bool,
    resuming: bool,
    resetting: bool,
    pub(crate) reset_confirm_open: bool,
    reset_cancel_focus: FocusHandle,
    reset_confirm_focus: FocusHandle,
    archiving: bool,
    secondary: bool,
    duplicate_dismissed: bool,
    primary_label: Option<String>,
    rail_view: MissionRailView,
    action_menu: Entity<PopoverMenu>,
    menu_actions: Vec<MissionMenuAction>,
    pub(crate) rename_modal: Option<MissionRenameModal>,
    mission_id_copy: Entity<CopyValueButton>,
    session_key_copies: HashMap<String, Entity<CopyValueButton>>,
}

impl MissionWorkspaceState {
    pub(crate) fn new(root: Entity<NativeRoot>, cx: &mut Context<NativeRoot>) -> Self {
        let menu_root = root.clone();
        let action_menu = cx.new(move |menu_cx| {
            PopoverMenu::new(
                "mission-actions",
                menu_cx.focus_handle(),
                Vec::new(),
                Rc::new(move |index, window, cx| {
                    menu_root.update(cx, |this, cx| {
                        this.handle_mission_menu_action(index, window, cx)
                    });
                }),
                menu_cx,
            )
            .min_width(px(160.))
            .trigger_size(IconButtonSize::Md)
            .trigger_icon("more-horizontal.svg")
            .trigger_tooltip("Mission actions")
        });
        let mission_id_copy =
            cx.new(|copy_cx| CopyValueButton::new(copy_cx.focus_handle(), None, "Copy mission ID"));
        let composer_root = root.clone();
        let composer_input = cx.new(move |input_cx| {
            let key_root = composer_root.clone();
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "Message the crew — @handle to address one runner",
                1,
                false,
            )
            .auto_grow(12)
            .text_size(13.)
            .key_interceptor(Rc::new(move |event, window, cx| {
                let key = event.keystroke.key.clone();
                let shift = event.keystroke.modifiers.shift;
                let prevent_default = {
                    let root = key_root.read(cx);
                    let roster = root.mission_composer_roster();
                    composer_key_down(&root.mission_workspace.composer, &roster, &key, shift)
                        .prevent_default
                };
                if prevent_default {
                    let deferred_root = key_root.clone();
                    window.defer(cx, move |window, cx| {
                        deferred_root.update(cx, |this, cx| {
                            this.on_mission_composer_key_down(&key, shift, window, cx)
                        });
                    });
                }
                prevent_default
            }))
        });
        composer_input.update(cx, |input, input_cx| {
            input.set_bare(true, input_cx);
            input.set_right_padding(0., input_cx);
        });
        let composer_subscription = cx.observe(&composer_input, |this, input, cx| {
            let draft = input.read(cx).text().to_owned();
            if draft != this.mission_workspace.composer.draft {
                this.mission_workspace.composer =
                    update_composer_draft(&this.mission_workspace.composer, draft);
                cx.notify();
            }
        });
        Self {
            mission_id: None,
            generation: 0,
            refresh_generation: 0,
            event_resync_generation: 0,
            mission: None,
            crew: None,
            sessions: Vec::new(),
            events: Vec::new(),
            runner_statuses: BTreeMap::new(),
            goal: None,
            feed_blocks: Vec::new(),
            askers_by_question: HashMap::new(),
            resolved_asks: HashMap::new(),
            pending_ask_choices: HashMap::new(),
            expanded_signal_payloads: HashSet::new(),
            submitting_asks: HashSet::new(),
            feed_scroll: ScrollHandle::new(),
            feed_was_near_bottom: true,
            feed_has_new_messages: false,
            roster: Vec::new(),
            composer: ComposerState::default(),
            composer_input,
            composer_posting: false,
            composer_anchor: None,
            _composer_subscription: composer_subscription,
            loading: false,
            loading_overlay_visible: false,
            error: None,
            warning: None,
            active_tab: MissionTab::Feed,
            open_tabs: Vec::new(),
            delivery_blocked: HashMap::new(),
            transitions: HashMap::new(),
            next_transition_generation: 0,
            stopping: false,
            resuming: false,
            resetting: false,
            reset_confirm_open: false,
            reset_cancel_focus: cx.focus_handle(),
            reset_confirm_focus: cx.focus_handle(),
            archiving: false,
            secondary: false,
            duplicate_dismissed: false,
            primary_label: None,
            rail_view: MissionRailView::default(),
            action_menu,
            menu_actions: Vec::new(),
            rename_modal: None,
            mission_id_copy,
            session_key_copies: HashMap::new(),
        }
    }

    fn reset(&mut self, mission_id: String, rail_view: MissionRailView) -> u64 {
        self.generation += 1;
        self.refresh_generation += 1;
        self.event_resync_generation += 1;
        self.mission_id = Some(mission_id);
        self.mission = None;
        self.crew = None;
        self.sessions.clear();
        self.events.clear();
        self.runner_statuses.clear();
        self.goal = None;
        self.feed_blocks.clear();
        self.askers_by_question.clear();
        self.resolved_asks.clear();
        self.pending_ask_choices.clear();
        self.expanded_signal_payloads.clear();
        self.submitting_asks.clear();
        self.feed_scroll = ScrollHandle::new();
        self.feed_was_near_bottom = true;
        self.feed_has_new_messages = false;
        self.roster.clear();
        self.composer = ComposerState::default();
        self.composer_posting = false;
        self.composer_anchor = None;
        self.loading = true;
        self.loading_overlay_visible = false;
        self.error = None;
        self.warning = None;
        self.active_tab = MissionTab::Feed;
        self.open_tabs.clear();
        self.delivery_blocked.clear();
        self.transitions.clear();
        self.stopping = false;
        self.resuming = false;
        self.resetting = false;
        self.reset_confirm_open = false;
        self.archiving = false;
        self.secondary = false;
        self.duplicate_dismissed = false;
        self.primary_label = None;
        self.rail_view = rail_view;
        self.rename_modal = None;
        self.session_key_copies.clear();
        self.generation
    }

    fn is_current(&self, mission_id: &str, generation: u64) -> bool {
        self.generation == generation && self.mission_id.as_deref() == Some(mission_id)
    }

    fn all_sessions_live(&self) -> bool {
        !self.sessions.is_empty()
            && self
                .sessions
                .iter()
                .all(|session| session.session.status == SessionStatus::Running)
    }

    fn any_session_live(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| session.session.status == SessionStatus::Running)
    }

    fn archived(&self) -> bool {
        self.mission
            .as_ref()
            .is_some_and(|mission| mission.archived_at.is_some())
    }

    fn lifecycle_busy(&self) -> bool {
        self.stopping || self.resuming || self.resetting || self.archiving
    }

    fn transition_kind(&self, session_id: &str) -> Option<MissionTransitionKind> {
        self.transitions.get(session_id).map(|item| item.kind)
    }

    fn rebuild_event_projection(&mut self) {
        let mut statuses = BTreeMap::new();
        for event in &self.events {
            if event.kind != EventKind::Signal
                || event.signal_type.as_ref().map(|kind| kind.as_str()) != Some("runner_status")
            {
                continue;
            }
            let state = event
                .payload
                .get("state")
                .and_then(serde_json::Value::as_str);
            let state = match state {
                Some("busy") => Some(SessionActivityState::Busy),
                Some("idle") => Some(SessionActivityState::Idle),
                _ => None,
            };
            if let Some(state) = state {
                statuses.insert(event.from.clone(), state);
            }
        }
        self.runner_statuses = statuses;
        self.feed_blocks = group_feed_blocks(&self.events);
        let asks = project_asks(&self.events);
        self.askers_by_question = asks.askers_by_question;
        self.resolved_asks = asks.resolved_asks;
        self.pending_ask_choices
            .retain(|question_id, _| !self.resolved_asks.contains_key(question_id));
        self.goal = if self.events.is_empty() {
            None
        } else {
            Some(
                self.events
                    .iter()
                    .find_map(|event| {
                        (event.kind == EventKind::Signal
                            && event.signal_type.as_ref().map(|kind| kind.as_str())
                                == Some("mission_goal"))
                        .then(|| {
                            event
                                .payload
                                .get("text")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_owned()
                        })
                    })
                    .unwrap_or_default(),
            )
        };
    }

    fn runner_statuses(&self) -> &BTreeMap<String, SessionActivityState> {
        &self.runner_statuses
    }

    fn goal(&self) -> Option<String> {
        self.goal.clone()
    }

    fn feed_is_near_bottom(&self) -> bool {
        let maximum = f32::from(self.feed_scroll.max_offset().height).max(0.);
        let position = (-f32::from(self.feed_scroll.offset().y)).clamp(0., maximum);
        maximum - position < 80.
    }

    fn handle_feed_append(&mut self, event: &Event) {
        self.feed_was_near_bottom = self.feed_is_near_bottom();
        if is_human_authored(event) || self.feed_was_near_bottom {
            self.feed_scroll.scroll_to_bottom();
            self.feed_was_near_bottom = true;
            self.feed_has_new_messages = false;
        } else {
            self.feed_has_new_messages = true;
        }
    }

    pub(crate) fn session_ids(&self) -> Vec<String> {
        self.sessions
            .iter()
            .map(|session| session.session.id.clone())
            .collect()
    }
}

#[derive(Clone)]
struct MissionLoadResult {
    mission: Mission,
    crew: Option<Crew>,
    roster: Vec<SlotWithRunner>,
    sessions: Vec<SessionRow>,
    events: Vec<Event>,
}

#[derive(Clone)]
struct MissionRailResizeDrag;

impl Render for MissionRailResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

impl NativeRoot {
    pub(crate) fn open_mission(
        &mut self,
        mission_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&self.route, AppRoute::Mission(active) if active == &mission_id)
            && (self.mission_workspace.loading || self.mission_workspace.mission.is_some())
        {
            return;
        }
        self.dismiss_sidebar_transients(cx);
        self.route = AppRoute::Mission(mission_id.clone());
        self.core
            .windows
            .set_subjects("main", vec![Subject::Mission(mission_id.clone())]);
        if window.is_window_active() {
            self.core.windows.mark_focused("main");
        } else {
            self.core.windows.mark_blurred("main");
        }
        self.core.broadcast_focus_map();
        let generation = self.mission_workspace.reset(
            mission_id.clone(),
            MissionRailView::from_setting(&self.settings.mission_rail_view),
        );
        self.mission_workspace
            .composer_input
            .update(cx, |input, input_cx| {
                input.reset("", input_cx);
                input.set_disabled(false, input_cx);
            });
        self.mission_workspace
            .mission_id_copy
            .update(cx, |copy, copy_cx| {
                copy.set_value(Some(mission_id.clone()), copy_cx)
            });
        window.focus(&self.root_focus);
        cx.notify();

        let loading_id = mission_id.clone();
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.mission_workspace.is_current(&loading_id, generation)
                    && this.mission_workspace.loading
                {
                    this.mission_workspace.loading_overlay_visible = true;
                    cx.notify();
                }
            });
        })
        .detach();

        let ticker_id = mission_id.clone();
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let keep_running = weak
                .update(cx, |this, cx| {
                    let current = this.mission_workspace.is_current(&ticker_id, generation);
                    if current {
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !keep_running {
                break;
            }
        })
        .detach();

        let core = self.core.clone();
        let load_id = mission_id.clone();
        let load = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_attach(&core, &load_id)
                .await
                .map_err(|error| error.to_string())?;
            let mission = runner_backend::ops::mission::mission_get(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let events = runner_backend::ops::mission::mission_events_replay(&core, &load_id)
                .map_err(|error| error.to_string())?;
            let crew = runner_backend::ops::crew::crew_get(&core, &mission.crew_id).ok();
            let roster =
                runner_backend::ops::slot::slot_list(&core, &mission.crew_id).unwrap_or_default();
            Ok::<_, String>(MissionLoadResult {
                mission,
                crew,
                roster,
                sessions,
                events,
            })
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = load.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.mission_workspace.is_current(&mission_id, generation)
                    || !matches!(
                        &this.route,
                        AppRoute::Mission(active) if active == &mission_id
                    )
                {
                    return;
                }
                match result {
                    Ok(mut loaded) => {
                        let mut seen = loaded
                            .events
                            .iter()
                            .map(|event| event.id.clone())
                            .collect::<HashSet<_>>();
                        for event in std::mem::take(&mut this.mission_workspace.events) {
                            if seen.insert(event.id.clone()) {
                                loaded.events.push(event);
                            }
                        }
                        loaded.events.sort_by(|a, b| a.id.cmp(&b.id));
                        let archived = loaded.mission.archived_at.is_some();
                        let valid_ids = loaded
                            .sessions
                            .iter()
                            .map(|session| session.session.id.clone())
                            .collect::<HashSet<_>>();
                        let remembered = this
                            .settings
                            .last_mission_terminal_ids
                            .get(&mission_id)
                            .filter(|session_id| valid_ids.contains(*session_id))
                            .cloned();
                        let workspace = &mut this.mission_workspace;
                        workspace.mission = Some(loaded.mission);
                        workspace.crew = loaded.crew;
                        workspace.roster = loaded.roster;
                        workspace.sessions = loaded.sessions;
                        workspace.events = loaded.events;
                        workspace.rebuild_event_projection();
                        workspace.feed_scroll.scroll_to_bottom();
                        workspace.loading = false;
                        workspace.error = None;
                        if archived {
                            workspace.active_tab = MissionTab::Feed;
                            workspace.open_tabs.clear();
                            this.settings.last_mission_terminal_ids.remove(&mission_id);
                            this.save_settings();
                        } else {
                            workspace.open_tabs = workspace
                                .sessions
                                .iter()
                                .map(|session| session.session.id.clone())
                                .collect();
                            workspace.active_tab = remembered
                                .map(MissionTab::Session)
                                .unwrap_or(MissionTab::Feed);
                        }
                        this.sync_mission_copy_entities(cx);
                        this.sync_mission_subject_ownership(window, cx);
                        if !this.mission_workspace.secondary {
                            if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                                this.mission_workspace.error = Some(error.to_string());
                            }
                        }
                        this.focus_active_mission_terminal(window);
                    }
                    Err(error) => {
                        this.mission_workspace.loading = false;
                        this.mission_workspace.error =
                            Some(action_failure("load the mission", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_mission_copy_entities(&mut self, cx: &mut Context<Self>) {
        let sessions = self.mission_workspace.sessions.clone();
        let valid = sessions
            .iter()
            .map(|session| session.session.id.clone())
            .collect::<HashSet<_>>();
        self.mission_workspace
            .session_key_copies
            .retain(|session_id, _| valid.contains(session_id));
        for session in sessions {
            let label = format!("Copy @{} session_key", session.handle);
            let value = session.agent_session_key.clone();
            let copy = self
                .mission_workspace
                .session_key_copies
                .entry(session.session.id.clone())
                .or_insert_with(|| {
                    cx.new(|copy_cx| {
                        CopyValueButton::new(copy_cx.focus_handle(), value.clone(), label)
                    })
                })
                .clone();
            copy.update(cx, |copy, copy_cx| copy.set_value(value, copy_cx));
        }
    }

    pub(crate) fn estimated_mission_terminal_size(&self, window: &Window) -> (u16, u16) {
        let bounds = window.bounds().size;
        let sidebar_width = if self.settings.sidebar_collapsed {
            0.
        } else {
            self.settings.sidebar_width * self.settings.app_zoom
        };
        let rail_width = if self.settings.mission_rail_open {
            self.settings.mission_rail_width * self.settings.app_zoom
        } else {
            0.
        };
        let width = (f32::from(bounds.width) - sidebar_width - rail_width - 16.).max(200.);
        let height = (f32::from(bounds.height)
            - (WORKSPACE_HEADER_HEIGHT + WORKSPACE_TABS_HEIGHT + 24.) * self.settings.app_zoom)
            .max(160.);
        let font_size = self.settings.terminal_font_size as f32 * self.settings.app_zoom;
        let cell_width = font_size * 0.6;
        let line_height = (font_size * terminal_element::LINE_HEIGHT_FACTOR).round();
        (
            (width / cell_width).floor().max(2.) as u16,
            (height / line_height).floor().max(2.) as u16,
        )
    }

    pub(crate) fn current_mission_terminal_size(&self, window: &Window) -> (u16, u16) {
        match &self.mission_workspace.active_tab {
            MissionTab::Session(session_id) => self
                .attached
                .get(session_id)
                .map(|chat| chat.terminal.size())
                .unwrap_or_else(|| self.estimated_mission_terminal_size(window)),
            MissionTab::Feed => self.estimated_mission_terminal_size(window),
        }
    }

    pub(crate) fn sync_mission_grid_hint(&self, window: &Window) {
        let (cols, rows) = if matches!(&self.route, AppRoute::Mission(_)) {
            self.current_mission_terminal_size(window)
        } else {
            self.estimated_mission_terminal_size(window)
        };
        if let Err(error) =
            runner_backend::ops::mission::mission_grid_hint_set(&self.core, cols, rows)
        {
            eprintln!("Runner mission grid hint update failed for {cols}x{rows}: {error}");
        }
    }

    fn ensure_mission_terminals_attached(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.mission_workspace.archived() || self.mission_workspace.secondary {
            return Ok(());
        }
        let size = self.current_mission_terminal_size(window);
        let mut errors = Vec::new();
        for session in self.mission_workspace.sessions.clone() {
            if !self
                .mission_workspace
                .open_tabs
                .contains(&session.session.id)
            {
                continue;
            }
            if let Err(error) = self.ensure_mission_terminal_attached(&session, size, window, cx) {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("\n"))
        }
    }

    fn ensure_mission_terminal_attached(
        &mut self,
        session: &SessionRow,
        size: (u16, u16),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let session_id = session.session.id.clone();
        if self.attached.contains_key(&session_id) {
            return Ok(());
        }
        // Mission delivery can wait on the draft gate, so PTY input must never run on GPUI's
        // render thread. The queued mode preserves input order on a per-session worker.
        let terminal = TerminalSession::attach_with_input_mode(
            self.core.clone(),
            session_id.clone(),
            size.0,
            size.1,
            Arc::clone(&self.waker),
            UserInputMode::Queued,
        )?;
        terminal.set_palette(self.settings.terminal_theme.palette());
        terminal.configure(
            self.settings.terminal_scrollback,
            match self.settings.terminal_cursor_style {
                app_settings::TerminalCursorStyle::Block => {
                    alacritty_terminal::vte::ansi::CursorShape::Block
                }
                app_settings::TerminalCursorStyle::Underline => {
                    alacritty_terminal::vte::ansi::CursorShape::Underline
                }
                app_settings::TerminalCursorStyle::Bar => {
                    alacritty_terminal::vte::ansi::CursorShape::Beam
                }
            },
        );
        self.bridge.attach(Arc::clone(&terminal))?;
        let terminal_scrollbar = cx.new(|_| Scrollbar::terminal(Arc::clone(&terminal)));
        let terminal_focus = cx.focus_handle();
        let terminal_input = cx.new(|_| TerminalInput::new(Arc::clone(&terminal)));
        let input_session_id = session_id.clone();
        let terminal_input_subscription = cx.observe(&terminal_input, move |this, input, cx| {
            if let Some(Err(error)) = input.update(cx, |input, _| input.take_write_result()) {
                let visible = matches!(&this.route, AppRoute::Mission(_))
                    && this
                        .mission_workspace
                        .sessions
                        .iter()
                        .any(|session| session.session.id == input_session_id);
                if visible {
                    this.mission_workspace.error = Some(error);
                }
            }
            cx.notify();
        });
        let terminal_input_on_focus_out = terminal_input.clone();
        let terminal_focus_subscription =
            cx.on_focus_out(&terminal_focus, window, move |_, _, window, cx| {
                terminal_input_on_focus_out.update(cx, |input, input_cx| {
                    if input.cancel_composition() {
                        window.invalidate_character_coordinates();
                        input_cx.notify();
                    }
                });
            });
        let baseline = terminal.output_activity().last_seq;
        self.attached.insert(
            session_id.clone(),
            AttachedChat {
                terminal,
                terminal_scrollbar,
                terminal_input,
                _terminal_input_subscription: terminal_input_subscription,
                _terminal_focus_subscription: terminal_focus_subscription,
                terminal_focus,
                scroll_accumulator: 0.,
            },
        );
        let fresh = session.session.status == SessionStatus::Running
            && session.session.started_at.is_some_and(|started_at| {
                Utc::now()
                    .signed_duration_since(started_at)
                    .num_seconds()
                    .abs()
                    <= 10
            });
        if fresh {
            self.begin_mission_transition(
                &session_id,
                MissionTransitionKind::Starting,
                Some(baseline.saturating_sub(1)),
                window,
                cx,
            );
        }
        Ok(())
    }

    fn begin_mission_transition(
        &mut self,
        session_id: &str,
        kind: MissionTransitionKind,
        baseline_seq: Option<u64>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mission_workspace.next_transition_generation += 1;
        let generation = self.mission_workspace.next_transition_generation;
        let baseline_seq = baseline_seq.unwrap_or_else(|| {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.output_activity().last_seq)
                .unwrap_or(0)
        });
        self.mission_workspace.transitions.insert(
            session_id.to_owned(),
            MissionTransition {
                kind,
                started_at: Instant::now(),
                baseline_seq,
                generation,
            },
        );
        let tracked_id = session_id.to_owned();
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let done = weak
                .update(cx, |this, cx| {
                    let Some(transition) =
                        this.mission_workspace.transitions.get(&tracked_id).copied()
                    else {
                        return true;
                    };
                    if transition.generation != generation {
                        return true;
                    }
                    let now = Instant::now();
                    let activity = this
                        .attached
                        .get(&tracked_id)
                        .map(|chat| chat.terminal.output_activity());
                    let settled = chat_lifecycle::transition_should_settle(
                        match transition.kind {
                            MissionTransitionKind::Starting => {
                                chat_lifecycle::TransitionKind::Starting
                            }
                            MissionTransitionKind::Resuming => {
                                chat_lifecycle::TransitionKind::Resuming
                            }
                        },
                        now.saturating_duration_since(transition.started_at),
                        activity.is_some_and(|activity| {
                            activity.tui_ready_seq > transition.baseline_seq
                        }),
                        activity
                            .is_some_and(|activity| activity.last_seq > transition.baseline_seq),
                        activity
                            .and_then(|activity| activity.last_output_at)
                            .map(|last| now.saturating_duration_since(last)),
                    );
                    if settled {
                        this.mission_workspace.transitions.remove(&tracked_id);
                        cx.notify();
                    }
                    settled
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn sync_mission_subject_ownership(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let AppRoute::Mission(mission_id) = &self.route else {
            return;
        };
        let primary = self
            .core
            .windows
            .primary_for(&Subject::Mission(mission_id.clone()));
        let secondary = primary.as_deref().is_some_and(|label| label != "main");
        if secondary != self.mission_workspace.secondary {
            self.mission_workspace.secondary = secondary;
            self.mission_workspace.duplicate_dismissed = false;
            self.mission_workspace.primary_label = primary;
            if secondary {
                window.focus(&self.root_focus);
            } else if let Err(error) = self.ensure_mission_terminals_attached(window, cx) {
                self.mission_workspace.error = Some(error.to_string());
            }
            cx.notify();
        } else {
            self.mission_workspace.primary_label = primary;
        }
    }

    fn focus_active_mission_terminal(&self, window: &mut Window) {
        if self.mission_workspace.rename_modal.is_some()
            || self.mission_workspace.reset_confirm_open
        {
            return;
        }
        let MissionTab::Session(session_id) = &self.mission_workspace.active_tab else {
            window.focus(&self.root_focus);
            return;
        };
        if self.mission_terminal_interactive(session_id) {
            if let Some(chat) = self.attached.get(session_id) {
                chat.terminal_focus.focus(window);
                return;
            }
        }
        window.focus(&self.root_focus);
    }

    fn mission_terminal_interactive(&self, session_id: &str) -> bool {
        matches!(&self.route, AppRoute::Mission(_))
            && !self.mission_workspace.secondary
            && !self.mission_workspace.archiving
            && self
                .mission_workspace
                .sessions
                .iter()
                .find(|session| session.session.id == session_id)
                .is_some_and(|session| {
                    session.session.status == SessionStatus::Running
                        && self.mission_workspace.transition_kind(session_id).is_none()
                })
    }

    pub(crate) fn handle_mission_workspace_event(
        &mut self,
        event: runner_backend::events::AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let AppRoute::Mission(mission_id) = &self.route else {
            return;
        };
        let mission_id = mission_id.clone();
        match event.name {
            "event/appended" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(mission_id.as_str())
                {
                    return;
                }
                let Some(value) = event.payload.get("event") else {
                    return;
                };
                let Ok(appended) = serde_json::from_value::<Event>(value.clone()) else {
                    return;
                };
                if !self
                    .mission_workspace
                    .events
                    .iter()
                    .any(|existing| existing.id == appended.id)
                {
                    self.mission_workspace.handle_feed_append(&appended);
                    self.mission_workspace.events.push(appended);
                    self.mission_workspace
                        .events
                        .sort_by(|a, b| a.id.cmp(&b.id));
                    self.mission_workspace.rebuild_event_projection();
                    cx.notify();
                }
            }
            "router/delivery-blocked" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    != Some(mission_id.as_str())
                {
                    return;
                }
                let Some(session_id) = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                else {
                    return;
                };
                let blocked = event
                    .payload
                    .get("blocked")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let unread_count = event
                    .payload
                    .get("unread_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                if blocked && unread_count > 0 {
                    self.mission_workspace
                        .delivery_blocked
                        .insert(session_id, DeliveryBlocked { unread_count });
                } else {
                    self.mission_workspace.delivery_blocked.remove(&session_id);
                }
                cx.notify();
            }
            "session/warning" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(mission_id.as_str())
                {
                    self.mission_workspace.warning = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    cx.notify();
                }
            }
            "session/input-error" => {
                let relevant = event
                    .payload
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|session_id| {
                        self.mission_workspace
                            .sessions
                            .iter()
                            .any(|session| session.session.id == session_id)
                    });
                if relevant {
                    self.mission_workspace.error = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(|message| action_failure("write to the terminal", message));
                    cx.notify();
                }
            }
            "session/exit" | "session/updated" => {
                if event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(mission_id.as_str())
                {
                    if event.name == "session/exit" {
                        if let Some(session_id) = event
                            .payload
                            .get("session_id")
                            .and_then(serde_json::Value::as_str)
                        {
                            self.mission_workspace.delivery_blocked.remove(session_id);
                            self.mission_workspace.transitions.remove(session_id);
                        }
                    }
                    self.refresh_open_mission(window, cx);
                }
            }
            "mission/changed" => {
                let relevant = event
                    .payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|changed| changed == mission_id);
                if relevant {
                    self.refresh_open_mission(window, cx);
                }
            }
            "mission/resync" => self.resync_mission_events(cx),
            "window_focus_map" => self.sync_mission_subject_ownership(window, cx),
            _ => {}
        }
    }

    fn resync_mission_events(&mut self, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        let generation = self.mission_workspace.generation;
        self.mission_workspace.event_resync_generation += 1;
        let resync_generation = self.mission_workspace.event_resync_generation;
        let core = self.core.clone();
        let resync_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_events_replay(&core, &resync_id)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if !this.mission_workspace.is_current(&mission_id, generation)
                    || this.mission_workspace.event_resync_generation != resync_generation
                {
                    return;
                }
                match result {
                    Ok(replayed) => {
                        let old_tail = this
                            .mission_workspace
                            .events
                            .last()
                            .map(|event| event.id.clone());
                        let mut seen = this
                            .mission_workspace
                            .events
                            .iter()
                            .map(|event| event.id.clone())
                            .collect::<HashSet<_>>();
                        for event in replayed {
                            if seen.insert(event.id.clone()) {
                                this.mission_workspace.events.push(event);
                            }
                        }
                        this.mission_workspace
                            .events
                            .sort_by(|a, b| a.id.cmp(&b.id));
                        let new_tail = this.mission_workspace.events.last().cloned();
                        if new_tail.as_ref().map(|event| &event.id) != old_tail.as_ref() {
                            if let Some(tail) = new_tail.as_ref() {
                                this.mission_workspace.handle_feed_append(tail);
                            }
                        }
                        this.mission_workspace.rebuild_event_projection();
                    }
                    Err(error) => {
                        this.mission_workspace.warning =
                            Some(action_failure("resync the mission feed", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        let generation = self.mission_workspace.generation;
        self.mission_workspace.refresh_generation += 1;
        let refresh_generation = self.mission_workspace.refresh_generation;
        let core = self.core.clone();
        let refresh_id = mission_id.clone();
        let refresh = cx.background_spawn(async move {
            let mission = runner_backend::ops::mission::mission_get(&core, &refresh_id)
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &refresh_id)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((mission, sessions))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = refresh.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.mission_workspace.is_current(&mission_id, generation)
                    || this.mission_workspace.refresh_generation != refresh_generation
                {
                    return;
                }
                match result {
                    Ok((mission, sessions)) => {
                        let archived = mission.archived_at.is_some();
                        let valid_ids = sessions
                            .iter()
                            .map(|session| session.session.id.clone())
                            .collect::<HashSet<_>>();
                        let workspace = &mut this.mission_workspace;
                        workspace.mission = Some(mission);
                        workspace.sessions = sessions;
                        workspace
                            .delivery_blocked
                            .retain(|session_id, _| valid_ids.contains(session_id));
                        workspace
                            .transitions
                            .retain(|session_id, _| valid_ids.contains(session_id));
                        workspace
                            .open_tabs
                            .retain(|session_id| valid_ids.contains(session_id));
                        if archived {
                            workspace.active_tab = MissionTab::Feed;
                            workspace.open_tabs.clear();
                            this.settings.last_mission_terminal_ids.remove(&mission_id);
                            this.save_settings();
                        } else if matches!(
                            &workspace.active_tab,
                            MissionTab::Session(session_id) if !valid_ids.contains(session_id)
                        ) {
                            workspace.active_tab = MissionTab::Feed;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.mission_workspace.error = Some(error.to_string());
                        }
                    }
                    Err(error) => this.mission_workspace.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn configure_mission_action_menu(&mut self, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_workspace.mission.as_ref() else {
            return;
        };
        let busy = self.mission_workspace.lifecycle_busy() || self.mission_workspace.secondary;
        let pinned = mission.pinned_at.is_some();
        self.mission_workspace.menu_actions = vec![
            MissionMenuAction::Pin,
            MissionMenuAction::Rename,
            MissionMenuAction::Reset,
            MissionMenuAction::Archive,
        ];
        self.mission_workspace
            .action_menu
            .update(cx, |menu, menu_cx| {
                menu.set_items(
                    vec![
                        UiMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                            .icon(if pinned { "pin-off.svg" } else { "pin.svg" })
                            .disabled(busy),
                        UiMenuItem::new("Rename")
                            .icon("square-pen.svg")
                            .disabled(busy),
                        UiMenuItem::new("Reset")
                            .icon("rotate-ccw.svg")
                            .disabled(busy || mission.status != MissionStatus::Running),
                        UiMenuItem::new("Archive")
                            .icon("archive.svg")
                            .destructive(true)
                            .disabled(busy),
                    ],
                    menu_cx,
                )
            });
    }

    fn handle_mission_menu_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.mission_workspace.menu_actions.get(index).cloned() else {
            return;
        };
        match action {
            MissionMenuAction::Pin => self.toggle_mission_pin(window, cx),
            MissionMenuAction::Rename => self.open_mission_rename(window, cx),
            MissionMenuAction::Reset => {
                if !self.mission_workspace.lifecycle_busy()
                    && self
                        .mission_workspace
                        .mission
                        .as_ref()
                        .is_some_and(|mission| mission.status == MissionStatus::Running)
                {
                    self.mission_workspace.reset_confirm_open = true;
                    self.mission_workspace.reset_cancel_focus.focus(window);
                    cx.notify();
                }
            }
            MissionMenuAction::Archive => self.archive_open_mission(window, cx),
        }
    }

    fn toggle_mission_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_workspace.mission.clone() else {
            return;
        };
        let mission_id = mission.id.clone();
        let core = self.core.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_pin_impl(
                &core,
                mission.id,
                mission.pinned_at.is_none(),
            )
            .await
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                if this.mission_workspace.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission_workspace.mission = Some(mission);
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                        this.core.events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                    }
                    Err(error) => {
                        this.mission_workspace.error =
                            Some(action_failure("update the mission pin", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self.mission_workspace.lifecycle_busy() {
            return;
        }
        self.mission_workspace.stopping = true;
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core.clone();
        let stop_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mission = runner_backend::ops::mission::mission_stop_impl(&core, stop_id.clone())
                .await
                .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &stop_id)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((mission, sessions))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_workspace.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.mission_workspace.stopping = false;
                match result {
                    Ok((mission, sessions)) => {
                        this.mission_workspace.mission = Some(mission);
                        this.mission_workspace.sessions = sessions;
                        this.mission_workspace.transitions.clear();
                        this.sync_mission_copy_entities(cx);
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                    }
                    Err(error) => {
                        this.mission_workspace.error =
                            Some(action_failure("stop the mission", error));
                    }
                }
                this.focus_active_mission_terminal(window);
                cx.notify();
            });
        })
        .detach();
    }

    fn resume_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self.mission_workspace.lifecycle_busy() {
            return;
        }
        let stopped = self
            .mission_workspace
            .sessions
            .iter()
            .filter(|session| session.session.status != SessionStatus::Running)
            .map(|session| session.session.id.clone())
            .collect::<Vec<_>>();
        if stopped.is_empty() {
            return;
        }
        let size = self.current_mission_terminal_size(window);
        self.mission_workspace.resuming = true;
        for session_id in &stopped {
            self.begin_mission_transition(
                session_id,
                MissionTransitionKind::Resuming,
                None,
                window,
                cx,
            );
        }
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core.clone();
        let resume_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mut sessions = runner_backend::ops::session::session_list(&core, &resume_id)
                .map_err(|error| error.to_string())?;
            let mut first_error = None;
            for candidate in sessions.clone() {
                let already_running = sessions
                    .iter()
                    .find(|session| session.session.id == candidate.session.id)
                    .is_some_and(|session| session.session.status == SessionStatus::Running);
                if already_running {
                    continue;
                }
                if let Err(error) = runner_backend::ops::session::session_resume(
                    &core,
                    &candidate.session.id,
                    Some(size.0),
                    Some(size.1),
                ) {
                    let message = error.to_string();
                    if is_concurrent_resume_error(&message) {
                        match runner_backend::ops::session::session_list(&core, &resume_id) {
                            Ok(refreshed) => sessions = refreshed,
                            Err(error) => {
                                first_error.get_or_insert_with(|| error.to_string());
                            }
                        }
                    } else {
                        first_error.get_or_insert(message);
                    }
                }
            }
            match runner_backend::ops::session::session_list(&core, &resume_id) {
                Ok(refreshed) => sessions = refreshed,
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
            Ok::<_, String>((sessions, first_error))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_workspace.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                this.mission_workspace.resuming = false;
                match result {
                    Ok((sessions, error)) => {
                        this.mission_workspace.sessions = sessions;
                        for session in &this.mission_workspace.sessions {
                            if !this
                                .mission_workspace
                                .open_tabs
                                .contains(&session.session.id)
                            {
                                this.mission_workspace
                                    .open_tabs
                                    .push(session.session.id.clone());
                            }
                        }
                        if let Some(error) = error {
                            this.mission_workspace.error =
                                Some(action_failure("resume the mission", error));
                        } else {
                            this.mission_workspace.error = None;
                        }
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.mission_workspace.error = Some(error.to_string());
                        }
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                    }
                    Err(error) => {
                        this.mission_workspace.error =
                            Some(action_failure("resume the mission", error));
                    }
                }
                this.focus_active_mission_terminal(window);
                cx.notify();
            });
        })
        .detach();
    }

    fn archive_open_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self.mission_workspace.lifecycle_busy() {
            return;
        }
        self.mission_workspace.archiving = true;
        self.sidebar_archiving_missions.insert(mission_id.clone());
        self.root_focus.focus(window);
        cx.notify();
        let core = self.core.clone();
        let archive_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_archive_impl(&core, archive_id)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.sidebar_archiving_missions.remove(&mission_id);
                if this.mission_workspace.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(_) => {
                        this.settings.last_mission_terminal_ids.remove(&mission_id);
                        this.save_settings();
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                        this.core.events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                        this.open_runners(window, cx);
                    }
                    Err(error) => {
                        this.mission_workspace.archiving = false;
                        this.mission_workspace.error =
                            Some(action_failure("archive the mission", error));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn open_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission) = self.mission_workspace.mission.as_ref() else {
            return;
        };
        let original = mission.title.clone();
        let input = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                original.clone(),
                "Mission name",
                false,
            )
            .text_size(13.)
        });
        input.update(cx, |input, input_cx| input.select_all(input_cx));
        let input_focus = input.read(cx).focus_handle();
        self.mission_workspace.rename_modal = Some(MissionRenameModal {
            mission_id: mission.id.clone(),
            original: mission.title.clone(),
            input,
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
        });
        input_focus.focus(window);
        cx.notify();
    }

    fn close_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .mission_workspace
            .rename_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.mission_workspace.rename_modal = None;
        self.focus_active_mission_terminal(window);
        cx.notify();
    }

    fn submit_mission_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.mission_workspace.rename_modal.as_mut() else {
            return;
        };
        if modal.submitting || modal.input.read(cx).is_composing() {
            return;
        }
        let title = modal.input.read(cx).text().trim().to_owned();
        if title.is_empty() || title == modal.original.trim() {
            self.close_mission_rename(window, cx);
            return;
        }
        modal.submitting = true;
        modal.error = None;
        let mission_id = modal.mission_id.clone();
        cx.notify();
        let core = self.core.clone();
        let rename_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_rename_impl(&core, rename_id, title)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.mission_workspace.mission_id.as_deref() != Some(mission_id.as_str()) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.mission_workspace.mission = Some(mission);
                        this.mission_workspace.rename_modal = None;
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                        this.core.events.emit(
                            "mission/changed",
                            &serde_json::json!({ "mission_id": mission_id }),
                        );
                        this.focus_active_mission_terminal(window);
                    }
                    Err(error) => {
                        if let Some(modal) = this.mission_workspace.rename_modal.as_mut() {
                            modal.submitting = false;
                            modal.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_mission_rename_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self
                .mission_workspace
                .rename_modal
                .as_ref()
                .is_some_and(|modal| !modal.input.read(cx).is_composing())
        {
            cx.stop_propagation();
            self.submit_mission_rename(window, cx);
        }
    }

    fn close_mission_reset_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mission_workspace.resetting {
            return;
        }
        self.mission_workspace.reset_confirm_open = false;
        self.focus_active_mission_terminal(window);
        cx.notify();
    }

    fn submit_mission_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self.mission_workspace.resetting {
            return;
        }
        self.mission_workspace.resetting = true;
        self.mission_workspace.error = None;
        let generation = self.mission_workspace.generation;
        let size = self.current_mission_terminal_size(window);
        let core = self.core.clone();
        let reset_id = mission_id.clone();
        let task = cx.background_spawn(async move {
            let mission = runner_backend::ops::mission::mission_reset_impl_with_size(
                &core,
                reset_id.clone(),
                Some(size),
            )
            .await
            .map_err(|error| error.to_string())?;
            let sessions = runner_backend::ops::session::session_list(&core, &reset_id)
                .map_err(|error| error.to_string())?;
            let events = runner_backend::ops::mission::mission_events_replay(&core, &reset_id)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((mission, sessions, events))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.mission_workspace.is_current(&mission_id, generation) {
                    return;
                }
                this.mission_workspace.resetting = false;
                match result {
                    Ok((mission, sessions, events)) => {
                        let old_ids = this.mission_workspace.session_ids();
                        for session_id in old_ids {
                            this.attached.remove(&session_id);
                        }
                        this.mission_workspace.mission = Some(mission);
                        this.mission_workspace.sessions = sessions;
                        this.mission_workspace.events = events;
                        this.mission_workspace.rebuild_event_projection();
                        this.mission_workspace.feed_scroll.scroll_to_bottom();
                        this.mission_workspace.feed_was_near_bottom = true;
                        this.mission_workspace.feed_has_new_messages = false;
                        this.mission_workspace.delivery_blocked.clear();
                        this.mission_workspace.transitions.clear();
                        this.mission_workspace.open_tabs = this
                            .mission_workspace
                            .sessions
                            .iter()
                            .map(|session| session.session.id.clone())
                            .collect();
                        this.mission_workspace.active_tab = MissionTab::Feed;
                        this.mission_workspace.reset_confirm_open = false;
                        this.settings.last_mission_terminal_ids.remove(&mission_id);
                        this.save_settings();
                        this.sync_mission_copy_entities(cx);
                        if let Err(error) = this.ensure_mission_terminals_attached(window, cx) {
                            this.mission_workspace.error = Some(error.to_string());
                        }
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
                        window.focus(&this.root_focus);
                    }
                    Err(error) => {
                        this.mission_workspace.error =
                            Some(action_failure("reset the mission", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_mission_reset_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .mission_workspace
            .mission
            .as_ref()
            .map(|mission| mission.title.clone())
            .unwrap_or_default();
        let resetting = self.mission_workspace.resetting;
        let root = cx.entity();
        let dismiss_root = root.clone();
        let dismiss_key_root = root.clone();
        let cancel_root = root.clone();
        let confirm_root = root;
        let focus_order = [
            self.mission_workspace.reset_cancel_focus.clone(),
            self.mission_workspace.reset_confirm_focus.clone(),
        ];
        let tab_focus_order = focus_order.clone();
        let consequence = |id: &'static str, text: &'static str| {
            div()
                .id(id)
                .flex()
                .items_start()
                .gap_2()
                .text_size(rems(12. / 16.))
                .line_height(rems(17. / 16.))
                .text_color(theme::muted())
                .child(div().text_color(theme::faint()).child("·"))
                .child(text)
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x0000008c))
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !resetting {
                    dismiss_root.update(cx, |this, cx| {
                        this.close_mission_reset_confirm(window, cx)
                    });
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !resetting => {
                        cx.stop_propagation();
                        dismiss_key_root.update(cx, |this, cx| {
                            this.close_mission_reset_confirm(window, cx)
                        });
                    }
                    "tab" if !resetting => {
                        cx.stop_propagation();
                        let current = tab_focus_order
                            .iter()
                            .position(|handle| handle.is_focused(window));
                        let index = current.map_or(
                            usize::from(event.keystroke.modifiers.shift),
                            |index| 1 - index,
                        );
                        tab_focus_order[index].focus(window);
                    }
                    _ => {}
                }
            })
            .child(
                div()
                    .w_full()
                    .max_w(rems(480. / 16.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .overflow_hidden()
                    .rounded(rems(12. / 16.))
                    .border_2()
                    .border_color(theme::warning())
                    .bg(theme::panel())
                    .shadow_2xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().h(rems(3. / 16.)).w_full().bg(theme::warning()))
                    .child(
                        div()
                            .px_6()
                            .pb_6()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .size(rems(36. / 16.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_full()
                                            .bg(theme::with_alpha(theme::warning(), 0.15))
                                            .child(
                                                svg()
                                                    .path("triangle-alert.svg")
                                                    .size(rems(18. / 16.))
                                                    .text_color(theme::warning()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap(rems(2. / 16.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(rems(1.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Reset mission?"),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(rems(12. / 16.))
                                                    .text_color(theme::faint())
                                                    .child(
                                                        "This wipes the run and starts the crew over.",
                                                    ),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(theme::border())
                                    .bg(theme::bg())
                                    .px_3()
                                    .py_3()
                                    .child(consequence(
                                        "reset-consequence-ptys",
                                        "All slot PTYs are killed and respawned fresh.",
                                    ))
                                    .child(consequence(
                                        "reset-consequence-events",
                                        "The event log is wiped — feed history is lost.",
                                    ))
                                    .child(consequence(
                                        "reset-consequence-conversations",
                                        "Agent conversations are dropped — claude-code starts fresh.",
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::faint())
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .children([
                                                div()
                                                    .font_family("JetBrains Mono")
                                                    .text_color(theme::muted())
                                                    .child(title),
                                                div().child(
                                                    " will keep its title, crew, and slots — just nothing else.",
                                                ),
                                            ]),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("cancel-mission-reset", "Cancel")
                                            .focus_handle(focus_order[0].clone())
                                            .disabled(resetting)
                                            .on_press(move |window, cx| {
                                                cancel_root.update(cx, |this, cx| {
                                                    this.close_mission_reset_confirm(window, cx)
                                                });
                                            }),
                                    )
                                    .child(
                                        Button::new(
                                            "confirm-mission-reset",
                                            if resetting {
                                                "Resetting…"
                                            } else {
                                                "Reset mission"
                                            },
                                        )
                                        .focus_handle(focus_order[1].clone())
                                        .variant(ButtonVariant::Warning)
                                        .icon("rotate-ccw.svg")
                                        .disabled(resetting)
                                        .on_press(move |window, cx| {
                                            confirm_root.update(cx, |this, cx| {
                                                this.submit_mission_reset(window, cx)
                                            });
                                        }),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn action_failure(action: &str, error: impl AsRef<str>) -> String {
    let raw = error.as_ref();
    let detail = raw
        .split_once(": ")
        .filter(|(prefix, _)| {
            prefix
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .map(|(_, detail)| detail)
        .unwrap_or(raw);
    format!("Couldn't {action}: {detail}")
}

impl NativeRoot {
    pub(crate) fn cycle_mission_tab(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.route, AppRoute::Mission(_)) {
            return;
        }
        let mut tabs = vec![MissionTab::Feed];
        if !self.mission_workspace.archived() && !self.mission_workspace.secondary {
            tabs.extend(
                self.mission_workspace
                    .open_tabs
                    .iter()
                    .filter(|session_id| {
                        self.mission_workspace
                            .sessions
                            .iter()
                            .any(|session| session.session.id == session_id.as_str())
                    })
                    .map(|session_id| MissionTab::Session(session_id.clone())),
            );
        }
        let Some(next) =
            mission_tab_in_direction(&tabs, &self.mission_workspace.active_tab, direction)
        else {
            return;
        };
        match next {
            MissionTab::Feed => self.select_mission_feed(window, cx),
            MissionTab::Session(session_id) => {
                self.select_mission_session(&session_id, window, cx);
            }
        }
    }

    fn select_mission_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mission_workspace.active_tab = MissionTab::Feed;
        if self.mission_workspace.feed_was_near_bottom {
            self.mission_workspace.feed_scroll.scroll_to_bottom();
            self.mission_workspace.feed_has_new_messages = false;
        }
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn select_mission_session(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mission_workspace.archived() || self.mission_workspace.secondary {
            return;
        }
        let valid = self
            .mission_workspace
            .sessions
            .iter()
            .any(|session| session.session.id == session_id);
        if !valid {
            self.select_mission_feed(window, cx);
            return;
        }
        if !self
            .mission_workspace
            .open_tabs
            .iter()
            .any(|open| open == session_id)
        {
            self.mission_workspace.open_tabs.push(session_id.to_owned());
        }
        if let Some(mission_id) = &self.mission_workspace.mission_id {
            self.settings
                .last_mission_terminal_ids
                .insert(mission_id.clone(), session_id.to_owned());
            self.save_settings();
        }
        self.mission_workspace.active_tab = MissionTab::Session(session_id.to_owned());
        if let Err(error) = self.ensure_mission_terminals_attached(window, cx) {
            self.mission_workspace.error = Some(error.to_string());
        }
        self.focus_active_mission_terminal(window);
        cx.notify();
    }

    fn close_mission_session_tab(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mission_workspace
            .open_tabs
            .retain(|open| open != session_id);
        if let Some(mission_id) = &self.mission_workspace.mission_id {
            if self
                .settings
                .last_mission_terminal_ids
                .get(mission_id)
                .is_some_and(|remembered| remembered == session_id)
            {
                self.settings.last_mission_terminal_ids.remove(mission_id);
                self.save_settings();
            }
        }
        if self.mission_workspace.active_tab == MissionTab::Session(session_id.to_owned()) {
            self.mission_workspace.active_tab = MissionTab::Feed;
            window.focus(&self.root_focus);
        }
        self.attached.remove(session_id);
        cx.notify();
    }

    fn on_mission_key_down(
        &mut self,
        session_id: &str,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        let keystroke = &event.keystroke;
        if runner_app::terminal_ime::terminal_key_route(
            chat.terminal_input.read(cx).is_composing(),
            keystroke.modifiers.platform,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.function,
            &keystroke.key,
        ) != runner_app::terminal_ime::TerminalKeyRoute::Raw
        {
            return;
        }
        match chat.terminal.send_key(
            &keystroke.key,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                chat.terminal.scroll_to_bottom();
                self.mission_workspace.error = None;
                cx.stop_propagation();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.mission_workspace.error = Some(error.to_string());
                cx.stop_propagation();
                cx.notify();
            }
        }
    }

    fn on_mission_scroll(
        &mut self,
        session_id: &str,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get_mut(session_id) else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        chat.scroll_accumulator += lines;
        let whole = chat.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            chat.scroll_accumulator -= whole as f32;
            chat.terminal.scroll(whole);
            cx.notify();
        }
    }

    fn on_mission_paste(
        &mut self,
        session_id: &str,
        _: &Paste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        if let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
        }) {
            let Some(terminal) = self
                .attached
                .get(session_id)
                .map(|chat| Arc::clone(&chat.terminal))
            else {
                return;
            };
            let paste = cx.background_spawn(async move {
                runner_backend::ops::session::session_paste_image(
                    image.bytes,
                    image.format.mime_type(),
                )?;
                terminal.write_user_bytes(b"\x16")
            });
            cx.spawn(async move |weak, cx| {
                let result = paste.await;
                let _ = weak.update(cx, |this, cx| {
                    match result {
                        Ok(()) => this.mission_workspace.error = None,
                        Err(error) => this.mission_workspace.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let Some(text) = item.text() else {
            return;
        };
        let Some(chat) = self.attached.get(session_id) else {
            return;
        };
        if let Err(error) = chat.terminal.paste(&text) {
            self.mission_workspace.error = Some(error.to_string());
        }
    }

    fn clear_mission_input(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.attached.get(session_id) else {
            self.mission_workspace.error = Some("Terminal is not attached.".into());
            cx.notify();
            return;
        };
        if let Err(error) = chat.terminal.write_user_bytes(b"\r") {
            self.mission_workspace.error = Some(error.to_string());
        }
        chat.terminal_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn render_mission_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.mission_workspace.mission.is_some() {
            self.configure_mission_action_menu(cx);
        }
        let header = self.render_mission_header(window, cx);
        let notices = self.render_mission_notices(cx);
        let body = if self.mission_workspace.loading {
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .children(self.mission_workspace.loading_overlay_visible.then(|| {
                    SessionOverlay::transition("mission-loading", SessionOverlayKind::Starting)
                        .label("Loading mission…")
                }))
                .into_any_element()
        } else if self.mission_workspace.mission.is_none() {
            self.render_mission_load_error(cx)
        } else {
            self.render_loaded_mission(window, cx)
        };
        let center = div()
            .min_w(px(0.))
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .children(notices)
            .child(body);
        let rail = self.render_mission_rail(cx);
        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .bg(theme::bg())
            .child(center)
            .child(rail)
            .on_drag_move::<MissionRailResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<MissionRailResizeDrag>, _, cx| {
                    let width = f32::from(event.bounds.right() - event.event.position.x)
                        / this.settings.app_zoom;
                    this.settings.mission_rail_width =
                        app_settings::clamp_mission_rail_width(width);
                    cx.notify();
                },
            ))
            .on_drop(cx.listener(|this, _: &MissionRailResizeDrag, _, _| {
                this.save_settings();
            }))
            .into_any_element()
    }

    fn render_mission_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mission = self.mission_workspace.mission.clone();
        let busy = self.mission_workspace.lifecycle_busy();
        let secondary = self.mission_workspace.secondary;
        let all_live = self.mission_workspace.all_sessions_live();
        let any_stopped = !self.mission_workspace.sessions.is_empty() && !all_live;
        let root = cx.entity();
        let resume_root = root.clone();
        let stop_root = root.clone();
        let open_rail_root = root;
        let controls = mission.as_ref().and_then(|mission| {
            (mission.status == MissionStatus::Running && !secondary).then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(
                        (!self.mission_workspace.resuming)
                            .then(|| self.mission_workspace.action_menu.clone()),
                    )
                    .children((any_stopped && !busy).then(|| {
                        SessionControl::new("mission-resume", SessionControlKind::Resume)
                            .variant(SessionControlVariant::Header)
                            .title("Respawn every stopped slot in this mission")
                            .on_press(move |window, cx| {
                                resume_root
                                    .update(cx, |this, cx| this.resume_open_mission(window, cx));
                            })
                    }))
                    .children((all_live && !busy).then(|| {
                        SessionControl::new("mission-stop", SessionControlKind::Stop)
                            .variant(SessionControlVariant::Header)
                            .title("Kill all PTYs; mission stays running so you can Resume")
                            .on_press(move |window, cx| {
                                stop_root.update(cx, |this, cx| this.stop_open_mission(window, cx));
                            })
                    }))
            })
        });
        let row = div()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .flex_none()
            .pl(px(self.workspace_titlebar_padding(window)))
            .pr_4()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .children(self.render_open_sidebar_button(cx))
            .children(self.settings.sidebar_collapsed.then(|| {
                div()
                    .mx_1()
                    .h(rems(20. / 16.))
                    .w(rems(1. / 16.))
                    .bg(theme::border())
            }))
            .child(
                svg()
                    .path("flag.svg")
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(rems(13. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .child(
                        mission
                            .as_ref()
                            .map(|mission| mission.title.clone())
                            .unwrap_or_else(|| "…".into()),
                    ),
            )
            .children(controls)
            .children((!self.settings.mission_rail_open).then(|| {
                div().ml_auto().child(
                    IconButton::new("open-mission-rail", "panel-right-hollow.svg")
                        .tooltip("Open runners panel")
                        .on_press(move |_, cx| {
                            open_rail_root.update(cx, |this, cx| {
                                this.settings.mission_rail_open = true;
                                this.save_settings();
                                cx.notify();
                            });
                        }),
                )
            }));
        self.render_titlebar_drag_area("mission-titlebar-drag", row, cx)
            .into_any_element()
    }

    fn render_mission_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if let Some(error) = self.mission_workspace.error.clone() {
            let root = cx.entity();
            notices.push(
                mission_notice("error", error, theme::danger(), "Dismiss", move |_, cx| {
                    root.update(cx, |this, cx| {
                        this.mission_workspace.error = None;
                        cx.notify();
                    });
                })
                .into_any_element(),
            );
        }
        if let Some(warning) = self.mission_workspace.warning.clone() {
            let root = cx.entity();
            notices.push(
                mission_notice(
                    "warning",
                    warning,
                    theme::warning(),
                    "Dismiss",
                    move |_, cx| {
                        root.update(cx, |this, cx| {
                            this.mission_workspace.warning = None;
                            cx.notify();
                        });
                    },
                )
                .into_any_element(),
            );
        }
        notices
    }

    fn render_mission_load_error(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return div().into_any_element();
        };
        let root = cx.entity();
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .text_center()
                    .child(
                        div()
                            .text_size(rems(14. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Mission unavailable"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::muted())
                            .child("Runner couldn't attach this mission."),
                    )
                    .child(
                        Button::new("retry-mission-load", "Retry")
                            .size(ButtonSize::Sm)
                            .on_press(move |window, cx| {
                                root.update(cx, |this, cx| {
                                    this.open_mission(mission_id.clone(), window, cx)
                                });
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_loaded_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let feed_active = self.mission_workspace.secondary
            || self.mission_workspace.active_tab == MissionTab::Feed;
        let tabs = self.render_mission_tabs(feed_active, cx);
        let pane = if feed_active {
            self.render_mission_feed_surface(window, cx)
        } else {
            match &self.mission_workspace.active_tab {
                MissionTab::Session(session_id) => {
                    let session = self
                        .mission_workspace
                        .sessions
                        .iter()
                        .find(|session| session.session.id == *session_id)
                        .cloned();
                    session
                        .map(|session| self.render_mission_terminal_pane(session, window, cx))
                        .unwrap_or_else(|| self.render_mission_feed_surface(window, cx))
                }
                MissionTab::Feed => self.render_mission_feed_surface(window, cx),
            }
        };
        let mut panes = div()
            .relative()
            .min_h(px(0.))
            .flex_1()
            .overflow_hidden()
            .child(pane);
        if self.mission_workspace.archiving {
            panes = panes.child(SessionOverlay::transition(
                "mission-archiving",
                SessionOverlayKind::Archiving,
            ));
        }
        if self.mission_workspace.secondary && !self.mission_workspace.duplicate_dismissed {
            panes = panes.child(self.render_duplicate_mission_overlay(cx));
        }
        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .child(tabs)
            .child(panes)
            .into_any_element()
    }

    fn render_mission_feed_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mission_running = self
            .mission_workspace
            .mission
            .as_ref()
            .is_some_and(|mission| mission.status == MissionStatus::Running);
        let can_compose = mission_running
            && !self.mission_workspace.archived()
            && !self.mission_workspace.secondary;
        let paused = mission_running
            && !self.mission_workspace.all_sessions_live()
            && !self.mission_workspace.resuming
            && !self.mission_workspace.archiving
            && !self.mission_workspace.secondary;
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(self.render_mission_feed(cx))
            .children(can_compose.then(|| self.render_mission_composer(window, cx)))
            .children(paused.then(|| self.render_mission_paused_overlay(cx)))
            .into_any_element()
    }

    fn render_mission_tabs(&self, feed_active: bool, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let feed_root = root.clone();
        let mut strip = div()
            .h(rems(WORKSPACE_TABS_HEIGHT / 16.))
            .flex_none()
            .px_6()
            .flex()
            .items_end()
            .gap_1()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                mission_tab("mission-feed-tab", "feed", feed_active).on_click(
                    move |_, window, cx| {
                        feed_root.update(cx, |this, cx| this.select_mission_feed(window, cx));
                    },
                ),
            );
        if !self.mission_workspace.archived() && !self.mission_workspace.secondary {
            for session_id in &self.mission_workspace.open_tabs {
                let Some(session) = self
                    .mission_workspace
                    .sessions
                    .iter()
                    .find(|session| &session.session.id == session_id)
                else {
                    continue;
                };
                let active =
                    self.mission_workspace.active_tab == MissionTab::Session(session_id.clone());
                let select_id = session_id.clone();
                let close_id = session_id.clone();
                let select_root = root.clone();
                let close_root = root.clone();
                strip = strip.child(Tooltip::new(
                    SharedString::from(format!("mission-tab-tooltip-{session_id}")),
                    format!("@{}", session.handle),
                    div()
                        .id(SharedString::from(format!("mission-tab-{session_id}")))
                        .relative()
                        .w(rems(WORKSPACE_TAB_WIDTH / 16.))
                        .h(rems(32. / 16.))
                        .flex_none()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_2()
                        .border_color(if active {
                            theme::accent()
                        } else {
                            gpui::transparent_black()
                        })
                        .cursor_pointer()
                        .text_size(rems(12. / 16.))
                        .text_color(if active {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .hover(|tab| tab.text_color(theme::text()))
                        .child(
                            svg()
                                .path("terminal.svg")
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(if active {
                                    theme::text()
                                } else {
                                    theme::muted()
                                }),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .truncate()
                                .font_family("JetBrains Mono")
                                .child(format!("@{}", session.handle)),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "close-mission-tab-{session_id}"
                                )))
                                .group("mission-tab-close")
                                .size(rems(18. / 16.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .text_color(theme::faint())
                                .hover(|button| {
                                    button.bg(theme::raised()).text_color(theme::text())
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    close_root.update(cx, |this, cx| {
                                        this.close_mission_session_tab(&close_id, window, cx)
                                    });
                                })
                                .child(
                                    svg()
                                        .path("close.svg")
                                        .size(rems(10. / 16.))
                                        .text_color(theme::faint())
                                        .group_hover("mission-tab-close", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            select_root.update(cx, |this, cx| {
                                this.select_mission_session(&select_id, window, cx)
                            });
                        }),
                ));
            }
        }
        strip.into_any_element()
    }

    fn render_mission_feed(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.mission_workspace.feed_was_near_bottom = self.mission_workspace.feed_is_near_bottom();
        if self.mission_workspace.feed_was_near_bottom {
            self.mission_workspace.feed_has_new_messages = false;
        }
        let blocks = self.mission_workspace.feed_blocks.clone();
        let rows = if blocks.is_empty() {
            vec![div()
                .px_4()
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .child("No events yet.")
                .into_any_element()]
        } else {
            blocks
                .into_iter()
                .map(|block| self.render_mission_feed_block(block, cx))
                .collect()
        };
        let root = cx.entity();
        let pill_root = root;
        let pane = div()
            .relative()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                div()
                    .id("mission-feed-scroll")
                    .min_h(px(0.))
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.mission_workspace.feed_scroll)
                    .px_6()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap(rems(18. / 16.))
                    .children(rows),
            )
            .children(self.mission_workspace.feed_has_new_messages.then(|| {
                div()
                    .absolute()
                    .bottom_4()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("mission-feed-new-messages")
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(theme::accent())
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::accent_ink())
                            .shadow_md()
                            .cursor_pointer()
                            .hover(|pill| pill.opacity(0.9))
                            .on_click(move |_, _, cx| {
                                pill_root.update(cx, |this, cx| {
                                    this.mission_workspace.feed_scroll.scroll_to_bottom();
                                    this.mission_workspace.feed_was_near_bottom = true;
                                    this.mission_workspace.feed_has_new_messages = false;
                                    cx.notify();
                                });
                            })
                            .child("New messages ↓"),
                    )
            }));
        pane.into_any_element()
    }

    fn mission_composer_roster(&self) -> Vec<ComposerRosterEntry> {
        self.mission_workspace
            .roster
            .iter()
            .map(|member| ComposerRosterEntry {
                handle: member.slot.slot_handle.clone(),
                role: member.runner.handle.clone(),
                runtime: member
                    .slot
                    .runtime_override
                    .clone()
                    .unwrap_or_else(|| member.runner.runtime.clone()),
            })
            .collect()
    }

    fn render_mission_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let roster = self.mission_composer_roster();
        let options = mention_options(&self.mission_workspace.composer, &roster);
        let picker_open = !options.is_empty();
        let active_index = self
            .mission_workspace
            .composer
            .active_index
            .min(options.len().saturating_sub(1));
        let target = self.mission_workspace.composer.target.clone();
        let posting = self.mission_workspace.composer_posting;
        let can_send = !posting && !self.mission_workspace.composer.draft.trim().is_empty();
        let root = cx.entity();
        let anchor_root = root.clone();
        let target_root = root.clone();
        let send_root = root.clone();
        let input = self.mission_workspace.composer_input.clone();
        let mut field = div()
            .id("mission-composer-field")
            .relative()
            .flex()
            .items_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_4()
            .py_3()
            .children(target.map(|target| {
                let clear_root = target_root.clone();
                div()
                    .id("mission-composer-target")
                    .flex_none()
                    .rounded_sm()
                    .bg(theme::with_alpha(theme::accent(), 0.15))
                    .px_1()
                    .py(rems(2. / 16.))
                    .cursor_pointer()
                    .font_family("JetBrains Mono")
                    .text_size(rems(12. / 16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::accent())
                    .on_click(move |_, window, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.clear_mission_composer_target(window, cx)
                        });
                    })
                    .child(format!("@{target}"))
            }))
            .child(div().min_w(px(0.)).flex_1().child(input))
            .child(
                div()
                    .id("mission-composer-send")
                    .flex_none()
                    .rounded_md()
                    .bg(theme::accent())
                    .px_3()
                    .py_1()
                    .opacity(if can_send { 1. } else { 0.5 })
                    .cursor(if can_send {
                        CursorStyle::PointingHand
                    } else {
                        CursorStyle::OperationNotAllowed
                    })
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(rems(12. / 16.))
                    .text_color(theme::accent_ink())
                    .on_click(move |_, window, cx| {
                        send_root.update(cx, |this, cx| {
                            this.post_current_mission_composer(window, cx)
                        });
                    })
                    .child(if posting { "Sending…" } else { "Send" }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        anchor_root.update(cx, |this, _| {
                            this.mission_workspace.composer_anchor = Some(bounds)
                        });
                    },
                )
                .absolute()
                .inset_0(),
            );

        if let (true, Some(anchor)) = (picker_open, self.mission_workspace.composer_anchor) {
            let picker_root = root.clone();
            let rows = options.into_iter().enumerate().map(|(index, entry)| {
                let option_root = picker_root.clone();
                let handle = entry.handle.clone();
                div()
                    .id(("mission-composer-option", index))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(if index == active_index {
                        theme::border_strong()
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(if index == active_index {
                        theme::raised()
                    } else {
                        gpui::transparent_black()
                    })
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|row| row.bg(theme::raised()))
                    .on_click(move |_, window, cx| {
                        option_root.update(cx, |this, cx| {
                            this.select_mission_composer_target(handle.clone(), window, cx)
                        });
                    })
                    .child(
                        div()
                            .flex_none()
                            .font_family("JetBrains Mono")
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::accent())
                            .child(format!("@{}", entry.handle)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(format!("{} · {}", entry.role, entry.runtime)),
                    )
                    .children((index == active_index).then(|| {
                        div()
                            .ml_auto()
                            .font_family("JetBrains Mono")
                            .text_size(rems(10. / 16.))
                            .text_color(theme::faint())
                            .child("↵")
                    }))
            });
            let menu = div()
                .id("mission-composer-roster")
                .relative()
                .max_h(rems(240. / 16.))
                .overflow_hidden()
                .rounded_lg()
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::panel())
                .p_1()
                .shadow_xl()
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .pb(rems(2. / 16.))
                        .text_size(rems(10. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child("ROSTER"),
                )
                .child(
                    div()
                        .id("mission-composer-roster-scroll")
                        .max_h(rems(205. / 16.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .into_any_element();
            let dismiss_root = root.clone();
            let dismiss: runner_app::ui::menu::DismissHandler = Rc::new(move |_, cx| {
                dismiss_root.update(cx, |this, cx| {
                    this.mission_workspace.composer.picker_dismissed = true;
                    cx.notify();
                });
            });
            field = field.child(runner_app::ui::menu::popup_layer(
                anchor,
                window,
                px(380. * self.settings.app_zoom),
                menu,
                dismiss,
            ));
        }

        div()
            .flex_none()
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .px(rems(40. / 16.))
            .pt(rems(14. / 16.))
            .pb_5()
            .child(field)
            .into_any_element()
    }

    fn on_mission_composer_key_down(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let transition = composer_key_down(
            &self.mission_workspace.composer,
            &self.mission_composer_roster(),
            key,
            shift,
        );
        if !transition.prevent_default {
            return;
        }
        let draft_changed = transition.state.draft != self.mission_workspace.composer.draft;
        self.mission_workspace.composer = transition.state;
        if draft_changed {
            let draft = self.mission_workspace.composer.draft.clone();
            self.mission_workspace
                .composer_input
                .update(cx, |input, input_cx| input.reset(draft, input_cx));
        }
        if let Some(post) = transition.post {
            self.post_mission_composer(post, window, cx);
        } else {
            self.mission_workspace
                .composer_input
                .read(cx)
                .focus_handle()
                .focus(window);
            cx.notify();
        }
    }

    fn select_mission_composer_target(
        &mut self,
        handle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mission_workspace.composer_posting {
            return;
        }
        self.mission_workspace.composer = select_composer_target(handle);
        self.mission_workspace
            .composer_input
            .update(cx, |input, input_cx| input.reset("", input_cx));
        self.mission_workspace
            .composer_input
            .read(cx)
            .focus_handle()
            .focus(window);
        cx.notify();
    }

    fn clear_mission_composer_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mission_workspace.composer_posting {
            return;
        }
        self.mission_workspace.composer.target = None;
        self.mission_workspace
            .composer_input
            .read(cx)
            .focus_handle()
            .focus(window);
        cx.notify();
    }

    fn post_current_mission_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.mission_workspace.composer.draft.trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.post_mission_composer(
            ComposerPost {
                text,
                to: self.mission_workspace.composer.target.clone(),
            },
            window,
            cx,
        );
    }

    fn post_mission_composer(
        &mut self,
        post: ComposerPost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self.mission_workspace.composer_posting {
            return;
        }
        self.mission_workspace.composer_posting = true;
        self.mission_workspace
            .composer_input
            .update(cx, |input, input_cx| input.set_disabled(true, input_cx));
        cx.notify();
        let generation = self.mission_workspace.generation;
        let core = self.core.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_post_human_message_impl(
                &core,
                runner_backend::ops::mission::PostHumanMessageInput {
                    mission_id: mission_id.clone(),
                    text: post.text,
                    to: post.to,
                },
            )
            .await
            .map(|_| mission_id)
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let current = result.as_ref().ok().is_some_and(|mission_id| {
                    this.mission_workspace.is_current(mission_id, generation)
                }) || result.is_err()
                    && this.mission_workspace.generation == generation;
                if !current {
                    return;
                }
                this.mission_workspace.composer_posting = false;
                this.mission_workspace
                    .composer_input
                    .update(cx, |input, input_cx| input.set_disabled(false, input_cx));
                match result {
                    Ok(_) => {
                        this.mission_workspace.composer = ComposerState::default();
                        this.mission_workspace
                            .composer_input
                            .update(cx, |input, input_cx| input.reset("", input_cx));
                    }
                    Err(error) => {
                        this.mission_workspace.error =
                            Some(action_failure("send the mission message", error));
                    }
                }
                this.mission_workspace
                    .composer_input
                    .read(cx)
                    .focus_handle()
                    .focus(window);
                cx.notify();
            });
        })
        .detach();
    }

    fn render_mission_feed_block(&self, block: FeedBlock, cx: &mut Context<Self>) -> AnyElement {
        let id = block.id().to_owned();
        match block {
            FeedBlock::Divider(event) => div()
                .id(SharedString::from(format!("mission-divider-{id}")))
                .px_4()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .h(rems(1. / 16.))
                        .min_w(px(0.))
                        .flex_1()
                        .bg(theme::border()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(rems(10. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(format!("MISSION STARTED · {}", format_event_time(&event))),
                )
                .child(
                    div()
                        .h(rems(1. / 16.))
                        .min_w(px(0.))
                        .flex_1()
                        .bg(theme::border()),
                )
                .into_any_element(),
            FeedBlock::MessageGroup { author, events } => self
                .render_mission_message_group(author, events)
                .into_any_element(),
            FeedBlock::Signal(event) => self.render_mission_signal_row(event, cx),
            FeedBlock::AskCard(event) => self.render_mission_ask_card(event, cx),
        }
    }

    fn render_mission_message_group(&self, author: String, events: Vec<Event>) -> AnyElement {
        let first = &events[0];
        let human = author == "human";
        let target = message_target(first, &self.mission_workspace.askers_by_question);
        let goal = first.kind == EventKind::Signal
            && first.signal_type.as_ref().map(|kind| kind.as_str()) == Some("mission_goal");
        div()
            .id(SharedString::from(format!(
                "mission-message-group-{}",
                first.id
            )))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RunnerAvatar::new(author.clone(), 35.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .child(
                                div()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(runner_app::ui::hue_for_seed(&author).color())
                                    .child(if human {
                                        "you".into()
                                    } else {
                                        format!("@{author}")
                                    }),
                            )
                            .children(goal.then(|| {
                                div()
                                    .rounded_sm()
                                    .bg(theme::raised())
                                    .px_1()
                                    .py(rems(2. / 16.))
                                    .text_size(rems(9. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .child("GOAL")
                            }))
                            .children(target.map(|target| {
                                div()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(format!("→ @{target}"))
                            }))
                            .child(div().flex_none().child(format_event_time(first))),
                    )
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_size(rems(13. / 16.))
                            .text_color(theme::text())
                            .children(events.into_iter().filter_map(|event| {
                                let text = message_text(&event);
                                if text.is_empty() {
                                    goal.then(|| {
                                        div()
                                            .text_color(theme::faint())
                                            .child("(no text)")
                                            .into_any_element()
                                    })
                                } else {
                                    Some(crate::mission_markdown::render_markdown(&event.id, &text))
                                }
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_mission_signal_row(&self, event: Event, cx: &mut Context<Self>) -> AnyElement {
        let event_id = event.id.clone();
        let toggle_id = event_id.clone();
        let signal = event
            .signal_type
            .as_ref()
            .map(|kind| kind.as_str())
            .unwrap_or("?");
        let warning = signal == "mission_warning";
        let expanded = self
            .mission_workspace
            .expanded_signal_payloads
            .contains(&event_id);
        let root = cx.entity();
        let payload = if signal == "ask_lead" {
            event
                .payload
                .get("question")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            serde_json::to_string_pretty(&event.payload)
                .unwrap_or_else(|_| event.payload.to_string())
        };
        div()
            .id(SharedString::from(format!("mission-signal-{event_id}")))
            .pl(rems(63. / 16.))
            .pr_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!(
                        "mission-signal-toggle-{toggle_id}"
                    )))
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .text_size(rems(11. / 16.))
                    .on_click(move |_, _, cx| {
                        root.update(cx, |this, cx| {
                            if !this
                                .mission_workspace
                                .expanded_signal_payloads
                                .remove(&event_id)
                            {
                                this.mission_workspace
                                    .expanded_signal_payloads
                                    .insert(event_id.clone());
                            }
                            cx.notify();
                        });
                    })
                    .child(
                        svg()
                            .path("zap.svg")
                            .size(rems(12. / 16.))
                            .flex_none()
                            .text_color(if warning {
                                theme::danger()
                            } else {
                                theme::faint()
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family("JetBrains Mono")
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(runner_app::ui::hue_for_seed(&event.from).color())
                            .child(format!("@{}", event.from)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(if warning {
                                theme::danger()
                            } else {
                                theme::faint()
                            })
                            .child(format!(
                                "signal · {signal}{} · {}",
                                event
                                    .to
                                    .as_ref()
                                    .map(|to| format!(" → @{to}"))
                                    .unwrap_or_default(),
                                format_event_time(&event)
                            )),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(rems(10. / 16.))
                            .text_color(if warning {
                                theme::danger()
                            } else {
                                theme::faint()
                            })
                            .child("payload")
                            .child(
                                svg()
                                    .path(if expanded {
                                        "chevron-up.svg"
                                    } else {
                                        "chevron-down.svg"
                                    })
                                    .size(rems(12. / 16.))
                                    .text_color(if warning {
                                        theme::danger()
                                    } else {
                                        theme::faint()
                                    }),
                            ),
                    ),
            )
            .children(expanded.then(|| {
                div()
                    .mt_2()
                    .ml(rems(18. / 16.))
                    .rounded_md()
                    .border_1()
                    .border_color(if warning {
                        theme::with_alpha(theme::danger(), 0.3)
                    } else {
                        theme::border()
                    })
                    .bg(if warning {
                        theme::with_alpha(theme::danger(), 0.05)
                    } else {
                        theme::bg()
                    })
                    .p_3()
                    .font_family("JetBrains Mono")
                    .text_size(rems(12. / 16.))
                    .line_height(rems(17. / 16.))
                    .text_color(if warning {
                        theme::danger()
                    } else {
                        theme::muted()
                    })
                    .child(payload)
            }))
            .into_any_element()
    }

    fn render_mission_ask_card(&self, event: Event, cx: &mut Context<Self>) -> AnyElement {
        let question_id = event.id.clone();
        let asker = self
            .mission_workspace
            .askers_by_question
            .get(&question_id)
            .cloned()
            .unwrap_or_else(|| "?".into());
        let prompt = event
            .payload
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let choices = event
            .payload
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| vec!["yes".into(), "no".into()]);
        let on_behalf = event
            .payload
            .get("on_behalf_of")
            .and_then(serde_json::Value::as_str);
        let chain = on_behalf.map_or_else(
            || "→ you".to_owned(),
            |handle| format!("@{handle} → @{asker} → you"),
        );
        let resolved = self
            .mission_workspace
            .resolved_asks
            .get(&question_id)
            .cloned();
        let pending_choice = self
            .mission_workspace
            .pending_ask_choices
            .get(&question_id)
            .cloned();
        let submitting = self
            .mission_workspace
            .submitting_asks
            .contains(&question_id);
        let root = cx.entity();
        let mut buttons = div().mt_3().flex().flex_col().gap_1();
        for (index, choice) in choices.into_iter().enumerate() {
            let action_root = root.clone();
            let action_question = question_id.clone();
            let action_choice = choice.clone();
            let picked = resolved.as_deref() == Some(choice.as_str())
                || pending_choice.as_deref() == Some(choice.as_str());
            buttons = buttons.child(
                div()
                    .id(SharedString::from(format!(
                        "mission-answer-{question_id}-{index}"
                    )))
                    .w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(if index == 0 || picked {
                        theme::accent()
                    } else {
                        theme::border()
                    })
                    .bg(if index == 0 {
                        theme::accent()
                    } else {
                        theme::panel()
                    })
                    .px_3()
                    .py_2()
                    .cursor(if submitting || resolved.is_some() {
                        CursorStyle::OperationNotAllowed
                    } else {
                        CursorStyle::PointingHand
                    })
                    .opacity(if submitting || resolved.is_some() {
                        0.6
                    } else {
                        1.
                    })
                    .text_size(rems(12. / 16.))
                    .font_weight(if index == 0 {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(if index == 0 {
                        theme::accent_ink()
                    } else if picked {
                        theme::accent()
                    } else {
                        theme::text()
                    })
                    .when(!submitting && resolved.is_none(), |button| {
                        button.hover(|button| button.border_color(theme::border_strong()))
                    })
                    .on_click(move |_, _, cx| {
                        action_root.update(cx, |this, cx| {
                            this.answer_mission_question(
                                action_question.clone(),
                                action_choice.clone(),
                                cx,
                            )
                        });
                    })
                    .child(choice),
            );
        }
        div()
            .id(SharedString::from(format!("mission-ask-{question_id}")))
            .px_4()
            .flex()
            .items_start()
            .gap_3()
            .child(RunnerAvatar::new(asker.clone(), 35.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(runner_app::ui::hue_for_seed(&asker).color())
                                    .child(format!("@{asker}")),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .rounded_sm()
                                    .bg(theme::with_alpha(theme::warning(), 0.1))
                                    .px_1()
                                    .py(rems(2. / 16.))
                                    .text_size(rems(9. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::warning())
                                    .child("NEEDS YOUR INPUT"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(chain),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child(format_event_time(&event)),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::with_alpha(theme::warning(), 0.6))
                            .bg(theme::with_alpha(theme::warning(), 0.1))
                            .p_4()
                            .child(if prompt.is_empty() {
                                div()
                                    .text_size(rems(13. / 16.))
                                    .text_color(theme::faint())
                                    .child("(no prompt)")
                                    .into_any_element()
                            } else {
                                crate::mission_markdown::render_markdown(&event.id, &prompt)
                            })
                            .child(buttons)
                            .children(resolved.map(|choice| {
                                div()
                                    .mt_1()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child(format!("answered: {choice}"))
                            })),
                    ),
            )
            .into_any_element()
    }

    fn answer_mission_question(
        &mut self,
        question_id: String,
        choice: String,
        cx: &mut Context<Self>,
    ) {
        let Some(mission_id) = self.mission_workspace.mission_id.clone() else {
            return;
        };
        if self
            .mission_workspace
            .resolved_asks
            .contains_key(&question_id)
            || !self
                .mission_workspace
                .submitting_asks
                .insert(question_id.clone())
        {
            return;
        }
        self.mission_workspace
            .pending_ask_choices
            .insert(question_id.clone(), choice.clone());
        cx.notify();
        let core = self.core.clone();
        let post_question = question_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_post_human_signal_impl(
                &core,
                runner_backend::ops::mission::PostHumanSignalInput {
                    mission_id,
                    signal_type: "human_response".into(),
                    payload: serde_json::json!({
                        "question_id": post_question,
                        "choice": choice,
                    }),
                },
            )
            .await
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.mission_workspace.submitting_asks.remove(&question_id);
                if let Err(error) = result {
                    this.mission_workspace
                        .pending_ask_choices
                        .remove(&question_id);
                    this.mission_workspace.error =
                        Some(action_failure("answer the question", error));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_mission_terminal_pane(
        &self,
        session: SessionRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session_id = session.session.id.clone();
        let overlay = resolve_slot_overlay(
            self.mission_workspace.archiving,
            if self.mission_workspace.resuming {
                Some(MissionTransitionKind::Resuming)
            } else {
                self.mission_workspace.transition_kind(&session_id)
            },
            session.session.status,
        );
        let Some(chat) = self.attached.get(&session_id) else {
            return div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child("Attaching terminal…")
                .into_any_element();
        };
        let terminal = Arc::clone(&chat.terminal);
        let terminal_input = chat.terminal_input.clone();
        let terminal_focus = chat.terminal_focus.clone();
        let terminal_scrollbar = chat.terminal_scrollbar.clone();
        let terminal_background =
            terminal_element::to_hsla(self.terminal_style().palette.background, 1.);
        let interactive = self.mission_terminal_interactive(&session_id);
        let key_id = session_id.clone();
        let scroll_id = session_id.clone();
        let paste_id = session_id.clone();
        let root = cx.entity();
        let mut terminal_surface = div()
            .id(SharedString::from(format!("mission-terminal-{session_id}")))
            .absolute()
            .inset_0()
            .key_context("Terminal")
            .track_focus(&terminal_focus)
            .flex()
            .py_3()
            .pl_3()
            .pr_1()
            .bg(terminal_background)
            .opacity(match overlay {
                SlotOverlayState::Starting | SlotOverlayState::Resuming => 0.,
                SlotOverlayState::Stopped => 0.45,
                SlotOverlayState::Archiving | SlotOverlayState::None => 1.,
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .child(TerminalElement::new(
                        terminal,
                        terminal_input,
                        terminal_focus,
                        true,
                        self.terminal_style(),
                    ))
                    .child(terminal_scrollbar),
            );
        if interactive {
            let key_root = root.clone();
            let scroll_root = root.clone();
            let paste_root = root.clone();
            terminal_surface = terminal_surface
                .on_key_down(move |event, window, cx| {
                    key_root.update(cx, |this, cx| {
                        this.on_mission_key_down(&key_id, event, window, cx)
                    });
                })
                .on_scroll_wheel(move |event, window, cx| {
                    scroll_root.update(cx, |this, cx| {
                        this.on_mission_scroll(&scroll_id, event, window, cx)
                    });
                })
                .on_action(move |action: &Paste, window, cx| {
                    paste_root.update(cx, |this, cx| {
                        this.on_mission_paste(&paste_id, action, window, cx)
                    });
                });
        }
        let mut pane = div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(terminal_background)
            .child(terminal_surface);
        if let Some(blocked) = (session.session.status == SessionStatus::Running)
            .then(|| {
                self.mission_workspace
                    .delivery_blocked
                    .get(&session_id)
                    .cloned()
            })
            .flatten()
        {
            let idle = self
                .mission_workspace
                .runner_statuses()
                .get(&session.handle)
                == Some(&SessionActivityState::Idle);
            let sidebar_width = if self.settings.sidebar_collapsed {
                0.
            } else {
                self.settings.sidebar_width * self.settings.app_zoom
            };
            let rail_width = if self.settings.mission_rail_open {
                self.settings.mission_rail_width * self.settings.app_zoom
            } else {
                0.
            };
            let pane_width = f32::from(window.viewport_size().width)
                - sidebar_width
                - rail_width
                - 16. * self.settings.app_zoom;
            pane = pane.child(self.render_inbox_blocked_pill(
                session_id.clone(),
                blocked.unread_count,
                idle,
                pane_width < 600. * self.settings.app_zoom,
                cx,
            ));
        }
        pane = match overlay {
            SlotOverlayState::Resuming => pane.child(SessionOverlay::transition(
                SharedString::from(format!("mission-resuming-{session_id}")),
                SessionOverlayKind::Resuming,
            )),
            SlotOverlayState::Starting => pane.child(SessionOverlay::transition(
                SharedString::from(format!("mission-starting-{session_id}")),
                SessionOverlayKind::Starting,
            )),
            SlotOverlayState::Stopped => pane.child(self.render_mission_paused_overlay(cx)),
            SlotOverlayState::Archiving | SlotOverlayState::None => pane,
        };
        pane.into_any_element()
    }

    fn render_inbox_blocked_pill(
        &self,
        session_id: String,
        unread_count: usize,
        idle: bool,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let clear_root = root;
        div()
            .absolute()
            .right_4()
            .top_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::with_alpha(theme::warning(), 0.25))
            .bg(theme::panel())
            .px_3()
            .py_2()
            .text_size(rems(12. / 16.))
            .shadow_lg()
            .child(
                svg()
                    .path("mail.svg")
                    .size(rems(14. / 16.))
                    .text_color(theme::warning()),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::warning())
                    .child(if unread_count > 1 {
                        format!("Inbox waiting ({unread_count})")
                    } else {
                        "Inbox waiting".into()
                    }),
            )
            .children((!narrow).then(|| {
                div()
                    .text_color(theme::muted())
                    .child("— typing detected, delivery paused")
            }))
            .children(idle.then(|| {
                div()
                    .id(SharedString::from(format!(
                        "clear-mission-draft-{session_id}"
                    )))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::with_alpha(theme::warning(), 0.4))
                    .bg(theme::with_alpha(theme::warning(), 0.1))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .text_size(rems(11. / 16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::warning())
                    .hover(|button| button.bg(theme::with_alpha(theme::warning(), 0.15)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        clear_root.update(cx, |this, cx| {
                            this.clear_mission_input(&session_id, window, cx)
                        });
                    })
                    .child("Clear input")
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(rems(10. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .child("↵"),
                    )
            }))
            .into_any_element()
    }

    fn render_mission_paused_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let any_live = self.mission_workspace.any_session_live();
        let root = cx.entity();
        let resume_root = root.clone();
        let archive_root = root;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_end()
            .justify_center()
            .pb(rems(56. / 16.))
            .px_4()
            .bg(theme::with_alpha(theme::bg(), 0.7))
            .child(
                div()
                    .w_full()
                    .max_w(rems(672. / 16.))
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_5()
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                svg()
                                    .path("pause.svg")
                                    .size(rems(1.))
                                    .text_color(theme::faint()),
                            )
                            .child(
                                div()
                                    .text_size(rems(15. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Mission paused"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .line_height(rems(18. / 16.))
                            .text_color(theme::muted())
                            .child(if any_live {
                                "One or more slots are paused. Resume the mission to respawn every paused slot — partial-mission states aren't a valid run."
                            } else {
                                "All slots are paused. Resume to respawn every slot and pick up the conversation — the event log is preserved."
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("resume-paused-mission", "Resume")
                                    .icon("play.svg")
                                    .variant(ButtonVariant::Primary)
                                    .on_press(move |window, cx| {
                                        resume_root.update(cx, |this, cx| {
                                            this.resume_open_mission(window, cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new("archive-paused-mission", "Archive")
                                    .icon("archive.svg")
                                    .on_press(move |window, cx| {
                                        archive_root.update(cx, |this, cx| {
                                            this.archive_open_mission(window, cx)
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_duplicate_mission_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .bg(theme::with_alpha(theme::bg(), 0.85))
            .child(
                div()
                    .w_full()
                    .max_w(rems(448. / 16.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_6()
                    .text_center()
                    .shadow_2xl()
                    .child(
                        div()
                            .size(rems(44. / 16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .child(
                                svg()
                                    .path("app-window.svg")
                                    .size(rems(20. / 16.))
                                    .text_color(theme::accent()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(15. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Open in another window"),
                    )
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .line_height(rems(20. / 16.))
                            .text_color(theme::muted())
                            .child("Another window is already driving this mission. Only one window can own the terminal at a time, so this view is read-only until you focus it here."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("focus-duplicate-mission", "Focus that window")
                                    .icon("app-window.svg")
                                    .variant(ButtonVariant::Primary)
                                    .disabled(true)
                                    .tooltip("Available with native multi-window support"),
                            )
                            .child(
                                Button::new("stay-in-duplicate-mission", "Stay here").on_press(
                                    move |_, cx| {
                                        root.update(cx, |this, cx| {
                                            this.mission_workspace.duplicate_dismissed = true;
                                            cx.notify();
                                        });
                                    },
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_mission_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.settings.mission_rail_open {
            return div().w(px(0.)).h_full().flex_none().into_any_element();
        }
        let root = cx.entity();
        let runners_root = root.clone();
        let meta_root = root.clone();
        let collapse_root = root;
        let rail_view = self.mission_workspace.rail_view;
        let header = div()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        rail_view_button(
                            "mission-rail-runners",
                            "users.svg",
                            rail_view == MissionRailView::Runners,
                        )
                        .on_click(move |_, _, cx| {
                            runners_root.update(cx, |this, cx| {
                                this.set_mission_rail_view(MissionRailView::Runners, cx)
                            });
                        }),
                    )
                    .child(
                        rail_view_button(
                            "mission-rail-meta",
                            "info.svg",
                            rail_view == MissionRailView::Meta,
                        )
                        .on_click(move |_, _, cx| {
                            meta_root.update(cx, |this, cx| {
                                this.set_mission_rail_view(MissionRailView::Meta, cx)
                            });
                        }),
                    ),
            )
            .child(
                div().ml_auto().child(
                    IconButton::new("collapse-mission-rail", "panel-right-filled.svg")
                        .tooltip("Collapse runners panel")
                        .on_press(move |_, cx| {
                            collapse_root.update(cx, |this, cx| {
                                this.settings.mission_rail_open = false;
                                this.save_settings();
                                cx.notify();
                            });
                        }),
                ),
            );
        let body = match rail_view {
            MissionRailView::Runners => self.render_runners_rail(cx),
            MissionRailView::Meta => self.render_mission_meta_panel(cx),
        };
        let drag = MissionRailResizeDrag;
        div()
            .relative()
            .w(rems(self.settings.mission_rail_width / 16.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_l_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(header)
            .child(body)
            .child(
                div()
                    .id("mission-rail-resize")
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(rems(4. / 16.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|strip| strip.bg(theme::with_alpha(theme::accent(), 0.4)))
                    .on_drag(drag, |drag: &MissionRailResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    }),
            )
            .into_any_element()
    }

    fn set_mission_rail_view(&mut self, view: MissionRailView, cx: &mut Context<Self>) {
        self.mission_workspace.rail_view = view;
        self.settings.mission_rail_view = view.setting().into();
        self.save_settings();
        cx.notify();
    }

    fn render_runners_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let statuses = self.mission_workspace.runner_statuses();
        let selected = match &self.mission_workspace.active_tab {
            MissionTab::Session(session_id) => Some(session_id.as_str()),
            MissionTab::Feed => None,
        };
        let lead_handle = self
            .mission_workspace
            .sessions
            .iter()
            .find(|session| session.lead)
            .or_else(|| self.mission_workspace.sessions.first())
            .map(|session| session.handle.as_str())
            .unwrap_or_default()
            .to_owned();
        let mut list = div()
            .id("mission-runners-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(rail_section_label("Runner sessions"));
        if self.mission_workspace.sessions.is_empty() {
            return list
                .child(
                    div()
                        .text_size(rems(12. / 16.))
                        .text_color(theme::faint())
                        .child("No runner sessions yet."),
                )
                .into_any_element();
        }
        let root = cx.entity();
        for session in &self.mission_workspace.sessions {
            let session_id = session.session.id.clone();
            let open_id = session_id.clone();
            let open_root = root.clone();
            let activity = statuses.get(&session.handle).copied();
            let presence = match session.session.status {
                SessionStatus::Crashed => RunnerPresence::Crashed,
                SessionStatus::Stopped => RunnerPresence::Stopped,
                SessionStatus::Running if activity == Some(SessionActivityState::Idle) => {
                    RunnerPresence::Idle
                }
                SessionStatus::Running => RunnerPresence::Busy,
            };
            let subtitle = match session.session.status {
                SessionStatus::Crashed => "crashed",
                SessionStatus::Stopped => "stopped",
                SessionStatus::Running if activity == Some(SessionActivityState::Idle) => "idle",
                SessionStatus::Running if activity == Some(SessionActivityState::Busy) => "busy",
                SessionStatus::Running => "running",
            };
            let copy = self
                .mission_workspace
                .session_key_copies
                .get(&session_id)
                .cloned();
            let active = selected == Some(session_id.as_str());
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "mission-runner-card-{session_id}"
                    )))
                    .w_full()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        theme::with_alpha(theme::accent(), 0.6)
                    } else {
                        theme::border()
                    })
                    .bg(theme::bg())
                    .cursor_pointer()
                    .hover(|card| card.border_color(theme::border_strong()))
                    .on_click(move |_, window, cx| {
                        open_root.update(cx, |this, cx| {
                            this.select_mission_session(&open_id, window, cx)
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                RunnerAvatar::new(session.handle.clone(), 25.).presence(presence),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family("Menlo")
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(
                                        runner_app::ui::hue_for_seed(&session.handle).color(),
                                    )
                                    .child(format!("@{}", session.handle)),
                            )
                            .children(
                                (session.handle == lead_handle).then(runner_app::ui::lead_badge),
                            )
                            .child(
                                div()
                                    .ml_auto()
                                    .size(rems(24. / 16.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme::border())
                                    .text_color(theme::faint())
                                    .child(svg().path("terminal.svg").size(rems(12. / 16.))),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .pt_2()
                            .flex()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme::with_alpha(theme::border(), 0.7))
                            .text_size(rems(10. / 16.))
                            .child(
                                div()
                                    .w(rems(72. / 16.))
                                    .flex_none()
                                    .text_color(theme::faint())
                                    .child("session_key"),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_1()
                                    .flex()
                                    .items_start()
                                    .gap_1()
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .font_family("Menlo")
                                            .text_color(theme::muted())
                                            .child(
                                                session
                                                    .agent_session_key
                                                    .clone()
                                                    .unwrap_or_else(|| "NULL".into()),
                                            ),
                                    )
                                    .children(copy),
                            ),
                    ),
            );
        }
        list.into_any_element()
    }

    fn reveal_mission_cwd(&mut self, cwd: String, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            let status = Command::new("open")
                .arg("-R")
                .arg(cwd)
                .status()
                .map_err(|error| error.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("Finder exited with status {status}"))
            }
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.mission_workspace.error =
                        Some(format!("Couldn't reveal the working directory: {error}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_mission_meta_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mission) = self.mission_workspace.mission.clone() else {
            return div().into_any_element();
        };
        let root = cx.entity();
        let crew_root = root.clone();
        let cwd_root = root;
        let goal = self.mission_workspace.goal();
        let mut panel = div()
            .id("mission-meta-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(rail_section_label("Mission detail"))
            .child(meta_section(
                "Mission ID",
                div()
                    .flex()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .font_family("Menlo")
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(mission.id.clone()),
                    )
                    .child(self.mission_workspace.mission_id_copy.clone()),
            ))
            .child(meta_section(
                "Goal",
                match goal {
                    Some(goal) if !goal.is_empty() => div()
                        .text_size(rems(12. / 16.))
                        .line_height(rems(18. / 16.))
                        .text_color(theme::text())
                        .child(goal)
                        .into_any_element(),
                    Some(_) => div()
                        .text_size(rems(12. / 16.))
                        .italic()
                        .text_color(theme::faint())
                        .child("No goal set.")
                        .into_any_element(),
                    None => div()
                        .text_size(rems(12. / 16.))
                        .italic()
                        .text_color(theme::faint())
                        .child("Loading…")
                        .into_any_element(),
                },
            ));
        panel = panel.child(meta_section(
            "Working dir",
            mission.cwd.clone().map_or_else(
                || {
                    div()
                        .text_size(rems(12. / 16.))
                        .italic()
                        .text_color(theme::faint())
                        .child("No cwd set.")
                        .into_any_element()
                },
                |cwd| {
                    let reveal = cwd.clone();
                    Tooltip::new(
                        "reveal-mission-cwd-tooltip",
                        "Reveal in Finder",
                        div()
                            .id("reveal-mission-cwd")
                            .w_full()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .px_2()
                            .py_2()
                            .cursor_pointer()
                            .font_family("Menlo")
                            .text_size(rems(11. / 16.))
                            .text_color(theme::text())
                            .hover(|button| button.border_color(theme::border_strong()))
                            .on_click(move |_, _, cx| {
                                let reveal = reveal.clone();
                                cwd_root.update(cx, |this, cx| this.reveal_mission_cwd(reveal, cx));
                            })
                            .child(cwd),
                    )
                    .expand()
                    .into_any_element()
                },
            ),
        ));
        let crew_name = self
            .mission_workspace
            .crew
            .as_ref()
            .map(|crew| crew.name.clone())
            .unwrap_or_else(|| "…".into());
        let crew_id = mission.crew_id.clone();
        panel
            .child(meta_section(
                "Crew",
                div()
                    .id("open-mission-crew")
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(rems(12. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent())
                    .hover(|link| link.underline())
                    .on_click(move |_, window, cx| {
                        crew_root.update(cx, |this, cx| {
                            this.open_crew_editor(crew_id.clone(), window, cx)
                        });
                    })
                    .child(
                        svg()
                            .path("users.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::muted()),
                    )
                    .child(crew_name),
            ))
            .child(meta_section(
                "Started",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(rems(12. / 16.))
                    .child(
                        svg()
                            .path("clock.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::muted()),
                    )
                    .child(format_relative_time(mission.started_at)),
            ))
            .child(div().h(rems(1. / 16.)).w_full().bg(theme::border()))
            .into_any_element()
    }

    pub(crate) fn render_mission_rename_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self
            .mission_workspace
            .rename_modal
            .as_ref()
            .expect("mission rename modal");
        let submitting = modal.submitting;
        let valid = !modal.input.read(cx).text().trim().is_empty();
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let dismiss_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(rems(1.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Rename mission"),
            )
            .child(
                IconButton::new("close-mission-rename", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close rename")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::on_mission_rename_key_down))
            .children(modal.error.clone().map(|error| {
                div()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(
                Field::new("mission-rename-name", "Name", modal.input.clone())
                    .focus_target(modal.input.read(cx).focus_handle())
                    .emphasized(true),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-mission-rename", "Cancel")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-mission-rename",
                    if submitting { "Saving…" } else { "Save" },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(submitting || !valid)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_mission_rename(window, cx));
                }),
            );
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                dismiss_root.update(cx, |this, cx| this.close_mission_rename(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(448.))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                modal.input.read(cx).focus_handle(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
                modal.close_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }
}

fn mission_notice(
    id: &'static str,
    text: String,
    tone: gpui::Hsla,
    dismiss: &'static str,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .mx_8()
        .mt_3()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(tone, 0.4))
        .bg(theme::with_alpha(tone, 0.1))
        .px_3()
        .py_2()
        .text_size(rems(13. / 16.))
        .text_color(tone)
        .child(div().flex_1().child(text))
        .child(
            div()
                .id(SharedString::from(format!("dismiss-mission-{id}")))
                .cursor_pointer()
                .text_size(rems(11. / 16.))
                .opacity(0.8)
                .hover(|button| button.opacity(1.))
                .on_click(move |_, window, cx| on_dismiss(window, cx))
                .child(dismiss),
        )
}

fn mission_tab(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(rems(WORKSPACE_TAB_WIDTH / 16.))
        .h(rems(32. / 16.))
        .flex_none()
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .border_b_2()
        .border_color(if active {
            theme::accent()
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .text_size(rems(12. / 16.))
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|tab| tab.text_color(theme::text()))
        .child(label.into())
}

fn rail_view_button(
    id: impl Into<gpui::ElementId>,
    icon: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .group("mission-rail-view-button")
        .size(rems(28. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .cursor_pointer()
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .when(active, |button| {
            button.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
        })
        .hover(|button| {
            button
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                .text_color(theme::text())
        })
        .child(
            svg()
                .path(icon)
                .size(rems(14. / 16.))
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .group_hover("mission-rail-view-button", |icon| {
                    icon.text_color(theme::text())
                }),
        )
}

fn rail_section_label(label: &'static str) -> AnyElement {
    div()
        .pt_5()
        .text_size(rems(10. / 16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label.to_uppercase())
        .into_any_element()
}

fn meta_section(label: &'static str, body: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(rems(10. / 16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::faint())
                .child(label.to_uppercase()),
        )
        .child(body)
        .into_any_element()
}

fn format_relative_time(started_at: chrono::DateTime<Utc>) -> String {
    let minutes = Utc::now()
        .signed_duration_since(started_at)
        .num_minutes()
        .max(0);
    if minutes < 1 {
        "just now".into()
    } else if minutes < 60 {
        format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        )
    } else {
        let hours = minutes / 60;
        if hours < 24 {
            format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
        } else {
            let days = hours / 24;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}

fn format_event_time(event: &Event) -> String {
    event
        .ts
        .with_timezone(&chrono::Local)
        .format("%-I:%M %p")
        .to_string()
}

fn mission_tab_in_direction(
    tabs: &[MissionTab],
    active: &MissionTab,
    direction: isize,
) -> Option<MissionTab> {
    if tabs.len() < 2 {
        return None;
    }
    let current = tabs.iter().position(|tab| tab == active);
    let next = current.map_or(0, |current| {
        (current as isize + direction).rem_euclid(tabs.len() as isize) as usize
    });
    Some(tabs[next].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_overlay_precedence_matches_the_shipped_workspace() {
        assert_eq!(
            resolve_slot_overlay(
                true,
                Some(MissionTransitionKind::Resuming),
                SessionStatus::Stopped,
            ),
            SlotOverlayState::Archiving
        );
        assert_eq!(
            resolve_slot_overlay(
                false,
                Some(MissionTransitionKind::Resuming),
                SessionStatus::Stopped,
            ),
            SlotOverlayState::Resuming
        );
        assert_eq!(
            resolve_slot_overlay(
                false,
                Some(MissionTransitionKind::Starting),
                SessionStatus::Stopped,
            ),
            SlotOverlayState::Starting
        );
        assert_eq!(
            resolve_slot_overlay(false, None, SessionStatus::Crashed),
            SlotOverlayState::Stopped
        );
        assert_eq!(
            resolve_slot_overlay(false, None, SessionStatus::Running),
            SlotOverlayState::None
        );
    }

    #[test]
    fn concurrent_resume_errors_match_the_backend_contract() {
        assert!(is_concurrent_resume_error(
            "session abc is already being resumed"
        ));
        assert!(is_concurrent_resume_error(
            "session abc is already running — attach instead"
        ));
        assert!(!is_concurrent_resume_error("session abc failed to spawn"));
    }

    #[test]
    fn mission_tab_shortcuts_cycle_feed_and_open_slots() {
        let tabs = vec![
            MissionTab::Feed,
            MissionTab::Session("coder".into()),
            MissionTab::Session("reviewer".into()),
        ];
        assert_eq!(
            mission_tab_in_direction(&tabs, &MissionTab::Feed, 1),
            Some(MissionTab::Session("coder".into()))
        );
        assert_eq!(
            mission_tab_in_direction(&tabs, &MissionTab::Feed, -1),
            Some(MissionTab::Session("reviewer".into()))
        );
        assert_eq!(
            mission_tab_in_direction(&tabs, &MissionTab::Session("reviewer".into()), 1),
            Some(MissionTab::Feed)
        );
        assert_eq!(
            mission_tab_in_direction(&tabs, &MissionTab::Session("closed".into()), -1),
            Some(MissionTab::Feed)
        );
    }
}

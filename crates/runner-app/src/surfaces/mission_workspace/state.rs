use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use futures::StreamExt as _;
use gpui::prelude::*;
use gpui::{px, App, Entity, WeakEntity, Window};
use runner_app::ui::{CopyValueButton, IconButtonSize, PopoverMenu, TextField};
use runner_backend::model::{Event, EventKind, SessionStatus};
use runner_backend::windows::Subject;

use super::*;
use crate::surfaces::mission_composer::{
    key_down as composer_key_down, update_draft as update_composer_draft, ComposerState,
};
use crate::surfaces::mission_feed::{group_feed_blocks, is_human_authored, project_asks};
use crate::*;

impl MissionWorkspace {
    pub(crate) fn new(
        window_label: String,
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = &app_store.read(cx).settings;
        let sidebar_collapsed = settings.sidebar_collapsed;
        let rail_visibility = SidebarVisibilityTransition::new(settings.mission_rail_open);
        let root_focus = cx.focus_handle();
        let drawer_focus = cx.focus_handle();
        let workspace = cx.entity();
        let menu_root = workspace.clone();
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
        let composer_root = workspace;
        let composer_input = cx.new(move |input_cx| {
            let key_root = composer_root.clone();
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "Message the crew — @handle to address one session",
                1,
                false,
            )
            .auto_grow(12)
            .text_size(theme::text_body())
            .key_interceptor(Rc::new(move |event, window, cx| {
                let key = event.keystroke.key.clone();
                let shift = event.keystroke.modifiers.shift;
                let prevent_default = {
                    let root = key_root.read(cx);
                    let roster = root.mission_composer_roster();
                    composer_key_down(&root.composer, &roster, &key, shift).prevent_default
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
            if draft != this.composer.draft {
                this.composer = update_composer_draft(&this.composer, draft);
                cx.notify();
            }
        });
        let (mission_event_tx, mut mission_event_rx) =
            futures::channel::mpsc::unbounded::<runner_backend::events::AppEvent>();
        let mut mission_events = app_store.read(cx).core.events.subscribe();
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
                                | "session/status"
                                | "session/spawned"
                                | "session/updated"
                                | "session/archived"
                                | "session/warning"
                                | "session/input-error"
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
        let store_revisions = app_store.read(cx).revisions;
        Self {
            window_label,
            shell,
            app_store: app_store.clone(),
            store_revisions,
            attached: HashMap::new(),
            mission_node_id: None,
            layout: MissionLayout::default(),
            drawer_focus,
            drawer_resizing: false,
            closing_drawer_shells: HashSet::new(),
            drawer_exit_codes: HashMap::new(),
            active: false,
            sidebar_collapsed,
            rail_visibility,
            root_focus,
            titlebar_drag_armed: false,
            mission_id: None,
            generation: 0,
            refresh_generation: 0,
            event_resync_generation: 0,
            mission: None,
            crew: None,
            sessions: Vec::new(),
            events: Vec::new(),
            session_statuses: BTreeMap::new(),
            session_observations: BTreeMap::new(),
            goal: None,
            feed_blocks: Vec::new(),
            feed_selection: None,
            feed_selecting: false,
            askers_by_question: HashMap::new(),
            resolved_asks: HashMap::new(),
            pending_ask_choices: HashMap::new(),
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
            last_measured_terminal_size: None,
            delivery_blocked: HashMap::new(),
            transitions: HashMap::new(),
            next_transition_generation: 0,
            slot_actions: HashSet::new(),
            slot_exit_codes: HashMap::new(),
            stop_all_confirm: false,
            restart_confirm: None,
            stopping: false,
            resuming: false,
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
            _store_subscription: cx
                .observe(&app_store, |this, _, cx| this.handle_app_store_update(cx)),
        }
    }

    pub(super) fn prepare_mission(
        &mut self,
        mission_id: String,
        rail_view: MissionRailView,
    ) -> u64 {
        self.generation += 1;
        self.refresh_generation += 1;
        self.event_resync_generation += 1;
        self.mission_id = Some(mission_id);
        self.attached.clear();
        self.mission_node_id = None;
        self.layout = MissionLayout::default();
        self.drawer_resizing = false;
        self.closing_drawer_shells.clear();
        self.drawer_exit_codes.clear();
        self.mission = None;
        self.crew = None;
        self.sessions.clear();
        self.events.clear();
        self.session_statuses.clear();
        self.goal = None;
        self.feed_blocks.clear();
        self.feed_selection = None;
        self.feed_selecting = false;
        self.askers_by_question.clear();
        self.resolved_asks.clear();
        self.pending_ask_choices.clear();
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
        self.last_measured_terminal_size = None;
        self.delivery_blocked.clear();
        self.transitions.clear();
        self.slot_actions.clear();
        self.slot_exit_codes.clear();
        self.stop_all_confirm = false;
        self.restart_confirm = None;
        self.stopping = false;
        self.resuming = false;
        self.archiving = false;
        self.secondary = false;
        self.duplicate_dismissed = false;
        self.primary_label = None;
        self.rail_view = rail_view;
        self.rename_modal = None;
        self.session_key_copies.clear();
        self.generation
    }

    pub(crate) fn release_window(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.attached.clear();
        self.feed_selection = None;
        self.feed_selecting = false;
        cx.notify();
    }

    pub(super) fn is_current(&self, mission_id: &str, generation: u64) -> bool {
        self.generation == generation && self.mission_id.as_deref() == Some(mission_id)
    }

    pub(super) fn all_sessions_live(&self) -> bool {
        !self.sessions.is_empty()
            && self
                .sessions
                .iter()
                .all(|session| session.session.status == SessionStatus::Running)
    }

    pub(super) fn any_session_live(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| session.session.status == SessionStatus::Running)
    }

    pub(super) fn archived(&self) -> bool {
        self.mission
            .as_ref()
            .is_some_and(|mission| mission.archived_at.is_some())
    }

    pub(super) fn lifecycle_busy(&self) -> bool {
        self.stopping || self.resuming || self.archiving || !self.slot_actions.is_empty()
    }

    pub(super) fn secondary_state(&self, cx: &App) -> runner_backend::ops::window::SecondaryState {
        let Some(mission_id) = self.mission_id.as_ref() else {
            return runner_backend::ops::window::SecondaryState::default();
        };
        runner_backend::ops::window::is_secondary_for(
            &self.core(cx).windows.snapshot(),
            &self.window_label,
            &Subject::Mission(mission_id.clone()),
        )
    }

    pub(super) fn transition_kind(&self, session_id: &str) -> Option<MissionTransitionKind> {
        self.transitions.get(session_id).map(|item| item.kind)
    }

    pub(super) fn rebuild_event_projection(&mut self) {
        let (statuses, observations) = project_session_statuses(&self.events);
        self.session_statuses = statuses;
        self.session_observations = observations;
        self.feed_blocks = group_feed_blocks(&self.events);
        if self.feed_selection.as_ref().is_some_and(|selection| {
            !self
                .events
                .iter()
                .any(|event| event.id == selection.event_id)
        }) {
            self.clear_feed_selection();
        }
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

    pub(super) fn slot_agent_status(
        &self,
        session_id: &str,
        cx: &App,
    ) -> runner_backend::session::status::AgentStatus {
        use runner_backend::session::status::{
            Activity, AgentStatus, Lifecycle, ObservationSource,
        };
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.session.id == session_id)
        else {
            return AgentStatus::default();
        };
        let snapshot = &self.app_store.read(cx).session_statuses;
        let mut status = snapshot
            .get(session_id)
            .cloned()
            .or_else(|| self.session_observations.get(&session.handle).cloned())
            .unwrap_or_else(|| AgentStatus {
                lifecycle: Lifecycle::Running,
                observation: runner_backend::session::status::AgentObservation {
                    activity: match self.session_statuses.get(&session.handle) {
                        Some(SessionActivityState::Busy) => Activity::Working,
                        Some(SessionActivityState::Idle) => Activity::Idle,
                        None => Activity::Unavailable,
                    },
                    source: if self.session_statuses.contains_key(&session.handle) {
                        ObservationSource::Baseline
                    } else {
                        ObservationSource::Unavailable
                    },
                    ..Default::default()
                },
                ..Default::default()
            });
        status.lifecycle = match session.session.status {
            SessionStatus::Running => Lifecycle::Running,
            SessionStatus::Stopped => Lifecycle::Stopped,
            SessionStatus::Crashed => Lifecycle::Error,
        };
        if let Some(transition) = self.transitions.get(session_id) {
            status.lifecycle = if transition.kind == MissionTransitionKind::Resuming {
                Lifecycle::Resuming
            } else {
                Lifecycle::Starting
            };
        }
        status
    }

    pub(super) fn session_statuses(&self) -> &BTreeMap<String, SessionActivityState> {
        &self.session_statuses
    }

    pub(super) fn goal(&self) -> Option<String> {
        self.goal.clone()
    }

    pub(super) fn permission_mode(&self) -> Option<String> {
        mission_permission_mode_label(&self.events)
    }

    pub(super) fn feed_is_near_bottom(&self) -> bool {
        let maximum = f32::from(self.feed_scroll.max_offset().height).max(0.);
        let position = (-f32::from(self.feed_scroll.offset().y)).clamp(0., maximum);
        maximum - position < 80.
    }

    pub(super) fn handle_feed_append(&mut self, event: &Event) {
        self.feed_was_near_bottom = self.feed_is_near_bottom();
        if is_human_authored(event) || self.feed_was_near_bottom {
            self.feed_scroll.scroll_to_bottom();
            self.feed_was_near_bottom = true;
            self.feed_has_new_messages = false;
        } else {
            self.feed_has_new_messages = true;
        }
    }

    pub(super) fn core<'a>(&self, cx: &'a App) -> &'a AppCore {
        &self.app_store.read(cx).core
    }

    pub(super) fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        &self.app_store.read(cx).settings
    }

    pub(super) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    pub(crate) fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        if self.sidebar_collapsed != collapsed {
            self.sidebar_collapsed = collapsed;
            cx.notify();
        }
    }

    pub(super) fn save_settings(&self, cx: &App) {
        self.app_store.read(cx).save_settings();
    }

    pub(super) fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    pub(super) fn load_mission_layout(&mut self, cx: &App) -> Result<()> {
        let Some(mission_id) = self.mission_id.as_deref() else {
            return Ok(());
        };
        let Some(row) = self
            .app_store
            .read(cx)
            .nodes
            .iter()
            .find(|row| {
                row.node_type == runner_backend::repo::node::NodeType::Mission
                    && row.ref_id.as_deref() == Some(mission_id)
            })
            .cloned()
        else {
            return Ok(());
        };
        self.mission_node_id = Some(row.id.clone());
        self.layout = MissionLayout::from_node_row(&row)?;
        Ok(())
    }

    pub(super) fn persist_mission_layout(&self, cx: &App) -> Result<()> {
        let node_id = self
            .mission_node_id
            .as_deref()
            .context("mission node is missing")?;
        runner_backend::ops::node::node_mission_layout_set(
            self.core(cx),
            node_id,
            self.layout.serialize()?,
        )?;
        Ok(())
    }

    pub(super) fn drawer_session_entry<'a>(
        &self,
        session_id: &str,
        cx: &'a App,
    ) -> Option<&'a DirectSessionEntry> {
        self.app_store
            .read(cx)
            .sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
    }

    pub(super) fn is_active(&self, _: &App) -> bool {
        self.active
    }

    pub(super) fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        crate::terminal::element::TerminalStyle {
            palette: app_settings::terminal_palette(self.settings(cx), theme::active_variant()),
            font: self.settings(cx).terminal_font_family.font(),
            font_size: self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom,
            app_zoom: self.settings(cx).app_zoom,
        }
    }

    pub(crate) fn apply_terminal_settings(&self, cx: &App) {
        let cursor = match self.settings(cx).terminal_cursor_style {
            app_settings::TerminalCursorStyle::Block => {
                alacritty_terminal::vte::ansi::CursorShape::Block
            }
            app_settings::TerminalCursorStyle::Underline => {
                alacritty_terminal::vte::ansi::CursorShape::Underline
            }
            app_settings::TerminalCursorStyle::Bar => {
                alacritty_terminal::vte::ansi::CursorShape::Beam
            }
        };
        for chat in self.attached.values() {
            chat.terminal.set_palette(app_settings::terminal_palette(
                self.settings(cx),
                theme::active_variant(),
            ));
            chat.terminal
                .configure(app_settings::TERMINAL_SCROLLBACK_LINES, cursor);
        }
    }

    fn handle_app_store_update(&mut self, cx: &mut Context<Self>) {
        let revisions = self.app_store.read(cx).revisions;
        let previous = self.store_revisions;
        let reactions = revisions.reactions_since(previous);
        self.store_revisions = revisions;
        if reactions.apply_terminal_settings {
            self.apply_terminal_settings(cx);
        }
        if reactions.reload_tabs && self.mission.is_some() && !self.archived() {
            if let Err(error) = self.load_mission_layout(cx) {
                self.error = Some(error.to_string());
            }
        }
        if revisions.settings != previous.settings
            || (reactions.terminal_wake && self.is_active(cx))
        {
            cx.notify();
        }
    }

    pub(super) fn leave_archived_mission(
        &mut self,
        mission_id: &str,
        was_archived: Option<bool>,
        is_archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |shell, shell_cx| {
                shell.leave_archived_mission(
                    mission_id,
                    was_archived,
                    is_archived,
                    window,
                    shell_cx,
                )
            });
        }
    }

    pub(super) fn open_crew_editor(
        &mut self,
        crew_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |shell, shell_cx| {
                shell.open_crew_editor(crew_id, window, shell_cx)
            });
        }
    }

    pub(super) fn set_sidebar_archiving(
        &self,
        mission_id: &str,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(shell) = self.shell.upgrade() {
            let mission_id = mission_id.to_owned();
            shell.update(cx, |shell, shell_cx| {
                shell.set_sidebar_mission_archiving(mission_id, archiving, shell_cx);
            });
        }
    }
}

/// Latest `session_status` row per handle: the busy/idle state and, when the
/// row carries one, the full hook-era status. `runner_status` rows from logs
/// written before #632 are read the same way.
pub(super) fn project_session_statuses(
    events: &[Event],
) -> (
    BTreeMap<String, SessionActivityState>,
    BTreeMap<String, runner_backend::session::status::AgentStatus>,
) {
    let mut statuses = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for event in events {
        if event.kind != EventKind::Signal
            || !event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.is_session_status())
        {
            continue;
        }
        if let Some(status) = event
            .payload
            .get("status")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
        {
            observations.insert(event.from.clone(), status);
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
    (statuses, observations)
}

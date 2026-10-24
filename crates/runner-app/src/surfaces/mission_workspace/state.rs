use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use futures::StreamExt as _;
use gpui::prelude::*;
use gpui::{px, App, Entity, WeakEntity, Window};
use runner_app::ui::{CopyValueButton, IconButtonSize, PopoverMenu, TextField};
use runner_core::protocol::model::{Event, EventKind, SessionStatus};
use runner_core::protocol::window::Subject;

use super::*;
use crate::surfaces::mission_composer::{
    key_down as composer_key_down, update_draft as update_composer_draft, ComposerState,
};
use crate::surfaces::mission_feed::{
    append_feed_block, group_feed_blocks, is_human_authored, project_asks,
};
use crate::*;

impl MissionWorkspace {
    pub(crate) fn clear_connection_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
    }

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
            .trigger_tooltip(crate::i18n::message("ui.mission_actions"))
        });
        let mission_id_copy = cx.new(|copy_cx| {
            CopyValueButton::new(
                copy_cx.focus_handle(),
                None,
                crate::i18n::message("ui.copy_mission_id"),
            )
        });
        let feed_scrollbar_metrics = Rc::new(Cell::new(ScrollbarMetrics::default()));
        let read_metrics = feed_scrollbar_metrics.clone();
        let scroll_root = workspace.downgrade();
        let feed_scrollbar = cx.new(|_| {
            Scrollbar::external(
                runner_app::ui::ScrollbarKind::App,
                Rc::new(move || read_metrics.get()),
                Rc::new(move |position, cx| {
                    let _ = scroll_root.update(cx, |this, cx| {
                        this.feed_list
                            .set_offset_from_scrollbar(gpui::point(px(0.), px(-position)));
                        cx.notify();
                    });
                }),
            )
        });
        let composer_root = workspace;
        let composer_input = cx.new(move |input_cx| {
            let key_root = composer_root.clone();
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.message_the_crew_handle_to_address_one_session"),
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
            let input = input.read(cx);
            let draft = input.text().to_owned();
            let caret = input.caret_offset();
            if draft != this.composer.draft || caret != this.composer.caret {
                this.composer = update_composer_draft(&this.composer, draft, caret);
            }
            cx.notify();
        });
        let (mission_event_tx, mut mission_event_rx) =
            futures::channel::mpsc::unbounded::<runner_core::protocol::ClientEvent>();
        let mut mission_events = app_store.read(cx).client.subscribe();
        cx.background_spawn(async move {
            loop {
                match mission_events.recv().await {
                    Ok(event)
                        if matches!(
                            event.name.as_str(),
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
                    Err(runner_core::protocol::EventError::Lagged(_)) => {
                        if mission_event_tx
                            .unbounded_send(runner_core::protocol::ClientEvent {
                                name: "mission/resync".to_owned(),
                                payload: serde_json::Value::Null,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(runner_core::protocol::EventError::Closed) => break,
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
            remote: None,
            remote_selected_handle: None,
            render_regions: HashMap::new(),
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
            sessions: Vec::new(),
            events: Vec::new(),
            event_ids: HashSet::new(),
            session_statuses: BTreeMap::new(),
            session_observations: BTreeMap::new(),
            goal: None,
            feed_blocks: Vec::new(),
            feed_rows: Vec::new(),
            feed_block_starts: Vec::new(),
            feed_markdown: RefCell::new(HashMap::new()),
            #[cfg(test)]
            feed_block_builds: std::cell::Cell::new(0),
            feed_selection: None,
            feed_selecting: false,
            askers_by_question: HashMap::new(),
            resolved_asks: HashMap::new(),
            pending_ask_choices: HashMap::new(),
            submitting_asks: HashSet::new(),
            feed_list: Self::new_feed_list(),
            feed_scrollbar,
            feed_scrollbar_metrics,
            feed_measured_width: None,
            feed_needs_height_hints: Cell::new(false),
            feed_vertical_padding: px(48.),
            feed_empty_panel_min_height: std::cell::Cell::new(px(0.)),
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
            last_grid_hint: None,
            grid_hint_pending: false,
            delivery_blocked: HashMap::new(),
            transitions: HashMap::new(),
            next_transition_generation: 0,
            slot_actions: HashSet::new(),
            slot_exit_codes: HashMap::new(),
            stop_all_confirm: false,
            restart_confirm: None,
            remove_confirm: None,
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
            _store_subscription: cx.observe_in(&app_store, window, |this, _, window, cx| {
                this.handle_app_store_update(cx);
                this.schedule_mission_grid_hint(window, cx);
            }),
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
        self.sessions.clear();
        self.events.clear();
        self.event_ids.clear();
        self.session_statuses.clear();
        self.goal = None;
        self.feed_blocks.clear();
        self.feed_rows.clear();
        self.feed_block_starts.clear();
        self.feed_markdown.borrow_mut().clear();
        self.feed_selection = None;
        self.feed_selecting = false;
        self.askers_by_question.clear();
        self.resolved_asks.clear();
        self.pending_ask_choices.clear();
        self.submitting_asks.clear();
        self.feed_list = Self::new_feed_list();
        self.feed_scrollbar_metrics.set(ScrollbarMetrics::default());
        self.feed_measured_width = None;
        self.feed_needs_height_hints.set(false);
        self.feed_empty_panel_min_height.set(px(0.));
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
        self.last_grid_hint = None;
        self.delivery_blocked.clear();
        self.transitions.clear();
        self.slot_actions.clear();
        self.slot_exit_codes.clear();
        self.stop_all_confirm = false;
        self.restart_confirm = None;
        self.remove_confirm = None;
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
        if let Some(remote) = &mut self.remote {
            remote.feed = None;
            remote.feed_subscription = None;
            self.generation += 1;
        }
        self.feed_list.scrollbar_drag_ended();
        self.attached.clear();
        self.feed_selection = None;
        self.feed_selecting = false;
        cx.notify();
    }

    pub(super) fn is_current(&self, mission_id: &str, generation: u64) -> bool {
        self.generation == generation && self.mission_id.as_deref() == Some(mission_id)
    }

    pub(super) fn all_sessions_live(&self) -> bool {
        !self.members().is_empty()
            && self
                .members()
                .iter()
                .all(|session| session.session.status == SessionStatus::Running)
    }

    pub(super) fn any_session_live(&self) -> bool {
        self.members()
            .iter()
            .any(|session| session.session.status == SessionStatus::Running)
    }

    pub(super) fn archived(&self) -> bool {
        self.mission_view().is_some_and(|mission| mission.archived)
    }

    pub(super) fn lifecycle_busy(&self) -> bool {
        self.stopping || self.resuming || self.archiving || !self.slot_actions.is_empty()
    }

    pub(super) fn secondary_state(
        &self,
        cx: &App,
    ) -> runner_core::protocol::window::SecondaryState {
        if self.remote.is_some() {
            return Default::default();
        }
        let Some(mission_id) = self.mission_id.as_ref() else {
            return runner_core::protocol::window::SecondaryState::default();
        };
        runner_core::protocol::window::is_secondary_for(
            &self.app_store.read(cx).window_entries.clone(),
            &self.window_label,
            &Subject::Mission(mission_id.clone()),
        )
    }

    pub(super) fn transition_kind(&self, session_id: &str) -> Option<MissionTransitionKind> {
        self.transitions.get(session_id).map(|item| item.kind)
    }

    pub(super) fn append_event(&mut self, event: Event) -> bool {
        if !self.event_ids.insert(event.id.clone()) {
            return false;
        }
        self.handle_feed_append(&event);
        let at_tail = self.events.last().is_none_or(|tail| tail.id < event.id);
        if !at_tail
            || (event.kind == EventKind::Message && event.payload["source_event"].is_string())
        {
            let index = self
                .events
                .partition_point(|existing| existing.id < event.id);
            self.events.insert(index, event);
            self.rebuild_event_projection();
        } else {
            let old_len = self.feed_list.item_count();
            let old_rows = self.feed_rows.len();
            let old_blocks = self.feed_blocks.len();
            if let Some(index) = append_feed_block(&mut self.feed_blocks, &event) {
                self.feed_empty_panel_min_height.set(px(0.));
                let first_event = if index < old_blocks {
                    match &self.feed_blocks[index] {
                        FeedBlock::MessageGroup { events, .. } => events.len() - 1,
                        _ => unreachable!(),
                    }
                } else {
                    self.feed_block_starts.push(old_rows);
                    0
                };
                self.append_feed_rows(index, first_event);
                self.splice_feed_items(old_rows..old_len, self.feed_item_count() - old_rows);
            }
            self.events.push(event);
            self.rebuild_auxiliary_projections();
        }
        true
    }

    pub(super) fn rebuild_event_projection(&mut self) {
        let following = self.feed_list.is_following_tail();
        let top = self.feed_list.logical_scroll_top();
        let anchor = self
            .feed_row_key(top.item_ix)
            .map(|(id, part)| (id.to_owned(), part));
        self.event_ids = self.events.iter().map(|event| event.id.clone()).collect();
        self.feed_blocks = group_feed_blocks(&self.events);
        self.append_remote_questions();
        self.feed_rows.clear();
        self.feed_block_starts.clear();
        for group in 0..self.feed_blocks.len() {
            self.feed_block_starts.push(self.feed_rows.len());
            self.append_feed_rows(group, 0);
        }
        self.feed_empty_panel_min_height.set(px(0.));
        self.feed_list
            .reset_with_uniform_height(self.feed_item_count(), px(FEED_ROW_HEIGHT_HINT));
        self.feed_needs_height_hints.set(false);
        if following {
            self.scroll_feed_to_bottom();
        } else {
            let item_ix = anchor
                .and_then(|(id, part)| {
                    (0..self.feed_rows.len())
                        .find(|index| self.feed_row_key(*index) == Some((id.as_str(), part)))
                })
                .unwrap_or(top.item_ix.min(self.feed_item_count().saturating_sub(1)));
            self.feed_list
                .scroll_to(gpui::ListOffset { item_ix, ..top });
        }
        self.feed_markdown
            .borrow_mut()
            .retain(|id, _| self.event_ids.contains(id));
        self.rebuild_auxiliary_projections();
    }

    fn append_feed_rows(&mut self, group: usize, first_event: usize) {
        match &self.feed_blocks[group] {
            FeedBlock::MessageGroup { events, .. } => {
                for (event, item) in events.iter().enumerate().skip(first_event) {
                    let markdown = self.cached_feed_markdown(&item.id, &message_text(item));
                    self.feed_rows
                        .extend((0..markdown.row_count()).map(|part| FeedRow::Message {
                            group,
                            event,
                            part,
                        }));
                }
            }
            _ => self.feed_rows.push(FeedRow::Block(group)),
        }
    }

    fn feed_row_key(&self, index: usize) -> Option<(&str, usize)> {
        match *self.feed_rows.get(index)? {
            FeedRow::Block(group) => Some((self.feed_blocks[group].id(), 0)),
            FeedRow::Message { group, event, part } => match &self.feed_blocks[group] {
                FeedBlock::MessageGroup { events, .. } => Some((&events[event].id, part)),
                _ => unreachable!(),
            },
        }
    }

    fn rebuild_auxiliary_projections(&mut self) {
        let (statuses, observations) = project_session_statuses(&self.events);
        self.session_statuses = statuses;
        self.session_observations = observations;
        if self
            .feed_selection
            .as_ref()
            .is_some_and(|selection| !self.event_ids.contains(&selection.event_id))
        {
            self.clear_feed_selection();
        }
        let asks = project_asks(&self.events);
        for question_id in asks
            .resolved_asks
            .keys()
            .filter(|id| self.resolved_asks.get(*id) != asks.resolved_asks.get(*id))
        {
            self.invalidate_feed_ask(question_id);
        }
        self.askers_by_question = asks.askers_by_question;
        self.resolved_asks = asks.resolved_asks;
        if self.remote.is_some() {
            for event in &self.events {
                if let (Some(id), Some(from)) = (
                    event
                        .payload
                        .get("question_id")
                        .and_then(serde_json::Value::as_str),
                    event
                        .payload
                        .get("remote_from")
                        .and_then(serde_json::Value::as_str),
                ) {
                    self.askers_by_question.insert(id.into(), from.into());
                }
            }
            if let Some(mission) = self.remote_summary().cloned() {
                for ask in mission.asks {
                    self.askers_by_question.insert(ask.question, ask.from);
                }
            }
        }
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
    ) -> runner_core::protocol::status::AgentStatus {
        if let Some(remote) = &self.remote {
            return remote
                .overview
                .as_ref()
                .and_then(|overview| overview.sessions.get(session_id))
                .map(|session| crate::surfaces::remote_client::model::agent_status(&session.status))
                .unwrap_or_default();
        }
        use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle, ObservationSource};
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
                observation: runner_core::protocol::status::AgentObservation {
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

    fn new_feed_list() -> gpui::ListState {
        let state = gpui::ListState::new(1, gpui::ListAlignment::Top, px(200.))
            .with_uniform_item_height(px(FEED_ROW_HEIGHT_HINT));
        state.set_follow_mode(gpui::FollowMode::Tail);
        state
    }

    pub(super) fn feed_is_near_bottom(&self) -> bool {
        self.feed_list.is_following_tail()
            || self
                .feed_list
                .bounds_for_item(self.feed_list.item_count().saturating_sub(1))
                .is_some_and(|bounds| {
                    bounds.bottom() + self.feed_vertical_padding
                        - self.feed_list.viewport_bounds().bottom()
                        < px(80.)
                })
    }

    pub(super) fn scroll_feed_to_bottom(&mut self) {
        self.feed_list.set_follow_mode(gpui::FollowMode::Tail);
        self.feed_was_near_bottom = true;
        self.feed_has_new_messages = false;
    }

    pub(super) fn handle_feed_append(&mut self, event: &Event) {
        self.feed_was_near_bottom = self.feed_is_near_bottom();
        if is_human_authored(event) || self.feed_was_near_bottom {
            self.scroll_feed_to_bottom();
        } else {
            self.feed_has_new_messages = true;
        }
    }

    pub(super) fn splice_feed_items(&self, range: std::ops::Range<usize>, count: usize) {
        if range.len() == count {
            self.feed_list.remeasure_items(range);
            return;
        }
        let following = self.feed_list.is_following_tail();
        let top = self.feed_list.logical_scroll_top();
        let replaces_top = range.contains(&top.item_ix);
        self.feed_list.splice(range, count);
        self.feed_needs_height_hints.set(true);
        // Splice clears an offset inside the replaced row; appending to that row must keep the text still.
        if !following && replaces_top {
            self.feed_list.scroll_to(top);
        }
    }

    pub(super) fn invalidate_feed_ask(&self, question_id: &str) {
        if let Some(index) = self.feed_blocks.iter().position(|block| match block {
            FeedBlock::AskCard(event) => {
                event
                    .payload
                    .get("question_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&event.id)
                    == question_id
            }
            FeedBlock::PendingAsk(ask) => ask.id == question_id,
            _ => false,
        }) {
            let row = self.feed_block_starts[index];
            self.splice_feed_items(row..row + 1, 1);
            if self.feed_item_count() > self.feed_rows.len().max(1) {
                self.feed_empty_panel_min_height.set(px(0.));
                let panel = self.feed_item_count() - 1;
                self.splice_feed_items(panel..panel + 1, 1);
            }
        }
    }

    pub(super) fn core<'a>(&self, cx: &'a App) -> &'a DaemonClient {
        &self.app_store.read(cx).client
    }

    pub(super) fn settings<'a>(&self, cx: &'a App) -> &'a AppSettings {
        self.remote
            .as_ref()
            .map(|remote| remote.settings.read(cx))
            .unwrap_or(&self.app_store.read(cx).settings)
    }

    pub(super) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        if let Some(remote) = &self.remote {
            return remote.settings.update(cx, |settings, _| update(settings));
        }
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
        if self.remote.is_none() {
            self.app_store.read(cx).save_settings();
        }
    }

    pub(super) fn refresh_store(&self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            return;
        }
        self.app_store
            .update(cx, |store, store_cx| store.refresh(refresh, store_cx));
    }

    pub(super) fn load_mission_layout(&mut self, cx: &App) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        let Some(mission_id) = self.mission_id.as_deref() else {
            return Ok(());
        };
        let Some(row) = self
            .app_store
            .read(cx)
            .nodes
            .iter()
            .find(|row| {
                row.node_type == runner_core::protocol::node::NodeType::Mission
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
        if self.remote.is_some() {
            return Ok(());
        }
        let node_id = self
            .mission_node_id
            .as_deref()
            .context("mission node is missing")?;
        self.core(cx)
            .node_mission_layout_set(node_id, self.layout.serialize()?)?;
        Ok(())
    }

    pub(super) fn drawer_session_entry(
        &self,
        session_id: &str,
        cx: &App,
    ) -> Option<crate::surfaces::front_page::model::SessionDisplay> {
        use crate::surfaces::front_page::model::SessionDisplay;
        if let Some(remote) = &self.remote {
            let session = remote.overview.as_ref()?.sessions.get(session_id)?;
            return Some(SessionDisplay::remote(
                session_id,
                session,
                false,
                false,
                remote.client.supports(&["resume"]),
            ));
        }
        self.app_store
            .read(cx)
            .sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
            .map(SessionDisplay::local)
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
        if revisions.settings != previous.settings || revisions.locale != previous.locale {
            if let Some(remote) = &self.remote {
                let mut global = self.app_store.read(cx).settings.clone();
                let own = remote.settings.read(cx);
                global.sidebar_width = own.sidebar_width;
                global.mission_rail_width = own.mission_rail_width;
                global.mission_rail_open = own.mission_rail_open;
                global
                    .last_mission_terminal_ids
                    .clone_from(&own.last_mission_terminal_ids);
                remote.settings.update(cx, |settings, cx| {
                    *settings = global;
                    cx.notify();
                });
            }
            let count = self.feed_list.item_count();
            self.feed_empty_panel_min_height.set(px(0.));
            self.splice_feed_items(0..count, count);
            self.feed_list
                .clone()
                .with_uniform_item_height(px(FEED_ROW_HEIGHT_HINT * self.settings(cx).app_zoom));
        }
        if reactions.apply_terminal_settings {
            self.apply_terminal_settings(cx);
        }
        if self.remote.is_none()
            && reactions.reload_tabs
            && self.mission.is_some()
            && !self.archived()
        {
            if let Err(error) = self.load_mission_layout(cx) {
                self.error = Some(error.to_string().into());
            }
        }
        if revisions.locale != previous.locale
            || revisions.settings != previous.settings
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

    pub(crate) fn set_archiving(
        &mut self,
        mission_id: &str,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        if self.mission_id.as_deref() == Some(mission_id) && self.archiving != archiving {
            self.archiving = archiving;
            cx.notify();
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
    BTreeMap<String, runner_core::protocol::status::AgentStatus>,
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

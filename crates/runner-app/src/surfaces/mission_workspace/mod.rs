mod actions;
mod attach;
mod composer;
mod drawer;
mod events;
mod feed;
mod input;
mod rail;
mod remote;
#[cfg(test)]
mod remote_tests;
mod routing;
mod state;
mod terminal_pane;
#[cfg(test)]
mod tests;
mod view;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{div, px, rems, App, Bounds, Entity, Pixels, SharedString, WeakEntity, Window};
use runner_app::ui::{
    CopyValueButton, PopoverMenu, Scrollbar, ScrollbarMetrics, SessionControlKind, TextField,
};
use runner_core::protocol::model::{
    Event, EventKind, Mission, MissionStatus, SessionStatus, SlotWithRole,
};
use runner_core::protocol::session::SessionRow;

use super::*;
use crate::surfaces::mission_composer::ComposerState;
use crate::surfaces::mission_feed::{message_text, FeedBlock};
use crate::surfaces::mission_markdown::{FeedMarkdown, FeedSelection};
use crate::*;

const WORKSPACE_TABS_HEIGHT: f32 = 38.;
const MISSION_RAIL_TRANSITION_MS: u64 = 200;
const FEED_ROW_HEIGHT_HINT: f32 = 100.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FeedRow {
    Block(usize),
    Message {
        group: usize,
        event: usize,
        part: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MissionTab {
    Feed,
    Session(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MissionRailView {
    #[default]
    Roles,
    Meta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissionTransitionKind {
    Starting,
    Resuming,
    Restarting,
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
    } else if matches!(
        transition,
        Some(MissionTransitionKind::Starting | MissionTransitionKind::Restarting)
    ) {
        SlotOverlayState::Starting
    } else if status != SessionStatus::Running {
        SlotOverlayState::Stopped
    } else {
        SlotOverlayState::None
    }
}

fn transition_to_begin_on_spawn(
    existing: Option<MissionTransitionKind>,
) -> Option<MissionTransitionKind> {
    existing
        .is_none()
        .then_some(MissionTransitionKind::Starting)
}

fn is_concurrent_resume_error(error: &str) -> bool {
    [
        "is already being resumed",
        "is already running — attach instead",
    ]
    .iter()
    .any(|fragment| error.contains(fragment))
}

fn mission_slot_actions_available(
    status: Option<MissionStatus>,
    archived: bool,
    secondary: bool,
) -> bool {
    status == Some(MissionStatus::Running) && !archived && !secondary
}

fn mission_divider_label(event: &Event) -> String {
    let kind = event
        .signal_type
        .as_ref()
        .map(|kind| kind.as_str())
        .unwrap_or_default();
    let time = format_event_time(event);
    match kind {
        "role_joined" | "role_removed" => {
            let handle = event.payload["handle"].as_str().unwrap_or_default();
            let (verb, action) = if kind == "role_joined" {
                (
                    crate::i18n::text("word.joined"),
                    crate::i18n::text("word.added"),
                )
            } else {
                (
                    crate::i18n::text("word.left"),
                    crate::i18n::text("word.removed"),
                )
            };
            crate::i18n::tf(
                "format.by_you",
                &[
                    ("handle", (handle).to_string()),
                    ("verb", (verb).to_string()),
                    ("action", (action).to_string()),
                    ("time", (time).to_string()),
                ],
            )
        }
        "mission_reopened" => {
            crate::i18n::tf("format.mission_reopened", &[("time", time.to_string())])
        }
        "mission_stopped" => {
            crate::i18n::tf("format.mission_stopped", &[("time", (time).to_string())])
        }
        _ => crate::i18n::tf("format.mission_started", &[("time", (time).to_string())]),
    }
}

fn stopped_slot_description(handle: &str, others: usize) -> String {
    let others = if others == 1 {
        crate::i18n::text("ui.1_other_slot_is").into()
    } else {
        crate::i18n::tf(
            "format.other_slots_are",
            &[("others", (others).to_string())],
        )
    };
    crate::i18n::tf(
        "format.s_pty_is_closed_still_running_resume_continues_its_conversation_where_it_left",
        &[
            ("handle", (handle).to_string()),
            ("others", (others).to_string()),
        ],
    )
}

fn slot_controls(status: SessionStatus) -> [SessionControlKind; 2] {
    [
        if status == SessionStatus::Running {
            SessionControlKind::Stop
        } else {
            SessionControlKind::Resume
        },
        SessionControlKind::Restart,
    ]
}

fn focused_slot_action_target(
    active_tab: &MissionTab,
    status: Option<SessionStatus>,
    action: SessionControlKind,
) -> Option<&str> {
    let MissionTab::Session(session_id) = active_tab else {
        return None;
    };
    match (action, status) {
        (SessionControlKind::Stop, Some(SessionStatus::Running))
        | (SessionControlKind::Resume, Some(SessionStatus::Stopped | SessionStatus::Crashed)) => {
            Some(session_id)
        }
        _ => None,
    }
}

fn slot_control_title(action: SessionControlKind, overrides: &keymap::KeymapOverrides) -> String {
    let (label, binding) = match action {
        SessionControlKind::Stop => (crate::i18n::text("ui.stop"), Some("stop-session")),
        SessionControlKind::Resume => (crate::i18n::text("ui.resume"), Some("resume-session")),
        SessionControlKind::Restart => (crate::i18n::text("ui.restart"), None),
        SessionControlKind::Resuming => (crate::i18n::text("ui.resuming"), None),
        SessionControlKind::Back => (crate::i18n::text("ui.back_to_role"), None),
    };
    binding
        .and_then(|id| keymap::effective_binding(id, overrides))
        .map_or_else(
            || label.to_owned(),
            |combo| format!("{label} · {}", keymap::format_combo(&combo)),
        )
}

fn restart_confirm_body(handle: &str, lead_handle: Option<&str>, is_lead: bool) -> String {
    if is_lead {
        crate::i18n::tf(
            "format.its_conversation_so_far_is_discarded_comes_back_with_the_launch_prompt_the_sam",
            &[("handle", (handle).to_string())],
        )
    } else if let Some(lead_handle) = lead_handle {
        crate::i18n::tf(
            "format.its_conversation_so_far_is_discarded_comes_back_with_its_brief_the_same_first",
            &[
                ("handle", (handle).to_string()),
                ("lead_handle", (lead_handle).to_string()),
            ],
        )
    } else {
        crate::i18n::tf(
            "format.restart_slot_conversation",
            &[("handle", (handle).to_string())],
        )
    }
}

fn stop_all_title(running_count: usize) -> String {
    if running_count == 1 {
        crate::i18n::text("ui.stop_the_running_slot").into()
    } else {
        crate::i18n::tf(
            "format.stop_all_running_slots",
            &[("running_count", (running_count).to_string())],
        )
    }
}

const STOP_ALL_BODY: &str =
    "ui.every_slot_s_pty_is_killed_and_whatever_turn_it_is_on_is_cut_off_the_mission_stays_open_ea";

fn mission_signal_author(event: &Event) -> (&str, String) {
    match event.from.as_str() {
        "router" | "runner" => ("system", "system".into()),
        "human"
            if event.signal_type.as_ref().map(|kind| kind.as_str()) == Some("slot_restarted") =>
        {
            ("human", "you".into())
        }
        author => (author, format!("@{author}")),
    }
}

fn slot_restart_signal_summary(event: &Event) -> Option<String> {
    (event.signal_type.as_ref().map(|kind| kind.as_str()) == Some("slot_restarted")).then(|| {
        crate::i18n::tf(
            "format.signal_slot_restarted_fresh_conversation_brief_re_sent",
            &[(
                "handle",
                (event
                    .payload
                    .get("handle")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?"))
                .to_string(),
            )],
        )
    })
}

fn mission_drawer_available(archived: bool, secondary: bool) -> bool {
    !archived && !secondary
}

fn drawer_session_should_take_focus(layout: &MissionLayout, session_id: &str) -> bool {
    layout.drawer.open() && layout.drawer.active_shell() == Some(session_id)
}

#[derive(Clone)]
enum MissionMenuAction {
    Pin,
    Rename,
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
    reason: Option<String>,
}

pub(crate) struct MissionWorkspace {
    pub(super) render_regions: HashMap<String, Entity<super::render_cache::CachedRegion<Self>>>,
    remote: Option<remote::RemoteMission>,
    remote_selected_handle: Option<String>,
    window_label: String,
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    attached: HashMap<String, AttachedChat>,
    mission_node_id: Option<String>,
    layout: MissionLayout,
    drawer_focus: FocusHandle,
    drawer_resizing: bool,
    closing_drawer_shells: HashSet<String>,
    drawer_exit_codes: HashMap<String, Option<i32>>,
    active: bool,
    sidebar_collapsed: bool,
    rail_visibility: SidebarVisibilityTransition,
    root_focus: FocusHandle,
    titlebar_drag_armed: bool,
    pub mission_id: Option<String>,
    generation: u64,
    refresh_generation: u64,
    event_resync_generation: u64,
    mission: Option<Mission>,
    sessions: Vec<SessionRow>,
    events: Vec<Event>,
    event_ids: HashSet<String>,
    session_statuses: BTreeMap<String, SessionActivityState>,
    session_observations: BTreeMap<String, runner_core::protocol::status::AgentStatus>,
    goal: Option<String>,
    feed_blocks: Vec<FeedBlock>,
    feed_rows: Vec<FeedRow>,
    feed_block_starts: Vec<usize>,
    feed_markdown: RefCell<HashMap<String, Rc<FeedMarkdown>>>,
    #[cfg(test)]
    feed_block_builds: std::cell::Cell<usize>,
    feed_selection: Option<FeedSelection>,
    feed_selecting: bool,
    askers_by_question: HashMap<String, String>,
    resolved_asks: HashMap<String, String>,
    pending_ask_choices: HashMap<String, String>,
    submitting_asks: HashSet<String>,
    feed_list: gpui::ListState,
    feed_scrollbar: Entity<Scrollbar>,
    feed_scrollbar_metrics: Rc<Cell<ScrollbarMetrics>>,
    feed_measured_width: Option<Pixels>,
    feed_needs_height_hints: Cell<bool>,
    feed_vertical_padding: Pixels,
    feed_empty_panel_min_height: std::cell::Cell<Pixels>,
    feed_was_near_bottom: bool,
    feed_has_new_messages: bool,
    roster: Vec<SlotWithRole>,
    composer: ComposerState,
    composer_input: Entity<TextField>,
    composer_posting: bool,
    composer_anchor: Option<Bounds<Pixels>>,
    _composer_subscription: Subscription,
    loading: bool,
    loading_overlay_visible: bool,
    error: Option<crate::i18n::Text>,
    warning: Option<crate::i18n::Text>,
    active_tab: MissionTab,
    open_tabs: Vec<String>,
    last_measured_terminal_size: Option<CachedTerminalSize>,
    last_grid_hint: Option<(u16, u16)>,
    grid_hint_pending: bool,
    delivery_blocked: HashMap<String, DeliveryBlocked>,
    transitions: HashMap<String, MissionTransition>,
    next_transition_generation: u64,
    slot_actions: HashSet<String>,
    slot_exit_codes: HashMap<String, Option<i32>>,
    stop_all_confirm: bool,
    restart_confirm: Option<String>,
    remove_confirm: Option<String>,
    stopping: bool,
    resuming: bool,
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
    _store_subscription: Subscription,
}

#[derive(Clone)]
struct MissionLoadResult {
    mission: Mission,
    roster: Vec<SlotWithRole>,
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

fn action_failure(
    action: impl Into<crate::i18n::Text>,
    error: impl AsRef<str>,
) -> crate::i18n::Text {
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
    crate::i18n::message_with(
        "format.couldn_t",
        &[("action", action.into()), ("detail", detail.into())],
    )
}

fn selectable_event_text(event: &Event) -> String {
    let message = message_text(event);
    if !message.is_empty() {
        message
    } else {
        event
            .payload
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }
}

fn mission_notice(
    id: &'static str,
    text: String,
    tone: runner_app::ui::Tone,
    dismiss: &'static str,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    runner_app::ui::notice_banner(text, tone).child(
        runner_app::ui::Button::new(format!("dismiss-mission-{id}"), dismiss)
            .size(runner_app::ui::ButtonSize::Sm)
            .variant(runner_app::ui::ButtonVariant::Ghost)
            .on_press(on_dismiss),
    )
}

/// Session tabs truncate their handle down to this width before the strip
/// starts scrolling, so a crowded strip stays inside the content column.
const MISSION_TAB_MIN_WIDTH: f32 = 112.;

fn mission_tab(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(rems(28. / 16.))
        .flex_none()
        .px(rems(12. / 16.))
        .flex()
        .items_center()
        .rounded_md()
        .when(active, |tab| tab.bg(theme::panel()))
        .cursor_pointer()
        .text_size(theme::text_body())
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|tab| tab.text_color(theme::text()))
        .child(label.into())
}

/// The permission mode a mission started with, read from the first
/// `mission_start` signal's payload (feature 527). Missions recorded
/// before the key existed have no section. Rendered as the spec's
/// human label: `bypass`, `auto`, or `role default`.
fn mission_permission_mode_label(events: &[Event]) -> Option<String> {
    events
        .iter()
        .find(|event| {
            event.kind == EventKind::Signal
                && event.signal_type.as_ref().map(|kind| kind.as_str()) == Some("mission_start")
        })
        .and_then(|event| event.payload.get("permission_mode"))
        .and_then(serde_json::Value::as_str)
        .map(|mode| match mode {
            "role-default" | "runner-default" => {
                crate::i18n::text("ui.mission_role_default").to_owned()
            }
            "auto" => crate::i18n::text("permission.auto").into(),
            "bypass" => crate::i18n::text("permission.bypass").into(),
            _ => mode.replace('-', " "),
        })
}

fn format_event_time(event: &Event) -> String {
    event
        .ts
        .with_timezone(&chrono::Local)
        .format(crate::i18n::text("date.time"))
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

impl Render for MissionWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_mission_workspace(window, cx)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CachedTerminalSize {
    measured: (u16, u16),
    layout_estimate: (u16, u16),
}

fn preferred_terminal_size(
    measured: Option<(u16, u16)>,
    cached: Option<CachedTerminalSize>,
    estimated: (u16, u16),
) -> (u16, u16) {
    measured
        .or_else(|| {
            cached
                .filter(|cached| cached.layout_estimate == estimated)
                .map(|cached| cached.measured)
        })
        .unwrap_or(estimated)
}

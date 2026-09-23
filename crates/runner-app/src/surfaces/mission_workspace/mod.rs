mod actions;
mod attach;
mod composer;
mod drawer;
mod events;
mod feed;
mod input;
mod rail;
mod routing;
mod state;
mod terminal_pane;
#[cfg(test)]
mod tests;
mod view;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use gpui::prelude::*;
use gpui::{div, px, rems, App, Bounds, Entity, Pixels, SharedString, WeakEntity, Window};
use runner_app::ui::{CopyValueButton, PopoverMenu, SessionControlKind, TextField};
use runner_backend::model::{
    Crew, Event, EventKind, Mission, MissionStatus, SessionStatus, SlotWithRole,
};
use runner_backend::ops::session::SessionRow;

use super::*;
use crate::surfaces::mission_composer::ComposerState;
use crate::surfaces::mission_feed::{message_text, FeedBlock};
use crate::surfaces::mission_markdown::FeedSelection;
use crate::*;

const WORKSPACE_TABS_HEIGHT: f32 = 38.;
const MISSION_RAIL_TRANSITION_MS: u64 = 200;

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

fn stopped_slot_description(handle: &str, others: usize) -> String {
    let others = if others == 1 {
        "1 other slot is".into()
    } else {
        format!("{others} other slots are")
    };
    format!("@{handle}'s PTY is closed; {others} still running. Resume continues its conversation where it left off. Restart discards it and starts over with the brief, the same first turn a cold start gives the slot.")
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
        SessionControlKind::Stop => ("Stop", Some("stop-session")),
        SessionControlKind::Resume => ("Resume", Some("resume-session")),
        SessionControlKind::Restart => ("Restart", None),
        SessionControlKind::Resuming => ("Resuming…", None),
        SessionControlKind::Back => ("Back to role", None),
    };
    binding
        .and_then(|id| keymap::effective_binding(id, overrides))
        .map_or_else(
            || label.to_owned(),
            |combo| format!("{label} · {}", keymap::format_combo(&combo)),
        )
}

fn restart_confirm_body(handle: &str, lead_handle: &str, is_lead: bool) -> String {
    if is_lead {
        format!("Its conversation so far is discarded. @{handle} comes back with the launch prompt, the same first turn a cold start gives it, and the other slots get a note that it starts over.")
    } else {
        format!("Its conversation so far is discarded. @{handle} comes back with its brief, the same first turn a cold start gives it, and @{lead_handle} gets a note that it must re-send anything @{handle} needs.")
    }
}

fn stop_all_title(running_count: usize) -> String {
    if running_count == 1 {
        "Stop the running slot?".into()
    } else {
        format!("Stop all {running_count} running slots?")
    }
}

const STOP_ALL_BODY: &str = "Every slot's PTY is killed and whatever turn it is on is cut off. The mission stays open; each slot can be resumed with its conversation, or restarted with its brief.";

fn slot_restart_signal_summary(event: &Event) -> Option<String> {
    (event.signal_type.as_ref().map(|kind| kind.as_str()) == Some("slot_restarted")).then(|| {
        format!(
            "signal · slot_restarted → @{} · fresh conversation, brief re-sent",
            event
                .payload
                .get("handle")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
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
}

pub(crate) struct MissionWorkspace {
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
    crew: Option<Crew>,
    sessions: Vec<SessionRow>,
    events: Vec<Event>,
    session_statuses: BTreeMap<String, SessionActivityState>,
    session_observations: BTreeMap<String, runner_backend::session::status::AgentStatus>,
    goal: Option<String>,
    feed_blocks: Vec<FeedBlock>,
    feed_selection: Option<FeedSelection>,
    feed_selecting: bool,
    askers_by_question: HashMap<String, String>,
    resolved_asks: HashMap<String, String>,
    pending_ask_choices: HashMap<String, String>,
    submitting_asks: HashSet<String>,
    feed_scroll: ScrollHandle,
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
    error: Option<String>,
    warning: Option<String>,
    active_tab: MissionTab,
    open_tabs: Vec<String>,
    last_measured_terminal_size: Option<CachedTerminalSize>,
    delivery_blocked: HashMap<String, DeliveryBlocked>,
    transitions: HashMap<String, MissionTransition>,
    next_transition_generation: u64,
    slot_actions: HashSet<String>,
    slot_exit_codes: HashMap<String, Option<i32>>,
    stop_all_confirm: bool,
    restart_confirm: Option<String>,
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
    crew: Option<Crew>,
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
        .text_size(theme::text_body())
        .text_color(tone)
        .child(div().flex_1().child(text))
        .child(
            div()
                .id(SharedString::from(format!("dismiss-mission-{id}")))
                .cursor_pointer()
                .text_size(theme::text_meta())
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
        .h(rems(32. / 16.))
        .flex_none()
        .px(rems(14. / 16.))
        .flex()
        .items_center()
        .border_b_2()
        .border_color(if active {
            theme::accent()
        } else {
            gpui::transparent_black()
        })
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
            "role-default" | "runner-default" => "role default".to_owned(),
            _ => mode.replace('-', " "),
        })
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

impl Render for MissionWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_mission_grid_hint(window, cx);
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

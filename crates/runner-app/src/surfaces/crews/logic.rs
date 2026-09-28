use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, rems, AnyElement, Context, FocusHandle, KeyDownEvent, SharedString, Window};
use runner_app::ui::{RoleAvatar, SelectOption};
use runner_backend::model::{Mission, MissionStatus, SlotWithRole, Timestamp};
use runner_backend::ops::crew::CrewMemberPreview;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::ops::role::RoleWithActivity;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};

use super::*;
use crate::*;

pub(super) fn text_action(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_click = Rc::new(on_click);
    let key_click = Rc::clone(&on_click);
    div()
        .id(id)
        .tab_index(0)
        .cursor_pointer()
        .text_size(theme::text_ui())
        .text_color(theme::muted())
        .hover(|text| text.text_color(theme::text()))
        .focus_visible(|text| text.text_color(theme::text()).underline())
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_click(window, cx);
            }
        })
        .child(label)
        .into_any_element()
}

pub(super) fn error_panel(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(theme::text_title())
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
}

pub(super) fn error_banner(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(theme::text_ui())
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
}

pub(super) fn selected_add_slot_role(form: &AddSlotForm) -> Option<&RoleWithActivity> {
    let selected = form.selected_role_id.as_deref()?;
    form.roles.iter().find(|role| role.role.id == selected)
}

pub(super) fn role_matches(role: &RoleWithActivity, query: &str) -> bool {
    query.is_empty()
        || role.role.handle.to_lowercase().contains(query)
        || role.role.display_name.to_lowercase().contains(query)
        || role.role.runtime.to_lowercase().contains(query)
}

pub(super) fn suggest_slot_handle(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..100)
        .map(|index| format!("{base}-{index}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| base.to_owned())
}

pub(super) fn add_slot_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    selected: Option<&RoleWithActivity>,
) -> Vec<SelectOption> {
    let default = selected
        .map(|role| format!("Role default ({})", role.role.runtime))
        .unwrap_or_else(|| "Role default".into());
    let mut options =
        vec![SelectOption::new("", default).description("Use the runtime configured on the role.")];
    options.extend(runtimes.iter().map(|runtime| {
        SelectOption::new(runtime.name.to_string(), runtime.display_name.clone())
            .description(runtime.description.clone())
    }));
    options
}

pub(super) fn runtime_models<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtimes
        .iter()
        .find(|runtime| runtime.name.key() == name)
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default()
}

pub(super) fn validate_slot_handle(handle: &str) -> Option<&'static str> {
    if handle.is_empty() {
        return None;
    }
    let bytes = handle.as_bytes();
    let valid = bytes.len() <= 32
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    (!valid).then_some(
        "Lowercase letters, digits, '-' or '_'; must start with a letter or digit; up to 32 chars.",
    )
}

pub(super) fn slot_handle_error(
    handle: &str,
    existing_handles: &HashSet<String>,
) -> Option<String> {
    validate_slot_handle(handle)
        .map(ToOwned::to_owned)
        .or_else(|| {
            existing_handles
                .contains(handle)
                .then(|| format!("'{handle}' is already used in this crew."))
        })
}

pub(super) fn add_slot_can_submit(form: &AddSlotForm) -> bool {
    !form.loading
        && !form.submitting
        && selected_add_slot_role(form).is_some()
        && !form.slot_handle_empty
        && form.slot_handle_error.is_none()
}

pub(super) fn add_slot_focus_order(
    form: &AddSlotForm,
    cx: &Context<NativeRoot>,
) -> Vec<FocusHandle> {
    if form.submitting {
        return Vec::new();
    }
    let mut order = vec![
        form.close_focus.clone(),
        form.query.read(cx).focus_handle(),
        form.slot_handle_hint_focus.clone(),
        form.slot_handle.read(cx).focus_handle(),
        form.runtime_hint_focus.clone(),
        form.runtime_select.read(cx).focus_handle(),
    ];
    if !form.runtime_override.is_empty() {
        order.extend([
            form.model_hint_focus.clone(),
            form.model_override.read(cx).focus_handle(),
        ]);
    }
    order.extend([form.cancel_focus.clone(), form.submit_focus.clone()]);
    order
}

pub(super) fn crew_usage_label(role: &RoleWithActivity) -> String {
    if role.activity.crew_count == 1 {
        "in 1 crew".into()
    } else {
        format!("in {} crews", role.activity.crew_count)
    }
}

pub(super) fn role_activity_label(role: &RoleWithActivity) -> String {
    if role.activity.active_sessions > 0 {
        if role.activity.active_sessions == 1 {
            "1 session".into()
        } else {
            format!("{} sessions", role.activity.active_sessions)
        }
    } else if role.activity.active_missions > 0 {
        if role.activity.active_missions == 1 {
            "1 mission".into()
        } else {
            format!("{} missions", role.activity.active_missions)
        }
    } else {
        "idle".into()
    }
}

pub(super) fn slot_command_summary(slot: &SlotWithRole) -> String {
    if let Some(runtime) = slot
        .slot
        .runtime_override
        .as_deref()
        .filter(|runtime| *runtime != slot.role.runtime)
    {
        let command = runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .find(|entry| entry.name.key() == runtime)
            .map(|entry| entry.command)
            .unwrap_or_else(|| runtime.to_owned());
        let mut overrides = Vec::new();
        if let Some(model) = slot.slot.model_override.as_deref() {
            overrides.push(format!("model {model}"));
        }
        if let Some(effort) = slot.slot.effort_override.as_deref() {
            overrides.push(format!("effort {effort}"));
        }
        if runtime == "codex" {
            if let Some(speed) = slot.slot.codex_speed_override {
                overrides.push(format!(
                    "speed {}",
                    match speed {
                        runner_backend::model::CodexSpeed::Standard => "Standard",
                        runner_backend::model::CodexSpeed::Fast => "Fast",
                    }
                ));
            }
        }
        return if overrides.is_empty() {
            format!("{command} (runtime defaults)")
        } else {
            format!("{command} (runtime defaults · {})", overrides.join(" · "))
        };
    }
    let mut command = vec![slot.role.command.clone()];
    command.extend(slot.role.args.clone());
    let command = command
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut overrides = Vec::new();
    if let Some(model) = slot.slot.model_override.as_deref() {
        overrides.push(format!("model {model}"));
    }
    if let Some(effort) = slot.slot.effort_override.as_deref() {
        overrides.push(format!("effort {effort}"));
    }
    if slot.role.runtime == "codex" {
        if let Some(speed) = slot.slot.codex_speed_override {
            overrides.push(format!(
                "speed {}",
                match speed {
                    runner_backend::model::CodexSpeed::Standard => "Standard",
                    runner_backend::model::CodexSpeed::Fast => "Fast",
                }
            ));
        }
    }
    if overrides.is_empty() {
        command
    } else {
        format!("{command} ({})", overrides.join(" · "))
    }
}

pub(super) fn move_item<T: Clone>(items: &[T], from: usize, to: usize) -> Vec<T> {
    let mut reordered = items.to_vec();
    if from >= reordered.len() || to >= reordered.len() || from == to {
        return reordered;
    }
    let item = reordered.remove(from);
    reordered.insert(to, item);
    reordered
}

pub(super) fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn add_slot_form_is_composing(form: &AddSlotForm, cx: &Context<NativeRoot>) -> bool {
    form.query.read(cx).is_composing()
        || form.slot_handle.read(cx).is_composing()
        || form.model_override.read(cx).is_composing()
}

/// Where one avatar of a crew picture sits in its square tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PictureCell {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) size: f32,
}

/// The crew picture's layout, the same at every size: one slot fills the
/// tile, two sit side by side centred vertically, three are two over one with
/// the third centred, and four or more are a 2×2 grid of the first four.
pub(super) fn picture_cells(slots: usize, tile: f32) -> Vec<PictureCell> {
    let gap = tile * 0.05;
    let half = (tile - gap) / 2.;
    let cell = |left: f32, top: f32| PictureCell {
        left,
        top,
        size: half,
    };
    match slots {
        0 => Vec::new(),
        1 => vec![PictureCell {
            left: 0.,
            top: 0.,
            size: tile,
        }],
        2 => {
            let top = (tile - half) / 2.;
            vec![cell(0., top), cell(half + gap, top)]
        }
        3 => vec![
            cell(0., 0.),
            cell(half + gap, 0.),
            cell((tile - half) / 2., half + gap),
        ],
        _ => vec![
            cell(0., 0.),
            cell(half + gap, 0.),
            cell(0., half + gap),
            cell(half + gap, half + gap),
        ],
    }
}

/// The crew's picture: its slots' pixel avatars, in slot order.
pub(super) fn crew_picture(handles: &[SharedString], tile: f32) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .size(rems(tile / 16.))
        .children(
            picture_cells(handles.len(), tile)
                .into_iter()
                .zip(handles)
                .map(|(cell, handle)| {
                    div()
                        .absolute()
                        .left(rems(cell.left / 16.))
                        .top(rems(cell.top / 16.))
                        .child(RoleAvatar::new(handle.clone(), cell.size))
                }),
        )
        .into_any_element()
}

/// `5 slots · lead @lead`.
pub(super) fn crew_summary(slots: usize, lead: Option<&str>) -> String {
    let count = match slots {
        0 => return "No slots yet".to_owned(),
        1 => "1 slot".to_owned(),
        count => format!("{count} slots"),
    };
    match lead {
        Some(lead) => format!("{count} · lead @{lead}"),
        None => count,
    }
}

/// What a slot runs once its overrides apply, and which values it overrides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SlotSetup {
    pub(super) runtime: String,
    /// The slot pins a runtime, even one matching the role's: a pin keeps the
    /// slot on it when the role's runtime changes.
    pub(super) runtime_overridden: bool,
    /// The slot runs its role's runtime, so it inherits the role's model and
    /// effort.
    pub(super) own_runtime: bool,
    pub(super) model: Option<String>,
    pub(super) model_overridden: bool,
    pub(super) effort: Option<String>,
    pub(super) effort_overridden: bool,
    pub(super) speed: Option<runner_backend::model::CodexSpeed>,
    pub(super) speed_overridden: bool,
}

impl SlotSetup {
    pub(super) fn overrides_any(&self) -> bool {
        self.runtime_overridden
            || self.model_overridden
            || self.effort_overridden
            || self.speed_overridden
    }
}

/// A slot on its role's runtime inherits the role's model and effort; on
/// another runtime it starts from that runtime's own defaults.
pub(super) fn slot_setup(slot: &SlotWithRole) -> SlotSetup {
    let set = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let pinned = set(slot.slot.runtime_override.as_deref());
    let runtime_overridden = pinned.is_some();
    let runtime = pinned.unwrap_or_else(|| slot.role.runtime.clone());
    let own_runtime = runtime == slot.role.runtime;
    let layer = |overridden: Option<&str>, role: Option<&str>| match set(overridden) {
        Some(value) => (Some(value), true),
        None => (own_runtime.then(|| set(role)).flatten(), false),
    };
    let (model, model_overridden) = layer(
        slot.slot.model_override.as_deref(),
        slot.role.model.as_deref(),
    );
    let (effort, effort_overridden) = layer(
        slot.slot.effort_override.as_deref(),
        slot.role.effort.as_deref(),
    );
    let speed = if runtime == "codex" {
        slot.slot.codex_speed_override.or(if own_runtime {
            slot.role.codex_speed
        } else {
            None
        })
    } else {
        None
    };
    let speed_overridden = runtime == "codex" && slot.slot.codex_speed_override.is_some();
    SlotSetup {
        runtime_overridden,
        own_runtime,
        runtime,
        model,
        model_overridden,
        effort,
        effort_overridden,
        speed,
        speed_overridden,
    }
}

/// The runtimes a select offers for a slot: the role's default, which pins
/// nothing, then every available agent runtime plus the role's own and the
/// slot's current one even when they are not. Picking a runtime pins it,
/// the role's own included.
pub(super) fn slot_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    role_runtime: &str,
    current: &str,
) -> Vec<SelectOption> {
    let role_default = format!(
        "Role default ({})",
        crate::surfaces::roles::logic::runtime_display_name(role_runtime)
    );
    std::iter::once(SelectOption::new("", role_default))
        .chain(
            runtimes
                .iter()
                .filter(|runtime| {
                    runtime.available
                        || runtime.name.key() == role_runtime
                        || runtime.name.key() == current
                })
                .map(|runtime| {
                    SelectOption::new(runtime.name.to_string(), runtime.display_name.clone())
                }),
        )
        .collect()
}

/// The missions the card and the list count for one crew, newest first.
pub(super) fn crew_missions<'a>(missions: &'a [MissionSummary], crew_id: &str) -> Vec<&'a Mission> {
    let mut missions = missions
        .iter()
        .map(|summary| &summary.mission)
        .filter(|mission| mission.crew_id == crew_id)
        .collect::<Vec<_>>();
    missions.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.id.cmp(&a.id)));
    missions
}

/// Missions the card shows before its footer expands it.
pub(super) const MISSIONS_SHOWN: usize = 4;

/// `14 run · none live`.
pub(super) fn missions_header(missions: &[&Mission]) -> String {
    let live = missions
        .iter()
        .filter(|mission| mission.status == MissionStatus::Running)
        .count();
    if live == 0 {
        format!("{} run · none live", missions.len())
    } else {
        format!("{} run · {live} live", missions.len())
    }
}

/// The card's footer, or `None` when every mission already shows.
pub(super) fn missions_footer(total: usize, expanded: bool) -> Option<String> {
    if total <= MISSIONS_SHOWN {
        None
    } else if expanded {
        Some("Show less".to_owned())
    } else {
        Some(format!("Show all {total} missions"))
    }
}

/// How long a mission ran: `18m`, `1h 12m`, or so far while it runs.
pub(super) fn mission_duration(mission: &Mission, now: Timestamp) -> String {
    let end = match (mission.status, mission.stopped_at) {
        (MissionStatus::Running, _) => now,
        (_, Some(stopped)) => stopped,
        (_, None) => return "—".to_owned(),
    };
    let minutes = (end - mission.started_at).num_minutes().max(0);
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    }
}

/// A date: `Sep 23` this year, `Sep 23, 2025` before it.
pub(super) fn short_date<Tz: chrono::TimeZone>(
    started: &chrono::DateTime<Tz>,
    now: &chrono::DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    if started.year() == now.year() {
        started.format("%b %-d").to_string()
    } else {
        started.format("%b %-d, %Y").to_string()
    }
}

/// Each runtime a crew's slots run, with its slot count, in slot order.
pub(super) fn runtime_counts(members: &[CrewMemberPreview]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for member in members {
        match counts
            .iter_mut()
            .find(|(runtime, _)| *runtime == member.runtime)
        {
            Some((_, count)) => *count += 1,
            None => counts.push((member.runtime.clone(), 1)),
        }
    }
    counts
}

/// The list's Last mission cell and whether it is live: the running count,
/// the latest start, or a dash for a crew that never ran.
pub(super) fn last_mission_label<Tz: chrono::TimeZone>(
    missions: &[&Mission],
    now: &chrono::DateTime<Tz>,
) -> (String, bool)
where
    Tz::Offset: std::fmt::Display,
{
    let running = missions
        .iter()
        .filter(|mission| mission.status == MissionStatus::Running)
        .count();
    if running > 0 {
        let label = if running == 1 {
            "1 mission running".to_owned()
        } else {
            format!("{running} missions running")
        };
        return (label, true);
    }
    let label = missions
        .iter()
        .map(|mission| mission.started_at)
        .max()
        .map(|started| {
            crate::surfaces::profile_page::short_timestamp(
                &started.with_timezone(&now.timezone()),
                now,
            )
        })
        .unwrap_or_else(|| "—".to_owned());
    (label, false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CrewColumn {
    Runtimes,
    Missions,
    LastMission,
}

pub(super) const CREW_ROW_PADDING_X: f32 = 12.;
pub(super) const CREW_CELL_MIN_WIDTH: f32 = 176.;
pub(super) const CREW_ROW_ACTIONS_WIDTH: f32 = 148.;

impl CrewColumn {
    const ORDER: [Self; 3] = [Self::Runtimes, Self::Missions, Self::LastMission];

    pub(super) fn width(self) -> f32 {
        match self {
            Self::Runtimes => 176.,
            Self::Missions => 96.,
            Self::LastMission => 176.,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Runtimes => "Runtimes",
            Self::Missions => "Missions",
            Self::LastMission => "Last mission",
        }
    }
}

/// The columns a table this wide shows beside Crew: a narrow table drops
/// Missions first, then Runtimes, then Last mission.
pub(super) fn crew_table_columns(table_width: f32) -> Vec<CrewColumn> {
    const PRIORITY: [CrewColumn; 3] = [
        CrewColumn::LastMission,
        CrewColumn::Runtimes,
        CrewColumn::Missions,
    ];
    let mut budget =
        table_width - 2. * CREW_ROW_PADDING_X - CREW_CELL_MIN_WIDTH - CREW_ROW_ACTIONS_WIDTH;
    let mut shown = Vec::new();
    for column in PRIORITY {
        if column.width() > budget {
            break;
        }
        budget -= column.width();
        shown.push(column);
    }
    CrewColumn::ORDER
        .into_iter()
        .filter(|column| shown.contains(column))
        .collect()
}

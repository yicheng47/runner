use runner_backend::model::{CodexSpeed, Runtime};

use chrono::{DateTime, TimeZone};
use gpui::prelude::*;
use gpui::{div, AnyElement, Context};
use runner_app::ui::SelectOption;
use runner_backend::model::Role;
use runner_backend::ops::role::RoleActivity;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};
use runner_backend::ops::slot::CrewMembership;
use runner_backend::router::runtime::PermissionMode;

use super::*;
pub(super) use crate::surfaces::profile_page::{
    local_short_timestamp, plural, prompt_meta, short_id, short_timestamp,
};
use crate::*;

#[derive(Clone, Copy)]
pub(super) enum RoleFormKind {
    Create,
    Edit,
}

pub(super) fn resolve_role_edit(role: &Role) -> RoleEditResolution {
    RoleEditResolution {
        runtime: role.runtime.clone(),
        command: role.command.clone(),
        model: role.model.clone().unwrap_or_default(),
        effort: role.effort.clone().unwrap_or_default(),
        speed: speed_value(role.codex_speed).to_string(),
    }
}

pub(super) fn speed_value(speed: Option<CodexSpeed>) -> &'static str {
    match speed {
        None => "inherit",
        Some(CodexSpeed::Standard) => "standard",
        Some(CodexSpeed::Fast) => "fast",
    }
}

pub(super) fn parse_speed(value: &str) -> Option<CodexSpeed> {
    match value {
        "standard" => Some(CodexSpeed::Standard),
        "fast" => Some(CodexSpeed::Fast),
        _ => None,
    }
}

pub(super) fn speed_options() -> Vec<SelectOption> {
    ["inherit", "standard", "fast"]
        .into_iter()
        .map(|value| {
            SelectOption::new(
                value,
                match value {
                    "standard" => "Standard",
                    "fast" => "Fast",
                    _ => "Inherit",
                },
            )
        })
        .collect()
}

pub(crate) fn ensure_runtime_present(
    core: &AppCore,
    runtimes: &mut Vec<RuntimeCatalogEntry>,
    name: &str,
) {
    if runtimes.iter().any(|runtime| runtime.name.key() == name) {
        return;
    }
    if let Ok(catalog) = runner_backend::ops::runtime::runtime_catalog(core) {
        if let Some(runtime) = catalog
            .into_iter()
            .find(|runtime| runtime.name.key() == name)
        {
            runtimes.push(runtime);
            runtimes.sort_by_key(|entry| Runtime::ALL.iter().position(|name| *name == entry.name));
        }
    }
}

pub(crate) fn runtime_entry<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> Option<&'a RuntimeCatalogEntry> {
    runtimes.iter().find(|runtime| runtime.name.key() == name)
}

pub(crate) fn runtime_models<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default()
}

pub(crate) fn runtime_model_placeholder(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    inherited_role: Option<&Role>,
) -> String {
    inherited_role
        .filter(|role| role.runtime == runtime)
        .and_then(|role| role.model.as_deref())
        .or_else(|| runtime_entry(runtimes, runtime)?.default_model.as_deref())
        .map(|model| format!("default ({model})"))
        .unwrap_or_else(|| "default".into())
}

pub(crate) fn runtime_default_effort_label(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
) -> String {
    runtime_entry(runtimes, runtime)
        .and_then(|runtime| runtime.default_effort.as_deref())
        .map(|effort| format!("Runtime default ({effort})"))
        .unwrap_or_else(|| "Runtime default".into())
}

pub(crate) fn runtime_efforts<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default()
}

/// The agent runtimes a select offers: the available ones, plus the role's
/// own and the current one even when they are not.
pub(crate) fn role_edit_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    role: &Role,
    current_runtime: &str,
) -> Vec<SelectOption> {
    runtimes
        .iter()
        .filter(|runtime| {
            runtime.available
                || runtime.name.key() == role.runtime
                || runtime.name.key() == current_runtime
        })
        .map(|runtime| {
            let option = SelectOption::new(runtime.name.to_string(), runtime.display_name.clone());
            if runtime.description.is_empty() {
                option
            } else {
                option.description(runtime.description.clone())
            }
        })
        .collect()
}

/// The effort choices for a runtime and model. A slot's blank choice inherits
/// the role's effort while it runs the role's own runtime.
pub(crate) fn effort_options(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    role: &Role,
    edits_slot: bool,
    model: &str,
) -> Vec<SelectOption> {
    runtime_entry(runtimes, runtime)
        .map(|runtime| {
            if edits_slot {
                runtime.efforts_for_model(model)
            } else {
                role_model_efforts(runtime, model)
            }
        })
        .unwrap_or_default()
        .iter()
        .map(|option| {
            let label = if option.value.is_empty() {
                if edits_slot && runtime == role.runtime {
                    role.effort
                        .as_deref()
                        .or_else(|| runtime_entry(runtimes, runtime)?.default_effort.as_deref())
                        .map(|effort| format!("Role default ({effort})"))
                        .unwrap_or_else(|| "Role default".into())
                } else {
                    runtime_default_effort_label(runtimes, runtime)
                }
            } else {
                option.label.clone()
            };
            let mut select = SelectOption::new(option.value.clone(), label);
            if let Some(description) = option.description.clone() {
                select = select.description(description);
            }
            select
        })
        .collect()
}

pub(super) fn create_role_effort_options(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    model: &str,
) -> Vec<SelectOption> {
    runtime_entry(runtimes, runtime)
        .map(|entry| role_model_efforts(entry, model))
        .unwrap_or_default()
        .iter()
        .map(|option| {
            let label = if option.value.is_empty() {
                runtime_default_effort_label(runtimes, runtime)
            } else {
                option.label.clone()
            };
            let mut select = SelectOption::new(option.value.clone(), label);
            if let Some(description) = &option.description {
                select = select.description(description.clone());
            }
            select
        })
        .collect()
}

fn role_model_efforts(runtime: &RuntimeCatalogEntry, model: &str) -> Vec<RuntimeCatalogOption> {
    let model = if model.trim().is_empty() {
        runtime.default_model.as_deref().unwrap_or_default()
    } else {
        model
    };
    if model.trim().is_empty() {
        runtime.efforts.clone()
    } else {
        runtime.efforts_for_model(model)
    }
}

pub(super) fn permission_modes(runtime: &str) -> &'static [PermissionMode] {
    runner_backend::runtimes::for_key(runtime)
        .permissions()
        .offered
}

/// The permission mode a role's args carry, or `None` for a runtime that has
/// no modes. A row saved with a mode its runtime no longer offers — a Trae row
/// with the old `--permission-mode auto` still infers as Auto (#599) — reads as
/// the fallback, which is what saving then writes.
pub(super) fn role_permission_mode(role: &Role) -> Option<PermissionMode> {
    let modes = permission_modes(&role.runtime);
    if modes.is_empty() {
        return None;
    }
    let inferred = runner_backend::runtimes::for_key(&role.runtime)
        .permissions()
        .infer(&role.args);
    Some(if modes.contains(&inferred) {
        inferred
    } else {
        PermissionMode::Default
    })
}

pub(super) fn validate_role_handle(handle: &str) -> Option<&'static str> {
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

pub(super) fn create_role_can_submit(form: &CreateRoleForm) -> bool {
    !form.submitting
        && !form.handle_empty
        && form.handle_error.is_none()
        && form.display_name_valid
        && runtime_entry(&form.runtimes, &form.runtime).is_some()
}

/// Whether the in-place editor holds anything a save would write.
pub(super) fn role_edit_is_dirty(form: &RoleEditForm, cx: &Context<NativeRoot>) -> bool {
    let role = &form.role;
    let stored = |value: &Option<String>| value.as_deref().and_then(trimmed_option);
    form.display_name.read(cx).text().trim() != role.display_name.trim()
        || form.runtime != role.runtime
        || role_edit_args(form, cx) != role_visible_args(role)
        || trimmed_option(form.model.read(cx).text()) != stored(&role.model)
        || trimmed_option(&form.effort) != stored(&role.effort)
        || parse_speed(&form.speed) != role.codex_speed
        || trimmed_option(form.working_dir.read(cx).text()) != stored(&role.working_dir)
        || trimmed_option(form.system_prompt.read(cx).text()) != stored(&role.system_prompt)
}

/// Mission permissions are fixed, so old role permission flags stay out of the form.
pub(super) fn role_visible_args(role: &Role) -> Vec<String> {
    runner_backend::runtimes::for_key(&role.runtime)
        .permissions()
        .strip(&role.args)
}

/// The args a save writes. The field joins args with spaces, so while it
/// still reads as the stored args, the stored vector goes back untouched and
/// an arg holding a space keeps its grouping; an edited field splits on
/// whitespace.
pub(super) fn role_edit_args(form: &RoleEditForm, cx: &Context<NativeRoot>) -> Vec<String> {
    let visible = role_visible_args(&form.role);
    let edited = split_args(form.args.read(cx).text());
    if edited == split_args(&visible.join(" ")) {
        visible
    } else {
        edited
    }
}

pub(super) fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn split_args(value: &str) -> Vec<String> {
    value.split_whitespace().map(ToOwned::to_owned).collect()
}

pub(super) fn create_role_form_is_composing(
    form: &CreateRoleForm,
    cx: &Context<NativeRoot>,
) -> bool {
    form.handle.read(cx).is_composing()
        || form.display_name.read(cx).is_composing()
        || form.args.read(cx).is_composing()
        || form.model.read(cx).is_composing()
        || form.working_dir.read(cx).is_composing()
        || form.system_prompt.read(cx).is_composing()
}

pub(super) fn role_edit_form_is_composing(form: &RoleEditForm, cx: &Context<NativeRoot>) -> bool {
    form.display_name.read(cx).is_composing()
        || form.args.read(cx).is_composing()
        || form.model.read(cx).is_composing()
        || form.working_dir.read(cx).is_composing()
        || form.system_prompt.read(cx).is_composing()
}

pub(super) fn error_banner(error: String) -> AnyElement {
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

/// A model or effort cell: the value, or `default` (dimmed) when unset.
pub(crate) fn role_setting_label(value: Option<&str>) -> (String, bool) {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => (value.to_owned(), false),
        None => ("default".to_owned(), true),
    }
}

/// A role can fill several slots of one crew; the heading counts crews.
pub(super) fn distinct_crew_count(crews: &[CrewMembership]) -> usize {
    crews
        .iter()
        .map(|membership| membership.crew_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

pub(super) fn crews_label(count: i64) -> String {
    if count > 0 {
        count.to_string()
    } else {
        "—".to_owned()
    }
}

/// `2 sessions · 1 mission` while the role runs anywhere, else `None`.
pub(super) fn live_activity_label(activity: &RoleActivity) -> Option<String> {
    let sessions = plural(activity.active_sessions, "session", "sessions");
    if activity.active_missions > 0 {
        let missions = plural(activity.active_missions, "mission", "missions");
        Some(format!("{sessions} · {missions}"))
    } else if activity.active_sessions > 0 {
        Some(sessions)
    } else {
        None
    }
}

/// The list's Last active cell and whether it is live: the live counts, the
/// last start, or a dash for a role that never ran.
pub(super) fn last_active_label<Tz: TimeZone>(
    activity: &RoleActivity,
    now: &DateTime<Tz>,
) -> (String, bool)
where
    Tz::Offset: std::fmt::Display,
{
    if let Some(live) = live_activity_label(activity) {
        return (live, true);
    }
    let last = activity
        .last_started_at
        .map(|started| short_timestamp(&started.with_timezone(&now.timezone()), now))
        .unwrap_or_else(|| "—".to_owned());
    (last, false)
}

pub(crate) fn runtime_display_name(runtime: &str) -> String {
    runner_backend::ops::runtime::runtime_list()
        .into_iter()
        .find(|definition| definition.name.key() == runtime)
        .map(|definition| definition.display_name)
        .unwrap_or_else(|| runtime.to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RoleColumn {
    Runtime,
    Model,
    Effort,
    Crews,
    LastActive,
}

pub(super) const ROLE_ROW_PADDING_X: f32 = 12.;
pub(super) const ROLE_CELL_MIN_WIDTH: f32 = 176.;
pub(super) const ROLE_ROW_ACTIONS_WIDTH: f32 = 92.;

impl RoleColumn {
    const ORDER: [Self; 5] = [
        Self::Runtime,
        Self::Model,
        Self::Effort,
        Self::Crews,
        Self::LastActive,
    ];

    pub(super) fn width(self) -> f32 {
        match self {
            Self::Runtime => 140.,
            Self::Model => 116.,
            Self::Effort => 84.,
            Self::Crews => 56.,
            Self::LastActive => 168.,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Runtime => "Runtime",
            Self::Model => "Model",
            Self::Effort => "Effort",
            Self::Crews => "Crews",
            Self::LastActive => "Last active",
        }
    }
}

/// The columns a table this wide shows beside Role. A narrow table drops the
/// least telling columns first, and the Role column takes whatever is left.
pub(super) fn role_table_columns(table_width: f32) -> Vec<RoleColumn> {
    const PRIORITY: [RoleColumn; 5] = [
        RoleColumn::LastActive,
        RoleColumn::Runtime,
        RoleColumn::Model,
        RoleColumn::Effort,
        RoleColumn::Crews,
    ];
    let mut budget =
        table_width - 2. * ROLE_ROW_PADDING_X - ROLE_CELL_MIN_WIDTH - ROLE_ROW_ACTIONS_WIDTH;
    let mut shown = Vec::new();
    for column in PRIORITY {
        if column.width() > budget {
            break;
        }
        budget -= column.width();
        shown.push(column);
    }
    RoleColumn::ORDER
        .into_iter()
        .filter(|column| shown.contains(column))
        .collect()
}

use runner_backend::model::Runtime;

use chrono::{DateTime, Datelike, TimeZone};
use gpui::prelude::*;
use gpui::{div, AnyElement, Context, FocusHandle};
use runner_app::ui::SelectOption;
use runner_backend::model::Role;
use runner_backend::ops::role::RoleActivity;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};
use runner_backend::ops::slot::CrewMembership;
use runner_backend::router::runtime::PermissionMode;

use super::*;
use crate::*;

#[derive(Clone, Copy)]
pub(super) enum RoleFormKind {
    Create,
    Edit,
}

pub(super) fn resolve_slot_runtime_layers(
    role_runtime: &str,
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> RuntimeLayerResolution {
    let runtime = runtime_override.unwrap_or(role_runtime);
    RuntimeLayerResolution {
        runtime: runtime.to_owned(),
        runtime_pinned: runtime_override.is_some(),
        model: model_override.map(ToOwned::to_owned),
        effort: effort_override.map(ToOwned::to_owned),
    }
}

pub(super) fn resolve_role_edit(
    role: &Role,
    slot: Option<&runner_backend::model::SlotWithRole>,
) -> RoleEditResolution {
    let layers = if let Some(slot) = slot {
        resolve_slot_runtime_layers(
            &role.runtime,
            slot.slot.runtime_override.as_deref(),
            slot.slot.model_override.as_deref(),
            slot.slot.effort_override.as_deref(),
        )
    } else {
        RuntimeLayerResolution {
            runtime: role.runtime.clone(),
            runtime_pinned: true,
            model: role.model.clone(),
            effort: role.effort.clone(),
        }
    };
    let command = if layers.runtime == role.runtime {
        role.command.clone()
    } else {
        runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .find(|runtime| runtime.name.key() == layers.runtime)
            .map(|runtime| runtime.command)
            .unwrap_or_else(|| role.command.clone())
    };
    RoleEditResolution {
        runtime: layers.runtime,
        runtime_pinned: slot.is_none() || layers.runtime_pinned,
        command,
        model: layers.model.unwrap_or_default(),
        effort: layers.effort.unwrap_or_default(),
    }
}

pub(super) fn ensure_runtime_present(
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
        }
    }
}

pub(super) fn runtime_entry<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> Option<&'a RuntimeCatalogEntry> {
    runtimes.iter().find(|runtime| runtime.name.key() == name)
}

pub(super) fn runtime_models<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default()
}

pub(super) fn runtime_model_placeholder(
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

pub(super) fn runtime_default_effort_label(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
) -> String {
    runtime_entry(runtimes, runtime)
        .and_then(|runtime| runtime.default_effort.as_deref())
        .map(|effort| format!("Runtime default ({effort})"))
        .unwrap_or_else(|| "Runtime default".into())
}

pub(super) fn runtime_efforts<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default()
}

pub(super) fn role_edit_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    role: &Role,
    current_runtime: &str,
    edits_slot: bool,
) -> Vec<SelectOption> {
    let mut options = Vec::new();
    if edits_slot {
        let label = runtime_entry(runtimes, &role.runtime)
            .map(|runtime| runtime.display_name.as_str())
            .unwrap_or(&role.runtime);
        options.push(SelectOption::new("", format!("Role default ({label})")));
    }
    options.extend(
        runtimes
            .iter()
            .filter(|runtime| {
                runtime.available
                    || runtime.name.key() == role.runtime
                    || runtime.name.key() == current_runtime
            })
            .map(|runtime| {
                let option =
                    SelectOption::new(runtime.name.to_string(), runtime.display_name.clone());
                if runtime.description.is_empty() {
                    option
                } else {
                    option.description(runtime.description.clone())
                }
            }),
    );
    options
}

pub(super) fn effort_options(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    role: &Role,
    edits_slot: bool,
    model: &str,
) -> Vec<SelectOption> {
    runtime_entry(runtimes, runtime)
        .map(|runtime| runtime.efforts_for_model(model))
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

pub(super) fn permission_modes(runtime: &str) -> &'static [PermissionMode] {
    match Runtime::parse(runtime) {
        Some(Runtime::ClaudeCode) => &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        Some(Runtime::Codex) => &[
            PermissionMode::Default,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        // TRAE CLI has no auto-approve middle ground — `default`,
        // `plan`, `bypass_permissions` — so offering Auto would write
        // nothing and read back as Default (#599).
        Some(Runtime::Trae) => &[PermissionMode::Default, PermissionMode::Bypass],
        Some(Runtime::Copilot) => &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass,
        ],
        Some(Runtime::Pi | Runtime::Shell) | None => &[],
    }
}

pub(super) fn permission_options(runtime: &str) -> Vec<SelectOption> {
    permission_modes(runtime)
        .iter()
        .copied()
        .map(|mode| {
            SelectOption::new(permission_mode_value(mode), permission_mode_label(mode))
                .description(permission_mode_description(runtime, mode))
                .danger(mode == PermissionMode::Bypass)
        })
        .collect()
}

pub(super) fn permission_mode_value(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept_edits",
        PermissionMode::Auto => "auto",
        PermissionMode::Bypass => "bypass",
    }
}

pub(super) fn parse_permission_mode(value: &str) -> PermissionMode {
    match value {
        "accept_edits" => PermissionMode::AcceptEdits,
        "auto" => PermissionMode::Auto,
        "bypass" => PermissionMode::Bypass,
        _ => PermissionMode::Default,
    }
}

pub(super) fn permission_mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "Default",
        PermissionMode::AcceptEdits => "Accept edits",
        PermissionMode::Auto => "Auto",
        PermissionMode::Bypass => "Bypass",
    }
}

pub(super) fn permission_mode_description(runtime: &str, mode: PermissionMode) -> &'static str {
    match (Runtime::parse(runtime), mode) {
        (Some(Runtime::ClaudeCode), PermissionMode::Default) => {
            "Ask for every tool, shell command, and write."
        }
        (Some(Runtime::ClaudeCode), PermissionMode::AcceptEdits) => "Auto-accept file edits and common filesystem commands; still ask for shell, network, and writes outside the workspace. Available on every plan.",
        (Some(Runtime::ClaudeCode), PermissionMode::Auto) => "Real auto with a server-side classifier. Requires Max / Team / Enterprise / API plan + a supported model (Opus 4.7 on Max). Not available on Pro.",
        (Some(Runtime::ClaudeCode), PermissionMode::Bypass) => "Skip every check. Runner accepts Claude Code's bypass disclaimer for the sessions it spawns; a role that passes its own --settings still sees the dialog.",
        (Some(Runtime::Codex), PermissionMode::Default) => {
            "Codex's built-in approval cadence (untrusted commands)."
        }
        (Some(Runtime::Codex), PermissionMode::Auto) => "Auto-run in the workspace and ask only when the model decides approval is needed (`--ask-for-approval on-request`).",
        (Some(Runtime::Codex), PermissionMode::Bypass) => "Never ask while keeping Codex's workspace-write sandbox (`--ask-for-approval never`).",
        (Some(Runtime::Trae), PermissionMode::Default) => "TRAE CLI's built-in approval cadence.",
        (Some(Runtime::Trae), PermissionMode::Bypass) => {
            "Bypass TRAE CLI permission prompts (`--permission-mode bypass_permissions`)."
        }
        (Some(Runtime::Copilot), PermissionMode::Default) => "Copilot's own manual mode: read-only tools run, writes and shell commands ask. Governed by defaultPermissionMode in ~/.copilot/settings.json.",
        (Some(Runtime::Copilot), PermissionMode::AcceptEdits) => "File creates and edits run without asking; shell commands, URLs and paths outside the cwd still prompt.",
        (Some(Runtime::Copilot), PermissionMode::Bypass) => "Every tool, path and URL is allowed. Same flag for the app-wide mission permission mode; chats never carry it (#596).",
        (Some(Runtime::Copilot), PermissionMode::Auto) => "",
        (Some(Runtime::Codex | Runtime::Trae), PermissionMode::AcceptEdits)
        | (Some(Runtime::Trae), PermissionMode::Auto)
        | (Some(Runtime::Pi), _)
        | (Some(Runtime::Shell) | None, _) => "",
    }
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
    let inferred = runner_backend::router::runtime::infer_permission_mode(
        Runtime::parse(&role.runtime),
        &role.args,
    );
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

pub(super) fn create_role_focus_order(
    form: &CreateRoleForm,
    cx: &Context<NativeRoot>,
) -> Vec<FocusHandle> {
    if form.submitting {
        return Vec::new();
    }
    let mut order = vec![
        form.close_focus.clone(),
        form.handle.read(cx).focus_handle(),
        form.display_name.read(cx).focus_handle(),
        form.runtime_select.read(cx).focus_handle(),
        form.args_hint_focus.clone(),
        form.args.read(cx).focus_handle(),
        form.model_hint_focus.clone(),
        form.model.read(cx).focus_handle(),
    ];
    if !permission_modes(&form.runtime).is_empty() {
        order.extend([
            form.permission_hint_focus.clone(),
            form.permission_select.read(cx).focus_handle(),
        ]);
    }
    order.extend([
        form.working_dir.read(cx).focus_handle(),
        form.browse_focus.clone(),
        form.system_prompt.read(cx).focus_handle(),
        form.cancel_focus.clone(),
        form.submit_focus.clone(),
    ]);
    order
}

pub(super) fn role_edit_focus_order(
    form: &RoleEditForm,
    cx: &Context<NativeRoot>,
) -> Vec<FocusHandle> {
    if form.submitting {
        return Vec::new();
    }
    let mut order = vec![
        form.close_focus.clone(),
        form.display_name.read(cx).focus_handle(),
    ];
    if form.slot.is_some() {
        order.push(form.runtime_hint_focus.clone());
    }
    order.push(form.runtime_select.read(cx).focus_handle());
    if form.slot.is_none() {
        order.extend([
            form.args_hint_focus.clone(),
            form.args.read(cx).focus_handle(),
        ]);
    }
    order.extend([
        form.model_hint_focus.clone(),
        form.model.read(cx).focus_handle(),
    ]);
    if !runtime_efforts(&form.runtimes, &form.runtime).is_empty() {
        order.extend([
            form.effort_hint_focus.clone(),
            form.effort_select.read(cx).focus_handle(),
        ]);
    }
    if form.slot.is_none() && !permission_modes(&form.runtime).is_empty() {
        order.extend([
            form.permission_hint_focus.clone(),
            form.permission_select.read(cx).focus_handle(),
        ]);
    }
    order.extend([
        form.working_dir.read(cx).focus_handle(),
        form.browse_focus.clone(),
        form.system_prompt.read(cx).focus_handle(),
        form.cancel_focus.clone(),
        form.submit_focus.clone(),
    ]);
    order
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
        || (!permission_modes(&form.runtime).is_empty()
            && Some(form.permission_mode) != role_permission_mode(role))
        || trimmed_option(form.working_dir.read(cx).text()) != stored(&role.working_dir)
        || trimmed_option(form.system_prompt.read(cx).text()) != stored(&role.system_prompt)
}

/// The args the form shows: the stored ones less the permission flags its
/// Permissions control owns.
pub(super) fn role_visible_args(role: &Role) -> Vec<String> {
    runner_backend::router::runtime::strip_permission_flags(
        Runtime::parse(&role.runtime),
        &role.args,
    )
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

pub(super) fn plural(count: i64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
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

/// Source lines the role page's collapsed prompt card renders.
pub(super) const PROMPT_PREVIEW_LINES: usize = 24;

/// The prompt card's header: `48 lines · 2.9 KB`.
pub(super) fn prompt_meta(prompt: &str) -> String {
    let lines = plural(prompt.lines().count() as i64, "line", "lines");
    let bytes = prompt.len();
    if bytes < 1024 {
        format!("{lines} · {bytes} B")
    } else {
        format!("{lines} · {:.1} KB", bytes as f64 / 1024.)
    }
}

/// The first lines of a prompt too long to show whole, or `None` when it fits.
pub(super) fn prompt_preview(prompt: &str) -> Option<&str> {
    if prompt.lines().count() <= PROMPT_PREVIEW_LINES {
        return None;
    }
    let end = prompt
        .match_indices('\n')
        .nth(PROMPT_PREVIEW_LINES - 1)
        .map_or(prompt.len(), |(index, _)| index);
    Some(&prompt[..end])
}

/// A model or effort cell: the value, or `default` (dimmed) when unset.
pub(super) fn role_setting_label(value: Option<&str>) -> (String, bool) {
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

/// `Sep 23, 17:41` this year, `Sep 23, 2025` before it.
pub(super) fn short_timestamp<Tz: TimeZone>(timestamp: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if timestamp.year() == now.year() {
        timestamp.format("%b %-d, %H:%M").to_string()
    } else {
        timestamp.format("%b %-d, %Y").to_string()
    }
}

pub(super) fn local_short_timestamp(timestamp: runner_backend::model::Timestamp) -> String {
    short_timestamp(
        &timestamp.with_timezone(&chrono::Local),
        &chrono::Local::now(),
    )
}

/// `01K0…RCODER01`: enough of an id to tell rows apart.
pub(super) fn short_id(id: &str) -> String {
    if id.len() <= 12 || !id.is_ascii() {
        return id.to_owned();
    }
    format!("{}…{}", &id[..4], &id[id.len() - 8..])
}

pub(super) fn runtime_display_name(runtime: &str) -> String {
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

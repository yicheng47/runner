use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, AnyElement, Context, FocusHandle, FontWeight, HighlightStyle, KeyDownEvent, StyledText,
    Window,
};
use runner_app::ui::SelectOption;
use runner_backend::model::SlotWithRole;
use runner_backend::ops::role::RoleWithActivity;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};

use super::*;
use crate::*;

pub(super) fn section_label(label: &'static str) -> AnyElement {
    div()
        .text_size(theme::text_caption())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label.to_uppercase())
        .into_any_element()
}

pub(super) fn slot_section_description() -> StyledText {
    let description = "Positions in the crew. Each slot binds a handle to a runner. The LEAD is the crew's face — receives human messages by default and dispatches back to other slots.";
    let lead_start = description
        .find("LEAD")
        .expect("slot description contains LEAD");
    StyledText::new(description).with_highlights([(
        lead_start..lead_start + 4,
        HighlightStyle {
            color: Some(theme::accent()),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    )])
}

pub(super) fn crew_name_state(value: &str, original: &str) -> (bool, bool, bool) {
    let trimmed = value.trim();
    (
        value != original,
        !trimmed.is_empty() && trimmed != original,
        trimmed.is_empty(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CrewNameRefresh {
    MarkClean,
    Reset,
    Preserve,
}

pub(super) fn crew_name_refresh(current: &str, persisted: &str, edited: bool) -> CrewNameRefresh {
    if current == persisted {
        CrewNameRefresh::MarkClean
    } else if edited {
        CrewNameRefresh::Preserve
    } else {
        CrewNameRefresh::Reset
    }
}

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
        .map(|role| format!("Runner default ({})", role.role.runtime))
        .unwrap_or_else(|| "Runner default".into());
    let mut options = vec![
        SelectOption::new("", default).description("Use the runtime configured on the runner.")
    ];
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

pub(super) fn create_crew_form_is_composing(
    form: &CreateCrewForm,
    cx: &Context<NativeRoot>,
) -> bool {
    form.name.read(cx).is_composing()
        || form.purpose.read(cx).is_composing()
        || form.goal.read(cx).is_composing()
}

pub(super) fn add_slot_form_is_composing(form: &AddSlotForm, cx: &Context<NativeRoot>) -> bool {
    form.query.read(cx).is_composing()
        || form.slot_handle.read(cx).is_composing()
        || form.model_override.read(cx).is_composing()
}

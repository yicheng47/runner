use super::*;

pub(super) fn selection_handler(
    root: &Entity<NativeRoot>,
    selection: StartChatSelection,
) -> SelectHandler {
    let root = root.clone();
    Rc::new(move |value, _, cx| {
        root.update(cx, |this, cx| {
            this.select_start_chat_choice(selection, &value, cx)
        });
    })
}

pub(super) fn role_options(roles: &[RoleChoice]) -> Vec<SelectOption> {
    roles
        .iter()
        .map(|role| {
            SelectOption::new(role.id.clone(), role.display_name.clone())
                .description(format!("@{}", role.handle))
                .leading(role_leading(&role.handle))
        })
        .collect()
}

pub(super) fn role_leading(handle: &str) -> SelectLeading {
    let seed = handle.to_owned();
    SelectLeading::new(format!("role:{handle}"), move |size| {
        RoleAvatar::new(seed.clone(), size).into_any_element()
    })
}

pub(super) fn runtime_leading(runtime: &str) -> SelectLeading {
    let name = runtime.to_owned();
    SelectLeading::new(format!("runtime:{runtime}"), move |size| {
        runtime_mark(&name, size)
    })
}

/// The agents a picker offers: the name, the command in monospace under it,
/// and the provider's mark.
pub(super) fn runtime_options<'a>(
    runtimes: impl IntoIterator<Item = &'a RuntimeChoice>,
) -> Vec<SelectOption> {
    runtimes
        .into_iter()
        .map(|runtime| {
            let mut option =
                SelectOption::new(runtime.name.to_string(), runtime.display_name.clone())
                    .leading(runtime_leading(runtime.name.key()))
                    .disabled(!runtime.available);
            if let Some(command) = &runtime.command {
                option = option.description(command.clone());
            }
            option
        })
        .collect()
}

/// The agents a role chat can run on: every selectable one, plus the role's
/// own when it is not. The role's own is the untouched choice.
pub(super) fn role_runtime_options(
    runtimes: &[RuntimeChoice],
    role: Option<&RoleChoice>,
) -> Vec<SelectOption> {
    let mut options = runtime_options(runtimes)
        .into_iter()
        .map(|option| {
            let marked = role.is_some_and(|role| role.runtime != option.value);
            option.without_description().marked(marked)
        })
        .collect::<Vec<_>>();
    if let Some(role) =
        role.filter(|role| options.iter().all(|option| option.value != role.runtime))
    {
        options.push(
            SelectOption::new(
                role.runtime.clone(),
                runtime_display_name(runtimes, &role.runtime),
            )
            .leading(runtime_leading(&role.runtime)),
        );
    }
    options
}

pub(super) fn option_select_options(options: &[RuntimeCatalogOption]) -> Vec<SelectOption> {
    options
        .iter()
        .map(runner_app::ui::select::effort_option)
        .collect()
}

pub(super) fn parse_speed(value: &str) -> Option<CodexSpeed> {
    match value {
        "standard" => Some(CodexSpeed::Standard),
        "fast" => Some(CodexSpeed::Fast),
        _ => None,
    }
}

pub(super) fn speed_label(speed: Option<CodexSpeed>) -> &'static str {
    match speed {
        None => crate::i18n::text("ui.inherit"),
        Some(CodexSpeed::Standard) => crate::i18n::text("ui.standard"),
        Some(CodexSpeed::Fast) => crate::i18n::text("ui.fast"),
    }
}

/// Speed's choices. A role that sets a speed has no Inherit: it could not undo
/// the role's value. On the role's own Codex agent a choice that differs from
/// `baseline` carries the amber dot.
pub(super) fn speed_options(baseline: Option<&str>, role_sets_speed: bool) -> Vec<SelectOption> {
    let option = |value: &'static str, key: &'static str| {
        SelectOption::new(value, crate::i18n::message(key))
            .marked(baseline.is_some_and(|baseline| baseline != value))
    };
    let mut options = Vec::new();
    if !role_sets_speed {
        options.push(option("inherit", "ui.inherit"));
    }
    options.push(option("standard", "ui.standard"));
    options.push(option("fast", "ui.fast"));
    options
}

pub(super) fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// What a blank Model reads: the role's explicit model while the chat runs on
/// the role's own agent, else `default` because no model reaches the launch.
pub(super) fn model_placeholder(role: Option<&RoleChoice>) -> i18n::Text {
    role.and_then(|role| trimmed(role.model.as_deref()))
        .map(i18n::Text::from)
        .unwrap_or_else(|| i18n::message("word.default"))
}

/// The role whose model, effort and speed the controls start from: only the
/// role the chat runs on the agent of.
pub(super) fn inheriting_role(modal: &StartChatModal) -> Option<&RoleChoice> {
    modal
        .on_own_agent()
        .then(|| modal.selected_role())
        .flatten()
}

/// The model that will reach the launch: the one typed, else the role's own,
/// which the backend keeps when only an effort is overridden. The `default`
/// placeholder is not one: nothing is sent for it.
pub(super) fn launch_model(modal: &StartChatModal, cx: &App) -> Option<String> {
    normalized_value(modal.model.read(cx).text()).or_else(|| {
        inheriting_role(modal)
            .and_then(|role| trimmed(role.model.as_deref()))
            .map(str::to_owned)
    })
}

/// Whether the agent drops an effort that has no model beside it. The launch
/// gives Antigravity `--effort` only together with `--model`, and only for a
/// pair its catalog lists, so its effort waits for a model that reaches the
/// launch.
pub(super) fn effort_needs_launch_model(runtime: &RuntimeChoice) -> bool {
    runtime.effort_needs_launch_model
}

pub(super) fn sync_role_runtime_select(modal: &StartChatModal, cx: &mut Context<NativeRoot>) {
    let options = role_runtime_options(&modal.runtimes, modal.selected_role());
    let value = modal.role_agent().unwrap_or_default().to_owned();
    modal.role_runtime_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(value, select_cx);
    });
}

/// Back to the role's own setup: its agent, model, effort and speed.
pub(super) fn clear_role_overrides(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    modal.role_runtime_override = None;
    sync_role_runtime_select(modal, cx);
    restore_baseline(modal, cx);
}

/// Every control back to what it starts from: no model typed, and the Effort
/// and Speed that leave them untouched.
pub(super) fn restore_baseline(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    modal
        .model
        .update(cx, |input, input_cx| input.reset("", input_cx));
    modal.effort = modal.baseline_effort();
    modal.speed = modal.baseline_speed();
    sync_runtime_controls(modal, cx);
}

pub(super) fn sync_runtime_controls(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let runtime = modal.active_runtime().cloned();
    let models = runtime
        .as_ref()
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default();
    modal.model_field.update(cx, |field, field_cx| {
        field.set_suggestions(models, field_cx);
        field.set_disabled(
            modal.submitting || modal.loading_catalog() || runtime.is_none(),
            field_cx,
        );
    });
    let placeholder = model_placeholder(inheriting_role(modal));
    modal.model.update(cx, |input, input_cx| {
        input.set_placeholder(placeholder, input_cx)
    });
    sync_model_marker(modal, cx);
    sync_effort_control(modal, cx);
    sync_speed_control(modal, cx);
}

/// The model carries the override dot while a role chat on the role's own
/// agent has a model other than the role's.
pub(super) fn sync_model_marker(modal: &StartChatModal, cx: &mut Context<NativeRoot>) {
    let marked = modal.role_overrides(cx).model;
    modal
        .model_field
        .update(cx, |field, field_cx| field.set_marker(marked, field_cx));
}

pub(super) fn sync_speed_control(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let visible = modal.codex_speed_visible();
    if !visible {
        modal.speed = "inherit".into();
    }
    let baseline = modal.baseline_speed();
    let role = inheriting_role(modal).filter(|role| modal.runtime_has_speed(&role.runtime));
    let options = speed_options(
        role.map(|_| baseline.as_str()),
        role.is_some_and(|role| role.codex_speed.is_some()),
    );
    if !options.iter().any(|option| option.value == modal.speed) {
        modal.speed = baseline;
    }
    modal.speed_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(modal.speed.clone(), select_cx);
        select.set_disabled(
            modal.submitting || modal.loading_catalog() || !visible,
            select_cx,
        );
    });
}

/// Effort's choices: the agent's levels for the model the chat will run (its
/// default model when none is set, every level when that is unknown too), the
/// role's own level even when the catalog does not list it, and a blank
/// `default` only while no level is known to start from. An agent that drops
/// an effort without a model offers none until one reaches the launch. On the
/// role's own agent a choice other than the role's carries the amber dot.
pub(super) fn sync_effort_control(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let baseline = modal.baseline_effort();
    let inherits = inheriting_role(modal).is_some();
    let launch = launch_model(modal, cx);
    let levels = modal
        .active_runtime()
        .map(|runtime| {
            let needs_model = effort_needs_launch_model(runtime);
            let default_model = (!needs_model)
                .then(|| trimmed(runtime.default_model.as_deref()))
                .flatten();
            match launch.as_deref().or(default_model) {
                Some(model) => runtime.efforts_for_model(model),
                None if needs_model => Vec::new(),
                None => runtime.efforts.clone(),
            }
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|level| !level.value.is_empty())
        .collect::<Vec<_>>();
    let mut options = option_select_options(&levels);
    if baseline.is_empty() {
        options.insert(
            0,
            SelectOption::new("", crate::i18n::message("word.default")),
        );
    } else if !levels.iter().any(|level| level.value == baseline) {
        options.insert(
            0,
            SelectOption::new(
                baseline.clone(),
                runner_app::ui::select::effort_label(&baseline, &baseline),
            ),
        );
    }
    for option in &mut options {
        option.marked = inherits && option.value != baseline;
    }
    if !options.iter().any(|option| option.value == modal.effort) {
        modal.effort = baseline;
    }
    let disabled = modal.submitting || modal.loading_catalog() || options.len() <= 1;
    modal.effort_select.update(cx, |select, select_cx| {
        select.set_options(options, select_cx);
        select.set_value(modal.effort.clone(), select_cx);
        select.set_disabled(disabled, select_cx);
    });
}

pub(super) fn set_start_chat_controls_disabled(
    modal: &mut StartChatModal,
    disabled: bool,
    cx: &mut Context<NativeRoot>,
) {
    modal.role_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled || modal.roles.is_empty(), select_cx)
    });
    modal.role_runtime_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled, select_cx)
    });
    sync_runtime_controls(modal, cx);
}

pub(super) fn start_chat_focus_order(modal: &StartChatModal, cx: &App) -> Vec<FocusHandle> {
    let mut order = vec![modal.close_focus.clone()];
    let controls = |order: &mut Vec<FocusHandle>| {
        if let Some(runtime) = modal.active_runtime() {
            order.push(modal.model.read(cx).focus_handle());
            if !runtime.efforts.is_empty() {
                order.push(modal.effort_select.read(cx).focus_handle());
            }
        }
        if modal.codex_speed_visible() {
            order.push(modal.speed_select.read(cx).focus_handle());
        }
    };
    {
        if !modal.roles.is_empty() {
            order.push(modal.role_select.read(cx).focus_handle());
        }
        if modal.selected_role().is_some() {
            order.push(modal.role_runtime_select.read(cx).focus_handle());
            controls(&mut order);
            let overrides = modal.role_overrides(cx);
            for (shown, focus) in [
                (
                    overrides.runtime,
                    &modal.reset_focus[ResetKind::Runtime as usize],
                ),
                (
                    overrides.model,
                    &modal.reset_focus[ResetKind::Model as usize],
                ),
                (
                    overrides.effort,
                    &modal.reset_focus[ResetKind::Effort as usize],
                ),
                (
                    overrides.speed,
                    &modal.reset_focus[ResetKind::Speed as usize],
                ),
            ] {
                if shown {
                    order.push(focus.clone());
                }
            }
        }
    }

    order.push(modal.cancel_focus.clone());
    if modal.can_submit() {
        order.push(modal.submit_focus.clone());
    }
    order
}

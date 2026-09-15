use runner_backend::model::Runtime;

use gpui::prelude::*;
use gpui::{div, px, rems, AnyElement, Context, FocusHandle, FontWeight};
use runner_app::ui::SelectOption;
use runner_backend::model::Runner;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};
use runner_backend::router::runtime::PermissionMode;

use super::*;
use crate::*;

#[derive(Clone, Copy)]
pub(super) enum RunnerFormKind {
    Create,
    Edit,
}

pub(super) fn resolve_slot_runtime_layers(
    runner_runtime: &str,
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> RuntimeLayerResolution {
    let runtime = runtime_override.unwrap_or(runner_runtime);
    RuntimeLayerResolution {
        runtime: runtime.to_owned(),
        runtime_pinned: runtime_override.is_some(),
        model: model_override.map(ToOwned::to_owned),
        effort: effort_override.map(ToOwned::to_owned),
    }
}

pub(super) fn resolve_runner_edit(
    runner: &Runner,
    slot: Option<&runner_backend::model::SlotWithRunner>,
) -> RunnerEditResolution {
    let layers = if let Some(slot) = slot {
        resolve_slot_runtime_layers(
            &runner.runtime,
            slot.slot.runtime_override.as_deref(),
            slot.slot.model_override.as_deref(),
            slot.slot.effort_override.as_deref(),
        )
    } else {
        RuntimeLayerResolution {
            runtime: runner.runtime.clone(),
            runtime_pinned: true,
            model: runner.model.clone(),
            effort: runner.effort.clone(),
        }
    };
    let command = if layers.runtime == runner.runtime {
        runner.command.clone()
    } else {
        runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .find(|runtime| runtime.name.key() == layers.runtime)
            .map(|runtime| runtime.command)
            .unwrap_or_else(|| runner.command.clone())
    };
    RunnerEditResolution {
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
    inherited_runner: Option<&Runner>,
) -> String {
    inherited_runner
        .filter(|runner| runner.runtime == runtime)
        .and_then(|runner| runner.model.as_deref())
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

pub(super) fn runner_edit_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    runner: &Runner,
    current_runtime: &str,
    edits_slot: bool,
) -> Vec<SelectOption> {
    let mut options = Vec::new();
    if edits_slot {
        let label = runtime_entry(runtimes, &runner.runtime)
            .map(|runtime| runtime.display_name.as_str())
            .unwrap_or(&runner.runtime);
        options.push(SelectOption::new("", format!("Runner default ({label})")));
    }
    options.extend(
        runtimes
            .iter()
            .filter(|runtime| {
                runtime.available
                    || runtime.name.key() == runner.runtime
                    || runtime.name.key() == current_runtime
            })
            .map(|runtime| {
                SelectOption::new(runtime.name.to_string(), runtime.display_name.clone())
                    .description(runtime.description.clone())
            }),
    );
    options
}

pub(super) fn effort_options(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    runner: &Runner,
    edits_slot: bool,
    model: &str,
) -> Vec<SelectOption> {
    runtime_entry(runtimes, runtime)
        .map(|runtime| runtime.efforts_for_model(model))
        .unwrap_or_default()
        .iter()
        .map(|option| {
            let label = if option.value.is_empty() {
                if edits_slot && runtime == runner.runtime {
                    runner
                        .effort
                        .as_deref()
                        .or_else(|| runtime_entry(runtimes, runtime)?.default_effort.as_deref())
                        .map(|effort| format!("Runner default ({effort})"))
                        .unwrap_or_else(|| "Runner default".into())
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
        Some(Runtime::Shell) | None => &[],
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

fn permission_mode_label(mode: PermissionMode) -> &'static str {
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
        (Some(Runtime::ClaudeCode), PermissionMode::Bypass) => "Skip every check. Runner accepts Claude Code's bypass disclaimer for the sessions it spawns; a runner that passes its own --settings still sees the dialog.",
        (Some(Runtime::Codex), PermissionMode::Default) => {
            "Codex's built-in approval cadence (untrusted commands)."
        }
        (Some(Runtime::Codex), PermissionMode::Auto) => "Auto-run in the workspace and ask only when the model decides approval is needed (`--ask-for-approval on-request`).",
        (Some(Runtime::Codex), PermissionMode::Bypass) => "Never ask while keeping Codex's workspace-write sandbox (`--ask-for-approval never`).",
        (Some(Runtime::Trae), PermissionMode::Default) => "TRAE CLI's built-in approval cadence.",
        (Some(Runtime::Trae), PermissionMode::Bypass) => {
            "Bypass TRAE CLI permission prompts (`--permission-mode bypass_permissions`)."
        }
        (Some(Runtime::Codex | Runtime::Trae), PermissionMode::AcceptEdits)
        | (Some(Runtime::Trae), PermissionMode::Auto)
        | (Some(Runtime::Shell) | None, _) => "",
    }
}

pub(super) fn validate_runner_handle(handle: &str) -> Option<&'static str> {
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

pub(super) fn create_runner_can_submit(form: &CreateRunnerForm) -> bool {
    !form.submitting
        && !form.handle_empty
        && form.handle_error.is_none()
        && form.display_name_valid
        && runtime_entry(&form.runtimes, &form.runtime).is_some()
}

pub(super) fn create_runner_focus_order(
    form: &CreateRunnerForm,
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

pub(super) fn runner_edit_focus_order(
    form: &RunnerEditForm,
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

pub(super) fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn split_args(value: &str) -> Vec<String> {
    value.split_whitespace().map(ToOwned::to_owned).collect()
}

pub(super) fn create_runner_form_is_composing(
    form: &CreateRunnerForm,
    cx: &Context<NativeRoot>,
) -> bool {
    form.handle.read(cx).is_composing()
        || form.display_name.read(cx).is_composing()
        || form.args.read(cx).is_composing()
        || form.model.read(cx).is_composing()
        || form.working_dir.read(cx).is_composing()
        || form.system_prompt.read(cx).is_composing()
}

pub(super) fn runner_edit_form_is_composing(
    form: &RunnerEditForm,
    cx: &Context<NativeRoot>,
) -> bool {
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

pub(super) fn format_timestamp(timestamp: runner_backend::model::Timestamp) -> String {
    timestamp
        .with_timezone(&chrono::Local)
        .format("%-m/%-d/%Y, %-I:%M:%S %p")
        .to_string()
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

pub(super) fn detail_card(
    title: &'static str,
    subtitle: Option<&'static str>,
    child: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .p_4()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .child(
                    div()
                        .text_size(theme::text_title())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(title),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(subtitle)
                })),
        )
        .child(child)
        .into_any_element()
}

pub(super) fn big_stat(label: &'static str, value: i64, accent: bool) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.))
        .rounded_sm()
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .p_3()
        .child(
            div()
                .text_size(theme::text_display_xl())
                .font_weight(FontWeight::BOLD)
                .line_height(rems(30. / 16.))
                .text_color(if accent {
                    theme::accent()
                } else {
                    theme::text()
                })
                .child(value.to_string()),
        )
        .child(
            div()
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(label.to_uppercase()),
        )
        .into_any_element()
}

pub(super) fn detail_row(label: &'static str, value: String) -> AnyElement {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .child(div().flex_none().text_color(theme::faint()).child(label))
        .child(
            div()
                .min_w(px(0.))
                .text_right()
                .text_color(theme::text())
                .child(value),
        )
        .into_any_element()
}

pub(super) fn detail_metadata_row(
    label: &'static str,
    value: String,
    monospace: bool,
    subtle: bool,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .text_size(theme::text_ui())
        .child(div().flex_none().text_color(theme::faint()).child(label))
        .child(
            div()
                .min_w(px(0.))
                .text_right()
                .when(monospace, |value| {
                    value.font_family(theme::UI_MONOSPACE_FONT)
                })
                .when(subtle, |value| {
                    value
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                })
                .when(!subtle, |value| value.text_color(theme::text()))
                .child(value),
        )
        .into_any_element()
}

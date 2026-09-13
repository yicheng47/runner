use super::logic::effort_options;
use super::logic::error_banner;
use super::logic::permission_mode_description;
use super::logic::permission_mode_value;
use super::logic::permission_modes;
use super::logic::permission_options;
use super::logic::runner_edit_focus_order;
use super::logic::runner_edit_form_is_composing;
use super::logic::runtime_efforts;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::split_args;
use super::logic::trimmed_option;
use super::logic::RunnerFormKind;
use runner_backend::model::Runtime;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, rems, AnyElement, Context, Entity, FontWeight, KeyDownEvent, PathPromptOptions, Window,
};
use runner_app::ui::{
    Button, ButtonVariant, Drawer, Field, IconButton, OverlayWidth, TextField, WorkingDirField,
};
use runner_backend::ops::runner::UpdateRunnerInput;
use runner_backend::router::runtime::PermissionMode;

use super::*;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn select_runner_edit_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        if !value.is_empty() && Runtime::parse(&value).is_none() {
            return;
        }
        let Some(form) = self.runner_surfaces.edit.as_mut() else {
            return;
        };
        if form.submitting {
            return;
        }
        let next_runtime = if form.slot.is_some() && value.is_empty() {
            form.runner.runtime.clone()
        } else {
            value.clone()
        };
        form.runtime_pinned = form.slot.is_none() || !value.is_empty();
        if next_runtime != form.runtime {
            form.model
                .update(cx, |input, input_cx| input.reset("", input_cx));
            if !runtime_efforts(&form.runtimes, &next_runtime)
                .iter()
                .any(|option| option.value == form.effort)
            {
                form.effort.clear();
            }
            let command = if next_runtime == form.runner.runtime {
                form.runner.command.clone()
            } else {
                runtime_entry(&form.runtimes, &next_runtime)
                    .map(|runtime| runtime.command.clone())
                    .unwrap_or_else(|| form.runner.command.clone())
            };
            form.command
                .update(cx, |input, input_cx| input.reset(command, input_cx));
        }
        form.runtime = next_runtime.clone();
        let model_placeholder =
            runtime_model_placeholder(&form.runtimes, &next_runtime, form.slot.is_some());
        form.model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        form.model_field.update(cx, |field, field_cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &next_runtime), field_cx)
        });
        form.effort_select.update(cx, |select, select_cx| {
            select.set_options(
                effort_options(
                    &form.runtimes,
                    &next_runtime,
                    &form.runner,
                    form.slot.is_some(),
                ),
                select_cx,
            );
            select.set_value(form.effort.clone(), select_cx);
        });
        if !permission_modes(&next_runtime).contains(&form.permission_mode) {
            form.permission_mode = PermissionMode::Default;
        }
        form.permission_select.update(cx, |select, select_cx| {
            select.set_options(permission_options(&next_runtime), select_cx);
            select.set_value(permission_mode_value(form.permission_mode), select_cx);
        });
        cx.notify();
    }

    fn close_runner_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .runner_surfaces
            .edit
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.runner_surfaces.edit = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_runner_edit_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.runner_surfaces.edit.as_ref().is_some_and(|form| {
                !form
                    .system_prompt
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
                    && !runner_edit_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_runner_edit(window, cx);
        }
    }

    fn browse_runner_edit_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self
            .runner_surfaces
            .edit
            .as_ref()
            .filter(|form| !form.submitting)
            .map(|form| form.working_dir.clone())
        else {
            return;
        };
        self.browse_runner_form_cwd(input, RunnerFormKind::Edit, cx);
    }

    pub(super) fn browse_runner_form_cwd(
        &mut self,
        input: Entity<TextField>,
        kind: RunnerFormKind,
        cx: &mut Context<Self>,
    ) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Pick a working directory".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                let current = match kind {
                    RunnerFormKind::Create => this
                        .runner_surfaces
                        .create
                        .as_ref()
                        .map(|form| form.working_dir.clone()),
                    RunnerFormKind::Edit => this
                        .runner_surfaces
                        .edit
                        .as_ref()
                        .map(|form| form.working_dir.clone()),
                };
                if current.as_ref() != Some(&input) {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            input.update(cx, |field, field_cx| {
                                field.reset(path.to_string_lossy().into_owned(), field_cx)
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => match kind {
                        RunnerFormKind::Create => {
                            if let Some(form) = this.runner_surfaces.create.as_mut() {
                                form.error = Some(error);
                            }
                        }
                        RunnerFormKind::Edit => {
                            if let Some(form) = this.runner_surfaces.edit.as_mut() {
                                form.error = Some(error);
                            }
                        }
                    },
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit_runner_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.runner_surfaces.edit.as_mut() else {
            return;
        };
        if form.submitting || form.display_name.read(cx).text().trim().is_empty() {
            return;
        }
        form.submitting = true;
        form.error = None;
        let edits_slot = form.slot.is_some();
        let update = UpdateRunnerInput {
            display_name: Some(form.display_name.read(cx).text().trim().to_owned()),
            runtime: (!edits_slot)
                .then(|| Runtime::parse(&form.runtime))
                .flatten(),
            command: (!edits_slot).then(|| form.command.read(cx).text().trim().to_owned()),
            args: (!edits_slot).then(|| split_args(form.args.read(cx).text())),
            working_dir: Some(trimmed_option(form.working_dir.read(cx).text())),
            system_prompt: Some(trimmed_option(form.system_prompt.read(cx).text())),
            env: None,
            model: (!edits_slot).then(|| trimmed_option(form.model.read(cx).text())),
            effort: (!edits_slot).then(|| trimmed_option(&form.effort)),
            permission_mode: (!edits_slot && !permission_modes(&form.runtime).is_empty())
                .then_some(form.permission_mode),
        };
        let slot_update = form.slot.as_ref().map(|slot| {
            (
                slot.slot.id.clone(),
                runner_backend::ops::slot::UpdateSlotInput {
                    slot_handle: None,
                    runtime_override: None,
                    model_override: Some(trimmed_option(form.model.read(cx).text())),
                    effort_override: Some(trimmed_option(&form.effort)),
                },
                form.runtime_pinned.then(|| form.runtime.clone()),
                slot.slot.crew_id.clone(),
            )
        });
        let runner_id = form.runner.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_update(&core, &runner_id, update)
                .map_err(|error| error.to_string())?;
            let crew_id =
                if let Some((slot_id, mut update, runtime_override, crew_id)) = slot_update {
                    update.runtime_override = Some(
                        runner_backend::ops::slot::validate_runtime_override(
                            runtime_override.as_deref(),
                        )
                        .map_err(|error| error.to_string())?,
                    );
                    runner_backend::ops::slot::slot_update(&core, &slot_id, update)
                        .map_err(|error| error.to_string())?;
                    Some(crew_id)
                } else {
                    None
                };
            Ok::<_, String>(crew_id)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(crew_id) => {
                        this.runner_surfaces.edit = None;
                        if let Ok(runners) = runner_backend::ops::runner::runner_list(this.core(cx))
                        {
                            this.app_store.update(cx, |store, store_cx| {
                                store.replace_runners(runners, store_cx)
                            });
                        }
                        this.load_runner_page(cx);
                        match this.route.clone() {
                            AppRoute::RunnerDetail(handle) => {
                                this.load_runner_detail(handle, cx);
                            }
                            AppRoute::CrewEditor(active)
                                if crew_id.as_ref().is_none_or(|crew_id| crew_id == &active) =>
                            {
                                this.load_crew_editor(active, cx);
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        if let Some(form) = this.runner_surfaces.edit.as_mut() {
                            form.submitting = false;
                            form.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_runner_edit_drawer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .runner_surfaces
            .edit
            .as_ref()
            .expect("runner edit form");
        let submitting = form.submitting;
        let edits_slot = form.slot.is_some();
        let can_submit = !submitting && form.display_name_valid;
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let browse_root = root.clone();
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child("Edit runner")
                    .child(
                        div()
                            .rounded_sm()
                            .bg(theme::raised())
                            .px(rems(6. / 16.))
                            .py(rems(2. / 16.))
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(format!("@{}", form.runner.handle)),
                    ),
            )
            .child(
                IconButton::new("close-runner-edit", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close runner editor")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_runner_edit(window, cx));
                    }),
            );
        let model_hint = if edits_slot {
            if form.runtime == form.runner.runtime {
                format!(
                    "slot override · blank inherits runner default ({})",
                    form.runner.model.as_deref().unwrap_or("default")
                )
            } else {
                "slot override · blank uses the agent's own model".into()
            }
        } else {
            "optional · blank uses the agent's own model · type a name or pick an alias".into()
        };
        let effort_hint = if edits_slot {
            if form.runtime == form.runner.runtime {
                format!(
                    "slot override · blank inherits runner default ({})",
                    form.runner.effort.as_deref().unwrap_or("default")
                )
            } else {
                "slot override · blank uses the agent's own effort".into()
            }
        } else {
            "optional · resolves to the agent's native effort flag".into()
        };
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::on_runner_edit_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                Field::new("edit-display-name", "Display name", form.display_name.clone())
                    .focus_target(form.display_name.read(cx).focus_handle()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Field::new("edit-runtime", "Agent", form.runtime_select.clone())
                            .focus_target(form.runtime_select.read(cx).focus_handle())
                            .when(edits_slot, |field| {
                                field.hint(
                                    "slot override · Runner default follows the template; an explicit agent pins this slot's engine",
                                    form.runtime_hint_focus.clone(),
                                )
                            }),
                    )
                    .children(form.agents_error.clone().map(|error| {
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::danger())
                            .child(error)
                    })),
            )
            .child(
                Field::new("edit-command", "Command", form.command.clone())
                    .focus_target(form.command.read(cx).focus_handle()),
            )
            .children((!edits_slot).then(|| {
                Field::new("edit-args", "Args", form.args.clone())
                    .focus_target(form.args.read(cx).focus_handle())
                    .hint(
                        "extra flags · whitespace-separated",
                        form.args_hint_focus.clone(),
                    )
            }))
            .child(
                Field::new("edit-model", "Model", form.model_field.clone())
                    .focus_target(form.model.read(cx).focus_handle())
                    .hint(model_hint, form.model_hint_focus.clone()),
            )
            .children((!runtime_efforts(&form.runtimes, &form.runtime).is_empty()).then(|| {
                Field::new(
                    "edit-effort",
                    "Thinking effort",
                    form.effort_select.clone(),
                )
                .focus_target(form.effort_select.read(cx).focus_handle())
                .hint(effort_hint, form.effort_hint_focus.clone())
            }))
            .children((!edits_slot && !permission_modes(&form.runtime).is_empty()).then(|| {
                Field::new(
                    "edit-permission-mode",
                    "Permission mode",
                    form.permission_select.clone(),
                )
                .focus_target(form.permission_select.read(cx).focus_handle())
                .hint(permission_mode_description(
                    &form.runtime,
                    form.permission_mode,
                ), form.permission_hint_focus.clone())
            }))
            .child(
                Field::new(
                    "edit-working-dir",
                    "Working directory",
                    WorkingDirField::new(
                        form.working_dir.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| {
                                this.browse_runner_edit_cwd(cx)
                            });
                        }),
                    )
                    .browse_focus(form.browse_focus.clone()),
                )
                .focus_target(form.working_dir.read(cx).focus_handle()),
            )
            .child(
                Field::new("edit-system-prompt", "System prompt", form.system_prompt.clone())
                    .focus_target(form.system_prompt.read(cx).focus_handle()),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-runner-edit", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_runner_edit(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-runner-edit",
                    if submitting { "Saving…" } else { "Save" },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_runner_edit(window, cx));
                }),
            );
        let drawer_root = root;
        Drawer::new(
            title,
            body,
            Rc::new(move |window, cx| {
                drawer_root.update(cx, |this, cx| this.close_runner_edit(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(FORM_WIDTH))
        .busy(submitting)
        .focus_order(runner_edit_focus_order(form, cx))
        .scrollbar(form.scroll.clone(), form.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }
}

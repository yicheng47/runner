use super::logic::create_role_can_submit;
use super::logic::create_role_focus_order;
use super::logic::create_role_form_is_composing;
use super::logic::error_banner;
use super::logic::permission_mode_description;
use super::logic::permission_mode_value;
use super::logic::permission_modes;
use super::logic::permission_options;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::split_args;
use super::logic::trimmed_option;
use super::logic::RoleFormKind;
use runner_backend::model::Runtime;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rems, AnyElement, Context, FontWeight, KeyDownEvent, Window};
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, OverlayWidth, WorkingDirField,
};
use runner_backend::ops::role::CreateRoleInput;
use runner_backend::router::runtime::PermissionMode;

use super::*;
use crate::*;

impl NativeRoot {
    pub(super) fn select_create_role_runtime(&mut self, runtime: String, cx: &mut Context<Self>) {
        if Runtime::parse(&runtime).is_none() {
            return;
        }
        let Some(form) = self.role_surfaces.create.as_mut() else {
            return;
        };
        if form.runtime == runtime || form.submitting {
            return;
        }
        form.runtime = runtime.clone();
        let command = runtime_entry(&form.runtimes, &runtime)
            .map(|entry| entry.command.clone())
            .unwrap_or_default();
        let model_placeholder = runtime_model_placeholder(&form.runtimes, &runtime, None);
        form.command
            .update(cx, |input, input_cx| input.reset(command, input_cx));
        form.model.update(cx, |input, input_cx| {
            input.reset("", input_cx);
            input.set_placeholder(model_placeholder, input_cx);
        });
        form.model_field.update(cx, |field, field_cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &runtime), field_cx);
            field.set_disabled(runtime.is_empty(), field_cx);
        });
        if !permission_modes(&runtime).contains(&form.permission_mode) {
            form.permission_mode = PermissionMode::Default;
        }
        form.permission_select.update(cx, |select, select_cx| {
            select.set_options(permission_options(&runtime), select_cx);
            select.set_value(permission_mode_value(form.permission_mode), select_cx);
        });
        self.request_model_catalog(&runtime, cx);
        cx.notify();
    }

    fn close_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .role_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.role_surfaces.create = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_create_role_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.role_surfaces.create.as_ref().is_some_and(|form| {
                !form
                    .system_prompt
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
                    && !create_role_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_create_role(window, cx);
        }
    }

    fn browse_create_role_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self
            .role_surfaces
            .create
            .as_ref()
            .filter(|form| !form.submitting)
            .map(|form| form.working_dir.clone())
        else {
            return;
        };
        self.browse_role_form_cwd(input, RoleFormKind::Create, cx);
    }

    fn submit_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.role_surfaces.create.as_mut() else {
            return;
        };
        if !create_role_can_submit(form) {
            return;
        }
        let Some(runtime) = Runtime::parse(&form.runtime) else {
            return;
        };
        form.submitting = true;
        form.error = None;
        let input = CreateRoleInput {
            handle: form.handle.read(cx).text().to_owned(),
            display_name: form.display_name.read(cx).text().trim().to_owned(),
            runtime,
            command: form.command.read(cx).text().trim().to_owned(),
            args: split_args(form.args.read(cx).text()),
            working_dir: trimmed_option(form.working_dir.read(cx).text()),
            system_prompt: trimmed_option(form.system_prompt.read(cx).text()),
            env: HashMap::new(),
            model: trimmed_option(form.model.read(cx).text()),
            effort: None,
            permission_mode: form.permission_mode,
        };
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::role::role_create(&core, input).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(role) => {
                        let handle = role.handle.clone();
                        this.role_surfaces.create = None;
                        if let Ok(roles) = runner_backend::ops::role::role_list(this.core(cx)) {
                            this.app_store
                                .update(cx, |store, store_cx| store.replace_roles(roles, store_cx));
                        }
                        this.load_role_page(cx);
                        this.open_role_detail(handle, window, cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.role_surfaces.create.as_mut() {
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

    pub(super) fn render_create_role_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .role_surfaces
            .create
            .as_ref()
            .expect("create role form");
        let submitting = form.submitting;
        let can_submit = create_role_can_submit(form);
        let handle_error = form.handle_error;
        let permission_description =
            permission_mode_description(&form.runtime, form.permission_mode);
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
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("New runner"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Reusable across crews and chats."),
                    ),
            )
            .child(
                IconButton::new("close-create-role", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close new runner")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_create_role(window, cx));
                    }),
            );
        let handle_input = div()
            .w_full()
            .flex()
            .items_center()
            .rounded(rems(4. / 16.))
            .border_1()
            .border_color(if handle_error.is_some() {
                theme::danger()
            } else {
                theme::border_strong()
            })
            .bg(theme::bg())
            .px(rems(10. / 16.))
            .py(rems(6. / 16.))
            .text_size(theme::text_title())
            .child(
                div()
                    .pr_1()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child("@"),
            )
            .child(div().min_w(px(0.)).flex_1().child(form.handle.clone()));
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_create_role_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                Field::new("new-role-handle", "Handle", handle_input)
                    .focus_target(form.handle.read(cx).focus_handle())
                    .when_some(handle_error, |field, error| field.error(error)),
            )
            .child(
                Field::new(
                    "new-role-display-name",
                    "Display name",
                    form.display_name.clone(),
                )
                .focus_target(form.display_name.read(cx).focus_handle()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Field::new("new-role-runtime", "Agent", form.runtime_select.clone())
                            .focus_target(form.runtime_select.read(cx).focus_handle()),
                    )
                    .children(form.agents_error.clone().map(|error| {
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::danger())
                            .child(error)
                    })),
            )
            .child(
                Field::new("new-role-command", "Command", form.command.clone())
                    .focus_target(form.command.read(cx).focus_handle()),
            )
            .child(
                Field::new("new-role-args", "Args", form.args.clone())
                    .focus_target(form.args.read(cx).focus_handle())
                    .hint(
                        "extra flags · whitespace-separated",
                        form.args_hint_focus.clone(),
                    ),
            )
            .child(
                Field::new("new-role-model", "Model", form.model_field.clone())
                    .focus_target(form.model.read(cx).focus_handle())
                    .hint(
                        "optional · blank uses the agent's own model · type a name or pick an alias",
                        form.model_hint_focus.clone(),
                    ),
            )
            .children((!permission_modes(&form.runtime).is_empty()).then(|| {
                Field::new(
                    "new-role-permission-mode",
                    "Permission mode",
                    form.permission_select.clone(),
                )
                .focus_target(form.permission_select.read(cx).focus_handle())
                .hint(permission_description, form.permission_hint_focus.clone())
            }))
            .child(
                Field::new(
                    "new-role-working-dir",
                    "Working directory",
                    WorkingDirField::new(
                        form.working_dir.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| {
                                this.browse_create_role_cwd(cx)
                            });
                        }),
                    )
                    .browse_focus(form.browse_focus.clone()),
                )
                .focus_target(form.working_dir.read(cx).focus_handle()),
            )
            .child(
                Field::new(
                    "new-role-system-prompt",
                    "Default system prompt",
                    form.system_prompt.clone(),
                )
                .focus_target(form.system_prompt.read(cx).focus_handle()),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-create-role", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_create_role(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-create-role",
                    if submitting {
                        "Creating…"
                    } else {
                        "Create runner"
                    },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_create_role(window, cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_create_role(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(FORM_WIDTH))
        .busy(submitting)
        .focus_order(create_role_focus_order(form, cx))
        .scrollbar(form.scroll.clone(), form.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }
}

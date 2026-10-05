use super::logic::effort_options;
use super::logic::ensure_runtime_present;
use super::logic::resolve_role_edit;
use super::logic::role_edit_runtime_options;
use super::logic::role_permission_mode;
use super::logic::role_visible_args;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::speed_options;
use super::logic::validate_role_handle;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{px, Context, Window};
use runner_app::ui::{working_dir_text_field, ModelField, StyledSelect, TextField};
use runner_core::protocol::model::Role;
use runner_core::protocol::permissions::PermissionMode;

use super::*;
use crate::*;

impl NativeRoot {
    pub(super) fn sync_role_edit_efforts(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.role_surfaces.edit.as_mut() else {
            return;
        };
        let options = effort_options(
            &form.runtimes,
            &form.runtime,
            &form.role,
            false,
            form.model.read(cx).text(),
        );
        if !options.iter().any(|option| option.value == form.effort) {
            form.effort.clear();
        }
        form.effort_select.update(cx, |select, cx| {
            select.set_disabled(form.submitting || options.len() <= 1, cx);
            select.set_options(options, cx);
            select.set_value(form.effort.clone(), cx);
        });
    }

    pub(super) fn sync_create_role_efforts(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.role_surfaces.create.as_mut() else {
            return;
        };
        let form = &mut form.fields;
        let options = super::logic::create_role_effort_options(
            &form.runtimes,
            &form.runtime,
            form.model.read(cx).text(),
        );
        if !options.iter().any(|option| option.value == form.effort) {
            form.effort.clear();
        }
        form.effort_select.update(cx, |select, cx| {
            select.set_disabled(form.submitting || options.len() <= 1, cx);
            select.set_options(options, cx);
            select.set_value(form.effort.clone(), cx);
        });
    }

    pub(crate) fn refresh_role_form_runtimes(&mut self, cx: &mut Context<Self>) {
        let (selectable, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let catalog_loaded = agents_error.is_none();
        let placeholder = if agents_checking {
            "Detecting agents…"
        } else {
            "No enabled agents detected"
        };

        if let Some(form) = self.role_surfaces.create.as_mut() {
            let form = &mut form.fields;
            form.agents_checking = agents_checking;
            form.agents_error.clone_from(&agents_error);
            if catalog_loaded {
                form.runtimes.clone_from(&selectable);
                let next_runtime = form
                    .runtimes
                    .iter()
                    .find(|runtime| runtime.name.key() == form.runtime)
                    .or_else(|| form.runtimes.first())
                    .map(|runtime| runtime.name.to_string())
                    .unwrap_or_default();
                if next_runtime != form.runtime {
                    form.runtime.clone_from(&next_runtime);
                    form.effort.clear();
                    if !crate::runtime_ui::catalog_capabilities(&form.runtimes, &next_runtime)
                        .codex_speed
                    {
                        form.speed = "inherit".into();
                        form.speed_select.update(cx, |select, select_cx| {
                            select.set_value("inherit", select_cx)
                        });
                    }
                    let command = runtime_entry(&form.runtimes, &next_runtime)
                        .map(|runtime| runtime.command.clone())
                        .unwrap_or_default();
                    form.command
                        .update(cx, |input, input_cx| input.reset(command, input_cx));
                    form.model
                        .update(cx, |input, input_cx| input.reset("", input_cx));
                }
                let model_placeholder =
                    runtime_model_placeholder(&form.runtimes, &next_runtime, None);
                form.model.update(cx, |input, input_cx| {
                    input.set_placeholder(model_placeholder, input_cx)
                });
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(
                        runner_app::ui::runtime_select_options(&form.runtimes),
                        select_cx,
                    );
                    select.set_value(next_runtime.clone(), select_cx);
                    select.set_disabled(form.runtimes.is_empty(), select_cx);
                    select.set_placeholder(placeholder, select_cx);
                });
                form.model_field.update(cx, |field, field_cx| {
                    field.set_suggestions(runtime_models(&form.runtimes, &next_runtime), field_cx);
                    field.set_disabled(next_runtime.is_empty(), field_cx);
                });
            }
        }

        let core = self.core(cx).clone();
        if let Some(form) = self.role_surfaces.edit.as_mut() {
            form.agents_checking = agents_checking;
            form.agents_error = agents_error;
            if catalog_loaded {
                let mut runtimes = selectable;
                ensure_runtime_present(&core, &mut runtimes, &form.role.runtime);
                ensure_runtime_present(&core, &mut runtimes, &form.runtime);
                form.runtimes = runtimes;
                let options = role_edit_runtime_options(&form.runtimes, &form.role, &form.runtime);
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx);
                    select.set_value(form.runtime.clone(), select_cx);
                    select.set_placeholder(placeholder, select_cx);
                });
                let model_placeholder =
                    runtime_model_placeholder(&form.runtimes, &form.runtime, None);
                form.model.update(cx, |input, input_cx| {
                    input.set_placeholder(model_placeholder, input_cx)
                });
                form.model_field.update(cx, |field, field_cx| {
                    field.set_suggestions(runtime_models(&form.runtimes, &form.runtime), field_cx)
                });
            }
        }
        self.sync_create_role_efforts(cx);
        self.sync_role_edit_efforts(cx);
        cx.notify();
    }

    pub(crate) fn open_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.role_surfaces.create.is_some() {
            self.enter_entity_route(AppRoute::NewRole, window, cx);
            return;
        }
        let return_route = self.route.clone();
        self.enter_entity_route(AppRoute::NewRole, window, cx);
        let (runtimes, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let runtime = runtimes
            .first()
            .map(|runtime| runtime.name.to_string())
            .unwrap_or_default();
        self.request_model_catalog(&runtime, cx);
        let command = runtime_entry(&runtimes, &runtime)
            .map(|runtime| runtime.command.clone())
            .unwrap_or_default();
        let handle = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "architect", true)
                .text_size(theme::text_title())
        });
        handle.update(cx, |input, input_cx| input.set_bare(true, input_cx));
        let display_name = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Architect", false)
                .text_size(theme::text_display())
        });
        let command = cx.new(|input_cx| {
            let mut input = TextField::new(input_cx.focus_handle(), command, "", true)
                .text_size(theme::text_title());
            input.set_disabled(true, input_cx);
            input
        });
        let args = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Extra flags", true)
                .text_size(theme::text_title())
        });
        let model = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", true).placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| {
            ModelField::new(model.clone(), runtime_models(&runtimes, &runtime), model_cx)
        });
        let model_placeholder = runtime_model_placeholder(&runtimes, &runtime, None);
        model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        model_field.update(cx, |field, field_cx| {
            field.set_disabled(runtime.is_empty(), field_cx)
        });
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let working_dir = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), default_working_dir, "")
                .text_size(theme::text_body())
        });
        let system_prompt = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                "",
                "What this role does, how it works and what it hands back. Markdown works.",
                6,
                true,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input.fill_height().with_scrollbar(input_cx)
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::runtime(
                "new-role-runtime",
                select_cx.focus_handle(),
                runtime.clone(),
                &runtimes,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_create_role_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
            .disabled(runtimes.is_empty())
            .placeholder(if agents_checking {
                "Detecting agents…"
            } else {
                "No enabled agents detected"
            })
        });
        let effort_root = root.clone();
        let effort_select = cx.new(|select_cx| {
            StyledSelect::new(
                "new-role-effort",
                select_cx.focus_handle(),
                "",
                super::logic::create_role_effort_options(&runtimes, &runtime, ""),
                Rc::new(move |value, _, cx| {
                    effort_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.create.as_mut() {
                            form.effort = value;
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .full_width(true)
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
        });
        let speed_root = root.clone();
        let speed_select = cx.new(|select_cx| {
            StyledSelect::new(
                "new-role-speed",
                select_cx.focus_handle(),
                "inherit",
                speed_options(),
                Rc::new(move |value, _, cx| {
                    speed_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.create.as_mut() {
                            form.speed = value;
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
        });
        let browse_focus = cx.focus_handle();
        let cancel_focus = cx.focus_handle();
        let submit_focus = cx.focus_handle();
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe(&handle, |this, input, cx| {
            let text = input.read(cx).text().to_owned();
            let lowercase = text.to_lowercase();
            if text != lowercase {
                input.update(cx, |input, input_cx| input.set_text(lowercase, input_cx));
                return;
            }
            let empty = text.is_empty();
            let error = validate_role_handle(&text);
            let Some(form) = this.role_surfaces.create.as_mut() else {
                return;
            };
            form.handle_empty = empty;
            form.handle_error = error;
            cx.notify();
        }));
        subscriptions.push(cx.observe(&display_name, |this, input, cx| {
            let valid = !input.read(cx).text().trim().is_empty();
            let Some(form) = this.role_surfaces.create.as_mut() else {
                return;
            };
            if form.display_name_valid != valid {
                form.display_name_valid = valid;
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe(&model, |this, _, cx| this.sync_create_role_efforts(cx)));
        for input in [&display_name, &args, &model, &working_dir, &system_prompt] {
            subscriptions.push(cx.observe(input, |_, _, cx| cx.notify()));
        }
        self.role_surfaces.prompt_preview = false;
        let focus = display_name.read(cx).focus_handle();
        self.role_surfaces.create = Some(CreateRoleForm {
            return_route,
            handle: handle.clone(),
            handle_empty: true,
            handle_error: None,
            fields: RoleFormFields {
                runtimes,
                runtime,
                display_name,
                command,
                args,
                model,
                model_field,
                effort: String::new(),
                effort_select,
                speed: "inherit".into(),
                speed_select,
                working_dir,
                system_prompt,
                prompt_mode_focus: [cx.focus_handle(), cx.focus_handle()],
                runtime_select,
                browse_focus,
                cancel_focus,
                submit_focus,
                display_name_valid: false,
                submitting: false,
                agents_checking,
                agents_error,
                error: None,
                _subscriptions: subscriptions,
            },
        });
        self.sync_create_role_efforts(cx);
        focus.focus(window, cx);
        cx.notify();
    }

    /// Opens the role page's in-place editor for `role`.
    pub(crate) fn open_role_edit(
        &mut self,
        role: Role,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut runtimes, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let resolution = resolve_role_edit(&role);
        ensure_runtime_present(self.core(cx), &mut runtimes, &resolution.runtime);
        self.request_model_catalog(&resolution.runtime, cx);
        let display_name = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                role.display_name.clone(),
                "",
                false,
            )
            .text_size(theme::text_display())
        });
        let command = cx.new(|input_cx| {
            let mut input = TextField::new(
                input_cx.focus_handle(),
                resolution.command.clone(),
                "",
                true,
            );
            input.set_disabled(true, input_cx);
            input
        });
        let args = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                role_visible_args(&role).join(" "),
                "Extra flags",
                true,
            )
        });
        let model_value = resolution.model.clone();
        let model = cx.new(move |input_cx| {
            TextField::new(input_cx.focus_handle(), model_value, "default", true)
                .placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| {
            ModelField::new(
                model.clone(),
                runtime_models(&runtimes, &resolution.runtime),
                model_cx,
            )
        });
        let model_placeholder = runtime_model_placeholder(&runtimes, &resolution.runtime, None);
        model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        let working_dir = cx.new(|input_cx| {
            working_dir_text_field(
                input_cx.focus_handle(),
                role.working_dir.clone().unwrap_or_default(),
                "",
            )
            .text_size(theme::text_body())
        });
        let system_prompt = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                role.system_prompt.clone().unwrap_or_default(),
                "Who this agent is and how it works: its strengths, its habits, what it leaves alone. Markdown works.",
                6,
                true,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input.fill_height().with_scrollbar(input_cx)
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_options = role_edit_runtime_options(&runtimes, &role, &resolution.runtime);
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-role-runtime",
                select_cx.focus_handle(),
                resolution.runtime.clone(),
                runtime_options,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_role_edit_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
            .placeholder(if agents_checking {
                "Detecting agents…"
            } else {
                "No enabled agents detected"
            })
        });
        let effort_root = root.clone();
        let effort_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-role-effort",
                select_cx.focus_handle(),
                resolution.effort.clone(),
                effort_options(
                    &runtimes,
                    &resolution.runtime,
                    &role,
                    false,
                    &resolution.model,
                ),
                Rc::new(move |value, _, cx| {
                    effort_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.edit.as_mut() {
                            form.effort = value;
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .full_width(true)
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
        });
        let speed_root = root.clone();
        let speed_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-role-speed",
                select_cx.focus_handle(),
                resolution.speed.clone(),
                speed_options(),
                Rc::new(move |value, _, cx| {
                    speed_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.edit.as_mut() {
                            form.speed = value;
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .width(px(ROLE_COLUMN_WIDTH))
            .min_menu_width(px(ROLE_COLUMN_WIDTH))
        });
        let permission_mode = role_permission_mode(&role).unwrap_or(PermissionMode::Default);
        let mut subscriptions = vec![cx.observe(&display_name, |this, input, cx| {
            let valid = !input.read(cx).text().trim().is_empty();
            let Some(form) = this.role_surfaces.edit.as_mut() else {
                return;
            };
            if form.display_name_valid != valid {
                form.display_name_valid = valid;
                cx.notify();
            }
        })];
        subscriptions.push(cx.observe(&model, |this, _, cx| {
            this.sync_role_edit_efforts(cx);
        }));
        self.role_surfaces.prompt_preview = false;
        // The page redraws on every keystroke to keep "Unsaved changes" honest.
        for input in [&display_name, &args, &model, &working_dir, &system_prompt] {
            subscriptions.push(cx.observe(input, |_, _, cx| cx.notify()));
        }
        self.role_surfaces.edit = Some(RoleEditForm {
            role,
            permission_mode,
            fields: RoleFormFields {
                runtimes,
                runtime: resolution.runtime,
                display_name: display_name.clone(),
                command,
                args,
                model,
                model_field,
                effort: resolution.effort,
                effort_select,
                speed: resolution.speed,
                speed_select,
                runtime_select,
                working_dir,
                system_prompt,
                prompt_mode_focus: [cx.focus_handle(), cx.focus_handle()],
                browse_focus: cx.focus_handle(),
                cancel_focus: cx.focus_handle(),
                submit_focus: cx.focus_handle(),
                display_name_valid: true,
                submitting: false,
                agents_checking,
                agents_error,
                error: None,
                _subscriptions: subscriptions,
            },
        });
        self.sync_role_edit_efforts(cx);
        display_name.read(cx).focus_handle().focus(window, cx);
        cx.notify();
    }
}

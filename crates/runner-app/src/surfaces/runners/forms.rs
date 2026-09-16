use super::logic::effort_options;
use super::logic::ensure_runtime_present;
use super::logic::parse_permission_mode;
use super::logic::permission_mode_value;
use super::logic::permission_modes;
use super::logic::permission_options;
use super::logic::resolve_role_edit;
use super::logic::role_edit_runtime_options;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::validate_role_handle;
use runner_backend::model::Runtime;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{px, Context, ScrollHandle, Window};
use runner_app::ui::{
    working_dir_text_field, ModelField, RuntimeSelect, Scrollbar, StyledSelect, TextField,
};
use runner_backend::model::Role;
use runner_backend::router::runtime::PermissionMode;

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
            form.slot.is_some(),
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
                    let command = runtime_entry(&form.runtimes, &next_runtime)
                        .map(|runtime| runtime.command.clone())
                        .unwrap_or_default();
                    form.command
                        .update(cx, |input, input_cx| input.reset(command, input_cx));
                    form.model
                        .update(cx, |input, input_cx| input.reset("", input_cx));
                    if !permission_modes(&next_runtime).contains(&form.permission_mode) {
                        form.permission_mode = PermissionMode::Default;
                    }
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
                form.permission_select.update(cx, |select, select_cx| {
                    select.set_options(permission_options(&next_runtime), select_cx);
                    select.set_value(permission_mode_value(form.permission_mode), select_cx);
                });
            }
        }

        let core = self.core(cx).clone();
        if let Some(form) = self.role_surfaces.edit.as_mut() {
            form.agents_checking = agents_checking;
            form.agents_error = agents_error;
            if catalog_loaded {
                let mut runtimes = selectable;
                ensure_runtime_present(&core, &mut runtimes, &form.runtime);
                form.runtimes = runtimes;
                let runtime_value = if form.slot.is_some() && !form.runtime_pinned {
                    String::new()
                } else {
                    form.runtime.clone()
                };
                let options = role_edit_runtime_options(
                    &form.runtimes,
                    &form.role,
                    &form.runtime,
                    form.slot.is_some(),
                );
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx);
                    select.set_value(runtime_value, select_cx);
                    select.set_placeholder(placeholder, select_cx);
                });
                let model_placeholder = runtime_model_placeholder(
                    &form.runtimes,
                    &form.runtime,
                    form.slot.as_ref().map(|_| &form.role),
                );
                form.model.update(cx, |input, input_cx| {
                    input.set_placeholder(model_placeholder, input_cx)
                });
                form.model_field.update(cx, |field, field_cx| {
                    field.set_suggestions(runtime_models(&form.runtimes, &form.runtime), field_cx)
                });
            }
        }
        self.sync_role_edit_efforts(cx);
        cx.notify();
    }

    pub(crate) fn open_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.role_surfaces.create.is_some() {
            return;
        }
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
                .text_size(theme::text_title())
        });
        let command = cx.new(|input_cx| {
            let mut input = TextField::new(input_cx.focus_handle(), command, "", true)
                .text_size(theme::text_title());
            input.set_disabled(true, input_cx);
            input
        });
        let args = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "--mcp-debug", true)
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
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "You are the architect for this crew. When a mission starts, decompose the goal into 2–4 tasks and assign each to a @handle in the crew.",
                5,
                true,
            )
            .text_size(theme::text_body())
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_select = cx.new(|select_cx| {
            RuntimeSelect::runtime(
                "new-role-runtime",
                select_cx.focus_handle(),
                runtime.clone(),
                &runtimes,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_create_role_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
            .detailed(true)
            .disabled(runtimes.is_empty())
            .placeholder(if agents_checking {
                "Detecting agents…"
            } else {
                "No enabled agents detected"
            })
        });
        let permission_root = root.clone();
        let permission_select = cx.new(|select_cx| {
            StyledSelect::new(
                "new-role-permission",
                select_cx.focus_handle(),
                permission_mode_value(PermissionMode::Auto),
                permission_options(&runtime),
                Rc::new(move |value, _, cx| {
                    permission_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.create.as_mut() {
                            form.permission_mode = parse_permission_mode(&value);
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), scroll_owner));
        let browse_focus = cx.focus_handle();
        let args_hint_focus = cx.focus_handle();
        let model_hint_focus = cx.focus_handle();
        let permission_hint_focus = cx.focus_handle();
        let close_focus = cx.focus_handle();
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
            if form.handle_empty != empty || form.handle_error != error {
                form.handle_empty = empty;
                form.handle_error = error;
                cx.notify();
            }
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
        self.role_surfaces.create = Some(CreateRoleForm {
            runtimes,
            runtime,
            permission_mode: PermissionMode::Auto,
            handle: handle.clone(),
            display_name,
            command,
            args,
            model,
            model_field,
            working_dir,
            system_prompt,
            runtime_select,
            permission_select,
            scroll,
            scrollbar,
            browse_focus,
            args_hint_focus,
            model_hint_focus,
            permission_hint_focus,
            close_focus,
            cancel_focus,
            submit_focus,
            handle_empty: true,
            handle_error: None,
            display_name_valid: false,
            submitting: false,
            agents_checking,
            agents_error,
            error: None,
            _subscriptions: subscriptions,
        });
        handle.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    pub(crate) fn open_role_edit(
        &mut self,
        role: Role,
        slot: Option<runner_backend::model::SlotWithRole>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut runtimes, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let resolution = resolve_role_edit(&role, slot.as_ref());
        ensure_runtime_present(self.core(cx), &mut runtimes, &resolution.runtime);
        self.request_model_catalog(&resolution.runtime, cx);
        let display_name = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                role.display_name.clone(),
                "",
                false,
            )
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
        let visible_args = if slot.is_some() {
            String::new()
        } else {
            runner_backend::router::runtime::strip_permission_flags(
                Runtime::parse(&role.runtime),
                &role.args,
            )
            .join(" ")
        };
        let args = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), visible_args, "--mcp-debug", true)
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
        let model_placeholder =
            runtime_model_placeholder(&runtimes, &resolution.runtime, slot.as_ref().map(|_| &role));
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
            TextField::textarea(
                input_cx.focus_handle(),
                role.system_prompt.clone().unwrap_or_default(),
                "",
                6,
                true,
            )
            .text_size(theme::text_body())
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_value = if slot.is_some() && !resolution.runtime_pinned {
            String::new()
        } else {
            resolution.runtime.clone()
        };
        let runtime_options =
            role_edit_runtime_options(&runtimes, &role, &resolution.runtime, slot.is_some());
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-role-runtime",
                select_cx.focus_handle(),
                runtime_value,
                runtime_options,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_role_edit_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
            .detailed(true)
            .monospace(true)
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
                    slot.is_some(),
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
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let permission_mode = if slot.is_some() {
            PermissionMode::Default
        } else {
            let inferred = runner_backend::router::runtime::infer_permission_mode(
                Runtime::parse(&role.runtime),
                &role.args,
            );
            // A row can carry a mode this runtime no longer offers —
            // a Trae row saved with the old `--permission-mode auto`
            // still infers as Auto (#599). Show the fallback rather
            // than a value that is not in the list; saving then
            // rewrites the row.
            if permission_modes(&resolution.runtime).contains(&inferred) {
                inferred
            } else {
                PermissionMode::Default
            }
        };
        let permission_root = root.clone();
        let permission_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-role-permission",
                select_cx.focus_handle(),
                permission_mode_value(permission_mode),
                permission_options(&resolution.runtime),
                Rc::new(move |value, _, cx| {
                    permission_root.update(cx, |this, cx| {
                        if let Some(form) = this.role_surfaces.edit.as_mut() {
                            form.permission_mode = parse_permission_mode(&value);
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let scroll = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), scroll_owner));
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
        self.role_surfaces.edit = Some(RoleEditForm {
            role,
            slot,
            runtimes,
            runtime: resolution.runtime,
            runtime_pinned: resolution.runtime_pinned,
            permission_mode,
            display_name: display_name.clone(),
            command,
            args,
            model,
            model_field,
            effort: resolution.effort,
            effort_select,
            permission_select,
            runtime_select,
            working_dir,
            system_prompt,
            scroll,
            scrollbar,
            browse_focus: cx.focus_handle(),
            runtime_hint_focus: cx.focus_handle(),
            args_hint_focus: cx.focus_handle(),
            model_hint_focus: cx.focus_handle(),
            effort_hint_focus: cx.focus_handle(),
            permission_hint_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            display_name_valid: true,
            submitting: false,
            agents_checking,
            agents_error,
            error: None,
            _subscriptions: subscriptions,
        });
        self.sync_role_edit_efforts(cx);
        display_name.read(cx).focus_handle().focus(window);
        cx.notify();
    }
}

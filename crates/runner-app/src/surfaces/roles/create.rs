use super::logic::create_role_can_submit;
use super::logic::create_role_form_is_composing;
use super::logic::parse_speed;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::split_args;
use super::logic::trimmed_option;
use super::logic::RoleFormKind;
use runner_backend::model::Runtime;
use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Context, KeyDownEvent, Window};
use runner_backend::ops::role::CreateRoleInput;
use runner_backend::router::runtime::PermissionMode;

use crate::*;

impl NativeRoot {
    pub(super) fn select_create_role_runtime(&mut self, runtime: String, cx: &mut Context<Self>) {
        if Runtime::parse(&runtime).is_none() {
            return;
        }
        let Some(form) = self.role_surfaces.create.as_mut() else {
            return;
        };
        let form = &mut form.fields;
        if form.runtime == runtime || form.submitting {
            return;
        }
        form.runtime = runtime.clone();
        form.effort.clear();
        if runtime != "codex" {
            form.speed = "inherit".into();
            form.speed_select.update(cx, |select, select_cx| {
                select.set_value("inherit", select_cx)
            });
        }
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
        self.sync_create_role_efforts(cx);
        self.request_model_catalog(&runtime, cx);
        cx.notify();
    }

    pub(super) fn close_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .role_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        let route = self
            .role_surfaces
            .create
            .take()
            .map(|form| form.return_route)
            .unwrap_or(AppRoute::Roles);
        self.open_page_route(route, window, cx);
        cx.notify();
    }

    pub(super) fn on_create_role_key_down(
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

    pub(super) fn browse_create_role_cwd(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn submit_create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            effort: trimmed_option(&form.effort),
            codex_speed: parse_speed(&form.speed),
            permission_mode: PermissionMode::Default,
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
}

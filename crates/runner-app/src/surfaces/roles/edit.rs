use super::logic::permission_modes;
use super::logic::role_edit_args;
use super::logic::role_edit_form_is_composing;
use super::logic::runtime_entry;
use super::logic::runtime_model_placeholder;
use super::logic::runtime_models;
use super::logic::trimmed_option;
use super::logic::RoleFormKind;
use runner_core::protocol::model::Runtime;

use gpui::prelude::*;
use gpui::{Context, Entity, KeyDownEvent, PathPromptOptions, Window};
use runner_app::ui::TextField;
use runner_core::protocol::permissions::PermissionMode;
use runner_core::protocol::role::UpdateRoleInput;

use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn select_role_edit_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        if !value.is_empty() && Runtime::parse(&value).is_none() {
            return;
        }
        let Some(form) = self.role_surfaces.edit.as_mut() else {
            return;
        };
        if form.submitting {
            return;
        }
        let next_runtime = value;
        if next_runtime != form.runtime {
            form.effort.clear();
            form.model
                .update(cx, |input, input_cx| input.reset("", input_cx));
            let command = if next_runtime == form.role.runtime {
                form.role.command.clone()
            } else {
                runtime_entry(&form.runtimes, &next_runtime)
                    .map(|runtime| runtime.command.clone())
                    .unwrap_or_else(|| form.role.command.clone())
            };
            form.command
                .update(cx, |input, input_cx| input.reset(command, input_cx));
        }
        form.runtime = next_runtime.clone();
        if !crate::runtime_ui::catalog_capabilities(&form.runtimes, &next_runtime).codex_speed {
            form.speed = "inherit".into();
            form.speed_select.update(cx, |select, select_cx| {
                select.set_value("inherit", select_cx)
            });
        }
        let model_placeholder = runtime_model_placeholder(&form.runtimes, &next_runtime, None);
        form.model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        form.model_field.update(cx, |field, field_cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &next_runtime), field_cx)
        });
        if !permission_modes(&next_runtime).contains(&form.permission_mode) {
            form.permission_mode = PermissionMode::Default;
        }
        self.sync_role_edit_efforts(cx);
        self.request_model_catalog(&next_runtime, cx);
        cx.notify();
    }

    /// The role page's in-place editor lives only on its own role's page, so a
    /// route that leaves it discards the draft, as Cancel does.
    pub(crate) fn drop_role_edit_for_route(&mut self, route: &AppRoute) {
        if !matches!(route, AppRoute::NewRole | AppRoute::Settings)
            && !self
                .role_surfaces
                .create
                .as_ref()
                .is_some_and(|form| form.submitting)
        {
            self.role_surfaces.create = None;
        }
        let stale = self.role_surfaces.edit.as_ref().is_some_and(|form| {
            !form.submitting
                && !matches!(route, AppRoute::Settings)
                && !matches!(route, AppRoute::RoleDetail(handle) if handle == &form.role.handle)
        });
        if stale {
            self.role_surfaces.edit = None;
        }
    }

    pub(super) fn close_role_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .role_surfaces
            .edit
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.role_surfaces.edit = None;
        window.focus(&self.root_focus, cx);
        cx.notify();
    }

    pub(super) fn on_role_edit_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.role_surfaces.edit.as_ref().is_some_and(|form| {
                !form
                    .system_prompt
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
                    && !role_edit_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_role_edit(window, cx);
        }
    }

    pub(super) fn browse_role_edit_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self
            .role_surfaces
            .edit
            .as_ref()
            .filter(|form| !form.submitting)
            .map(|form| form.working_dir.clone())
        else {
            return;
        };
        self.browse_role_form_cwd(input, RoleFormKind::Edit, cx);
    }

    pub(super) fn browse_role_form_cwd(
        &mut self,
        input: Entity<TextField>,
        kind: RoleFormKind,
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
                    RoleFormKind::Create => this
                        .role_surfaces
                        .create
                        .as_ref()
                        .map(|form| form.working_dir.clone()),
                    RoleFormKind::Edit => this
                        .role_surfaces
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
                        RoleFormKind::Create => {
                            if let Some(form) = this.role_surfaces.create.as_mut() {
                                form.error = Some(error);
                            }
                        }
                        RoleFormKind::Edit => {
                            if let Some(form) = this.role_surfaces.edit.as_mut() {
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

    pub(super) fn submit_role_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.role_surfaces.edit.as_mut() else {
            return;
        };
        if form.submitting || form.display_name.read(cx).text().trim().is_empty() {
            return;
        }
        form.submitting = true;
        form.error = None;
        let update = UpdateRoleInput {
            display_name: Some(form.display_name.read(cx).text().trim().to_owned()),
            runtime: Runtime::parse(&form.runtime),
            command: Some(form.command.read(cx).text().trim().to_owned()),
            args: Some(role_edit_args(form, cx)),
            working_dir: Some(trimmed_option(form.working_dir.read(cx).text())),
            system_prompt: Some(trimmed_option(form.system_prompt.read(cx).text())),
            env: None,
            model: Some(trimmed_option(form.model.read(cx).text())),
            effort: Some(trimmed_option(&form.effort)),
            codex_speed: Some(super::logic::parse_speed(&form.speed)),
            permission_mode: (!permission_modes(&form.runtime).is_empty())
                .then_some(form.permission_mode),
        };
        let role_id = form.role.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.role_update(&role_id, update)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(()) => {
                        this.role_surfaces.edit = None;
                        if let Ok(roles) = this.core(cx).role_list() {
                            this.app_store
                                .update(cx, |store, store_cx| store.replace_roles(roles, store_cx));
                        }
                        this.load_role_page(cx);
                        if let AppRoute::RoleDetail(handle) = this.route.clone() {
                            this.load_role_detail(handle, cx);
                        }
                    }
                    Err(error) => {
                        if let Some(form) = this.role_surfaces.edit.as_mut() {
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

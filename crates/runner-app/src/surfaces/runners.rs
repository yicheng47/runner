use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, relative, rems, svg, AnyElement, Context, CursorStyle, Entity, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, PathPromptOptions, ScrollHandle, SharedString,
    Subscription, Window,
};
use runner_app::ui::{
    working_dir_text_field, Button, ButtonSize, ButtonVariant, ConfirmDialog, ContextMenu, Drawer,
    EmptyStateCard, Field, IconButton, IconButtonSize, MenuItem as UiMenuItem, Modal, ModelField,
    OverlayWidth, PaginatedListPage, RuntimeBadge, RuntimeSelect, Scrollbar, SearchInput,
    SelectOption, StyledSelect, TextField, Tooltip, WorkingDirField,
};
use runner_backend::model::Runner;
use runner_backend::ops::runner::{
    CreateRunnerInput, RunnerActivity, RunnerWithActivity, UpdateRunnerInput,
};
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};
use runner_backend::ops::slot::CrewMembership;
use runner_backend::router::runtime::PermissionMode;

use super::*;
use crate::list_controls::{ListControls, LIST_QUERY_DEBOUNCE_MS};
use crate::*;

const FORM_WIDTH: f32 = 576.;
const FIELD_WIDTH: f32 = 528.;

#[derive(Default)]
struct RunnerDetailState {
    handle: String,
    runner: Option<Runner>,
    activity: Option<RunnerActivity>,
    crews: Vec<CrewMembership>,
    loaded: bool,
    loading: bool,
    error: Option<String>,
}

#[derive(Clone)]
enum RunnerMenuAction {
    Open(String),
    Delete { id: String, handle: String },
}

struct RunnerDeleteConfirm {
    id: String,
    handle: String,
}

struct CreateRunnerForm {
    runtimes: Vec<RuntimeCatalogEntry>,
    runtime: String,
    permission_mode: PermissionMode,
    handle: Entity<TextField>,
    display_name: Entity<TextField>,
    command: Entity<TextField>,
    args: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    working_dir: Entity<TextField>,
    system_prompt: Entity<TextField>,
    runtime_select: Entity<RuntimeSelect>,
    permission_select: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    browse_focus: FocusHandle,
    args_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    permission_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    handle_empty: bool,
    handle_error: Option<&'static str>,
    display_name_valid: bool,
    submitting: bool,
    agents_checking: bool,
    agents_error: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct RunnerEditForm {
    runner: Runner,
    slot: Option<runner_backend::model::SlotWithRunner>,
    runtimes: Vec<RuntimeCatalogEntry>,
    runtime: String,
    runtime_pinned: bool,
    permission_mode: PermissionMode,
    display_name: Entity<TextField>,
    command: Entity<TextField>,
    args: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    effort: String,
    effort_select: Entity<StyledSelect>,
    permission_select: Entity<StyledSelect>,
    runtime_select: Entity<RuntimeSelect>,
    working_dir: Entity<TextField>,
    system_prompt: Entity<TextField>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    browse_focus: FocusHandle,
    runtime_hint_focus: FocusHandle,
    args_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    effort_hint_focus: FocusHandle,
    permission_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    display_name_valid: bool,
    submitting: bool,
    agents_checking: bool,
    agents_error: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct RunnerSurfaces {
    list: ListControls<RunnerWithActivity>,
    search: Entity<SearchInput>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    detail: RunnerDetailState,
    create: Option<CreateRunnerForm>,
    edit: Option<RunnerEditForm>,
    context_menu: Option<Entity<ContextMenu>>,
    delete_confirm: Option<RunnerDeleteConfirm>,
    delete_busy: bool,
    chat_pending: Option<String>,
}

impl RunnerSurfaces {
    pub(crate) fn new(root: Entity<NativeRoot>, cx: &mut Context<NativeRoot>) -> Self {
        let search_root = root;
        let search = cx.new(move |search_cx| {
            SearchInput::new(
                "",
                "Search runners",
                "Search runners…",
                Rc::new(move |query, cx| {
                    search_root.update(cx, |this, cx| this.set_runner_query(query, cx));
                }),
                search_cx,
            )
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        Self {
            list: ListControls::default(),
            search,
            scroll,
            scrollbar,
            detail: RunnerDetailState::default(),
            create: None,
            edit: None,
            context_menu: None,
            delete_confirm: None,
            delete_busy: false,
            chat_pending: None,
        }
    }
}

impl NativeRoot {
    pub(crate) fn refresh_runner_form_runtimes(&mut self, cx: &mut Context<Self>) {
        let (selectable, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let catalog_loaded = agents_error.is_none();
        let placeholder = if agents_checking {
            "Detecting agents…"
        } else {
            "No enabled agents detected"
        };

        if let Some(form) = self.runner_surfaces.create.as_mut() {
            form.agents_checking = agents_checking;
            form.agents_error.clone_from(&agents_error);
            if catalog_loaded {
                form.runtimes.clone_from(&selectable);
                let next_runtime = form
                    .runtimes
                    .iter()
                    .find(|runtime| runtime.name == form.runtime)
                    .or_else(|| form.runtimes.first())
                    .map(|runtime| runtime.name.clone())
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
                    runtime_model_placeholder(&form.runtimes, &next_runtime, false);
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
        if let Some(form) = self.runner_surfaces.edit.as_mut() {
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
                let options = runner_edit_runtime_options(
                    &form.runtimes,
                    &form.runner,
                    &form.runtime,
                    form.slot.is_some(),
                );
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx);
                    select.set_value(runtime_value, select_cx);
                    select.set_placeholder(placeholder, select_cx);
                });
                let model_placeholder =
                    runtime_model_placeholder(&form.runtimes, &form.runtime, form.slot.is_some());
                form.model.update(cx, |input, input_cx| {
                    input.set_placeholder(model_placeholder, input_cx)
                });
                form.model_field.update(cx, |field, field_cx| {
                    field.set_suggestions(runtime_models(&form.runtimes, &form.runtime), field_cx)
                });
                form.effort_select.update(cx, |select, select_cx| {
                    select.set_options(
                        effort_options(
                            &form.runtimes,
                            &form.runtime,
                            &form.runner,
                            form.slot.is_some(),
                        ),
                        select_cx,
                    );
                    select.set_value(form.effort.clone(), select_cx);
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn open_create_runner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.runner_surfaces.create.is_some() {
            return;
        }
        let (runtimes, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let runtime = runtimes
            .first()
            .map(|runtime| runtime.name.clone())
            .unwrap_or_default();
        let command = runtime_entry(&runtimes, &runtime)
            .map(|runtime| runtime.command.clone())
            .unwrap_or_default();
        let handle = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "architect", true).text_size(14.)
        });
        handle.update(cx, |input, input_cx| input.set_bare(true, input_cx));
        let display_name = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Architect", false).text_size(14.)
        });
        let command = cx.new(|input_cx| {
            let mut input =
                TextField::new(input_cx.focus_handle(), command, "", true).text_size(14.);
            input.set_disabled(true, input_cx);
            input
        });
        let args = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "--mcp-debug", true).text_size(14.)
        });
        let model = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", true).placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| {
            ModelField::new(model.clone(), runtime_models(&runtimes, &runtime), model_cx)
        });
        let model_placeholder = runtime_model_placeholder(&runtimes, &runtime, false);
        model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        model_field.update(cx, |field, field_cx| {
            field.set_disabled(runtime.is_empty(), field_cx)
        });
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let working_dir = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), default_working_dir, "").text_size(13.)
        });
        let system_prompt = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "You are the architect for this crew. When a mission starts, decompose the goal into 2–4 tasks and assign each to a @handle in the crew.",
                5,
                true,
            )
            .text_size(13.)
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_select = cx.new(|select_cx| {
            RuntimeSelect::runtime(
                "new-runner-runtime",
                select_cx.focus_handle(),
                runtime.clone(),
                &runtimes,
                Rc::new(move |value, _, cx| {
                    runtime_root
                        .update(cx, |this, cx| this.select_create_runner_runtime(value, cx));
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
                "new-runner-permission",
                select_cx.focus_handle(),
                permission_mode_value(PermissionMode::Auto),
                permission_options(&runtime),
                Rc::new(move |value, _, cx| {
                    permission_root.update(cx, |this, cx| {
                        if let Some(form) = this.runner_surfaces.create.as_mut() {
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
            let error = validate_runner_handle(&text);
            let Some(form) = this.runner_surfaces.create.as_mut() else {
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
            let Some(form) = this.runner_surfaces.create.as_mut() else {
                return;
            };
            if form.display_name_valid != valid {
                form.display_name_valid = valid;
                cx.notify();
            }
        }));
        self.runner_surfaces.create = Some(CreateRunnerForm {
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

    pub(crate) fn open_runner_edit(
        &mut self,
        runner: Runner,
        slot: Option<runner_backend::model::SlotWithRunner>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut runtimes, agents_checking, agents_error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let mut resolution = resolve_runner_edit(&runner, slot.as_ref());
        ensure_runtime_present(self.core(cx), &mut runtimes, &resolution.runtime);
        if !runtime_efforts(&runtimes, &resolution.runtime)
            .iter()
            .any(|option| option.value == resolution.effort)
        {
            resolution.effort.clear();
        }
        let display_name = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                runner.display_name.clone(),
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
            runner_backend::router::runtime::strip_permission_flags(&runner.runtime, &runner.args)
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
            runtime_model_placeholder(&runtimes, &resolution.runtime, slot.is_some());
        model.update(cx, |input, input_cx| {
            input.set_placeholder(model_placeholder, input_cx)
        });
        let working_dir = cx.new(|input_cx| {
            working_dir_text_field(
                input_cx.focus_handle(),
                runner.working_dir.clone().unwrap_or_default(),
                "",
            )
            .text_size(13.)
        });
        let system_prompt = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                runner.system_prompt.clone().unwrap_or_default(),
                "",
                6,
                true,
            )
            .text_size(13.)
        });
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_value = if slot.is_some() && !resolution.runtime_pinned {
            String::new()
        } else {
            resolution.runtime.clone()
        };
        let runtime_options =
            runner_edit_runtime_options(&runtimes, &runner, &resolution.runtime, slot.is_some());
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-runner-runtime",
                select_cx.focus_handle(),
                runtime_value,
                runtime_options,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_runner_edit_runtime(value, cx));
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
                "edit-runner-effort",
                select_cx.focus_handle(),
                resolution.effort.clone(),
                effort_options(&runtimes, &resolution.runtime, &runner, slot.is_some()),
                Rc::new(move |value, _, cx| {
                    effort_root.update(cx, |this, cx| {
                        if let Some(form) = this.runner_surfaces.edit.as_mut() {
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
            runner_backend::router::runtime::infer_permission_mode(&runner.runtime, &runner.args)
        };
        let permission_root = root.clone();
        let permission_select = cx.new(|select_cx| {
            StyledSelect::new(
                "edit-runner-permission",
                select_cx.focus_handle(),
                permission_mode_value(permission_mode),
                permission_options(&resolution.runtime),
                Rc::new(move |value, _, cx| {
                    permission_root.update(cx, |this, cx| {
                        if let Some(form) = this.runner_surfaces.edit.as_mut() {
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
        let subscriptions = vec![cx.observe(&display_name, |this, input, cx| {
            let valid = !input.read(cx).text().trim().is_empty();
            let Some(form) = this.runner_surfaces.edit.as_mut() else {
                return;
            };
            if form.display_name_valid != valid {
                form.display_name_valid = valid;
                cx.notify();
            }
        })];
        self.runner_surfaces.edit = Some(RunnerEditForm {
            runner,
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
        display_name.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    pub(crate) fn render_entity_overlays(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = self.runner_surfaces.context_menu.clone() {
            overlays.push(menu.into_any_element());
        }
        if let Some(menu) = self.crew_surfaces.context_menu.clone() {
            overlays.push(menu.into_any_element());
        }
        if self.runner_surfaces.create.is_some() {
            overlays.push(self.render_create_runner_modal(cx));
        }
        if self.runner_surfaces.edit.is_some() {
            overlays.push(self.render_runner_edit_drawer(cx));
        }
        if self.runner_surfaces.delete_confirm.is_some() {
            overlays.push(self.render_runner_delete_confirm(cx));
        }
        if self.start_mission_modal.is_some() {
            overlays.push(self.render_start_mission_modal(cx));
        }
        if matches!(self.route, AppRoute::Mission(_)) {
            let workspace = self.mission_workspace.clone();
            overlays.extend(workspace.update(cx, |workspace, workspace_cx| {
                workspace.render_mission_overlays(workspace_cx)
            }));
        }
        overlays.extend(self.render_crew_overlays(cx));
        overlays
    }

    fn select_create_runner_runtime(&mut self, runtime: String, cx: &mut Context<Self>) {
        let Some(form) = self.runner_surfaces.create.as_mut() else {
            return;
        };
        if form.runtime == runtime || form.submitting {
            return;
        }
        form.runtime = runtime.clone();
        let command = runtime_entry(&form.runtimes, &runtime)
            .map(|entry| entry.command.clone())
            .unwrap_or_default();
        let model_placeholder = runtime_model_placeholder(&form.runtimes, &runtime, false);
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
        cx.notify();
    }

    fn close_create_runner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .runner_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.runner_surfaces.create = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_create_runner_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.runner_surfaces.create.as_ref().is_some_and(|form| {
                !form
                    .system_prompt
                    .read(cx)
                    .focus_handle()
                    .is_focused(window)
                    && !create_runner_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_create_runner(window, cx);
        }
    }

    fn browse_create_runner_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self
            .runner_surfaces
            .create
            .as_ref()
            .filter(|form| !form.submitting)
            .map(|form| form.working_dir.clone())
        else {
            return;
        };
        self.browse_runner_form_cwd(input, RunnerFormKind::Create, cx);
    }

    fn submit_create_runner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.runner_surfaces.create.as_mut() else {
            return;
        };
        if !create_runner_can_submit(form) {
            return;
        }
        form.submitting = true;
        form.error = None;
        let input = CreateRunnerInput {
            handle: form.handle.read(cx).text().to_owned(),
            display_name: form.display_name.read(cx).text().trim().to_owned(),
            runtime: form.runtime.clone(),
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
            runner_backend::ops::runner::runner_create(&core, input)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(runner) => {
                        let handle = runner.handle.clone();
                        this.runner_surfaces.create = None;
                        if let Ok(runners) = runner_backend::ops::runner::runner_list(this.core(cx))
                        {
                            this.app_store.update(cx, |store, store_cx| {
                                store.replace_runners(runners, store_cx)
                            });
                        }
                        this.load_runner_page(cx);
                        this.open_runner_detail(handle, window, cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.runner_surfaces.create.as_mut() {
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

    fn render_create_runner_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .runner_surfaces
            .create
            .as_ref()
            .expect("create runner form");
        let submitting = form.submitting;
        let can_submit = create_runner_can_submit(form);
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
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("New runner"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Reusable across crews and chats."),
                    ),
            )
            .child(
                IconButton::new("close-create-runner", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close new runner")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_create_runner(window, cx));
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
            .text_size(rems(14. / 16.))
            .child(
                div()
                    .pr_1()
                    .font_family("JetBrains Mono")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child("@"),
            )
            .child(div().min_w(px(0.)).flex_1().child(form.handle.clone()));
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_create_runner_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                Field::new("new-runner-handle", "Handle", handle_input)
                    .focus_target(form.handle.read(cx).focus_handle())
                    .when_some(handle_error, |field, error| field.error(error)),
            )
            .child(
                Field::new(
                    "new-runner-display-name",
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
                        Field::new("new-runner-runtime", "Agent", form.runtime_select.clone())
                            .focus_target(form.runtime_select.read(cx).focus_handle()),
                    )
                    .children(form.agents_error.clone().map(|error| {
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::danger())
                            .child(error)
                    })),
            )
            .child(
                Field::new("new-runner-command", "Command", form.command.clone())
                    .focus_target(form.command.read(cx).focus_handle()),
            )
            .child(
                Field::new("new-runner-args", "Args", form.args.clone())
                    .focus_target(form.args.read(cx).focus_handle())
                    .hint(
                        "extra flags · whitespace-separated",
                        form.args_hint_focus.clone(),
                    ),
            )
            .child(
                Field::new("new-runner-model", "Model", form.model_field.clone())
                    .focus_target(form.model.read(cx).focus_handle())
                    .hint(
                        "optional · blank uses the agent's own model · type a name or pick an alias",
                        form.model_hint_focus.clone(),
                    ),
            )
            .children((!permission_modes(&form.runtime).is_empty()).then(|| {
                Field::new(
                    "new-runner-permission-mode",
                    "Permission mode",
                    form.permission_select.clone(),
                )
                .focus_target(form.permission_select.read(cx).focus_handle())
                .hint(permission_description, form.permission_hint_focus.clone())
            }))
            .child(
                Field::new(
                    "new-runner-working-dir",
                    "Working directory",
                    WorkingDirField::new(
                        form.working_dir.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| {
                                this.browse_create_runner_cwd(cx)
                            });
                        }),
                    )
                    .browse_focus(form.browse_focus.clone()),
                )
                .focus_target(form.working_dir.read(cx).focus_handle()),
            )
            .child(
                Field::new(
                    "new-runner-system-prompt",
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
                Button::new("cancel-create-runner", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_create_runner(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-create-runner",
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
                    submit_root.update(cx, |this, cx| this.submit_create_runner(window, cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_create_runner(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(FORM_WIDTH))
        .busy(submitting)
        .focus_order(create_runner_focus_order(form, cx))
        .scrollbar(form.scroll.clone(), form.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }

    fn select_runner_edit_runtime(&mut self, value: String, cx: &mut Context<Self>) {
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

    fn browse_runner_form_cwd(
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
            runtime: (!edits_slot).then(|| form.runtime.clone()),
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
                    runtime_override: Some(form.runtime_pinned.then(|| form.runtime.clone())),
                    model_override: Some(trimmed_option(form.model.read(cx).text())),
                    effort_override: Some(trimmed_option(&form.effort)),
                },
                slot.slot.crew_id.clone(),
            )
        });
        let runner_id = form.runner.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_update(&core, &runner_id, update)
                .map_err(|error| error.to_string())?;
            let crew_id = if let Some((slot_id, update, crew_id)) = slot_update {
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

    fn render_runner_edit_drawer(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
                            .font_family("JetBrains Mono")
                            .text_size(rems(12. / 16.))
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
                            .text_size(rems(11. / 16.))
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

    fn render_runner_delete_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .runner_surfaces
            .delete_confirm
            .as_ref()
            .expect("runner delete confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        ConfirmDialog::new(
            format!("Delete runner @{}?", confirm.handle),
            format!(
                "This removes @{} from every crew it's in and deletes archived session history for that runner. Unarchived chats must be archived first. Crews and missions are kept.",
                confirm.handle
            ),
            "Delete runner",
            "Deleting…",
            self.runner_surfaces.delete_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_runner_delete(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.runner_surfaces.delete_busy {
                        this.runner_surfaces.delete_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }

    fn confirm_runner_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.runner_surfaces.delete_confirm.as_ref() else {
            return;
        };
        if self.runner_surfaces.delete_busy {
            return;
        }
        self.runner_surfaces.delete_busy = true;
        let id = confirm.id.clone();
        let handle = confirm.handle.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_delete(&core, &id)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.runner_surfaces.delete_busy = false;
                match result {
                    Ok(()) => {
                        this.runner_surfaces.delete_confirm = None;
                        if let Ok(runners) = runner_backend::ops::runner::runner_list(this.core(cx))
                        {
                            this.app_store.update(cx, |store, store_cx| {
                                store.replace_runners(runners, store_cx)
                            });
                        }
                        this.load_runner_page(cx);
                        this.show_toast(
                            format!("Deleted runner @{handle}."),
                            crate::toast::ToastTone::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        this.runner_surfaces.delete_confirm = None;
                        this.show_toast(error, crate::toast::ToastTone::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_runners(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Runners {
            self.runner_surfaces.list.reset();
            self.runner_surfaces
                .search
                .update(cx, |search, search_cx| search.reset_value("", search_cx));
            self.runner_surfaces
                .scroll
                .set_offset(gpui::Point::new(px(0.), px(0.)));
        }
        self.enter_entity_route(AppRoute::Runners, window, cx);
        self.load_runner_page(cx);
    }

    pub(crate) fn open_runner_detail(
        &mut self,
        handle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_entity_route(AppRoute::RunnerDetail(handle.clone()), window, cx);
        self.load_runner_detail(handle, cx);
    }

    pub(crate) fn enter_entity_route(
        &mut self,
        route: AppRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sidebar_transients(cx);
        self.runner_surfaces.context_menu = None;
        self.crew_surfaces.context_menu = None;
        self.set_route(route, cx);
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn set_runner_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.runner_surfaces.list.set_query(query);
        if update.load_now {
            self.load_runner_page(cx);
        }
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(LIST_QUERY_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this
                    .runner_surfaces
                    .list
                    .apply_debounced_query(update.generation)
                {
                    this.load_runner_page(cx);
                }
            });
        })
        .detach();
    }

    fn set_runner_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.runner_surfaces.list.set_page(page) {
            self.load_runner_page(cx);
        }
    }

    pub(crate) fn load_runner_page(&mut self, cx: &mut Context<Self>) {
        let request = self.runner_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_list_with_activity(
                &core,
                request.page as i64,
                request.page_size as i64,
                &request.query,
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(page) => {
                        if this.runner_surfaces.list.apply_success(request_id, page) {
                            this.load_runner_page(cx);
                        }
                    }
                    Err(error) => {
                        this.runner_surfaces.list.apply_error(request_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_runner_detail(&mut self, handle: String, cx: &mut Context<Self>) {
        if self.runner_surfaces.detail.handle == handle {
            let detail = &mut self.runner_surfaces.detail;
            detail.loading = !detail.loaded;
            detail.error = None;
        } else {
            self.runner_surfaces.detail = RunnerDetailState {
                handle: handle.clone(),
                loading: true,
                ..Default::default()
            };
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested = handle.clone();
            let result = (|| {
                let runner = runner_backend::ops::runner::runner_get_by_handle(&core, &handle)?;
                let activity = runner_backend::ops::runner::runner_activity(&core, &runner.id)?;
                let crews = runner_backend::ops::slot::runner_crews_list(&core, &runner.id)?;
                Ok::<_, runner_backend::error::Error>((runner, activity, crews))
            })();
            result
                .map(|(runner, activity, crews)| (requested.clone(), runner, activity, crews))
                .map_err(|error| (requested, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((handle, runner, activity, crews))
                        if matches!(
                            &this.route,
                            AppRoute::RunnerDetail(active) if active == &handle
                        ) =>
                    {
                        this.runner_surfaces.detail = RunnerDetailState {
                            handle,
                            runner: Some(runner),
                            activity: Some(activity),
                            crews,
                            loaded: true,
                            loading: false,
                            error: None,
                        };
                    }
                    Ok(_) => {}
                    Err((handle, error))
                        if matches!(
                            &this.route,
                            AppRoute::RunnerDetail(active) if active == &handle
                        ) =>
                    {
                        let detail = &mut this.runner_surfaces.detail;
                        detail.loading = false;
                        if error.to_lowercase().contains("not found") {
                            detail.loaded = true;
                            detail.runner = None;
                            detail.activity = None;
                            detail.crews.clear();
                        }
                        detail.error = Some(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_entity_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let route = if self.route == AppRoute::Settings {
            self.settings_return_route.clone()
        } else {
            self.route.clone()
        };
        match route {
            AppRoute::Chat => self.render_active_tab(window, cx),
            AppRoute::Runners => self.render_runners_page(cx),
            AppRoute::RunnerDetail(_) => self.render_runner_detail(cx),
            AppRoute::Crews | AppRoute::CrewEditor(_) => self.render_crew_surface(window, cx),
            AppRoute::Mission(_) => self.mission_workspace.clone().into_any_element(),
            AppRoute::ArchivedChat => self.render_archived_chat(window, cx),
            AppRoute::Settings => self.render_active_tab(window, cx),
        }
    }

    fn render_runners_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root.clone();
        let query = self.runner_surfaces.list.query.clone();
        let cards = self
            .runner_surfaces
            .list
            .items
            .clone()
            .into_iter()
            .map(|item| self.render_runner_card(item, cx))
            .collect::<Vec<_>>();
        let no_matches = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_8()
            .py(rems(56. / 16.))
            .text_center()
            .child(
                svg()
                    .path("search-x.svg")
                    .size(rems(20. / 16.))
                    .text_color(theme::faint()),
            )
            .child(
                div()
                    .text_size(rems(14. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .child(format!("No runners match \"{query}\"")),
            )
            .child(
                div()
                    .text_size(rems(12. / 16.))
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child("Search checks handles and names."),
            )
            .child(
                Button::new("clear-runner-search", "Clear search")
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.runner_surfaces
                                .search
                                .update(cx, |search, search_cx| search.set_value("", search_cx));
                        });
                    }),
            );
        let empty_state = EmptyStateCard::new(
            svg()
                .path("terminal.svg")
                .size(rems(22. / 16.))
                .text_color(theme::accent()),
            "No runners yet",
            "A runner is a reusable CLI agent — claude-code, codex, a custom shell — that crews pull in. Add one to start composing crews.",
            Button::new("empty-new-runner", "+ New runner")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    empty_create_root.update(cx, |this, cx| {
                        this.open_create_runner(window, cx)
                    });
                }),
        );
        PaginatedListPage::new(
            "Runners",
            div().child("Reusable CLI agents — pick one for a crew slot or chat directly."),
            Button::new("new-runner", "+ New runner")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_runner(window, cx));
                }),
            "runners",
            empty_state,
            self.runner_surfaces.search.clone(),
            no_matches,
            self.runner_surfaces.list.page,
            self.runner_surfaces.list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| this.set_runner_page(page, cx));
            }),
            div().flex().flex_col().gap_3().children(cards),
            self.runner_surfaces.scroll.clone(),
            self.runner_surfaces.scrollbar.clone(),
        )
        .counts(
            self.runner_surfaces.list.filtered_count,
            self.runner_surfaces.list.total_count,
        )
        .load_state(
            self.runner_surfaces.list.loading,
            self.runner_surfaces.list.loaded,
            self.runner_surfaces.list.error.clone().map(Into::into),
        )
        .into_any_element()
    }

    fn render_runner_card(&self, item: RunnerWithActivity, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let open_root = root.clone();
        let key_root = root.clone();
        let chat_root = root.clone();
        let chat_key_root = root.clone();
        let menu_root = root;
        let handle = item.runner.handle.clone();
        let open_handle = handle.clone();
        let menu_item = item.clone();
        let chat_runner = item.runner.clone();
        let chat_key_runner = item.runner.clone();
        let sessions_label = plural(item.activity.active_sessions, "session", "sessions");
        let missions_label = plural(item.activity.active_missions, "mission", "missions");
        let crews_label = if item.activity.crew_count == 1 {
            "in 1 crew".to_owned()
        } else {
            format!("in {} crews", item.activity.crew_count)
        };
        let live = item.activity.active_sessions > 0 || item.activity.active_missions > 0;
        let pending = self.runner_surfaces.chat_pending.as_deref() == Some(item.runner.id.as_str());
        let command = if item.runner.args.is_empty() {
            item.runner.command.clone()
        } else {
            format!("{} {}", item.runner.command, item.runner.args.join(" "))
        };
        div()
            .id(SharedString::from(format!(
                "runner-card-{}",
                item.runner.id
            )))
            .group("runner-card")
            .tab_index(0)
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_4()
            .cursor_pointer()
            .hover(|card| card.border_color(theme::border_strong()))
            .focus_visible(|card| card.border_color(theme::faint()))
            .on_click(move |_, window, cx| {
                open_root.update(cx, |this, cx| {
                    this.open_runner_detail(open_handle.clone(), window, cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let handle = handle.clone();
                    key_root.update(cx, |this, cx| this.open_runner_detail(handle, window, cx));
                }
            })
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_family("JetBrains Mono")
                                            .text_size(rems(1.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child(format!("@{}", item.runner.handle)),
                                    )
                                    .child(
                                        div()
                                            .font_family("JetBrains Mono")
                                            .text_size(rems(11. / 16.))
                                            .text_color(theme::faint())
                                            .child(item.runner.runtime.clone()),
                                    )
                                    .child(Tooltip::new(
                                        SharedString::from(format!(
                                            "runner-chat-tooltip-{}",
                                            item.runner.id
                                        )),
                                        "Start a new chat",
                                        div()
                                            .id(SharedString::from(format!(
                                                "runner-chat-{}",
                                                item.runner.id
                                            )))
                                            .tab_index(0)
                                            .tab_stop(!pending)
                                            .ml_1()
                                            .flex()
                                            .items_center()
                                            .gap(rems(6. / 16.))
                                            .rounded_sm()
                                            .px(rems(6. / 16.))
                                            .py(rems(2. / 16.))
                                            .text_size(rems(11. / 16.))
                                            .line_height(rems(1.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::accent())
                                            .opacity(if pending { 0.6 } else { 1. })
                                            .cursor(if pending {
                                                CursorStyle::Arrow
                                            } else {
                                                CursorStyle::PointingHand
                                            })
                                            .when(!pending, |button| {
                                                button.hover(|button| {
                                                    button
                                                        .bg(theme::with_alpha(theme::accent(), 0.1))
                                                })
                                            })
                                            .focus_visible(|button| {
                                                button.bg(theme::with_alpha(theme::accent(), 0.1))
                                            })
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                if !pending {
                                                    let runner = chat_runner.clone();
                                                    chat_root.update(cx, |this, cx| {
                                                        this.start_runner_chat(runner, window, cx)
                                                    });
                                                }
                                            })
                                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                                if !pending
                                                    && matches!(
                                                        event.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    )
                                                {
                                                    cx.stop_propagation();
                                                    let runner = chat_key_runner.clone();
                                                    chat_key_root.update(cx, |this, cx| {
                                                        this.start_runner_chat(runner, window, cx)
                                                    });
                                                }
                                            })
                                            .child(
                                                // The speech-bubble tail pulls the glyph's optical center down.
                                                svg()
                                                    .path("message-square.svg")
                                                    .size(rems(12. / 16.))
                                                    .relative()
                                                    .bottom(rems(1. / 16.))
                                                    .flex_none()
                                                    .text_color(theme::accent()),
                                            )
                                            .child(div().h(rems(1.)).flex().items_center().child(
                                                if pending { "Starting…" } else { "Chat" },
                                            )),
                                    )),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .max_h(rems(38. / 16.))
                                    .overflow_hidden()
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::muted())
                                    .child(item.runner.display_name.clone()),
                            )
                            .child(
                                div()
                                    .mt(rems(6. / 16.))
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child(format!("$ {command}")),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(rems(12. / 16.))
                            .child(
                                div()
                                    .text_color(if live {
                                        theme::accent()
                                    } else {
                                        theme::faint()
                                    })
                                    .child(if live {
                                        if item.activity.active_missions > 0 {
                                            format!("{sessions_label} · {missions_label}")
                                        } else {
                                            sessions_label
                                        }
                                    } else {
                                        crews_label
                                    }),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!(
                                        "runner-actions-{}",
                                        item.runner.id
                                    )),
                                    "more-horizontal.svg",
                                )
                                .size(IconButtonSize::Sm)
                                .stop_click_propagation(true)
                                .tooltip("More actions")
                                .on_press(move |window, cx| {
                                    let position = window.mouse_position();
                                    let item = menu_item.clone();
                                    menu_root.update(cx, |this, cx| {
                                        this.open_runner_menu(item, position, window, cx)
                                    });
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_runner_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let back_root = root.clone();
        let back_key_root = root.clone();
        let edit_root = root.clone();
        let chat_root = root.clone();
        let detail = &self.runner_surfaces.detail;
        let handle = detail.handle.clone();
        let runner = detail.runner.clone();
        let pending = runner.as_ref().is_some_and(|runner| {
            self.runner_surfaces.chat_pending.as_deref() == Some(runner.id.as_str())
        });
        let body = if detail.loading {
            div()
                .text_size(rems(14. / 16.))
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if let Some(runner) = runner.clone() {
            self.render_runner_detail_body(
                runner,
                detail.activity.clone(),
                detail.crews.clone(),
                cx,
            )
        } else {
            div()
                .rounded_sm()
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
                .bg(theme::with_alpha(theme::danger(), 0.1))
                .px_3()
                .py_2()
                .text_size(rems(14. / 16.))
                .text_color(theme::danger())
                .child(format!("Runner @{handle} not found."))
                .into_any_element()
        };
        let header_runner = runner.clone();
        let chat_runner = runner.clone();
        div()
            .id("runner-detail-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(rems(1024. / 16.))
                    .flex()
                    .flex_col()
                    .gap_6()
                    .px_8()
                    .py_8()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(rems(14. / 16.))
                                    .text_color(theme::muted())
                                    .child(
                                        div()
                                            .id("runner-detail-back")
                                            .tab_index(0)
                                            .cursor_pointer()
                                            .hover(|text| text.text_color(theme::text()))
                                            .focus_visible(|text| {
                                                text.text_color(theme::text()).underline()
                                            })
                                            .on_click(move |_, window, cx| {
                                                back_root.update(cx, |this, cx| {
                                                    this.open_runners(window, cx)
                                                });
                                            })
                                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    cx.stop_propagation();
                                                    back_key_root.update(cx, |this, cx| {
                                                        this.open_runners(window, cx)
                                                    });
                                                }
                                            })
                                            .child("Runners"),
                                    )
                                    .child(div().text_color(theme::border_strong()).child("›"))
                                    .child(
                                        div()
                                            .font_family("JetBrains Mono")
                                            .text_size(rems(1.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child(format!("@{handle}")),
                                    )
                                    .children(runner.as_ref().map(|runner| {
                                        RuntimeBadge::new(runner.runtime.clone()).uppercase(true)
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new("edit-runner", "Edit")
                                            .tooltip("Edit runner")
                                            .disabled(header_runner.is_none())
                                            .on_press(move |window, cx| {
                                                if let Some(runner) = header_runner.clone() {
                                                    edit_root.update(cx, |this, cx| {
                                                        this.open_runner_edit(
                                                            runner, None, window, cx,
                                                        )
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(
                                            "runner-detail-chat",
                                            if pending { "Starting…" } else { "Chat now" },
                                        )
                                        .variant(ButtonVariant::Primary)
                                        .tooltip("Start a one-on-one PTY with this runner")
                                        .disabled(chat_runner.is_none() || pending)
                                        .on_press(
                                            move |window, cx| {
                                                if let Some(runner) = chat_runner.clone() {
                                                    chat_root.update(cx, |this, cx| {
                                                        this.start_runner_chat(runner, window, cx)
                                                    });
                                                }
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .children(runner.as_ref().map(|runner| {
                        div()
                            .text_size(rems(14. / 16.))
                            .text_color(theme::muted())
                            .child(runner.display_name.clone())
                    }))
                    .children(detail.error.clone().map(error_banner))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_runner_detail_body(
        &self,
        runner: Runner,
        activity: Option<RunnerActivity>,
        crews: Vec<CrewMembership>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let crew_rows = if crews.is_empty() {
            div()
                .text_size(rems(14. / 16.))
                .text_color(theme::faint())
                .italic()
                .child("Not in any crew yet. Add it to one from Crew Detail.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(crews.into_iter().map(|membership| {
                    let crew_root = root.clone();
                    let crew_key_root = root.clone();
                    let crew_id = membership.crew_id.clone();
                    let crew_key_id = crew_id.clone();
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(theme::border())
                        .py_2()
                        .text_size(rems(14. / 16.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(membership.crew_name),
                                )
                                .children(membership.lead.then(|| {
                                    div()
                                        .rounded_sm()
                                        .bg(theme::with_alpha(theme::accent(), 0.1))
                                        .px(rems(6. / 16.))
                                        .py(rems(2. / 16.))
                                        .text_size(rems(10. / 16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme::accent())
                                        .child("LEAD")
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("open-runner-crew-{}", crew_id)))
                                .tab_index(0)
                                .cursor_pointer()
                                .text_size(rems(12. / 16.))
                                .text_color(theme::accent())
                                .hover(|text| text.underline())
                                .focus_visible(|text| text.underline())
                                .on_click(move |_, window, cx| {
                                    crew_root.update(cx, |this, cx| {
                                        this.open_crew_editor(crew_id.clone(), window, cx)
                                    });
                                })
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        crew_key_root.update(cx, |this, cx| {
                                            this.open_crew_editor(crew_key_id.clone(), window, cx)
                                        });
                                    }
                                })
                                .child("Open →"),
                        )
                }))
                .into_any_element()
        };
        let sessions = activity.as_ref().map_or(0, |value| value.active_sessions);
        let missions = activity.as_ref().map_or(0, |value| value.active_missions);
        let crew_count = activity.as_ref().map_or(0, |value| value.crew_count);
        let last_seen = activity
            .and_then(|value| value.last_started_at)
            .map(format_timestamp)
            .unwrap_or_else(|| "—".into());
        let args = runner.args.join(" ");
        div()
            .w_full()
            .flex()
            .items_start()
            .gap_4()
            .child(
                div()
                    .w(relative(2. / 3.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(detail_card(
                        "Default system prompt",
                        Some("Used whenever this runner spawns. Override per crew/mission slot later (v0.x)."),
                        if let Some(prompt) = runner.system_prompt.clone() {
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(rems(12. / 16.))
                                .line_height(rems(19.5 / 16.))
                                .text_color(theme::text())
                                .child(prompt)
                                .into_any_element()
                        } else {
                            div()
                                .text_size(rems(14. / 16.))
                                .italic()
                                .text_color(theme::faint())
                                .child("No system prompt set.")
                                .into_any_element()
                        },
                    ))
                    .child(detail_card("Crews using this runner", None, crew_rows))
                    .child(detail_card(
                        "Chat now",
                        Some("Spawn a one-on-one PTY. Chats don't join any mission's coordination bus."),
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::muted())
                            .child("Working directory")
                            .child(
                                div()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme::border_strong())
                                    .bg(theme::bg())
                                    .p(rems(6. / 16.))
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::faint())
                                    .child(runner.working_dir.clone().unwrap_or_else(|| "—".into())),
                            )
                            .child(
                                div()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child("Inherits the runner's working directory. Click Edit to change it, or override per-chat from the chat itself."),
                            ),
                    )),
            )
            .child(
                div()
                    .w(relative(1. / 3.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(detail_card(
                        "Activity",
                        None,
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(big_stat("sessions", sessions, sessions > 0))
                                    .child(big_stat("missions", missions, missions > 0)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .border_t_1()
                                    .border_color(theme::border())
                                    .pt_3()
                                    .text_size(rems(12. / 16.))
                                    .child(detail_row("In crews", crew_count.to_string()))
                                    .child(detail_row("Last seen", last_seen)),
                            ),
                    ))
                    .child(detail_card(
                        "Details",
                        None,
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(detail_metadata_row(
                                "Handle",
                                format!("@{}", runner.handle),
                                true,
                                false,
                            ))
                            .child(detail_metadata_row(
                                "Runtime",
                                runner.runtime,
                                false,
                                false,
                            ))
                            .child(detail_metadata_row(
                                "Command",
                                runner.command,
                                true,
                                false,
                            ))
                            .children((!args.is_empty()).then(|| {
                                detail_metadata_row("Args", args, true, false)
                            }))
                            .child(detail_metadata_row(
                                "Created",
                                format_timestamp(runner.created_at),
                                false,
                                false,
                            ))
                            .child(detail_metadata_row("ID", runner.id, true, true)),
                    )),
            )
            .into_any_element()
    }

    fn open_runner_menu(
        &mut self,
        item: RunnerWithActivity,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            RunnerMenuAction::Open(item.runner.handle.clone()),
            RunnerMenuAction::Delete {
                id: item.runner.id,
                handle: item.runner.handle,
            },
        ];
        let items = vec![
            UiMenuItem::new("Edit details").icon("pencil.svg"),
            UiMenuItem::new("Delete runner")
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "runner-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_runner_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.runner_surfaces.context_menu = None;
                        cx.notify();
                    });
                }),
            )
            .width(px(176.))
        });
        let focus = menu.read(cx).focus_handle();
        self.runner_surfaces.context_menu = Some(menu);
        focus.focus(window);
        cx.notify();
    }

    fn handle_runner_menu_action(
        &mut self,
        action: RunnerMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            RunnerMenuAction::Open(handle) => self.open_runner_detail(handle, window, cx),
            RunnerMenuAction::Delete { id, handle } => {
                self.runner_surfaces.delete_confirm = Some(RunnerDeleteConfirm { id, handle });
                cx.notify();
            }
        }
    }

    fn start_runner_chat(&mut self, runner: Runner, window: &mut Window, cx: &mut Context<Self>) {
        if self.runner_surfaces.chat_pending.is_some() {
            return;
        }
        let detail_origin = matches!(self.route, AppRoute::RunnerDetail(_));
        self.runner_surfaces.chat_pending = Some(runner.id.clone());
        let cwd = if runner.working_dir.is_none() {
            let default = self.settings(cx).default_working_dir.trim();
            (!default.is_empty()).then(|| default.to_owned())
        } else {
            None
        };
        let core = self.core(cx).clone();
        let runner_id = runner.id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::session::session_start_direct(
                &core,
                runner_id,
                None,
                None,
                None,
                None,
                cwd,
                Some(INITIAL_COLS),
                Some(INITIAL_ROWS),
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.runner_surfaces.chat_pending = None;
                match result {
                    Ok(spawned) => {
                        let attach = (|| -> Result<()> {
                            this.refresh_sessions(cx);
                            this.reload_tabs(cx)?;
                            this.tabs.activate_session(&spawned.id);
                            this.sync_active_project_from_active_tab(cx);
                            this.set_route(AppRoute::Chat, cx);
                            this.ensure_active_tab_attached(window, cx)?;
                            Ok(())
                        })();
                        match attach {
                            Ok(()) => {
                                this.remember_active_runner(cx);
                                this.mark_active_tab_viewed(window, cx);
                                this.sync_active_chat_detail(cx);
                                this.begin_chat_transition(
                                    &spawned.id,
                                    chat_lifecycle::TransitionKind::Starting,
                                    Some(0),
                                    window,
                                    cx,
                                );
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                    }
                    Err(error)
                        if detail_origin && matches!(this.route, AppRoute::RunnerDetail(_)) =>
                    {
                        this.runner_surfaces.detail.error = Some(error);
                    }
                    Err(error) => this.show_toast(error, crate::toast::ToastTone::Error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[derive(Clone, Copy)]
enum RunnerFormKind {
    Create,
    Edit,
}

struct RunnerEditResolution {
    runtime: String,
    runtime_pinned: bool,
    command: String,
    model: String,
    effort: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeLayerResolution {
    runtime: String,
    runtime_pinned: bool,
    model: Option<String>,
    effort: Option<String>,
}

fn resolve_slot_runtime_layers(
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

fn resolve_runner_edit(
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
            .find(|runtime| runtime.name == layers.runtime)
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

fn ensure_runtime_present(core: &AppCore, runtimes: &mut Vec<RuntimeCatalogEntry>, name: &str) {
    if runtimes.iter().any(|runtime| runtime.name == name) {
        return;
    }
    if let Ok(catalog) = runner_backend::ops::runtime::runtime_catalog(core) {
        if let Some(runtime) = catalog.into_iter().find(|runtime| runtime.name == name) {
            runtimes.push(runtime);
        }
    }
}

fn runtime_entry<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> Option<&'a RuntimeCatalogEntry> {
    runtimes.iter().find(|runtime| runtime.name == name)
}

fn runtime_models<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default()
}

fn runtime_model_placeholder(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    edits_slot: bool,
) -> String {
    if edits_slot {
        return "default".into();
    }
    runtime_entry(runtimes, runtime)
        .and_then(|runtime| runtime.default_model.as_deref())
        .map(|model| format!("default ({model})"))
        .unwrap_or_else(|| "default".into())
}

fn runtime_default_effort_label(runtimes: &[RuntimeCatalogEntry], runtime: &str) -> String {
    runtime_entry(runtimes, runtime)
        .and_then(|runtime| runtime.default_effort.as_deref())
        .map(|effort| format!("Runtime default ({effort})"))
        .unwrap_or_else(|| "Runtime default".into())
}

fn runtime_efforts<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtime_entry(runtimes, name)
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default()
}

fn runner_edit_runtime_options(
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
                    || runtime.name == runner.runtime
                    || runtime.name == current_runtime
            })
            .map(|runtime| {
                SelectOption::new(runtime.name.clone(), runtime.display_name.clone())
                    .description(runtime.description.clone())
            }),
    );
    options
}

fn effort_options(
    runtimes: &[RuntimeCatalogEntry],
    runtime: &str,
    runner: &Runner,
    edits_slot: bool,
) -> Vec<SelectOption> {
    runtime_efforts(runtimes, runtime)
        .iter()
        .map(|option| {
            let label = if option.value.is_empty() {
                if edits_slot {
                    if runtime == runner.runtime {
                        format!(
                            "Runner default ({})",
                            runner.effort.as_deref().unwrap_or("default")
                        )
                    } else {
                        "Runtime default".into()
                    }
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

fn permission_modes(runtime: &str) -> &'static [PermissionMode] {
    match runtime {
        "claude-code" => &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        "codex" | "trae" => &[
            PermissionMode::Default,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ],
        "qoder" => &[PermissionMode::Default, PermissionMode::Auto],
        _ => &[],
    }
}

fn permission_options(runtime: &str) -> Vec<SelectOption> {
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

fn permission_mode_value(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept_edits",
        PermissionMode::Auto => "auto",
        PermissionMode::Bypass => "bypass",
    }
}

fn parse_permission_mode(value: &str) -> PermissionMode {
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

fn permission_mode_description(runtime: &str, mode: PermissionMode) -> &'static str {
    match (runtime, mode) {
        ("claude-code", PermissionMode::Default) => {
            "Ask for every tool, shell command, and write."
        }
        ("claude-code", PermissionMode::AcceptEdits) => "Auto-accept file edits and common filesystem commands; still ask for shell, network, and writes outside the workspace. Available on every plan.",
        ("claude-code", PermissionMode::Auto) => "Real auto with a server-side classifier. Requires Max / Team / Enterprise / API plan + a supported model (Opus 4.7 on Max). Not available on Pro.",
        ("claude-code", PermissionMode::Bypass) => "Skip every check. Triggers a one-time consent dialog the first time per user account.",
        ("codex", PermissionMode::Default) => {
            "Codex's built-in approval cadence (untrusted commands)."
        }
        ("codex", PermissionMode::Auto) => "Auto-run in the workspace and ask only when the model decides approval is needed (`--ask-for-approval on-request`).",
        ("codex", PermissionMode::Bypass) => "Never ask while keeping Codex's workspace-write sandbox (`--ask-for-approval never`).",
        ("qoder", PermissionMode::Default) => "Use Qoder's built-in permission mode.",
        ("qoder", PermissionMode::Auto) => {
            "Run with Qoder's verified `--permission-mode auto` mode."
        }
        ("trae", PermissionMode::Default) => "TRAE CLI's built-in approval cadence.",
        ("trae", PermissionMode::Auto) => {
            "Use TRAE CLI's native auto-reviewer (`--permission-mode auto`)."
        }
        ("trae", PermissionMode::Bypass) => {
            "Bypass TRAE CLI permission prompts (`--permission-mode bypass_permissions`)."
        }
        _ => "",
    }
}

fn validate_runner_handle(handle: &str) -> Option<&'static str> {
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

fn create_runner_can_submit(form: &CreateRunnerForm) -> bool {
    !form.submitting
        && !form.handle_empty
        && form.handle_error.is_none()
        && form.display_name_valid
        && runtime_entry(&form.runtimes, &form.runtime).is_some()
}

fn create_runner_focus_order(
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

fn runner_edit_focus_order(form: &RunnerEditForm, cx: &Context<NativeRoot>) -> Vec<FocusHandle> {
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

fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn split_args(value: &str) -> Vec<String> {
    value.split_whitespace().map(ToOwned::to_owned).collect()
}

fn create_runner_form_is_composing(form: &CreateRunnerForm, cx: &Context<NativeRoot>) -> bool {
    form.handle.read(cx).is_composing()
        || form.display_name.read(cx).is_composing()
        || form.args.read(cx).is_composing()
        || form.model.read(cx).is_composing()
        || form.working_dir.read(cx).is_composing()
        || form.system_prompt.read(cx).is_composing()
}

fn runner_edit_form_is_composing(form: &RunnerEditForm, cx: &Context<NativeRoot>) -> bool {
    form.display_name.read(cx).is_composing()
        || form.args.read(cx).is_composing()
        || form.model.read(cx).is_composing()
        || form.working_dir.read(cx).is_composing()
        || form.system_prompt.read(cx).is_composing()
}

fn plural(count: i64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn format_timestamp(timestamp: runner_backend::model::Timestamp) -> String {
    timestamp
        .with_timezone(&chrono::Local)
        .format("%-m/%-d/%Y, %-I:%M:%S %p")
        .to_string()
}

fn error_banner(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(rems(14. / 16.))
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
}

fn detail_card(
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
                        .text_size(rems(14. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(title),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .text_size(rems(11. / 16.))
                        .text_color(theme::faint())
                        .child(subtitle)
                })),
        )
        .child(child)
        .into_any_element()
}

fn big_stat(label: &'static str, value: i64, accent: bool) -> AnyElement {
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
                .text_size(rems(30. / 16.))
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
                .text_size(rems(10. / 16.))
                .text_color(theme::faint())
                .child(label.to_uppercase()),
        )
        .into_any_element()
}

fn detail_row(label: &'static str, value: String) -> AnyElement {
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

fn detail_metadata_row(
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
        .text_size(rems(12. / 16.))
        .child(div().flex_none().text_color(theme::faint()).child(label))
        .child(
            div()
                .min_w(px(0.))
                .text_right()
                .when(monospace, |value| value.font_family("JetBrains Mono"))
                .when(subtle, |value| {
                    value.text_size(rems(10. / 16.)).text_color(theme::faint())
                })
                .when(!subtle, |value| value.text_color(theme::text()))
                .child(value),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with_defaults(
        default_model: Option<&str>,
        default_effort: Option<&str>,
    ) -> RuntimeCatalogEntry {
        RuntimeCatalogEntry {
            name: "codex".into(),
            display_name: "Codex".into(),
            command: "codex".into(),
            description: "OpenAI Codex CLI".into(),
            default_enabled: true,
            available: true,
            default_model: default_model.map(str::to_owned),
            default_effort: default_effort.map(str::to_owned),
            models: Vec::new(),
            efforts: Vec::new(),
        }
    }

    #[test]
    fn runner_handle_validation_matches_the_shipped_contract() {
        for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
            assert_eq!(validate_runner_handle(valid), None, "{valid}");
        }
        for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
            assert!(validate_runner_handle(invalid).is_some(), "{invalid}");
        }
    }

    #[test]
    fn runtime_default_labels_include_known_values() {
        let runtimes = [runtime_with_defaults(Some("gpt-5.6-sol"), Some("xhigh"))];
        assert_eq!(
            runtime_model_placeholder(&runtimes, "codex", false),
            "default (gpt-5.6-sol)"
        );
        assert_eq!(
            runtime_model_placeholder(&runtimes, "codex", true),
            "default"
        );
        assert_eq!(
            runtime_default_effort_label(&runtimes, "codex"),
            "Runtime default (xhigh)"
        );

        let runtimes = [runtime_with_defaults(None, None)];
        assert_eq!(
            runtime_model_placeholder(&runtimes, "codex", false),
            "default"
        );
        assert_eq!(
            runtime_default_effort_label(&runtimes, "codex"),
            "Runtime default"
        );
    }

    #[test]
    fn slot_runtime_layers_leave_blank_overrides_to_inherit_runner_defaults() {
        assert_eq!(
            resolve_slot_runtime_layers("codex", None, None, None),
            RuntimeLayerResolution {
                runtime: "codex".into(),
                runtime_pinned: false,
                model: None,
                effort: None,
            }
        );
    }

    #[test]
    fn same_runtime_pin_keeps_blank_model_and_effort_overrides() {
        assert_eq!(
            resolve_slot_runtime_layers("codex", Some("codex"), None, None),
            RuntimeLayerResolution {
                runtime: "codex".into(),
                runtime_pinned: true,
                model: None,
                effort: None,
            }
        );
    }

    #[test]
    fn different_runtime_uses_runtime_defaults_unless_overridden() {
        assert_eq!(
            resolve_slot_runtime_layers("codex", Some("claude-code"), None, None),
            RuntimeLayerResolution {
                runtime: "claude-code".into(),
                runtime_pinned: true,
                model: None,
                effort: None,
            }
        );
        assert_eq!(
            resolve_slot_runtime_layers("codex", Some("claude-code"), Some("opus"), Some("max"),),
            RuntimeLayerResolution {
                runtime: "claude-code".into(),
                runtime_pinned: true,
                model: Some("opus".into()),
                effort: Some("max".into()),
            }
        );
    }
}

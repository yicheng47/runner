use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, FontWeight, KeyDownEvent, PathPromptOptions, ScrollHandle,
    SharedString, Window,
};
use runner_backend::model::Runner;
use runner_backend::ops::runtime::{
    filter_selectable_runtime_catalog, RuntimeCatalogEntry, RuntimeCatalogOption,
};

use runner_app::ui::{
    effective_working_dir, working_dir_placeholder, working_dir_text_field, Button, ButtonVariant,
    Field, IconButton, Modal, ModelField, OverlayWidth, Scrollbar, SelectHandler, SelectOption,
    StyledSelect, TextField, WorkingDirField,
};

use super::*;
use crate::*;

const START_CHAT_MODE_FILE: &str = "start-chat-mode";
const MODAL_WIDTH: f32 = 560.;
const FIELD_WIDTH: f32 = 476.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChatMode {
    Runner,
    Runtime,
}

impl ChatMode {
    fn from_persisted(value: Option<&str>) -> Self {
        if value.is_some_and(|value| value.trim() == "runtime") {
            Self::Runtime
        } else {
            Self::Runner
        }
    }

    fn persisted(self) -> &'static str {
        match self {
            Self::Runner => "runner",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Clone)]
enum ChatTarget {
    NewTab,
    Pane { tab_id: String, pane_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartChatSelection {
    Runner,
    RunnerRuntime,
    Runtime,
    Effort,
}

pub(crate) struct StartChatModal {
    target: ChatTarget,
    project_id: Option<String>,
    mode: ChatMode,
    runners: Vec<Runner>,
    runtimes: Vec<RuntimeCatalogEntry>,
    runner_id: Option<String>,
    runtime_name: Option<String>,
    runner_runtime_override: Option<String>,
    effort: String,
    title: Entity<TextField>,
    cwd: Entity<TextField>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    runner_select: Entity<StyledSelect>,
    runner_runtime_select: Entity<StyledSelect>,
    runtime_select: Entity<StyledSelect>,
    effort_select: Entity<StyledSelect>,
    scroll_handle: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    runner_mode_focus: FocusHandle,
    direct_mode_focus: FocusHandle,
    browse_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    agents_checking: bool,
    agents_error: Option<String>,
    submitting: bool,
    error: Option<String>,
}

impl StartChatModal {
    fn selected_runner(&self) -> Option<&Runner> {
        self.runner_id
            .as_deref()
            .and_then(|runner_id| self.runners.iter().find(|runner| runner.id == runner_id))
    }

    fn selected_runtime(&self) -> Option<&RuntimeCatalogEntry> {
        self.runtime_name
            .as_deref()
            .and_then(|name| self.runtimes.iter().find(|runtime| runtime.name == name))
    }

    fn override_runtime(&self) -> Option<&RuntimeCatalogEntry> {
        self.runner_runtime_override
            .as_deref()
            .and_then(|name| self.runtimes.iter().find(|runtime| runtime.name == name))
    }

    fn active_runtime(&self) -> Option<&RuntimeCatalogEntry> {
        match self.mode {
            ChatMode::Runner => self.override_runtime(),
            ChatMode::Runtime => self.selected_runtime(),
        }
    }

    fn can_submit(&self) -> bool {
        !self.submitting
            && match self.mode {
                ChatMode::Runner => self.selected_runner().is_some(),
                ChatMode::Runtime => self.selected_runtime().is_some(),
            }
    }

    fn is_composing(&self, cx: &Context<NativeRoot>) -> bool {
        self.title.read(cx).is_composing()
            || self.cwd.read(cx).is_composing()
            || self.model.read(cx).is_composing()
    }
}

#[derive(Debug, Eq, PartialEq)]
enum StartRequest {
    Runner {
        runner_id: String,
        runtime: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        cwd: Option<String>,
    },
    Runtime {
        runtime: String,
        model: Option<String>,
        effort: Option<String>,
        cwd: Option<String>,
    },
}

impl NativeRoot {
    pub(crate) fn open_new_tab_modal(
        &mut self,
        _: &NewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route == AppRoute::Settings || self.start_chat_modal.is_some() {
            return;
        }
        let active_project_id = self.active_project_id(cx);
        let project = active_project_id
            .as_deref()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(ChatTarget::NewTab, None, project, window, cx);
    }

    pub(crate) fn open_sidebar_chat_modal(
        &mut self,
        project_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.start_chat_modal.is_some() {
            return;
        }
        let project = project_id
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(ChatTarget::NewTab, None, project, window, cx);
    }

    pub(crate) fn open_pane_chat_modal(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.tabs.active_tab_id().map(str::to_owned) else {
            return;
        };
        let active_project_id = self.active_project_id(cx);
        let project = active_project_id
            .as_deref()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        self.open_start_chat_modal(
            ChatTarget::Pane {
                tab_id,
                pane_id: pane_id.to_owned(),
            },
            self.last_focused_runner_id.clone(),
            project,
            window,
            cx,
        );
    }

    fn open_start_chat_modal(
        &mut self,
        target: ChatTarget,
        default_runner_id: Option<String>,
        project: Option<runner_backend::repo::project::ProjectRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut error = None;
        match runner_backend::ops::runner::runner_list(self.core(cx)) {
            Ok(runners) => self.app_store.update(cx, |store, store_cx| {
                store.replace_runners(runners, store_cx)
            }),
            Err(load_error) => error = Some(load_error.to_string()),
        }

        let (runtimes, agents_checking, agents_error) =
            load_selectable_runtimes(self.core(cx), self.settings(cx));
        let persisted_mode = read_start_chat_mode(&self.core(cx).app_data_dir);
        let mode = if default_runner_id.is_some() {
            ChatMode::Runner
        } else {
            persisted_mode
        };
        let runner_id = default_runner_id
            .filter(|runner_id| {
                self.app_store
                    .read(cx)
                    .runners
                    .iter()
                    .any(|runner| runner.id == *runner_id)
            })
            .or_else(|| {
                self.app_store
                    .read(cx)
                    .runners
                    .first()
                    .map(|runner| runner.id.clone())
            });
        let runtime_name = runtimes
            .iter()
            .find(|runtime| runtime.name == self.settings(cx).default_runtime)
            .or_else(|| runtimes.first())
            .map(|runtime| runtime.name.clone());
        let title = match mode {
            ChatMode::Runner => runner_id
                .as_deref()
                .and_then(|runner_id| {
                    self.app_store
                        .read(cx)
                        .runners
                        .iter()
                        .find(|runner| runner.id == runner_id)
                })
                .map(|runner| default_title_for_runner(&runner.handle))
                .unwrap_or_default(),
            ChatMode::Runtime => runtime_name
                .as_deref()
                .and_then(|name| runtimes.iter().find(|runtime| runtime.name == name))
                .map(|runtime| default_title_for_runtime(&runtime.display_name))
                .unwrap_or_default(),
        };
        let cwd_placeholder = cwd_placeholder(
            mode,
            runner_id.as_deref().and_then(|runner_id| {
                self.app_store
                    .read(cx)
                    .runners
                    .iter()
                    .find(|runner| runner.id == runner_id)
            }),
            &self.settings(cx).default_working_dir,
        );
        let title_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), title, "e.g. quick-debug", false).text_size(13.)
        });
        let (project_id, project_cwd) = project_start_scope(project.as_ref());
        let cwd_input = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), project_cwd, cwd_placeholder)
                .text_size(12.)
        });
        let model_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", false).placeholder_as_value(true)
        });
        let root = cx.entity();
        let runner_handler = selection_handler(&root, StartChatSelection::Runner);
        let runners = self.app_store.read(cx).runners.clone();
        let runner_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-runner",
                select_cx.focus_handle(),
                runner_id.clone().unwrap_or_default(),
                runner_options(&runners),
                runner_handler,
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
            .detailed(true)
            .monospace(true)
            .placeholder("No runners yet")
            .disabled(runners.is_empty())
        });
        let runner_runtime_options = runner_runtime_options(
            &runtimes,
            runner_id.as_deref().and_then(|id| {
                self.app_store
                    .read(cx)
                    .runners
                    .iter()
                    .find(|runner| runner.id == id)
            }),
        );
        let runner_runtime_handler = selection_handler(&root, StartChatSelection::RunnerRuntime);
        let runner_runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-runner-runtime",
                select_cx.focus_handle(),
                "",
                runner_runtime_options,
                runner_runtime_handler,
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let runtime_handler = selection_handler(&root, StartChatSelection::Runtime);
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-runtime",
                select_cx.focus_handle(),
                runtime_name.clone().unwrap_or_default(),
                runtime_options(&runtimes),
                runtime_handler,
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
            .disabled(runtimes.is_empty())
        });
        let effort_handler = selection_handler(&root, StartChatSelection::Effort);
        let effort_select = cx.new(|select_cx| {
            StyledSelect::new(
                "start-chat-effort",
                select_cx.focus_handle(),
                "",
                Vec::new(),
                effort_handler,
                select_cx,
            )
            .width(px(232.))
            .min_menu_width(px(240.))
        });
        let model_field = cx.new(|model_cx| ModelField::new(model_input.clone(), &[], model_cx));
        let scroll_handle = ScrollHandle::new();
        let scroll_owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll_handle.clone(), scroll_owner));
        let runner_mode_focus = cx.focus_handle();
        let direct_mode_focus = cx.focus_handle();
        let browse_focus = cx.focus_handle();
        let close_focus = cx.focus_handle();
        let cancel_focus = cx.focus_handle();
        let submit_focus = cx.focus_handle();
        let title_focus = title_input.read(cx).focus_handle();

        self.layout_picker_open = false;
        self.sidebar_preview_open = false;
        self.start_chat_modal = Some(StartChatModal {
            target,
            project_id,
            mode,
            runners: self.app_store.read(cx).runners.clone(),
            runtimes,
            runner_id,
            runtime_name,
            runner_runtime_override: None,
            effort: String::new(),
            title: title_input,
            cwd: cwd_input,
            model: model_input,
            model_field,
            runner_select,
            runner_runtime_select,
            runtime_select,
            effort_select,
            scroll_handle,
            scrollbar,
            runner_mode_focus,
            direct_mode_focus,
            browse_focus,
            close_focus,
            cancel_focus,
            submit_focus,
            agents_checking,
            agents_error,
            submitting: false,
            error: error.take(),
        });
        if let Some(modal) = self.start_chat_modal.as_mut() {
            sync_runtime_controls(modal, cx);
        }
        title_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn refresh_start_chat_runtimes(&mut self, cx: &mut Context<Self>) {
        let (runtimes, agents_checking, agents_error) =
            load_selectable_runtimes(self.core(cx), self.settings(cx));
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        let catalog_loaded = agents_error.is_none();
        let previous_runtime = modal.runtime_name.clone();
        let previous_override = modal.runner_runtime_override.clone();
        modal.agents_checking = agents_checking;
        modal.agents_error = agents_error;

        if catalog_loaded {
            modal.runtimes = runtimes;
            if modal
                .runner_runtime_override
                .as_ref()
                .is_some_and(|name| !modal.runtimes.iter().any(|runtime| runtime.name == *name))
            {
                modal.runner_runtime_override = None;
            }
            if modal
                .runtime_name
                .as_ref()
                .is_none_or(|name| !modal.runtimes.iter().any(|runtime| runtime.name == *name))
            {
                modal.runtime_name = modal.runtimes.first().map(|runtime| runtime.name.clone());
            }
            if modal.runtime_name != previous_runtime
                || modal.runner_runtime_override != previous_override
            {
                modal.effort.clear();
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
            }
            if modal.mode == ChatMode::Runtime && modal.runtime_name != previous_runtime {
                let derived = modal
                    .selected_runtime()
                    .map(|runtime| default_title_for_runtime(&runtime.display_name))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
            }
            modal.runtime_select.update(cx, |select, select_cx| {
                select.set_options(runtime_options(&modal.runtimes), select_cx);
                select.set_value(modal.runtime_name.clone().unwrap_or_default(), select_cx);
            });
            modal.runner_runtime_select.update(cx, |select, select_cx| {
                select.set_options(
                    runner_runtime_options(&modal.runtimes, modal.selected_runner()),
                    select_cx,
                );
                select.set_value(
                    modal.runner_runtime_override.clone().unwrap_or_default(),
                    select_cx,
                );
            });
            sync_runtime_controls(modal, cx);
        }
        cx.notify();
    }

    pub(crate) fn remember_active_runner(&mut self, cx: &App) {
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        self.last_focused_runner_id = self
            .session_entry(&session_id, cx)
            .and_then(|entry| entry.runner_id.clone());
    }

    fn close_start_chat_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .start_chat_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.start_chat_modal = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    fn set_start_chat_mode(&mut self, mode: ChatMode, cx: &mut Context<Self>) {
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let app_data_dir = self.core(cx).app_data_dir.clone();
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.mode == mode || modal.submitting {
            return;
        }
        modal.mode = mode;
        modal.runner_runtime_override = None;
        modal.effort.clear();
        modal
            .model
            .update(cx, |input, input_cx| input.reset("", input_cx));
        sync_runtime_controls(modal, cx);
        let derived = match mode {
            ChatMode::Runner => modal
                .selected_runner()
                .map(|runner| default_title_for_runner(&runner.handle))
                .unwrap_or_default(),
            ChatMode::Runtime => modal
                .selected_runtime()
                .map(|runtime| default_title_for_runtime(&runtime.display_name))
                .unwrap_or_default(),
        };
        update_auto_title(&modal.title, derived, cx);
        let placeholder = cwd_placeholder(mode, modal.selected_runner(), &default_working_dir);
        modal.cwd.update(cx, |input, input_cx| {
            input.set_placeholder(placeholder, input_cx)
        });
        let _ = write_start_chat_mode(&app_data_dir, mode);
        cx.notify();
    }

    fn select_start_chat_choice(
        &mut self,
        selection: StartChatSelection,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let default_working_dir = self.settings(cx).default_working_dir.clone();
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        match selection {
            StartChatSelection::Runner => {
                modal.runner_id = Some(value.to_owned());
                let derived = modal
                    .selected_runner()
                    .map(|runner| default_title_for_runner(&runner.handle))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
                let placeholder =
                    cwd_placeholder(modal.mode, modal.selected_runner(), &default_working_dir);
                modal.cwd.update(cx, |input, input_cx| {
                    input.set_placeholder(placeholder, input_cx)
                });
                let options = runner_runtime_options(&modal.runtimes, modal.selected_runner());
                modal.runner_runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx)
                });
            }
            StartChatSelection::RunnerRuntime => {
                modal.runner_runtime_override = (!value.is_empty()).then(|| value.to_owned());
                modal.effort.clear();
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
                sync_runtime_controls(modal, cx);
            }
            StartChatSelection::Runtime => {
                modal.runtime_name = Some(value.to_owned());
                modal.effort.clear();
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
                let derived = modal
                    .selected_runtime()
                    .map(|runtime| default_title_for_runtime(&runtime.display_name))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
                sync_runtime_controls(modal, cx);
            }
            StartChatSelection::Effort => modal.effort = value.to_owned(),
        }
        cx.notify();
    }

    fn browse_start_chat_cwd(&mut self, cx: &mut Context<Self>) {
        if self
            .start_chat_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        let Some(cwd_input) = self
            .start_chat_modal
            .as_ref()
            .map(|modal| modal.cwd.clone())
        else {
            return;
        };
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
                let Some(modal) = this.start_chat_modal.as_mut() else {
                    return;
                };
                if modal.cwd != cwd_input {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            modal.cwd.update(cx, |input, input_cx| {
                                input.reset(path.to_string_lossy().into_owned(), input_cx)
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_start_chat_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.close_start_chat_modal(window, cx);
            }
            "enter"
                if self
                    .start_chat_modal
                    .as_ref()
                    .is_some_and(|modal| !modal.is_composing(cx)) =>
            {
                cx.stop_propagation();
                self.submit_start_chat(window, cx);
            }
            _ => {}
        }
    }

    fn submit_start_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_ref() else {
            return;
        };
        if !modal.can_submit() || modal.is_composing(cx) {
            return;
        }
        let target = modal.target.clone();
        let cwd = effective_working_dir(
            modal.cwd.read(cx).text(),
            modal.mode == ChatMode::Runner
                && modal
                    .selected_runner()
                    .and_then(|runner| runner.working_dir.as_deref())
                    .is_some_and(|path| !path.trim().is_empty()),
            &self.settings(cx).default_working_dir,
        );
        let model = normalized_value(modal.model.read(cx).text());
        let effort = normalized_value(&modal.effort);
        let title = modal.title.read(cx).text().trim().to_owned();
        let project_id = modal.project_id.clone();
        let request = build_start_request(
            modal.mode,
            modal.selected_runner().map(|runner| runner.id.as_str()),
            modal
                .selected_runtime()
                .map(|runtime| runtime.name.as_str()),
            modal.runner_runtime_override.as_deref(),
            model,
            effort,
            cwd,
        )
        .expect("validated start chat selection");
        let initial_size = match &target {
            ChatTarget::NewTab => (INITIAL_COLS, INITIAL_ROWS),
            ChatTarget::Pane { tab_id, pane_id }
                if self.tabs.active_tab_id() == Some(tab_id.as_str()) =>
            {
                self.tabs
                    .active()
                    .map(|layout| self.estimated_terminal_size(layout, pane_id, window, cx))
                    .unwrap_or((INITIAL_COLS, INITIAL_ROWS))
            }
            ChatTarget::Pane { .. } => {
                if let Some(modal) = self.start_chat_modal.as_mut() {
                    modal.error = Some("The target tab is no longer active".into());
                }
                cx.notify();
                return;
            }
        };
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.submitting = true;
            modal.error = None;
            set_start_chat_controls_disabled(modal, true, cx);
        }
        cx.notify();

        let mut spawned_id = None;
        let mut rename_error = None;
        let result = (|| -> Result<String> {
            let spawned = match request {
                StartRequest::Runner {
                    runner_id,
                    runtime,
                    model,
                    effort,
                    cwd,
                } => runner_backend::ops::session::session_start_direct(
                    self.core(cx),
                    runner_id,
                    runtime,
                    model,
                    effort,
                    project_id,
                    cwd,
                    Some(initial_size.0),
                    Some(initial_size.1),
                )?,
                StartRequest::Runtime {
                    runtime,
                    model,
                    effort,
                    cwd,
                } => runner_backend::ops::session::session_start_runtime(
                    self.core(cx),
                    &runtime,
                    project_id,
                    cwd,
                    Some(initial_size.0),
                    Some(initial_size.1),
                    model,
                    effort,
                )?,
            };
            spawned_id = Some(spawned.id.clone());
            if !title.is_empty() {
                if let Err(error) = runner_backend::ops::session::session_rename(
                    self.core(cx),
                    &spawned.id,
                    Some(title),
                ) {
                    rename_error = Some(format!(
                        "Chat started, but its title could not be saved: {error}"
                    ));
                }
            }
            self.refresh_sessions(cx);
            match target {
                ChatTarget::NewTab => {
                    self.reload_tabs(cx)?;
                    self.tabs.activate_session(&spawned.id);
                    self.sync_active_project_from_active_tab(cx);
                }
                ChatTarget::Pane { pane_id, .. } => {
                    self.tabs.assign_to_active(&pane_id, &spawned.id)?;
                    self.persist_active_tab(cx)?;
                    self.reload_tabs(cx)?;
                    self.tabs.activate_session(&spawned.id);
                    self.sync_active_project_from_active_tab(cx);
                }
            }
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.start_chat_modal = None;
                self.set_route(AppRoute::Chat, cx);
                self.error = rename_error;
                self.remember_active_runner(cx);
                self.mark_active_tab_viewed(window, cx);
                self.sync_active_chat_detail(cx);
                self.begin_chat_transition(
                    &session_id,
                    chat_lifecycle::TransitionKind::Starting,
                    Some(0),
                    window,
                    cx,
                );
            }
            Err(start_error) => {
                if let Some(session_id) = spawned_id {
                    self.start_chat_modal = None;
                    let _ = self.reload_tabs(cx);
                    self.tabs.activate_session(&session_id);
                    self.sync_active_project_from_active_tab(cx);
                    let _ = self.ensure_active_tab_attached(window, cx);
                    self.remember_active_runner(cx);
                    self.mark_active_tab_viewed(window, cx);
                    self.sync_active_chat_detail(cx);
                    self.begin_chat_transition(
                        &session_id,
                        chat_lifecycle::TransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                    self.error = Some(start_error.to_string());
                } else if let Some(modal) = self.start_chat_modal.as_mut() {
                    modal.submitting = false;
                    modal.error = Some(start_error.to_string());
                    set_start_chat_controls_disabled(modal, false, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_start_chat_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.start_chat_modal.as_ref().expect("modal is open");
        let active_runtime = modal.active_runtime().cloned();
        let mode = modal.mode;
        let submitting = modal.submitting;
        let can_submit = modal.can_submit();
        let settings_root = cx.entity();

        let runner_fields = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                Field::new(
                    "start-chat-runner-field",
                    "Runner",
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(modal.runner_select.clone())
                        .when(modal.runners.is_empty(), |field| {
                            field.child(
                                div()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::warning())
                                    .child("No runners yet. Create one from the runner page first."),
                            )
                        }),
                )
                .focus_target(modal.runner_select.read(cx).focus_handle())
                .emphasized(true),
            )
            .child(
                Field::new(
                    "start-chat-runner-agent-field",
                    "Agent",
                    modal.runner_runtime_select.clone(),
                )
                    .focus_target(modal.runner_runtime_select.read(cx).focus_handle())
                    .emphasized(true)
                    .subtitle("Overriding runs this persona on another agent; its model and effort become configurable below."),
            )
            .when_some(modal.override_runtime().cloned(), |fields, runtime| {
                fields.child(render_shared_model_effort_fields(
                    &runtime,
                    modal.model_field.clone(),
                    modal.effort_select.clone(),
                    cx,
                ))
            });

        let direct_fields = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                Field::new(
                    "start-chat-direct-agent-field",
                    "Agent",
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(modal.runtime_select.clone())
                        .when(modal.runtimes.is_empty(), |field| {
                            field.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::warning())
                                    .when(modal.agents_checking, |message| {
                                        message.child("Detecting agents…")
                                    })
                                    .when(!modal.agents_checking, |message| {
                                        let root = settings_root.clone();
                                        message.child("No enabled agents detected. ").child(
                                            div()
                                                .id("start-chat-open-agent-settings")
                                                .cursor_pointer()
                                                .text_color(theme::warning())
                                                .hover(|link| link.text_color(theme::text()))
                                                .child("Configure one in Settings → Agents.")
                                                .on_click(move |_, window, cx| {
                                                    root.update(cx, |this, cx| {
                                                        this.start_chat_modal = None;
                                                        this.enter_settings_route(
                                                            Some("agents"),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        )
                                    }),
                            )
                        })
                        .children(modal.agents_error.clone().map(|error| {
                            div()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::danger())
                                .child(error)
                        })),
                )
                .focus_target(modal.runtime_select.read(cx).focus_handle())
                .emphasized(true),
            )
            .when_some(active_runtime, |fields, runtime| {
                fields.child(render_shared_model_effort_fields(
                    &runtime,
                    modal.model_field.clone(),
                    modal.effort_select.clone(),
                    cx,
                ))
            });

        let root = cx.entity();
        let browse_root = root.clone();
        let content = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_start_chat_key_down))
            .children(modal.error.as_ref().map(|error| {
                div()
                    .rounded(rems(4. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(SharedString::from(error.clone()))
            }))
            .child(
                div()
                    .flex()
                    .w_full()
                    .p(rems(2. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .child(self.render_mode_button(
                        "Runner",
                        ChatMode::Runner,
                        mode,
                        submitting,
                        modal.runner_mode_focus.clone(),
                        cx,
                    ))
                    .child(self.render_mode_button(
                        "Direct",
                        ChatMode::Runtime,
                        mode,
                        submitting,
                        modal.direct_mode_focus.clone(),
                        cx,
                    )),
            )
            .child(match mode {
                ChatMode::Runner => runner_fields.into_any_element(),
                ChatMode::Runtime => direct_fields.into_any_element(),
            })
            .child(
                Field::new("start-chat-title-field", "Chat name", modal.title.clone())
                    .focus_target(modal.title.read(cx).focus_handle())
                    .emphasized(true)
                    .subtitle("Optional. Leave blank to use the default label."),
            )
            .child(
                Field::new(
                    "start-chat-working-dir-field",
                    "Working directory",
                    WorkingDirField::new(
                        modal.cwd.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| this.browse_start_chat_cwd(cx));
                        }),
                    )
                    .browse_focus(modal.browse_focus.clone()),
                )
                .focus_target(modal.cwd.read(cx).focus_handle())
                .emphasized(true)
                .subtitle("Leave blank to use the default working directory."),
            );

        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root;
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
                            .text_color(theme::text())
                            .child("Start a chat"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Spawns a direct PTY in the selected directory."),
                    ),
            )
            .child(
                IconButton::new("close-start-chat", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close start chat")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
                    }),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-start-chat", "Cancel")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-start-chat",
                    if submitting {
                        "Starting…"
                    } else {
                        "Start chat"
                    },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_start_chat(window, cx));
                }),
            );
        let modal_close_root = cx.entity();
        Modal::new(
            title,
            content,
            Rc::new(move |window, cx| {
                modal_close_root.update(cx, |this, cx| this.close_start_chat_modal(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(MODAL_WIDTH))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            start_chat_focus_order(modal, cx)
        })
        .scrollbar(modal.scroll_handle.clone(), modal.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }

    fn render_mode_button(
        &self,
        label: &'static str,
        mode: ChatMode,
        active: ChatMode,
        disabled: bool,
        focus_handle: FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let key_root = root.clone();
        let click_focus = focus_handle.clone();
        let mut button = div()
            .id(match mode {
                ChatMode::Runner => "start-chat-mode-runner",
                ChatMode::Runtime => "start-chat-mode-runtime",
            })
            .track_focus(&focus_handle)
            .tab_index(0)
            .tab_stop(!disabled)
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .h(rems(30. / 16.))
            .rounded_md()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                click_focus.focus(window);
            })
            .text_color(if active == mode {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active == mode, |button| button.bg(theme::border()))
            .opacity(if disabled { 0.6 } else { 1. })
            .focus_visible(|button| button.text_color(theme::text()));
        if !disabled {
            button = button
                .cursor_pointer()
                .hover(|button| button.text_color(theme::text()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_start_chat_mode(mode, cx);
                }))
                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        key_root.update(cx, |this, cx| this.set_start_chat_mode(mode, cx));
                    }
                });
        }
        button.child(label).into_any_element()
    }
}

fn selection_handler(root: &Entity<NativeRoot>, selection: StartChatSelection) -> SelectHandler {
    let root = root.clone();
    Rc::new(move |value, _, cx| {
        root.update(cx, |this, cx| {
            this.select_start_chat_choice(selection, &value, cx)
        });
    })
}

fn runner_options(runners: &[Runner]) -> Vec<SelectOption> {
    runners
        .iter()
        .map(|runner| {
            SelectOption::new(runner.id.clone(), format!("@{}", runner.handle))
                .description(summarize_runner(runner))
        })
        .collect()
}

fn runtime_options(runtimes: &[RuntimeCatalogEntry]) -> Vec<SelectOption> {
    runtimes
        .iter()
        .map(|runtime| SelectOption::new(runtime.name.clone(), runtime.display_name.clone()))
        .collect()
}

fn runner_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    runner: Option<&Runner>,
) -> Vec<SelectOption> {
    let suffix = runner
        .map(|runner| format!(" ({})", runtime_display_name(runtimes, &runner.runtime)))
        .unwrap_or_default();
    std::iter::once(SelectOption::new("", format!("Runner default{suffix}")))
        .chain(runtime_options(runtimes))
        .collect()
}

fn option_select_options(options: &[RuntimeCatalogOption]) -> Vec<SelectOption> {
    options
        .iter()
        .map(|option| {
            let mut select = SelectOption::new(option.value.clone(), option.label.clone());
            if let Some(description) = &option.description {
                select = select.description(description.clone());
            }
            select
        })
        .collect()
}

fn sync_runtime_controls(modal: &mut StartChatModal, cx: &mut Context<NativeRoot>) {
    let runtime = modal.active_runtime().cloned();
    let models = runtime
        .as_ref()
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default();
    let efforts = runtime
        .as_ref()
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default();
    modal.model_field.update(cx, |field, field_cx| {
        field.set_suggestions(models, field_cx);
        field.set_disabled(modal.submitting || runtime.is_none(), field_cx);
    });
    modal.effort_select.update(cx, |select, select_cx| {
        select.set_options(option_select_options(efforts), select_cx);
        select.set_value(modal.effort.clone(), select_cx);
        select.set_disabled(modal.submitting || efforts.is_empty(), select_cx);
    });
}

fn set_start_chat_controls_disabled(
    modal: &mut StartChatModal,
    disabled: bool,
    cx: &mut Context<NativeRoot>,
) {
    modal
        .title
        .update(cx, |field, field_cx| field.set_disabled(disabled, field_cx));
    modal
        .cwd
        .update(cx, |field, field_cx| field.set_disabled(disabled, field_cx));
    modal.runner_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled || modal.runners.is_empty(), select_cx)
    });
    modal.runner_runtime_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled, select_cx)
    });
    modal.runtime_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled || modal.runtimes.is_empty(), select_cx)
    });
    sync_runtime_controls(modal, cx);
}

fn start_chat_focus_order(modal: &StartChatModal, cx: &Context<NativeRoot>) -> Vec<FocusHandle> {
    let mut order = vec![
        modal.close_focus.clone(),
        modal.runner_mode_focus.clone(),
        modal.direct_mode_focus.clone(),
    ];
    match modal.mode {
        ChatMode::Runner => {
            if !modal.runners.is_empty() {
                order.push(modal.runner_select.read(cx).focus_handle());
            }
            order.push(modal.runner_runtime_select.read(cx).focus_handle());
        }
        ChatMode::Runtime => {
            if !modal.runtimes.is_empty() {
                order.push(modal.runtime_select.read(cx).focus_handle());
            }
        }
    }
    if let Some(runtime) = modal.active_runtime() {
        order.push(modal.model.read(cx).focus_handle());
        if !runtime.efforts.is_empty() {
            order.push(modal.effort_select.read(cx).focus_handle());
        }
    }
    order.push(modal.title.read(cx).focus_handle());
    order.push(modal.cwd.read(cx).focus_handle());
    order.push(modal.browse_focus.clone());
    order.push(modal.cancel_focus.clone());
    if modal.can_submit() {
        order.push(modal.submit_focus.clone());
    }
    order
}

fn render_shared_model_effort_fields(
    runtime: &RuntimeCatalogEntry,
    model: Entity<ModelField>,
    effort: Entity<StyledSelect>,
    cx: &Context<NativeRoot>,
) -> AnyElement {
    let has_effort = !runtime.efforts.is_empty();
    let model_input = model.read(cx).input();
    let model_focus = model_input.read(cx).focus_handle();
    let effort_focus = effort.read(cx).focus_handle();
    div()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(rems(if has_effort {
                    232. / 16.
                } else {
                    FIELD_WIDTH / 16.
                }))
                .child(
                    Field::new("start-chat-model-field", "Model", model)
                        .focus_target(model_focus)
                        .emphasized(true),
                ),
        )
        .when(has_effort, |fields| {
            fields.child(
                div().w(rems(232. / 16.)).child(
                    Field::new("start-chat-effort-field", "Thinking effort", effort)
                        .focus_target(effort_focus)
                        .emphasized(true),
                ),
            )
        })
        .into_any_element()
}

pub(crate) fn load_selectable_runtimes(
    core: &AppCore,
    settings: &AppSettings,
) -> (Vec<RuntimeCatalogEntry>, bool, Option<String>) {
    let checking = core
        .runtime_discovery
        .read()
        .map(|discovery| discovery.checking)
        .unwrap_or(false);
    match runner_backend::ops::runtime::runtime_catalog(core) {
        Ok(catalog) => {
            let enabled = catalog
                .iter()
                .filter(|runtime| settings.is_agent_enabled(&runtime.name, runtime.default_enabled))
                .map(|runtime| runtime.name.clone())
                .collect::<Vec<_>>();
            (
                filter_selectable_runtime_catalog(catalog, Some(&enabled)),
                checking,
                None,
            )
        }
        Err(error) => (Vec::new(), checking, Some(error.to_string())),
    }
}

fn summarize_runner(runner: &Runner) -> String {
    format!(
        "{} · {}",
        runner.runtime,
        runner.working_dir.as_deref().unwrap_or("no working dir")
    )
}

fn runtime_display_name(runtimes: &[RuntimeCatalogEntry], name: &str) -> String {
    runtimes
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.display_name.clone())
        .or_else(|| {
            runner_backend::ops::runtime::runtime_list()
                .into_iter()
                .find(|runtime| runtime.name == name)
                .map(|runtime| runtime.display_name)
        })
        .unwrap_or_else(|| name.to_owned())
}

fn default_title_for_runner(handle: &str) -> String {
    format!("Chat with @{handle}")
}

fn default_title_for_runtime(label: &str) -> String {
    label.to_owned()
}

fn auto_title_after_selection(edited: bool, current: &str, derived: String) -> String {
    if edited {
        current.to_owned()
    } else {
        derived
    }
}

fn update_auto_title(title: &Entity<TextField>, derived: String, cx: &mut Context<NativeRoot>) {
    let (edited, next) = {
        let input = title.read(cx);
        (
            input.edited(),
            auto_title_after_selection(input.edited(), input.text(), derived),
        )
    };
    if edited {
        return;
    }
    title.update(cx, |input, input_cx| input.reset(next, input_cx));
}

fn cwd_placeholder(mode: ChatMode, runner: Option<&Runner>, default_path: &str) -> String {
    match mode {
        ChatMode::Runner => working_dir_placeholder(
            runner.and_then(|runner| runner.working_dir.as_deref()),
            default_path,
        ),
        ChatMode::Runtime => working_dir_placeholder(None, default_path),
    }
}

fn project_start_scope(
    project: Option<&runner_backend::repo::project::ProjectRow>,
) -> (Option<String>, String) {
    project.map_or_else(
        || (None, String::new()),
        |project| (Some(project.id.clone()), project.cwd.clone()),
    )
}

#[cfg(test)]
fn effort_options_for_runtime<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtimes
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.efforts.as_slice())
        .unwrap_or_default()
}

fn normalized_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn build_start_request(
    mode: ChatMode,
    runner_id: Option<&str>,
    runtime_name: Option<&str>,
    runner_runtime_override: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
    cwd: Option<String>,
) -> Option<StartRequest> {
    match mode {
        ChatMode::Runner => runner_id.map(|runner_id| StartRequest::Runner {
            runner_id: runner_id.to_owned(),
            runtime: runner_runtime_override.map(str::to_owned),
            model: runner_runtime_override.and(model),
            effort: runner_runtime_override.and(effort),
            cwd,
        }),
        ChatMode::Runtime => runtime_name.map(|runtime| StartRequest::Runtime {
            runtime: runtime.to_owned(),
            model,
            effort,
            cwd,
        }),
    }
}

fn start_chat_mode_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(START_CHAT_MODE_FILE)
}

fn read_start_chat_mode(app_data_dir: &Path) -> ChatMode {
    let value = fs::read_to_string(start_chat_mode_path(app_data_dir)).ok();
    ChatMode::from_persisted(value.as_deref())
}

fn write_start_chat_mode(app_data_dir: &Path, mode: ChatMode) -> std::io::Result<()> {
    fs::write(start_chat_mode_path(app_data_dir), mode.persisted())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(name: &str, efforts: &[&str]) -> RuntimeCatalogEntry {
        RuntimeCatalogEntry {
            name: name.into(),
            display_name: name.into(),
            command: name.into(),
            description: name.into(),
            default_enabled: true,
            available: true,
            models: Vec::new(),
            efforts: efforts
                .iter()
                .map(|value| RuntimeCatalogOption {
                    value: (*value).into(),
                    label: (*value).into(),
                    description: None,
                })
                .collect(),
        }
    }

    #[test]
    fn title_auto_derives_until_the_user_edits_it() {
        assert_eq!(
            auto_title_after_selection(
                false,
                "Chat with @coder",
                default_title_for_runner("reviewer")
            ),
            "Chat with @reviewer"
        );
        assert_eq!(
            auto_title_after_selection(true, "my chat", default_title_for_runtime("Codex")),
            "my chat"
        );
    }

    #[test]
    fn effort_options_follow_the_selected_runtime_catalog() {
        let runtimes = [runtime("codex", &["", "low", "max"]), runtime("qoder", &[])];
        assert_eq!(
            effort_options_for_runtime(&runtimes, "codex")
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["", "low", "max"]
        );
        assert!(effort_options_for_runtime(&runtimes, "qoder").is_empty());
        assert!(effort_options_for_runtime(&runtimes, "missing").is_empty());
    }

    #[test]
    fn start_request_applies_overrides_only_to_the_active_runtime() {
        assert_eq!(
            build_start_request(
                ChatMode::Runner,
                Some("coder"),
                Some("codex"),
                None,
                Some("gpt-5.6-sol".into()),
                Some("high".into()),
                Some("/repo".into()),
            ),
            Some(StartRequest::Runner {
                runner_id: "coder".into(),
                runtime: None,
                model: None,
                effort: None,
                cwd: Some("/repo".into()),
            })
        );
        assert_eq!(
            build_start_request(
                ChatMode::Runner,
                Some("coder"),
                Some("codex"),
                Some("claude-code"),
                Some("opus".into()),
                Some("max".into()),
                None,
            ),
            Some(StartRequest::Runner {
                runner_id: "coder".into(),
                runtime: Some("claude-code".into()),
                model: Some("opus".into()),
                effort: Some("max".into()),
                cwd: None,
            })
        );
        assert_eq!(
            build_start_request(
                ChatMode::Runtime,
                Some("coder"),
                Some("codex"),
                Some("claude-code"),
                Some("gpt-5.6-sol".into()),
                Some("high".into()),
                None,
            ),
            Some(StartRequest::Runtime {
                runtime: "codex".into(),
                model: Some("gpt-5.6-sol".into()),
                effort: Some("high".into()),
                cwd: None,
            })
        );
        assert_eq!(
            build_start_request(ChatMode::Runner, None, None, None, None, None, None),
            None
        );
    }

    #[test]
    fn mode_preference_round_trips_and_invalid_values_fall_back_to_runner() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runner);

        write_start_chat_mode(temp.path(), ChatMode::Runtime).unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runtime);

        fs::write(start_chat_mode_path(temp.path()), "unexpected").unwrap();
        assert_eq!(read_start_chat_mode(temp.path()), ChatMode::Runner);
    }

    #[test]
    fn project_scope_seeds_cwd_before_runner_and_settings_defaults() {
        let project = runner_backend::repo::project::ProjectRow {
            id: "project-1".into(),
            name: "Runner".into(),
            cwd: "/project".into(),
            position: 0,
            created_at: "now".into(),
        };
        let (project_id, seeded_cwd) = project_start_scope(Some(&project));
        assert_eq!(project_id.as_deref(), Some("project-1"));
        assert_eq!(seeded_cwd, "/project");
        assert_eq!(
            effective_working_dir(&seeded_cwd, true, "/settings"),
            Some("/project".into())
        );

        let (project_id, seeded_cwd) = project_start_scope(None);
        assert_eq!(project_id, None);
        assert!(seeded_cwd.is_empty());
        assert_eq!(
            effective_working_dir(&seeded_cwd, false, "/settings"),
            Some("/settings".into())
        );
    }
}

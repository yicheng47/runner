use std::fs;
use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    anchored, deferred, div, point, px, relative, rems, AnchoredPositionMode, AnyElement, Context,
    FontWeight, KeyDownEvent, MouseButton, PathPromptOptions, SharedString, Window,
};
use runner_backend::model::Runner;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};

use crate::modal_text_input::ModalTextInput;

use super::*;

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
enum ModalPicker {
    Runner,
    RunnerRuntime,
    Runtime,
    Model,
    Effort,
}

#[derive(Clone)]
struct ModalChoice {
    value: String,
    label: String,
    description: Option<String>,
}

pub(crate) struct StartChatModal {
    target: ChatTarget,
    mode: ChatMode,
    runners: Vec<Runner>,
    runtimes: Vec<RuntimeCatalogEntry>,
    runner_id: Option<String>,
    runtime_name: Option<String>,
    runner_runtime_override: Option<String>,
    effort: String,
    picker: Option<ModalPicker>,
    title: Entity<ModalTextInput>,
    cwd: Entity<ModalTextInput>,
    model: Entity<ModalTextInput>,
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
        if self.start_chat_modal.is_some() {
            return;
        }
        self.open_start_chat_modal(ChatTarget::NewTab, None, window, cx);
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
        self.open_start_chat_modal(
            ChatTarget::Pane {
                tab_id,
                pane_id: pane_id.to_owned(),
            },
            self.last_focused_runner_id.clone(),
            window,
            cx,
        );
    }

    fn open_start_chat_modal(
        &mut self,
        target: ChatTarget,
        default_runner_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut error = None;
        match runner_backend::ops::runner::runner_list(&self.core) {
            Ok(runners) => self.runners = runners,
            Err(load_error) => error = Some(load_error.to_string()),
        }

        let (runtimes, agents_checking, agents_error) = load_selectable_runtimes(&self.core);
        let persisted_mode = read_start_chat_mode(&self.core.app_data_dir);
        let mode = if default_runner_id.is_some() {
            ChatMode::Runner
        } else {
            persisted_mode
        };
        let runner_id = default_runner_id
            .filter(|runner_id| self.runners.iter().any(|runner| runner.id == *runner_id))
            .or_else(|| self.runners.first().map(|runner| runner.id.clone()));
        let runtime_name = runtimes.first().map(|runtime| runtime.name.clone());
        let title = match mode {
            ChatMode::Runner => runner_id
                .as_deref()
                .and_then(|runner_id| self.runners.iter().find(|runner| runner.id == runner_id))
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
            runner_id
                .as_deref()
                .and_then(|runner_id| self.runners.iter().find(|runner| runner.id == runner_id)),
        );
        let title_input = cx.new(|input_cx| {
            ModalTextInput::new(input_cx.focus_handle(), title, "e.g. quick-debug", false)
        });
        let cwd_input = cx.new(|input_cx| {
            ModalTextInput::new(input_cx.focus_handle(), "", cwd_placeholder, true)
        });
        let model_input =
            cx.new(|input_cx| ModalTextInput::new(input_cx.focus_handle(), "", "default", true));
        let title_focus = title_input.read(cx).focus_handle();

        self.layout_picker_open = false;
        self.start_chat_modal = Some(StartChatModal {
            target,
            mode,
            runners: self.runners.clone(),
            runtimes,
            runner_id,
            runtime_name,
            runner_runtime_override: None,
            effort: String::new(),
            picker: None,
            title: title_input,
            cwd: cwd_input,
            model: model_input,
            agents_checking,
            agents_error,
            submitting: false,
            error: error.take(),
        });
        title_focus.focus(window);
        cx.notify();
    }

    pub(crate) fn refresh_start_chat_runtimes(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        let (runtimes, agents_checking, agents_error) = load_selectable_runtimes(&self.core);
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
        }
        cx.notify();
    }

    pub(crate) fn remember_active_runner(&mut self) {
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        self.last_focused_runner_id = self
            .session_entry(&session_id)
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
        self.focus_active_terminal(window);
        cx.notify();
    }

    fn set_start_chat_mode(&mut self, mode: ChatMode, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.mode == mode || modal.submitting {
            return;
        }
        modal.mode = mode;
        modal.runner_runtime_override = None;
        modal.effort.clear();
        modal.picker = None;
        modal
            .model
            .update(cx, |input, input_cx| input.reset("", input_cx));
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
        let placeholder = cwd_placeholder(mode, modal.selected_runner());
        modal.cwd.update(cx, |input, input_cx| {
            input.set_placeholder(placeholder, input_cx)
        });
        let _ = write_start_chat_mode(&self.core.app_data_dir, mode);
        cx.notify();
    }

    fn toggle_start_chat_picker(&mut self, picker: ModalPicker, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.submitting {
            return;
        }
        modal.picker = (modal.picker != Some(picker)).then_some(picker);
        cx.notify();
    }

    fn select_start_chat_choice(
        &mut self,
        picker: ModalPicker,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        match picker {
            ModalPicker::Runner => {
                modal.runner_id = Some(value.to_owned());
                let derived = modal
                    .selected_runner()
                    .map(|runner| default_title_for_runner(&runner.handle))
                    .unwrap_or_default();
                update_auto_title(&modal.title, derived, cx);
                let placeholder = cwd_placeholder(modal.mode, modal.selected_runner());
                modal.cwd.update(cx, |input, input_cx| {
                    input.set_placeholder(placeholder, input_cx)
                });
            }
            ModalPicker::RunnerRuntime => {
                modal.runner_runtime_override = (!value.is_empty()).then(|| value.to_owned());
                modal.effort.clear();
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
            }
            ModalPicker::Runtime => {
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
            }
            ModalPicker::Model => {
                modal.model.update(cx, |input, input_cx| {
                    input.reset(value.to_owned(), input_cx)
                });
            }
            ModalPicker::Effort => modal.effort = value.to_owned(),
        }
        modal.picker = None;
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

    fn dismiss_start_chat_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.start_chat_modal.as_mut() {
            if modal.picker.take().is_some() {
                cx.notify();
            }
        }
        cx.stop_propagation();
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
        let cwd = normalized_value(modal.cwd.read(cx).text());
        let model = normalized_value(modal.model.read(cx).text());
        let effort = normalized_value(&modal.effort);
        let title = modal.title.read(cx).text().trim().to_owned();
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
                    .map(|layout| self.estimated_terminal_size(layout, pane_id, window))
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
                    &self.core,
                    runner_id,
                    runtime,
                    model,
                    effort,
                    None,
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
                    &self.core,
                    &runtime,
                    None,
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
                    &self.core,
                    &spawned.id,
                    Some(title),
                ) {
                    rename_error = Some(format!(
                        "Chat started, but its title could not be saved: {error}"
                    ));
                }
            }
            self.refresh_sessions();
            match target {
                ChatTarget::NewTab => {
                    self.reload_tabs()?;
                    self.tabs.activate_session(&spawned.id);
                }
                ChatTarget::Pane { pane_id, .. } => {
                    self.tabs.assign_to_active(&pane_id, &spawned.id)?;
                    self.persist_active_tab()?;
                    self.reload_tabs()?;
                    self.tabs.activate_session(&spawned.id);
                }
            }
            self.ensure_active_tab_attached(window, cx)?;
            Ok(spawned.id)
        })();

        match result {
            Ok(session_id) => {
                self.start_chat_modal = None;
                self.error = rename_error;
                self.remember_active_runner();
                if let Some(chat) = self.attached.get(&session_id) {
                    chat.terminal_focus.focus(window);
                }
            }
            Err(start_error) => {
                if let Some(session_id) = spawned_id {
                    self.start_chat_modal = None;
                    let _ = self.reload_tabs();
                    self.tabs.activate_session(&session_id);
                    let _ = self.ensure_active_tab_attached(window, cx);
                    self.remember_active_runner();
                    self.error = Some(start_error.to_string());
                } else if let Some(modal) = self.start_chat_modal.as_mut() {
                    modal.submitting = false;
                    modal.error = Some(start_error.to_string());
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_start_chat_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.start_chat_modal.as_ref().expect("modal is open");
        let selected_runner = modal.selected_runner().cloned();
        let selected_runtime = modal.selected_runtime().cloned();
        let override_runtime = modal.override_runtime().cloned();
        let active_runtime = modal.active_runtime().cloned();
        let mode = modal.mode;
        let submitting = modal.submitting;
        let can_submit = modal.can_submit();
        let picker = modal.picker;
        let title_input = modal.title.clone();
        let cwd_input = modal.cwd.clone();
        let model_input = modal.model.clone();

        let runner_choices = modal
            .runners
            .iter()
            .map(|runner| ModalChoice {
                value: runner.id.clone(),
                label: format!("@{}", runner.handle),
                description: Some(summarize_runner(runner)),
            })
            .collect::<Vec<_>>();
        let runtime_choices = modal
            .runtimes
            .iter()
            .map(runtime_choice)
            .collect::<Vec<_>>();
        let mut runner_runtime_choices = vec![ModalChoice {
            value: String::new(),
            label: format!(
                "Runner default{}",
                selected_runner
                    .as_ref()
                    .map(|runner| format!(
                        " ({})",
                        runtime_display_name(&modal.runtimes, &runner.runtime)
                    ))
                    .unwrap_or_default()
            ),
            description: None,
        }];
        runner_runtime_choices.extend(runtime_choices.clone());
        let runner_picker = self.render_modal_select(
            "start-chat-runner",
            ModalPicker::Runner,
            selected_runner
                .as_ref()
                .map(|runner| format!("@{}", runner.handle))
                .unwrap_or_else(|| "No runners yet".into()),
            selected_runner.as_ref().map(summarize_runner),
            modal.runner_id.clone().unwrap_or_default(),
            runner_choices,
            FIELD_WIDTH,
            true,
            !submitting && !modal.runners.is_empty(),
            picker,
            cx,
        );
        let runner_runtime_picker = self.render_modal_select(
            "start-chat-runner-runtime",
            ModalPicker::RunnerRuntime,
            runner_runtime_choices
                .iter()
                .find(|choice| {
                    choice.value == modal.runner_runtime_override.as_deref().unwrap_or_default()
                })
                .map(|choice| choice.label.clone())
                .unwrap_or_else(|| "Runner default".into()),
            None,
            modal.runner_runtime_override.clone().unwrap_or_default(),
            runner_runtime_choices,
            FIELD_WIDTH,
            false,
            !submitting,
            picker,
            cx,
        );
        let runtime_picker = self.render_modal_select(
            "start-chat-runtime",
            ModalPicker::Runtime,
            selected_runtime
                .as_ref()
                .map(|runtime| runtime.display_name.clone())
                .unwrap_or_else(|| "No agents detected".into()),
            None,
            modal.runtime_name.clone().unwrap_or_default(),
            runtime_choices,
            FIELD_WIDTH,
            false,
            !submitting && !modal.runtimes.is_empty(),
            picker,
            cx,
        );

        let runner_fields = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(modal_field("Runner", None, runner_picker))
            .when(modal.runners.is_empty(), |fields| {
                fields.child(
                    div()
                        .mt(rems(-14. / 16.))
                        .text_size(rems(11. / 16.))
                        .text_color(theme::warning())
                        .child("No runners yet. Create one from the runner page first."),
                )
            })
            .child(modal_field(
                "Agent",
                Some("Overriding runs this persona on another agent; its model and effort become configurable below."),
                runner_runtime_picker,
            ))
            .when_some(override_runtime, |fields, runtime| {
                fields.child(self.render_model_effort_fields(
                    &runtime,
                    model_input.clone(),
                    &modal.effort,
                    picker,
                    submitting,
                    cx,
                ))
            });

        let direct_fields = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(modal_field("Agent", None, runtime_picker))
            .when(modal.runtimes.is_empty(), |fields| {
                fields.child(
                    div()
                        .mt(rems(-14. / 16.))
                        .text_size(rems(11. / 16.))
                        .text_color(theme::warning())
                        .child(if modal.agents_checking {
                            "Detecting agents…"
                        } else {
                            "No enabled agents detected. Configure one in Settings → Agents."
                        }),
                )
            })
            .when_some(modal.agents_error.clone(), |fields, error| {
                fields.child(
                    div()
                        .mt(rems(-14. / 16.))
                        .text_size(rems(11. / 16.))
                        .text_color(theme::danger())
                        .child(error),
                )
            })
            .when_some(active_runtime, |fields, runtime| {
                fields.child(self.render_model_effort_fields(
                    &runtime,
                    model_input,
                    &modal.effort,
                    picker,
                    submitting,
                    cx,
                ))
            });

        let content =
            div()
                .flex()
                .flex_col()
                .gap_5()
                .children(modal.error.as_ref().map(|error| {
                    div()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::with_alpha(theme::danger(), 0.4))
                        .bg(theme::with_alpha(theme::danger(), 0.1))
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(theme::danger())
                        .child(SharedString::from(error.clone()))
                }))
                .child(
                    div()
                        .flex()
                        .w_full()
                        .p(rems(2. / 16.))
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        .child(self.render_mode_button(
                            "Runner",
                            ChatMode::Runner,
                            mode,
                            submitting,
                            cx,
                        ))
                        .child(self.render_mode_button(
                            "Direct",
                            ChatMode::Runtime,
                            mode,
                            submitting,
                            cx,
                        )),
                )
                .child(match mode {
                    ChatMode::Runner => runner_fields.into_any_element(),
                    ChatMode::Runtime => direct_fields.into_any_element(),
                })
                .child(modal_field(
                    "Chat name",
                    Some("Optional. Leave blank to use the default label."),
                    title_input,
                ))
                .child(modal_field(
                    "Working directory",
                    None,
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .child(div().flex_1().min_w(px(0.)).child(cwd_input))
                        .child(
                            modal_button("browse-start-chat-cwd", "Browse…", false, submitting)
                                .when(!submitting, |button| {
                                    button.on_click(cx.listener(|this, _, _, cx| {
                                        this.browse_start_chat_cwd(cx);
                                    }))
                                }),
                        ),
                ))
                .child(
                    div()
                        .mt(rems(-14. / 16.))
                        .text_size(rems(11. / 16.))
                        .text_color(theme::muted())
                        .child("Leave blank to use the default working directory."),
                );

        div()
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x00000099))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_start_chat_modal(window, cx)),
            )
            .child(
                div()
                    .w(rems(MODAL_WIDTH / 16.))
                    .h(rems(650. / 16.))
                    .max_h(relative(0.85))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::muted())
                    .bg(theme::composer_bg())
                    .shadow_lg()
                    .on_key_down(cx.listener(Self::on_start_chat_key_down))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.dismiss_start_chat_picker(cx)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(rems(74. / 16.))
                            .px_6()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(theme::border())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(rems(1.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child("Start a chat"),
                                    )
                                    .child(
                                        div().text_xs().text_color(theme::muted()).child(
                                            "Spawns a direct PTY in the selected directory.",
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-start-chat")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(rems(28. / 16.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(rems(18. / 16.))
                                    .text_color(theme::muted())
                                    .hover(|button| {
                                        button.bg(theme::border()).text_color(theme::text())
                                    })
                                    .child("×")
                                    .when(!submitting, |button| {
                                        button.on_click(cx.listener(|this, _, window, cx| {
                                            this.close_start_chat_modal(window, cx);
                                        }))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("start-chat-modal-content")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .px_6()
                            .py_5()
                            .child(content),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(rems(4.))
                            .px_6()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .child(
                                modal_button("cancel-start-chat", "Cancel", false, submitting)
                                    .when(!submitting, |button| {
                                        button.on_click(cx.listener(|this, _, window, cx| {
                                            this.close_start_chat_modal(window, cx);
                                        }))
                                    }),
                            )
                            .child(
                                modal_button(
                                    "submit-start-chat",
                                    if submitting {
                                        "Starting…"
                                    } else {
                                        "Start chat"
                                    },
                                    true,
                                    !can_submit,
                                )
                                .when(can_submit, |button| {
                                    button.on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_start_chat(window, cx);
                                    }))
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_mode_button(
        &self,
        label: &'static str,
        mode: ChatMode,
        active: ChatMode,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(match mode {
                ChatMode::Runner => "start-chat-mode-runner",
                ChatMode::Runtime => "start-chat-mode-runtime",
            })
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .h(rems(30. / 16.))
            .rounded_md()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(if active == mode {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active == mode, |button| button.bg(theme::border()))
            .when(!disabled, |button| {
                button
                    .cursor_pointer()
                    .hover(|button| button.text_color(theme::text()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_start_chat_mode(mode, cx);
                    }))
            })
            .child(label)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_modal_select(
        &self,
        id: &'static str,
        picker_kind: ModalPicker,
        selected_label: String,
        selected_description: Option<String>,
        selected_value: String,
        choices: Vec<ModalChoice>,
        width: f32,
        detailed: bool,
        enabled: bool,
        open_picker: Option<ModalPicker>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = open_picker == Some(picker_kind);
        let button = div()
            .id(id)
            .w(rems(width / 16.))
            .h(rems(if detailed { 52. / 16. } else { 36. / 16. }))
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(if open {
                theme::muted()
            } else {
                theme::border()
            })
            .bg(theme::bg())
            .opacity(if enabled { 1. } else { 0.6 })
            .when(enabled, |button| {
                button
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .cursor_pointer()
                    .hover(|button| button.border_color(theme::muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_start_chat_picker(picker_kind, cx);
                    }))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(selected_label),
                    )
                    .children(selected_description.map(|description| {
                        div()
                            .truncate()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(description)
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .text_color(theme::muted())
                    .child("⌄"),
            );

        let menu = open.then(|| {
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .offset(point(
                        px(0.),
                        px((if detailed { 56. } else { 40. }) * self.settings.app_zoom),
                    ))
                    .child(
                        div()
                            .id("start-chat-options-menu")
                            .w(rems(width / 16.))
                            .max_h(rems(14.))
                            .overflow_y_scroll()
                            .p_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::composer_bg())
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .children(choices.into_iter().enumerate().map(
                                move |(index, choice)| {
                                    let active = choice.value == selected_value;
                                    let value = choice.value.clone();
                                    div()
                                        .id(SharedString::from(format!(
                                            "start-chat-option-{}-{index}",
                                            picker_kind as usize
                                        )))
                                        .w_full()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .when(active, |row| row.bg(theme::border()))
                                        .hover(|row| row.bg(theme::border()))
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(rems(13. / 16.))
                                                .text_color(theme::text())
                                                .child(choice.label),
                                        )
                                        .children(choice.description.map(|description| {
                                            div()
                                                .truncate()
                                                .text_size(rems(11. / 16.))
                                                .text_color(theme::muted())
                                                .child(description)
                                        }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_start_chat_choice(picker_kind, &value, cx);
                                        }))
                                },
                            )),
                    ),
            )
            .with_priority(2)
        });
        div()
            .relative()
            .w(rems(width / 16.))
            .child(button)
            .children(menu)
            .into_any_element()
    }

    fn render_model_effort_fields(
        &self,
        runtime: &RuntimeCatalogEntry,
        model_input: Entity<ModalTextInput>,
        effort: &str,
        open_picker: Option<ModalPicker>,
        submitting: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let effort_options =
            effort_options_for_runtime(std::slice::from_ref(runtime), &runtime.name);
        let has_effort = !effort_options.is_empty();
        let model_width = if has_effort { 232. } else { FIELD_WIDTH };
        let selected_model = model_input.read(cx).text().to_owned();
        let model_choices = runtime.models.iter().map(option_choice).collect::<Vec<_>>();
        let model_menu = (open_picker == Some(ModalPicker::Model)).then(|| {
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .offset(point(px(0.), px(40. * self.settings.app_zoom)))
                    .child(
                        div()
                            .id("start-chat-model-options-menu")
                            .w(rems(model_width / 16.))
                            .max_h(rems(14.))
                            .overflow_y_scroll()
                            .p_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::composer_bg())
                            .shadow_lg()
                            .occlude()
                            .children(model_choices.into_iter().enumerate().map(
                                |(index, choice)| {
                                    let active = choice.value == selected_model;
                                    let value = choice.value.clone();
                                    div()
                                        .id(("start-chat-model-option", index))
                                        .w_full()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .when(active, |row| row.bg(theme::border()))
                                        .hover(|row| row.bg(theme::border()))
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(rems(13. / 16.))
                                                .text_color(theme::text())
                                                .child(choice.label),
                                        )
                                        .children(choice.description.map(|description| {
                                            div()
                                                .truncate()
                                                .text_size(rems(11. / 16.))
                                                .text_color(theme::muted())
                                                .child(description)
                                        }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.select_start_chat_choice(
                                                ModalPicker::Model,
                                                &value,
                                                cx,
                                            );
                                        }))
                                },
                            )),
                    ),
            )
            .with_priority(2)
        });
        let model = div()
            .id("start-chat-model-field")
            .relative()
            .w(rems(model_width / 16.))
            .flex()
            .items_center()
            .child(div().flex_1().min_w(px(0.)).child(model_input))
            .child(
                div()
                    .id("start-chat-model-options")
                    .absolute()
                    .right(rems(6. / 16.))
                    .top(rems(6. / 16.))
                    .size(rems(1.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_sm()
                    .text_color(theme::muted())
                    .when(!submitting, |button| {
                        button
                            .cursor_pointer()
                            .hover(|button| button.bg(theme::border()))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_start_chat_picker(ModalPicker::Model, cx);
                            }))
                    })
                    .child("⌄"),
            )
            .children(model_menu);

        let fields = div()
            .flex()
            .items_start()
            .gap_3()
            .child(modal_field("Model", None, model));
        if !has_effort {
            return fields.into_any_element();
        }
        let effort_choices = effort_options.iter().map(option_choice).collect::<Vec<_>>();
        let effort_label = effort_options
            .iter()
            .find(|option| option.value == effort)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| "default".into());
        fields
            .child(modal_field(
                "Thinking effort",
                None,
                self.render_modal_select(
                    "start-chat-effort",
                    ModalPicker::Effort,
                    effort_label,
                    None,
                    effort.to_owned(),
                    effort_choices,
                    232.,
                    false,
                    !submitting,
                    open_picker,
                    cx,
                ),
            ))
            .into_any_element()
    }
}

fn modal_field(
    label: &'static str,
    subtitle: Option<&'static str>,
    child: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(label),
        )
        .child(child)
        .children(subtitle.map(|subtitle| {
            div()
                .text_size(rems(11. / 16.))
                .text_color(theme::muted())
                .child(subtitle)
        }))
        .into_any_element()
}

fn modal_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    disabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(rems(34. / 16.))
        .px_4()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(if primary {
            theme::accent()
        } else {
            theme::border()
        })
        .bg(if primary {
            theme::accent()
        } else {
            theme::composer_bg()
        })
        .opacity(if disabled { 0.5 } else { 1. })
        .when(!disabled, |button| {
            button.cursor_pointer().hover(|button| {
                if primary {
                    button.opacity(0.9)
                } else {
                    button.bg(theme::border())
                }
            })
        })
        .text_size(rems(13. / 16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if primary {
            theme::accent_ink()
        } else {
            theme::text()
        })
        .child(label)
}

fn load_selectable_runtimes(core: &AppCore) -> (Vec<RuntimeCatalogEntry>, bool, Option<String>) {
    let checking = core
        .runtime_discovery
        .read()
        .map(|discovery| discovery.checking)
        .unwrap_or(false);
    match runner_backend::ops::runtime::selectable_runtime_catalog(core, None) {
        Ok(catalog) => (catalog, checking, None),
        Err(error) => (Vec::new(), checking, Some(error.to_string())),
    }
}

fn runtime_choice(runtime: &RuntimeCatalogEntry) -> ModalChoice {
    ModalChoice {
        value: runtime.name.clone(),
        label: runtime.display_name.clone(),
        description: None,
    }
}

fn option_choice(option: &RuntimeCatalogOption) -> ModalChoice {
    ModalChoice {
        value: option.value.clone(),
        label: option.label.clone(),
        description: option.description.clone(),
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

fn update_auto_title(
    title: &Entity<ModalTextInput>,
    derived: String,
    cx: &mut Context<NativeRoot>,
) {
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

fn cwd_placeholder(mode: ChatMode, runner: Option<&Runner>) -> String {
    match mode {
        ChatMode::Runner => runner
            .and_then(|runner| runner.working_dir.clone())
            .unwrap_or_else(|| "(no working directory)".into()),
        ChatMode::Runtime => "(no working directory)".into(),
    }
}

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
}

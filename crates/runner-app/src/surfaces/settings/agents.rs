use runner_backend::model::Runtime;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, Div, Entity, FontWeight, PathPromptOptions, Render,
    SharedString, Subscription, WeakEntity, Window,
};
use runner_app::ui::button::spinner;
use runner_app::ui::{
    Badge, BrowseField, Button, ButtonSize, ButtonVariant, FieldValidation, PaneHeader,
    SettingsCard, TextField, Toggle, Tone, Tooltip,
};
use runner_backend::ops::runtime::RuntimeCatalogEntry;
use runner_backend::runtime_status::{
    OverrideValidationError, RuntimeCommandSource, RuntimeExecutableStatus, RuntimeRowState,
    RuntimeStatusResponse, ShellDiscoveryStatus,
};
use runner_backend::shell_path::DiscoveryOutcome;

use crate::app_settings::AppSettings;
use crate::app_store::AppStore;
use crate::chat_icon::ChatIcon;
use crate::theme;
use crate::NativeRoot;

const NO_INSTALLED_AGENTS: &str =
    "No agents installed. Install one of the agents below to get started.";
const ALL_AGENTS_INSTALLED: &str = "All supported agents are installed.";
const BROWSE_EXECUTABLE_HINT: &str =
    "Use Browse to select an executable Runner did not find on PATH.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BadgeTone {
    Accent,
    Neutral,
    Danger,
    Warning,
    Muted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimePresentation {
    badge: &'static str,
    tone: BadgeTone,
    spinning: bool,
    caption: Option<String>,
    auto_path: String,
    show_reset: bool,
}

/// The Update button and its running-sessions caption, present only while
/// npm has a newer version than the one installed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct UpdateAction {
    disabled: bool,
    caption: Option<String>,
}

pub(crate) struct AgentsPane {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    status: Option<RuntimeStatusResponse>,
    catalog: Vec<RuntimeCatalogEntry>,
    error: Option<String>,
    loading: bool,
    refreshing: bool,
    overrides: HashMap<Runtime, Entity<TextField>>,
    validation: HashMap<Runtime, String>,
    validation_drafts: HashMap<Runtime, String>,
    saving: HashSet<Runtime>,
    focused: HashSet<Runtime>,
    live_sessions: HashMap<Runtime, usize>,
    _subscriptions: Vec<Subscription>,
}

impl AgentsPane {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut overrides = HashMap::new();
        let mut subscriptions = Vec::new();
        for runtime in runner_backend::ops::runtime::runtime_list() {
            let runtime_name = runtime.name;
            let enter_shell = shell.clone();
            let escape_pane = cx.weak_entity();
            let escape_name = runtime_name;
            let field = cx.new(|input_cx| {
                TextField::new(
                    input_cx.focus_handle(),
                    "",
                    format!("Auto — {} not found on PATH", runtime.command),
                    true,
                )
                .text_size(theme::text_meta())
                .key_interceptor(Rc::new(move |event, window, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" => {
                            let shell = enter_shell.clone();
                            window.defer(cx, move |window, cx| {
                                let _ =
                                    shell.update(cx, |root, _| root.focus_settings_page(window));
                            });
                            true
                        }
                        "escape" => {
                            let pane = escape_pane.clone();
                            let runtime = escape_name;
                            window.defer(cx, move |_, cx| {
                                let _ = pane.update(cx, |this, pane_cx| {
                                    this.discard_override(runtime, pane_cx)
                                });
                            });
                            true
                        }
                        _ => false,
                    }
                }))
            });
            let focus = field.read(cx).focus_handle();
            let focus_runtime = runtime_name;
            subscriptions.push(cx.on_focus_in(&focus, window, move |this, _, cx| {
                this.focused.insert(focus_runtime);
                cx.notify();
            }));
            let blur_runtime = runtime_name;
            subscriptions.push(cx.on_focus_out(&focus, window, move |this, _, _, cx| {
                this.focused.remove(&blur_runtime);
                this.commit_override(blur_runtime, cx);
            }));
            let draft_runtime = runtime_name;
            subscriptions.push(cx.observe(&field, move |this, field, cx| {
                let draft = field.read(cx).text().to_owned();
                if this
                    .validation_drafts
                    .get(&draft_runtime)
                    .is_some_and(|invalid_draft| invalid_draft != &draft)
                {
                    this.validation.remove(&draft_runtime);
                    this.validation_drafts.remove(&draft_runtime);
                    field.update(cx, |field, field_cx| {
                        field.set_validation(FieldValidation::Valid, field_cx)
                    });
                }
                cx.notify();
            }));
            overrides.insert(runtime_name, field);
        }
        subscriptions.push(cx.observe(&app_store, |this, _, cx| {
            this.reconcile_default_preference(cx);
            cx.notify();
        }));

        Self {
            shell,
            app_store,
            status: None,
            catalog: Vec::new(),
            error: None,
            loading: false,
            refreshing: false,
            overrides,
            validation: HashMap::new(),
            validation_drafts: HashMap::new(),
            saving: HashSet::new(),
            focused: HashSet::new(),
            live_sessions: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let status = runner_backend::ops::runtime::runtime_status_list(&core)
                .map_err(|error| error.to_string())?;
            let catalog = runner_backend::ops::runtime::runtime_catalog(&core)
                .map_err(|error| error.to_string())?;
            let live = runner_backend::ops::session::live_session_counts(&core).unwrap_or_default();
            Ok::<_, String>((status, catalog, live))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((status, catalog, live)) => {
                        this.catalog = catalog;
                        this.live_sessions = live;
                        this.apply_status(status, cx);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Recounts live sessions per agent, which the Update guard reads, as
    /// sessions start and stop while Settings is open.
    pub(crate) fn refresh_live_sessions(&mut self, cx: &mut Context<Self>) {
        let core = self.app_store.read(cx).core.clone();
        let task =
            cx.background_spawn(
                async move { runner_backend::ops::session::live_session_counts(&core) },
            );
        cx.spawn(async move |weak, cx| {
            if let Ok(live) = task.await {
                let _ = weak.update(cx, |this, cx| {
                    if this.live_sessions != live {
                        this.live_sessions = live;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn open_update(
        &mut self,
        runtime: &RuntimeExecutableStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(installed), Some(available)) = (
            runtime.installed_version.clone(),
            runtime.available_version.clone(),
        ) else {
            return;
        };
        let request = crate::surfaces::agent_update::AgentUpdateRequest {
            runtime: runtime.name,
            display_name: runtime.display_name.clone(),
            command: runtime.command.clone(),
            installed,
            available,
        };
        let shell = self.shell.clone();
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |root, root_cx| {
                root.open_agent_update(request, window, root_cx)
            });
        });
    }

    fn refresh_discovery(&mut self, cx: &mut Context<Self>) {
        if self.refreshing
            || self
                .status
                .as_ref()
                .is_some_and(|status| status.shell.checking)
        {
            return;
        }
        self.refreshing = true;
        let core = self.app_store.read(cx).core.clone();
        let model_runtimes = self.app_store.read(cx).settings.model_runtimes();
        let task = cx.background_spawn(async move {
            let status = runner_backend::ops::runtime::runtime_refresh(&core, &model_runtimes)
                .map_err(|error| error.to_string())?;
            let catalog = runner_backend::ops::runtime::runtime_catalog(&core)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((status, catalog))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.refreshing = false;
                match result {
                    Ok((status, catalog)) => {
                        this.catalog = catalog;
                        this.apply_status(status, cx);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_status(&mut self, status: RuntimeStatusResponse, cx: &mut Context<Self>) {
        for runtime in &status.runtimes {
            let Some(field) = self.overrides.get(&runtime.name).cloned() else {
                continue;
            };
            let presentation = runtime_presentation(
                runtime,
                status.shell.outcome,
                runtime.invalid_reason.as_deref(),
                self.saving.contains(&runtime.name),
            );
            let sync_value =
                !self.focused.contains(&runtime.name) && !self.saving.contains(&runtime.name);
            field.update(cx, |field, field_cx| {
                field.set_placeholder(presentation.auto_path, field_cx);
                if sync_value {
                    field.reset(runtime.override_path.clone().unwrap_or_default(), field_cx);
                    field.set_validation(
                        runtime
                            .invalid_reason
                            .clone()
                            .map(FieldValidation::error)
                            .unwrap_or_default(),
                        field_cx,
                    );
                }
            });
            if sync_value {
                if let Some(error) = runtime.invalid_reason.clone() {
                    self.validation.insert(runtime.name, error);
                    self.validation_drafts.insert(
                        runtime.name,
                        runtime.override_path.clone().unwrap_or_default(),
                    );
                } else {
                    self.validation.remove(&runtime.name);
                    self.validation_drafts.remove(&runtime.name);
                }
            }
        }
        self.status = Some(status);
        self.reconcile_default_preference(cx);
    }

    fn reconcile_default_preference(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.status.as_ref() else {
            return;
        };
        let mut settings = self.app_store.read(cx).settings.clone();
        if reconcile_default_runtime(&mut settings, status, &self.catalog) {
            self.app_store.update(cx, |store, store_cx| {
                store.update_settings(
                    |current| {
                        if current.default_runtime.is_empty() {
                            false
                        } else {
                            current.default_runtime.clear();
                            true
                        }
                    },
                    true,
                    store_cx,
                );
            });
        }
    }

    fn set_default_runtime(&mut self, runtime: Option<Runtime>, cx: &mut Context<Self>) {
        let changed = self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| update_default_runtime(settings, runtime),
                true,
                store_cx,
            )
        });
        if changed {
            let shell = self.shell.clone();
            cx.defer(move |cx| {
                if let Some(shell) = shell.upgrade() {
                    shell.update(cx, |shell, shell_cx| {
                        shell.sync_start_chat_default_runtime(shell_cx)
                    });
                }
            });
        }
    }

    fn set_enabled(&mut self, runtime: Runtime, enabled: bool, cx: &mut Context<Self>) {
        let changed = self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| update_agent_enabled_preferences(settings, runtime, enabled),
                true,
                store_cx,
            )
        });
        if !changed {
            return;
        }
        self.reconcile_default_preference(cx);
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            if let Some(shell) = shell.upgrade() {
                shell.update(cx, |shell, shell_cx| {
                    shell.refresh_start_chat_runtimes(shell_cx);
                    shell.refresh_role_form_runtimes(shell_cx);
                });
            }
        });
        cx.notify();
    }

    fn set_validation(
        &mut self,
        runtime: Runtime,
        validation: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = validation.clone() {
            self.validation.insert(runtime, validation);
            let draft = self
                .overrides
                .get(&runtime)
                .map(|field| field.read(cx).text().to_owned())
                .unwrap_or_default();
            self.validation_drafts.insert(runtime, draft);
        } else {
            self.validation.remove(&runtime);
            self.validation_drafts.remove(&runtime);
        }
        if let Some(field) = self.overrides.get(&runtime).cloned() {
            field.update(cx, |field, field_cx| {
                field.set_validation(
                    validation.map(FieldValidation::error).unwrap_or_default(),
                    field_cx,
                )
            });
        }
    }

    fn commit_override(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        if self.saving.contains(&runtime) {
            return;
        }
        let Some(field) = self.overrides.get(&runtime).cloned() else {
            return;
        };
        let draft = field.read(cx).text().trim().to_owned();
        let current = self
            .status
            .as_ref()
            .and_then(|status| status.runtimes.iter().find(|row| row.name == runtime));
        if current.and_then(|row| row.override_path.as_deref()) == Some(draft.as_str())
            || (draft.is_empty()
                && current
                    .and_then(|row| row.override_path.as_deref())
                    .is_none())
        {
            self.set_validation(
                runtime,
                current.and_then(|row| row.invalid_reason.clone()),
                cx,
            );
            return;
        }
        self.saving.insert(runtime);
        self.set_validation(runtime, None, cx);
        field.update(cx, |field, field_cx| field.set_disabled(true, field_cx));
        let core = self.app_store.read(cx).core.clone();
        let runtime_name = runtime;
        let task_runtime = runtime_name;
        // A new executable is a new model source; only query it when the user
        // has this agent enabled.
        let refresh_models = self
            .app_store
            .read(cx)
            .settings
            .model_runtimes()
            .contains(&runtime);
        let task = cx.background_spawn(async move {
            let status = if draft.is_empty() {
                runner_backend::ops::runtime::runtime_clear_override(&core, task_runtime)
                    .map_err(|error| error.to_string())
            } else {
                runner_backend::ops::runtime::runtime_set_override(&core, task_runtime, &draft)
                    .map_err(|error| override_validation_error_message(&error))
            };
            if status.is_ok() && refresh_models {
                runner_backend::ops::runtime::runtime_request_models(&core, &[task_runtime]);
            }
            status
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.saving.remove(&runtime_name);
                if let Some(field) = this.overrides.get(&runtime_name).cloned() {
                    field.update(cx, |field, field_cx| field.set_disabled(false, field_cx));
                }
                match result {
                    Ok(status) => {
                        this.apply_status(status, cx);
                        this.error = None;
                    }
                    Err(error) => this.set_validation(runtime_name, Some(error), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn discard_override(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        let current = self
            .status
            .as_ref()
            .and_then(|status| status.runtimes.iter().find(|row| row.name == runtime));
        let value = current
            .and_then(|row| row.override_path.clone())
            .unwrap_or_default();
        let validation = current.and_then(|row| row.invalid_reason.clone());
        if let Some(field) = self.overrides.get(&runtime).cloned() {
            field.update(cx, |field, field_cx| field.reset(value, field_cx));
        }
        self.set_validation(runtime, validation, cx);
        cx.notify();
    }

    fn browse_override(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        if self.saving.contains(&runtime) {
            return;
        }
        let Some(field) = self.overrides.get(&runtime).cloned() else {
            return;
        };
        let display_name = self
            .status
            .as_ref()
            .and_then(|status| status.runtimes.iter().find(|row| row.name == runtime))
            .map(|row| row.display_name.clone())
            .unwrap_or_else(|| runtime.to_string());
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("Choose {display_name} executable").into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            field.update(cx, |field, field_cx| {
                                field.reset(path.to_string_lossy().into_owned(), field_cx)
                            });
                            this.commit_override(runtime, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.set_validation(runtime, Some(error), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_override(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        if self.saving.contains(&runtime) {
            return;
        }
        if let Some(field) = self.overrides.get(&runtime).cloned() {
            field.update(cx, |field, field_cx| field.reset("", field_cx));
        }
        self.commit_override(runtime, cx);
    }

    fn render_installed_header(&self, count: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let checking = self.refreshing
            || self
                .status
                .as_ref()
                .is_some_and(|status| status.shell.checking);
        let pane = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                runtime_section_header("Installed", count)
                    .debug_selector(|| "AGENTS_INSTALLED_HEADER".into())
                    .child(div().flex_1())
                    .child(
                        Button::new(
                            "agents-refresh",
                            if checking { "Checking…" } else { "Refresh" },
                        )
                        .icon("refresh-cw.svg")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .loading(checking)
                        .on_press(move |_, cx| {
                            pane.update(cx, |this, pane_cx| this.refresh_discovery(pane_cx));
                        }),
                    ),
            )
            .child(runtime_section_caption(shell_description(
                self.status.as_ref().map(|status| &status.shell),
            )))
            .into_any_element()
    }

    fn render_default_action(
        &self,
        runtime: Runtime,
        eligible: bool,
        checking: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.app_store.read(cx).settings.default_runtime == runtime.key() {
            Some(
                div()
                    .debug_selector(|| format!("AGENT_DEFAULT_{runtime}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(rems(5. / 16.))
                    .rounded(rems(10. / 16.))
                    .bg(theme::with_alpha(theme::accent(), 0.1))
                    .px(rems(9. / 16.))
                    .py(rems(3. / 16.))
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent())
                    .child(
                        svg()
                            .path("check.svg")
                            .size(rems(11. / 16.))
                            .text_color(theme::accent()),
                    )
                    .child("Default")
                    .into_any_element(),
            )
        } else {
            eligible.then(|| {
                let pane = cx.entity();
                div()
                    .debug_selector(|| format!("AGENT_SET_DEFAULT_{runtime}"))
                    .child(
                        Button::new(
                            SharedString::from(format!("agent-set-default-{runtime}")),
                            "Set as default",
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(checking)
                        .on_press(move |_, cx| {
                            pane.update(cx, |this, pane_cx| {
                                this.set_default_runtime(Some(runtime), pane_cx)
                            });
                        }),
                    )
                    .into_any_element()
            })
        }
    }

    fn render_browse_field(&self, runtime: Runtime, cx: &mut Context<Self>) -> Option<BrowseField> {
        let pane = cx.entity();
        self.overrides.get(&runtime).cloned().map(|field| {
            BrowseField::new(
                field,
                self.saving.contains(&runtime),
                Rc::new(move |_, cx| {
                    pane.update(cx, |this, pane_cx| this.browse_override(runtime, pane_cx));
                }),
            )
            .browse_id(SharedString::from(format!("agent-browse-{runtime}")))
            .browse_label("Browse")
        })
    }

    fn render_not_installed_row(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mark = ChatIcon::for_runtime(runtime.name.key());
        let catalog = self.catalog.iter().find(|entry| entry.name == runtime.name);
        let validation = self.validation.get(&runtime.name).map(String::as_str);
        div()
            .debug_selector(|| format!("AGENT_CARD_{}", runtime.name))
            .flex()
            .flex_col()
            .gap_3()
            .px_5()
            .py_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .child(
                        svg()
                            .path(mark.path)
                            .size(rems(1.))
                            .flex_none()
                            .text_color(mark.color(theme::text(), true)),
                    )
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(runtime.display_name.clone()),
                    ),
            )
            .children(
                catalog
                    .filter(|entry| !entry.description.is_empty() || !entry.install_url.is_empty())
                    .map(|entry| {
                        let description = (!entry.description.is_empty()).then(|| {
                            runtime_section_caption(entry.description.clone())
                                .debug_selector(|| format!("AGENT_DESCRIPTION_{}", runtime.name))
                        });
                        let install = (!entry.install_url.is_empty()).then(|| {
                            let url = entry.install_url.clone();
                            div()
                                .debug_selector(|| format!("AGENT_INSTALL_{}", runtime.name))
                                .child(
                                    Button::new(
                                        SharedString::from(format!(
                                            "agent-install-{}",
                                            runtime.name
                                        )),
                                        "Install instructions",
                                    )
                                    .icon("external-link.svg")
                                    .size(ButtonSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .tooltip(entry.install_url.clone())
                                    .on_press(move |_, cx| cx.open_url(&url)),
                                )
                        });
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(description)
                            .children(install)
                    }),
            )
            .children(self.render_browse_field(runtime.name, cx))
            .children(validation.map(|message| {
                runtime_caption(message.to_owned(), true)
                    .debug_selector(|| format!("AGENT_VALIDATION_{}", runtime.name))
            }))
            .into_any_element()
    }

    fn render_runtime_row(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let validation = self.validation.get(&runtime.name).map(String::as_str);
        let saving = self.saving.contains(&runtime.name);
        let mut presentation = runtime_presentation(
            runtime,
            self.status.as_ref().and_then(|status| status.shell.outcome),
            validation,
            saving,
        );
        let enabled =
            runtime_default_enabled(&self.catalog, runtime.name).is_some_and(|default_enabled| {
                self.app_store
                    .read(cx)
                    .settings
                    .is_agent_enabled(runtime.name, default_enabled)
            });
        let pane = cx.entity();
        let toggle_pane = pane.clone();
        let reset_pane = pane;
        let toggle_runtime = runtime.name;
        let reset_runtime = runtime.name;
        let field = self.overrides.get(&runtime.name).cloned();
        let browse_field = self.render_browse_field(runtime.name, cx);
        presentation.show_reset |= field
            .as_ref()
            .is_some_and(|field| !field.read(cx).text().trim().is_empty());
        let mark = ChatIcon::for_runtime(runtime.name.key());
        let card_selector = format!("AGENT_CARD_{}", runtime.name);
        let default_action = self.render_default_action(
            runtime.name,
            runtime_default_eligible(runtime, &self.catalog, &self.app_store.read(cx).settings),
            runtime.state == RuntimeRowState::Checking,
            cx,
        );
        let update = update_action(
            runtime,
            self.live_sessions.get(&runtime.name).copied().unwrap_or(0),
            crate::platform_ui::AGENT_UPDATE_NEEDS_STOPPED_SESSIONS,
        );
        let update_caption = update.as_ref().and_then(|update| {
            update
                .caption
                .clone()
                .map(|caption| (caption, update.disabled))
        });
        let update_button = update.map(|update| {
            let pane = cx.entity();
            let status = runtime.clone();
            div()
                .debug_selector(|| format!("AGENT_UPDATE_{}", runtime.name))
                .child(
                    Button::new(
                        SharedString::from(format!("agent-update-{}", runtime.name)),
                        "Update",
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .disabled(update.disabled)
                    .on_press(move |window, cx| {
                        pane.update(cx, |this, pane_cx| {
                            this.open_update(&status, window, pane_cx)
                        });
                    }),
                )
        });
        div()
            .debug_selector(|| card_selector)
            .flex()
            .flex_col()
            .gap_3()
            .px_5()
            .py_4()
            .child(
                div()
                    .debug_selector(|| format!("AGENT_HEADER_{}", runtime.name))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(rems(10. / 16.))
                    .child(
                        div()
                            .debug_selector(|| format!("AGENT_IDENTITY_{}", runtime.name))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .child(
                                svg()
                                    .path(mark.path)
                                    .size(rems(1.))
                                    .flex_none()
                                    .text_color(mark.color(theme::text(), true)),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(runtime.display_name.clone()),
                            )
                            .children(default_action)
                            .when(
                                !matches!(
                                    presentation.tone,
                                    BadgeTone::Accent | BadgeTone::Neutral
                                ),
                                |identity| {
                                    identity.child(runtime_badge(runtime.name, &presentation))
                                },
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| format!("AGENT_ACTIONS_{}", runtime.name))
                            .ml_auto()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .children(update_button)
                            .child(
                                div()
                                    .debug_selector(|| format!("AGENT_TOGGLE_{}", runtime.name))
                                    .child(Tooltip::new(
                                        SharedString::from(format!(
                                            "agent-toggle-tip-{}",
                                            runtime.name
                                        )),
                                        format!(
                                            "{} {}",
                                            if enabled { "Disable" } else { "Enable" },
                                            runtime.display_name
                                        ),
                                        Toggle::new(
                                            SharedString::from(format!(
                                                "agent-toggle-{}",
                                                runtime.name
                                            )),
                                            enabled,
                                        )
                                        .on_change(
                                            move |enabled, _, cx| {
                                                toggle_pane.update(cx, |this, pane_cx| {
                                                    this.set_enabled(
                                                        toggle_runtime,
                                                        enabled,
                                                        pane_cx,
                                                    )
                                                });
                                            },
                                        ),
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().min_w(px(0.)).flex_1().children(browse_field))
                    .children(presentation.show_reset.then(|| {
                        Button::new(
                            SharedString::from(format!("agent-reset-{}", runtime.name)),
                            "Reset",
                        )
                        .variant(ButtonVariant::Ghost)
                        .disabled(saving)
                        .on_press(move |_, cx| {
                            reset_pane.update(cx, |this, pane_cx| {
                                this.reset_override(reset_runtime, pane_cx)
                            });
                        })
                    })),
            )
            .children(runtime_defaults_visible(runtime).then(|| {
                runtime_property_line(runtime.name, "MODEL", "Model")
                    .child(runtime_property_value(runtime_default_value(
                        runtime.default_model.as_deref(),
                    )))
                    .child(div().text_color(theme::muted()).child("·"))
                    .child(div().text_color(theme::muted()).child("Effort"))
                    .child(runtime_property_value(runtime_default_value(
                        runtime.default_effort.as_deref(),
                    )))
            }))
            .children(
                presentation
                    .caption
                    .map(|caption| runtime_caption(caption, validation.is_some())),
            )
            .children(update_caption.map(|(caption, blocked)| {
                runtime_caption(caption, false)
                    .debug_selector(|| format!("AGENT_UPDATE_CAPTION_{}", runtime.name))
                    .when(blocked, |caption| caption.text_color(theme::warning()))
            }))
            .into_any_element()
    }
}

fn runtime_section_header(label: &'static str, count: Option<usize>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(theme::text_body())
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
        .children(count.map(|count| Badge::new(count.to_string(), Tone::Muted)))
}

fn runtime_section_caption(caption: impl Into<SharedString>) -> Div {
    div()
        .text_size(theme::text_ui())
        .line_height(rems(17.4 / 16.))
        .text_color(theme::muted())
        .child(caption.into())
}

fn runtime_property_line(runtime: Runtime, key: &'static str, label: &'static str) -> Div {
    let line_selector = format!("AGENT_{key}_LINE_{runtime}");
    let label_selector = format!("AGENT_{key}_LABEL_{runtime}");
    div()
        .debug_selector(|| line_selector)
        .flex()
        .items_baseline()
        .gap_2()
        .text_size(theme::text_meta())
        .line_height(rems(15.4 / 16.))
        .child(
            div()
                .debug_selector(|| label_selector)
                .flex_none()
                .text_color(theme::muted())
                .child(label),
        )
}

fn runtime_property_value(value: String) -> Div {
    div()
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_color(theme::text())
        .child(value)
}

fn runtime_caption(caption: String, danger: bool) -> Div {
    div()
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_size(theme::text_meta())
        .line_height(rems(15.4 / 16.))
        .text_color(if danger {
            theme::danger()
        } else {
            theme::faint()
        })
        .child(caption)
}

impl Render for AgentsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = if let Some(status) = &self.status {
            let (installed, not_installed) = partition_runtimes(&status.runtimes);
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(
                    div()
                        .debug_selector(|| "AGENTS_INSTALLED".into())
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(self.render_installed_header(Some(installed.len()), cx))
                        .when(installed.is_empty(), |section| {
                            section.child(
                                runtime_section_caption(NO_INSTALLED_AGENTS)
                                    .debug_selector(|| "AGENTS_INSTALLED_EMPTY".into()),
                            )
                        })
                        .children(installed.into_iter().map(|runtime| {
                            SettingsCard::new([self.render_runtime_row(runtime, cx)])
                        })),
                )
                .children(if not_installed.is_empty() {
                    (!status.shell.checking).then(|| {
                        runtime_section_caption(ALL_AGENTS_INSTALLED)
                            .debug_selector(|| "AGENTS_NOT_INSTALLED_EMPTY".into())
                    })
                } else {
                    Some(
                        div()
                            .debug_selector(|| "AGENTS_NOT_INSTALLED".into())
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        runtime_section_header(
                                            "Not installed",
                                            Some(not_installed.len()),
                                        )
                                        .debug_selector(|| "AGENTS_NOT_INSTALLED_HEADER".into()),
                                    )
                                    .child(runtime_section_caption(BROWSE_EXECUTABLE_HINT)),
                            )
                            .children(not_installed.into_iter().map(|runtime| {
                                SettingsCard::new([self.render_not_installed_row(runtime, cx)])
                            })),
                    )
                })
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.render_installed_header(None, cx))
                .children(
                    runner_backend::ops::runtime::runtime_list()
                        .into_iter()
                        .map(|_| {
                            SettingsCard::new([div()
                                .h(rems(160. / 16.))
                                .bg(theme::with_alpha(theme::raised(), 0.2))
                                .into_any_element()])
                        }),
                )
        };
        let retry = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                "Agents",
                "Discover, enable, and override built-in agent executables.",
            ))
            .child(sections)
            .child(
                div()
                    .text_size(theme::text_ui())
                    .line_height(rems(18. / 16.))
                    .text_color(theme::faint())
                    .child("Disabled agents stay configured but are hidden from agent pickers. Overrides apply to new sessions that use the agent's default command; roles with a custom command keep it."),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.3))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_4()
                    .py_3()
                    .text_size(theme::text_ui())
                    .text_color(theme::danger())
                    .child(div().min_w(px(0.)).child(error))
                    .child(
                        Button::new("agents-retry", "Retry")
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_press(move |_, cx| {
                                retry.update(cx, |this, pane_cx| this.refresh(pane_cx));
                            }),
                    )
            }))
    }
}

fn partition_runtimes(
    runtimes: &[RuntimeExecutableStatus],
) -> (Vec<&RuntimeExecutableStatus>, Vec<&RuntimeExecutableStatus>) {
    runtimes.iter().partition(|runtime| match runtime.state {
        RuntimeRowState::Detected
        | RuntimeRowState::Override
        | RuntimeRowState::Checking
        | RuntimeRowState::ProbeTimedOut => true,
        RuntimeRowState::NotFound | RuntimeRowState::InvalidOverride => false,
    })
}

fn runtime_default_enabled(catalog: &[RuntimeCatalogEntry], name: Runtime) -> Option<bool> {
    catalog
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.default_enabled)
}

fn runtime_default_eligible(
    runtime: &RuntimeExecutableStatus,
    catalog: &[RuntimeCatalogEntry],
    settings: &AppSettings,
) -> bool {
    matches!(
        runtime.effective_source,
        Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
    ) && runtime_default_enabled(catalog, runtime.name)
        .is_some_and(|default| settings.is_agent_enabled(runtime.name, default))
}

fn reconcile_default_runtime(
    settings: &mut AppSettings,
    status: &RuntimeStatusResponse,
    catalog: &[RuntimeCatalogEntry],
) -> bool {
    if settings.default_runtime.is_empty() {
        return false;
    }
    let enabled = Runtime::parse(&settings.default_runtime)
        .and_then(|runtime| {
            runtime_default_enabled(catalog, runtime)
                .map(|default| settings.is_agent_enabled(runtime, default))
        })
        .unwrap_or(false);
    let available = status.runtimes.iter().any(|runtime| {
        runtime.name.key() == settings.default_runtime
            && matches!(
                runtime.effective_source,
                Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
            )
    });
    if !enabled || (!status.shell.checking && !available) {
        settings.default_runtime.clear();
        true
    } else {
        false
    }
}

fn update_agent_enabled_preferences(
    settings: &mut AppSettings,
    runtime: Runtime,
    enabled: bool,
) -> bool {
    let before = (
        settings.default_runtime.clone(),
        settings.disabled_agents.clone(),
        settings.enabled_agents.clone(),
    );
    if enabled {
        settings.disabled_agents.remove(runtime.key());
        settings.enabled_agents.insert(runtime.to_string());
    } else {
        settings.disabled_agents.insert(runtime.to_string());
        settings.enabled_agents.remove(runtime.key());
        if settings.default_runtime == runtime.key() {
            settings.default_runtime.clear();
        }
    }
    before
        != (
            settings.default_runtime.clone(),
            settings.disabled_agents.clone(),
            settings.enabled_agents.clone(),
        )
}

fn update_default_runtime(settings: &mut AppSettings, runtime: Option<Runtime>) -> bool {
    if settings.default_runtime == runtime.map(Runtime::key).unwrap_or_default() {
        return false;
    }
    settings.default_runtime = runtime.map(Runtime::key).unwrap_or_default().to_owned();
    true
}

fn override_validation_error_message(error: &OverrideValidationError) -> String {
    error.message.clone()
}

fn runtime_defaults_visible(runtime: &RuntimeExecutableStatus) -> bool {
    runtime.state != RuntimeRowState::NotFound
}

fn runtime_default_value(value: Option<&str>) -> String {
    value.unwrap_or("runtime default").to_owned()
}

fn runtime_presentation(
    runtime: &RuntimeExecutableStatus,
    probe_outcome: Option<DiscoveryOutcome>,
    validation: Option<&str>,
    saving: bool,
) -> RuntimePresentation {
    let state = if validation.is_some() {
        RuntimeRowState::InvalidOverride
    } else {
        runtime.state
    };
    let (badge, tone, spinning) = if saving {
        ("Saving…", BadgeTone::Muted, true)
    } else {
        match state {
            RuntimeRowState::Detected => ("Detected", BadgeTone::Accent, false),
            RuntimeRowState::Override => ("Override", BadgeTone::Neutral, false),
            RuntimeRowState::NotFound => ("Not found", BadgeTone::Danger, false),
            RuntimeRowState::Checking => ("Checking…", BadgeTone::Muted, true),
            RuntimeRowState::ProbeTimedOut => (
                if probe_outcome == Some(DiscoveryOutcome::Timeout) {
                    "Probe timed out"
                } else {
                    "Detection failed"
                },
                BadgeTone::Warning,
                false,
            ),
            RuntimeRowState::InvalidOverride => ("Invalid", BadgeTone::Danger, false),
        }
    };
    let caption = if let Some(validation) = validation {
        Some(validation.to_owned())
    } else {
        match state {
            RuntimeRowState::Override => Some(runtime.detected_path.as_ref().map_or_else(
                || format!("{} was not found automatically.", runtime.command),
                |path| format!("Detected: {path}"),
            )),
            RuntimeRowState::NotFound => Some(format!(
                "Install {} or set an explicit executable path.",
                runtime.display_name
            )),
            RuntimeRowState::ProbeTimedOut => {
                let failure = match probe_outcome {
                    Some(DiscoveryOutcome::Timeout) => "Login shell timed out",
                    Some(DiscoveryOutcome::WindowsRegistryError) => "Windows PATH refresh failed",
                    _ => "Shell detection failed",
                };
                Some(runtime.detected_path.as_ref().map_or_else(
                    || format!("{failure}. Refresh or set an explicit executable path."),
                    |path| format!("{failure} — using the last resolved path: {path}"),
                ))
            }
            RuntimeRowState::Checking if runtime.detected_path.is_some() => runtime
                .detected_path
                .as_ref()
                .map(|path| format!("Using last detected path: {path}")),
            _ => None,
        }
    };
    let version = validation
        .is_none()
        .then(|| version_caption(runtime))
        .flatten();
    let caption = match (version, caption) {
        (Some(version), Some(caption)) => Some(format!("{version} · {caption}")),
        (Some(version), None) => Some(match &runtime.detected_path {
            Some(path) if state == RuntimeRowState::Detected => {
                format!("{version} · Detected: {path}")
            }
            _ => version,
        }),
        (None, caption) => caption,
    };
    let auto_path = if runtime.state == RuntimeRowState::Checking {
        "Auto — detecting…".to_owned()
    } else if let Some(path) = &runtime.detected_path {
        format!("Auto — {path}")
    } else {
        format!("Auto — {} not found on PATH", runtime.command)
    };
    RuntimePresentation {
        badge,
        tone,
        spinning,
        caption,
        auto_path,
        show_reset: runtime.override_path.is_some(),
    }
}

/// `2.1.266`, or `0.153.4 → 0.155.0` while an update is available.
fn version_caption(runtime: &RuntimeExecutableStatus) -> Option<String> {
    let installed = runtime.installed_version.as_deref()?;
    Some(match runtime.available_version.as_deref() {
        Some(available) => format!("{installed} → {available}"),
        None => installed.to_owned(),
    })
}

/// Update is offered only while npm has a newer version, and never for a
/// runtime without an update command. Where the platform locks running
/// executables it is disabled while any session of the agent is alive;
/// elsewhere running sessions only earn a caption.
fn update_action(
    runtime: &RuntimeExecutableStatus,
    running: usize,
    needs_stopped_sessions: bool,
) -> Option<UpdateAction> {
    let installed = runtime.installed_version.as_deref()?;
    runtime.available_version.as_ref()?;
    let name = &runtime.display_name;
    let caption = match (running, needs_stopped_sessions) {
        (0, _) => None,
        (1, true) => Some(format!("Stop the running {name} session first.")),
        (running, true) => Some(format!("Stop the {running} running {name} sessions first.")),
        (1, false) => Some(format!(
            "1 running {name} session keeps {installed} until it relaunches."
        )),
        (running, false) => Some(format!(
            "{running} running {name} sessions keep {installed} until they relaunch."
        )),
    };
    Some(UpdateAction {
        disabled: needs_stopped_sessions && running > 0,
        caption,
    })
}

fn runtime_badge(runtime: Runtime, presentation: &RuntimePresentation) -> AnyElement {
    let (background, foreground) = match presentation.tone {
        BadgeTone::Accent => (theme::with_alpha(theme::accent(), 0.1), theme::accent()),
        BadgeTone::Neutral => (theme::with_alpha(theme::text(), 0.05), theme::text()),
        BadgeTone::Danger => (theme::with_alpha(theme::danger(), 0.1), theme::danger()),
        BadgeTone::Warning => (theme::with_alpha(theme::warning(), 0.1), theme::warning()),
        BadgeTone::Muted => (theme::with_alpha(theme::faint(), 0.1), theme::faint()),
    };
    div()
        .debug_selector(|| format!("AGENT_BADGE_{runtime}"))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_full()
        .bg(background)
        .px_2()
        .py(rems(2. / 16.))
        .text_size(theme::text_caption())
        .font_weight(FontWeight::MEDIUM)
        .text_color(foreground)
        .child(if presentation.spinning {
            spinner(
                SharedString::from(format!("runtime-{runtime}-spinner")),
                10.,
                foreground,
            )
        } else {
            div()
                .size(rems(6. / 16.))
                .rounded_full()
                .bg(foreground)
                .into_any_element()
        })
        .child(presentation.badge)
        .into_any_element()
}

fn shell_description(shell: Option<&ShellDiscoveryStatus>) -> String {
    let Some(shell) = shell else {
        return "Loading shell discovery status…".into();
    };
    let selected = shell.shell.as_deref();
    let duration = shell
        .duration_ms
        .map(|duration| format!(" in {:.1} s", duration as f64 / 1_000.))
        .unwrap_or_default();
    if shell.checking {
        #[cfg(windows)]
        if selected.is_none() {
            return if shell.using_last_known_good {
                "Refreshing Windows PATH; agents keep using the last saved environment.".into()
            } else {
                "Checking the Windows registry for PATH…".into()
            };
        }
        let Some(selected) = selected else {
            return "Checking login shell for PATH and proxy settings…".into();
        };
        return if shell.using_last_known_good {
            format!("Checking {selected}; agents keep using the last saved environment.")
        } else {
            format!("Checking {selected} for PATH and proxy settings…")
        };
    }
    match shell.outcome {
        Some(DiscoveryOutcome::Ok) => format!(
            "PATH captured from your login shell ({}){duration}. Spawned agents inherit it.",
            selected.unwrap_or("")
        ),
        Some(DiscoveryOutcome::WindowsRegistry) => format!(
            "PATH refreshed from the Windows registry{duration}. Spawned agents inherit it."
        ),
        Some(DiscoveryOutcome::WindowsRegistryError) => {
            "Could not read Windows PATH from the registry; refresh or set an override below."
                .into()
        }
        Some(DiscoveryOutcome::Timeout) => format!(
            "{} did not respond{duration}; agents keep using the last saved environment.",
            selected.unwrap_or("")
        ),
        Some(DiscoveryOutcome::SpawnError) => format!(
            "Could not start {}; refresh after fixing the login shell or set an override below.",
            selected.unwrap_or("")
        ),
        Some(DiscoveryOutcome::EmptyCapture) => format!(
            "{} returned no usable environment; refresh or set an override below.",
            selected.unwrap_or("")
        ),
        Some(DiscoveryOutcome::NoShell) => {
            "No supported login shell was configured; set an executable override below.".into()
        }
        None => selected.map_or_else(
            || "Waiting to check the login shell.".into(),
            |shell| format!("Waiting to check {shell}."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_backend::runtime_status::ShellDiscoveryStatus;

    #[test]
    fn no_shell_description_reports_missing_login_shell() {
        let shell = ShellDiscoveryStatus {
            shell: None,
            outcome: Some(DiscoveryOutcome::NoShell),
            duration_ms: None,
            checking: false,
            using_last_known_good: false,
            last_known_good_captured_at: None,
        };
        assert_eq!(
            shell_description(Some(&shell)),
            "No supported login shell was configured; set an executable override below."
        );
    }

    #[test]
    fn windows_registry_description_does_not_claim_a_shell_probe() {
        let shell = ShellDiscoveryStatus {
            shell: None,
            outcome: Some(DiscoveryOutcome::WindowsRegistry),
            duration_ms: Some(25),
            checking: false,
            using_last_known_good: false,
            last_known_good_captured_at: None,
        };
        assert_eq!(
            shell_description(Some(&shell)),
            "PATH refreshed from the Windows registry in 0.0 s. Spawned agents inherit it."
        );
    }

    #[test]
    fn windows_registry_error_description_does_not_claim_a_shell_failure() {
        let shell = ShellDiscoveryStatus {
            shell: None,
            outcome: Some(DiscoveryOutcome::WindowsRegistryError),
            duration_ms: Some(25),
            checking: false,
            using_last_known_good: true,
            last_known_good_captured_at: Some("saved".into()),
        };
        assert_eq!(
            shell_description(Some(&shell)),
            "Could not read Windows PATH from the registry; refresh or set an override below."
        );
    }

    fn runtime(state: RuntimeRowState) -> RuntimeExecutableStatus {
        RuntimeExecutableStatus {
            name: Runtime::Codex,
            display_name: "Codex".into(),
            command: "codex".into(),
            default_model: None,
            default_effort: None,
            detected_path: None,
            override_path: None,
            effective_command: None,
            effective_source: None,
            state,
            invalid_reason: None,
            installed_version: None,
            available_version: None,
        }
    }

    fn status(runtime: RuntimeExecutableStatus, checking: bool) -> RuntimeStatusResponse {
        RuntimeStatusResponse {
            shell: ShellDiscoveryStatus {
                shell: Some("/bin/zsh".into()),
                outcome: Some(DiscoveryOutcome::Ok),
                duration_ms: Some(25),
                checking,
                using_last_known_good: false,
                last_known_good_captured_at: None,
            },
            runtimes: vec![runtime],
        }
    }

    #[test]
    fn partitions_all_six_states_in_catalog_order() {
        let rows = [
            RuntimeRowState::NotFound,
            RuntimeRowState::Detected,
            RuntimeRowState::InvalidOverride,
            RuntimeRowState::Override,
            RuntimeRowState::Checking,
            RuntimeRowState::ProbeTimedOut,
        ]
        .map(runtime);
        let (installed, not_installed) = partition_runtimes(&rows);
        assert_eq!(
            installed.iter().map(|row| row.state).collect::<Vec<_>>(),
            [
                RuntimeRowState::Detected,
                RuntimeRowState::Override,
                RuntimeRowState::Checking,
                RuntimeRowState::ProbeTimedOut,
            ]
        );
        assert_eq!(
            not_installed
                .iter()
                .map(|row| row.state)
                .collect::<Vec<_>>(),
            [RuntimeRowState::NotFound, RuntimeRowState::InvalidOverride]
        );
    }

    #[test]
    fn partition_tracks_probe_and_override_transitions_without_stored_state() {
        let mut rows = [runtime(RuntimeRowState::NotFound)];
        for (state, installed_count) in [
            (RuntimeRowState::NotFound, 0),
            (RuntimeRowState::Checking, 1),
            (RuntimeRowState::Detected, 1),
            (RuntimeRowState::Checking, 1),
            (RuntimeRowState::ProbeTimedOut, 1),
            (RuntimeRowState::NotFound, 0),
            (RuntimeRowState::Override, 1),
            (RuntimeRowState::InvalidOverride, 0),
            (RuntimeRowState::NotFound, 0),
        ] {
            rows[0].state = state;
            let (installed, not_installed) = partition_runtimes(&rows);
            assert_eq!(installed.len(), installed_count, "{state:?}");
            assert_eq!(not_installed.len(), 1 - installed_count, "{state:?}");
            assert_eq!(
                runtime_presentation(&rows[0], None, None, false).spinning,
                state == RuntimeRowState::Checking,
            );
        }
    }

    #[test]
    fn derives_every_runtime_row_state() {
        let cases = [
            (RuntimeRowState::Detected, None, "Detected"),
            (RuntimeRowState::Override, None, "Override"),
            (RuntimeRowState::NotFound, None, "Not found"),
            (RuntimeRowState::Checking, None, "Checking…"),
            (
                RuntimeRowState::ProbeTimedOut,
                Some(DiscoveryOutcome::Timeout),
                "Probe timed out",
            ),
            (RuntimeRowState::InvalidOverride, None, "Invalid"),
        ];
        for (state, outcome, badge) in cases {
            assert_eq!(
                runtime_presentation(&runtime(state), outcome, None, false).badge,
                badge
            );
        }
        assert_eq!(
            runtime_presentation(
                &runtime(RuntimeRowState::Detected),
                None,
                Some("Not executable"),
                false,
            )
            .badge,
            "Invalid"
        );
        assert_eq!(
            runtime_presentation(
                &runtime(RuntimeRowState::ProbeTimedOut),
                Some(DiscoveryOutcome::WindowsRegistryError),
                None,
                false,
            )
            .caption
            .as_deref(),
            Some("Windows PATH refresh failed. Refresh or set an explicit executable path.")
        );
    }

    fn versioned(
        state: RuntimeRowState,
        installed: Option<&str>,
        available: Option<&str>,
    ) -> RuntimeExecutableStatus {
        let mut row = runtime(state);
        row.detected_path = Some("/Users/jason/.nvm/bin/codex".into());
        row.installed_version = installed.map(str::to_owned);
        row.available_version = available.map(str::to_owned);
        row
    }

    #[test]
    fn caption_leads_with_the_version_and_the_update_arrow() {
        let caption = |row: &RuntimeExecutableStatus, validation: Option<&str>| {
            runtime_presentation(row, Some(DiscoveryOutcome::Ok), validation, false).caption
        };
        assert_eq!(
            caption(
                &versioned(RuntimeRowState::Detected, Some("0.153.4"), None),
                None
            )
            .as_deref(),
            Some("0.153.4 · Detected: /Users/jason/.nvm/bin/codex")
        );
        assert_eq!(
            caption(
                &versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0")),
                None
            )
            .as_deref(),
            Some("0.153.4 → 0.155.0 · Detected: /Users/jason/.nvm/bin/codex")
        );
        assert_eq!(
            caption(
                &versioned(RuntimeRowState::Override, Some("0.155.1"), None),
                None
            )
            .as_deref(),
            Some("0.155.1 · Detected: /Users/jason/.nvm/bin/codex")
        );
        assert_eq!(
            caption(&versioned(RuntimeRowState::Detected, None, None), None),
            None
        );
        assert_eq!(
            caption(
                &versioned(RuntimeRowState::Detected, Some("0.153.4"), None),
                Some("Not an executable file.")
            )
            .as_deref(),
            Some("Not an executable file.")
        );
    }

    #[test]
    fn update_button_shows_only_with_a_newer_version_and_guards_by_platform() {
        let current = versioned(RuntimeRowState::Detected, Some("0.155.0"), None);
        assert_eq!(update_action(&current, 3, true), None);
        assert_eq!(
            update_action(&versioned(RuntimeRowState::Detected, None, None), 0, false),
            None
        );
        let mut trae = versioned(RuntimeRowState::Detected, Some("0.1.0"), None);
        trae.name = Runtime::Trae;
        trae.display_name = "TRAE CLI".into();
        assert_eq!(update_action(&trae, 0, false), None);

        let stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
        assert_eq!(
            update_action(&stale, 0, false),
            Some(UpdateAction {
                disabled: false,
                caption: None
            })
        );
        assert_eq!(
            update_action(&stale, 0, true),
            Some(UpdateAction {
                disabled: false,
                caption: None
            })
        );
        assert_eq!(
            update_action(&stale, 3, false),
            Some(UpdateAction {
                disabled: false,
                caption: Some("3 running Codex sessions keep 0.153.4 until they relaunch.".into()),
            })
        );
        assert_eq!(
            update_action(&stale, 1, false).and_then(|action| action.caption),
            Some("1 running Codex session keeps 0.153.4 until it relaunches.".into())
        );
        assert_eq!(
            update_action(&stale, 3, true),
            Some(UpdateAction {
                disabled: true,
                caption: Some("Stop the 3 running Codex sessions first.".into()),
            })
        );
        assert_eq!(
            update_action(&stale, 1, true).and_then(|action| action.caption),
            Some("Stop the running Codex session first.".into())
        );
    }

    #[test]
    fn update_button_sits_before_the_toggle_only_while_an_update_exists() {
        let mut cx = gpui::TestAppContext::single();
        let mut stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
        stale.effective_source = Some(RuntimeCommandSource::Detected);
        let (_temp, pane) = test_pane(vec![stale], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let update = window.debug_bounds("AGENT_UPDATE_codex").unwrap();
        let toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
        assert!(update.right() <= toggle.left());
        assert!(update.top() < toggle.bottom() && update.bottom() > toggle.top());
        assert!(window.debug_bounds("AGENT_UPDATE_CAPTION_codex").is_none());

        pane.update(&mut window, |pane, _, cx| {
            pane.live_sessions.insert(Runtime::Codex, 2);
            cx.notify();
        })
        .unwrap();
        window.run_until_parked();
        assert!(window.debug_bounds("AGENT_UPDATE_CAPTION_codex").is_some());

        // A fresh window: gpui keeps debug bounds of elements no longer drawn.
        let mut cx = gpui::TestAppContext::single();
        let mut current = versioned(RuntimeRowState::Detected, Some("0.155.0"), None);
        current.effective_source = Some(RuntimeCommandSource::Detected);
        let (_temp, pane) = test_pane(vec![current], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_some());
        assert!(window.debug_bounds("AGENT_UPDATE_codex").is_none());
    }

    #[test]
    fn formats_known_partial_and_unknown_runtime_defaults() {
        let mut runtime = runtime(RuntimeRowState::Detected);
        runtime.default_model = Some("claude-fable-5[1m]".into());
        runtime.default_effort = Some("xhigh".into());
        assert_eq!(
            runtime_default_value(runtime.default_model.as_deref()),
            "claude-fable-5[1m]"
        );
        assert_eq!(
            runtime_default_value(runtime.default_effort.as_deref()),
            "xhigh"
        );

        runtime.default_effort = None;
        assert_eq!(
            runtime_default_value(runtime.default_effort.as_deref()),
            "runtime default"
        );
    }

    fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
        use runner_backend::{
            db, event_bus, events, mcp, router, session, shell_path, windows, AppCore,
        };
        use std::sync::{Arc, Mutex, RwLock};
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&path.join("runner.db")).unwrap()),
            app_data_dir: path.into(),
            sessions: session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            usage: Arc::new(runner_backend::usage::UsageService::default()),
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        };
        cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                path.join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        })
    }

    fn test_pane(
        rows: Vec<RuntimeExecutableStatus>,
        checking: bool,
        cx: &mut gpui::TestAppContext,
    ) -> (tempfile::TempDir, gpui::WindowHandle<AgentsPane>) {
        test_pane_with_settings(rows, checking, AppSettings::default(), cx)
    }

    fn test_catalog() -> Vec<RuntimeCatalogEntry> {
        runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .map(|entry| RuntimeCatalogEntry {
                name: entry.name,
                display_name: entry.display_name,
                command: entry.command,
                native_fork: entry.native_fork,
                description: if entry.name == Runtime::Trae {
                    String::new()
                } else {
                    "Test agent description".into()
                },
                install_url: if entry.name == Runtime::Trae {
                    String::new()
                } else {
                    "https://example.com/install".into()
                },
                default_enabled: true,
                available: false,
                default_model: None,
                default_effort: None,
                models: Vec::new(),
                efforts: Vec::new(),
            })
            .collect()
    }

    fn test_pane_with_settings(
        rows: Vec<RuntimeExecutableStatus>,
        checking: bool,
        settings: AppSettings,
        cx: &mut gpui::TestAppContext,
    ) -> (tempfile::TempDir, gpui::WindowHandle<AgentsPane>) {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path(), cx);
        store.update(cx, |store, _| store.settings = settings);
        let pane = cx.add_window(|window, cx| {
            window.resize(gpui::size(px(1200.), px(1200.)));
            let mut pane = AgentsPane::new(WeakEntity::new_invalid(), store, window, cx);
            pane.catalog = test_catalog();
            let mut snapshot = status(runtime(RuntimeRowState::NotFound), checking);
            snapshot.runtimes = rows;
            pane.apply_status(snapshot, cx);
            pane
        });
        cx.run_until_parked();
        (temp, pane)
    }

    #[test]
    fn all_installed_collapses_not_installed_to_one_line() {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window.debug_bounds("AGENTS_INSTALLED_EMPTY").is_none());
        assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").is_none());
        let empty = window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").unwrap();
        assert!(empty.size.height <= px(18.));
        let installed = window.debug_bounds("AGENTS_INSTALLED").unwrap();
        let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
        assert!(installed.top() <= card.top() && installed.bottom() >= card.bottom());
        assert!(empty.top() >= installed.bottom());
    }

    #[test]
    fn checking_does_not_claim_all_agents_are_installed() {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Checking)], true, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").is_none());
        assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").is_none());
        let installed = window.debug_bounds("AGENTS_INSTALLED").unwrap();
        let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
        assert!(installed.top() <= card.top() && installed.bottom() >= card.bottom());
        assert!(window.debug_bounds("AGENT_BADGE_codex").is_some());
    }

    #[test]
    fn none_installed_invites_installation_above_all_missing_cards() {
        let mut cx = gpui::TestAppContext::single();
        let rows = runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .map(|entry| {
                let mut row = runtime(RuntimeRowState::NotFound);
                row.name = entry.name;
                row.display_name = entry.display_name;
                row.command = entry.command;
                row
            })
            .collect();
        let (_temp, pane) = test_pane(rows, false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let invitation = window.debug_bounds("AGENTS_INSTALLED_EMPTY").unwrap();
        let header = window.debug_bounds("AGENTS_NOT_INSTALLED_HEADER").unwrap();
        assert!(invitation.bottom() < header.top());
        assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").is_none());
        let mut previous_bottom = header.bottom();
        for selector in [
            "AGENT_CARD_codex",
            "AGENT_CARD_claude-code",
            "AGENT_CARD_copilot",
            "AGENT_CARD_pi",
            "AGENT_CARD_trae",
            "AGENT_CARD_antigravity",
        ] {
            let card = window.debug_bounds(selector).unwrap();
            assert!(card.top() > previous_bottom, "{selector}");
            previous_bottom = card.bottom();
        }
    }

    #[test]
    fn install_link_shares_the_description_line_and_both_are_omitted_when_empty() {
        let mut cx = gpui::TestAppContext::single();
        let rows = runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .filter(|entry| matches!(entry.name, Runtime::Codex | Runtime::Trae))
            .map(|entry| {
                let mut row = runtime(RuntimeRowState::NotFound);
                row.name = entry.name;
                row.display_name = entry.display_name;
                row.command = entry.command;
                row
            })
            .collect();
        let (_temp, pane) = test_pane(rows, false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let description = window.debug_bounds("AGENT_DESCRIPTION_codex").unwrap();
        let install = window.debug_bounds("AGENT_INSTALL_codex").unwrap();
        assert!(install.left() > description.right());
        assert!(install.top() < description.bottom() && install.bottom() > description.top());
        assert!(window.debug_bounds("AGENT_CARD_trae").is_some());
        assert!(window.debug_bounds("AGENT_DESCRIPTION_trae").is_none());
        assert!(window.debug_bounds("AGENT_INSTALL_trae").is_none());
    }

    #[test]
    fn invalid_override_keeps_validation_and_moves_after_a_valid_override() {
        let mut row = runtime(RuntimeRowState::InvalidOverride);
        row.override_path = Some("/missing/codex".into());
        row.invalid_reason = Some("Codex executable does not exist: /missing/codex".into());
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![row], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window.debug_bounds("AGENT_VALIDATION_codex").is_some());
        assert!(window.debug_bounds("AGENT_INSTALL_codex").is_some());
        assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_none());
        assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_none());
        assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none());
        assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
        pane.update(&mut window, |pane, _, cx| {
            let mut row = runtime(RuntimeRowState::Override);
            row.override_path = Some("/valid/codex".into());
            row.effective_source = Some(RuntimeCommandSource::Override);
            pane.apply_status(status(row, false), cx);
            assert!(pane.validation.is_empty());
            cx.notify();
        })
        .unwrap();
        window.run_until_parked();
        assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_some());
        assert!(window.debug_bounds("AGENT_TOGGLE_codex").is_some());
        assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_some());
        assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
        assert!(window.debug_bounds("AGENTS_NOT_INSTALLED_EMPTY").is_some());
        let installed = window.debug_bounds("AGENTS_INSTALLED").unwrap();
        let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
        assert!(installed.top() <= card.top() && installed.bottom() >= card.bottom());
        pane.update(&mut window, |pane, _, cx| {
            pane.apply_status(status(runtime(RuntimeRowState::NotFound), false), cx);
            cx.notify();
        })
        .unwrap();
        window.run_until_parked();
        let not_installed = window.debug_bounds("AGENTS_NOT_INSTALLED").unwrap();
        let card = window.debug_bounds("AGENT_CARD_codex").unwrap();
        assert!(not_installed.top() <= card.top() && not_installed.bottom() >= card.bottom());
    }

    #[test]
    fn runtime_cards_show_model_effort_without_mcp_at_two_rem_sizes() {
        use gpui::{size, TestAppContext, VisualTestContext};

        struct Host(Entity<AgentsPane>);
        impl Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().p_6().child(self.0.clone())
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let host = cx.add_window(|window, cx| {
            window.resize(size(px(1200.), px(900.)));
            let pane = cx.new(|cx| AgentsPane::new(WeakEntity::new_invalid(), store, window, cx));
            pane.update(cx, |pane, cx| {
                let mut claude = runtime(RuntimeRowState::Detected);
                claude.name = Runtime::ClaudeCode;
                claude.display_name = "Claude Code".into();
                claude.command = "claude".into();
                claude.default_model = Some("claude-fable-5[1m]".into());
                claude.default_effort = Some("xhigh".into());
                let mut status = status(claude, false);
                status.runtimes.push(runtime(RuntimeRowState::NotFound));
                pane.apply_status(status, cx);
            });
            Host(pane)
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(host.into(), &cx);
        for rem in [16., 20.8] {
            host.update(&mut window, |_, window, _| {
                window.set_rem_size(px(rem));
                window.refresh();
            })
            .unwrap();
            window.run_until_parked();
            let claude = window.debug_bounds("AGENT_CARD_claude-code").unwrap();
            let codex = window.debug_bounds("AGENT_CARD_codex").unwrap();
            assert!(
                claude.bottom() + px(rem) <= codex.top(),
                "{rem}: {claude:?} {codex:?}"
            );

            let model = window.debug_bounds("AGENT_MODEL_LINE_claude-code").unwrap();
            assert!(model.bottom() <= claude.bottom());
            assert!(window.debug_bounds("AGENT_MCP_LINE_claude-code").is_none());
            assert!(window.debug_bounds("AGENT_MCP_LINE_codex").is_none());
            assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_none());
        }
    }

    #[test]
    fn hides_runtime_defaults_when_the_runtime_is_not_found() {
        assert!(runtime_defaults_visible(&runtime(
            RuntimeRowState::Detected
        )));
        assert!(runtime_defaults_visible(&runtime(
            RuntimeRowState::Checking
        )));
        assert!(!runtime_defaults_visible(&runtime(
            RuntimeRowState::NotFound
        )));
    }

    #[test]
    fn default_eligibility_requires_enabled_sources() {
        let catalog = test_catalog();
        let mut settings = AppSettings::default();
        settings.disabled_agents.insert("codex".into());
        for state in [
            RuntimeRowState::Detected,
            RuntimeRowState::Override,
            RuntimeRowState::Checking,
            RuntimeRowState::ProbeTimedOut,
            RuntimeRowState::NotFound,
            RuntimeRowState::InvalidOverride,
        ] {
            let mut row = runtime(state);
            assert!(!runtime_default_eligible(
                &row,
                &catalog,
                &AppSettings::default()
            ));
            row.effective_source = Some(RuntimeCommandSource::Detected);
            assert!(runtime_default_eligible(
                &row,
                &catalog,
                &AppSettings::default()
            ));
            assert!(!runtime_default_eligible(&row, &catalog, &settings));
        }
    }

    #[test]
    fn card_actions_pin_and_switch_persisted_default() {
        let mut codex = runtime(RuntimeRowState::Detected);
        codex.effective_source = Some(RuntimeCommandSource::Detected);
        let mut claude = codex.clone();
        claude.name = Runtime::ClaudeCode;
        claude.display_name = "Claude Code".into();
        let mut cx = gpui::TestAppContext::single();
        let (temp, pane) = test_pane(vec![codex, claude], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        assert!(window.debug_bounds("AGENT_DEFAULT_codex").is_none());
        assert!(window.debug_bounds("AGENT_ENABLED_codex").is_none());
        assert!(window.debug_bounds("AGENT_BADGE_codex").is_none());
        assert!(window.debug_bounds("AGENT_BADGE_claude-code").is_none());
        for (selector, expected) in [
            ("AGENT_SET_DEFAULT_codex", "codex"),
            ("AGENT_SET_DEFAULT_claude-code", "claude-code"),
        ] {
            let button = window.debug_bounds(selector).unwrap();
            window.simulate_click(button.center(), gpui::Modifiers::default());
            window.run_until_parked();
            pane.update(&mut window, |pane, _, cx| {
                assert_eq!(pane.app_store.read(cx).settings.default_runtime, expected);
            })
            .unwrap();
            assert_eq!(
                AppSettings::load(&temp.path().join("settings.json"))
                    .unwrap()
                    .default_runtime,
                expected
            );
            let (marker_selector, identity_selector) = if expected == "codex" {
                ("AGENT_DEFAULT_codex", "AGENT_IDENTITY_codex")
            } else {
                ("AGENT_DEFAULT_claude-code", "AGENT_IDENTITY_claude-code")
            };
            let marker = window.debug_bounds(marker_selector).unwrap();
            let identity = window.debug_bounds(identity_selector).unwrap();
            assert!(
                identity.left() <= marker.left()
                    && identity.right() >= marker.right()
                    && identity.top() <= marker.top()
                    && identity.bottom() >= marker.bottom()
            );
        }
    }

    #[test]
    fn checking_disables_default_action_and_preserves_explicit_marker() {
        for explicit in [false, true] {
            let mut row = runtime(RuntimeRowState::Checking);
            row.effective_source = Some(RuntimeCommandSource::Detected);
            let mut cx = gpui::TestAppContext::single();
            let (_temp, pane) = test_pane_with_settings(
                vec![row],
                true,
                AppSettings {
                    default_runtime: if explicit { "codex" } else { "" }.into(),
                    ..AppSettings::default()
                },
                &mut cx,
            );
            let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
            assert!(window.debug_bounds("AGENT_BADGE_codex").is_some());
            if explicit {
                assert!(window.debug_bounds("AGENT_DEFAULT_codex").is_some());
                assert!(window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none());
            } else {
                let button = window.debug_bounds("AGENT_SET_DEFAULT_codex").unwrap();
                window.simulate_click(button.center(), gpui::Modifiers::default());
                window.run_until_parked();
            }
            pane.update(&mut window, |pane, _, cx| {
                assert_eq!(
                    pane.app_store.read(cx).settings.default_runtime,
                    if explicit { "codex" } else { "" }
                );
                pane.apply_status(status(runtime(RuntimeRowState::NotFound), false), cx);
                assert!(pane.app_store.read(cx).settings.default_runtime.is_empty());
            })
            .unwrap();
        }
    }

    #[test]
    fn unavailable_or_disabled_agents_have_no_default_action() {
        for (state, source, disabled) in [
            (RuntimeRowState::NotFound, None, false),
            (
                RuntimeRowState::InvalidOverride,
                Some(RuntimeCommandSource::Detected),
                false,
            ),
            (RuntimeRowState::Checking, None, false),
            (
                RuntimeRowState::ProbeTimedOut,
                Some(RuntimeCommandSource::Catalog),
                false,
            ),
            (
                RuntimeRowState::Detected,
                Some(RuntimeCommandSource::Detected),
                true,
            ),
        ] {
            let mut row = runtime(state);
            row.effective_source = source;
            let mut settings = AppSettings::default();
            if disabled {
                settings.disabled_agents.insert("codex".into());
            }
            let mut cx = gpui::TestAppContext::single();
            let (_temp, pane) = test_pane_with_settings(
                vec![row],
                state == RuntimeRowState::Checking,
                settings,
                &mut cx,
            );
            let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
            assert!(
                window.debug_bounds("AGENT_SET_DEFAULT_codex").is_none(),
                "{state:?}"
            );
            assert!(
                window.debug_bounds("AGENT_DEFAULT_codex").is_none(),
                "{state:?}"
            );
        }
    }

    #[test]
    fn installed_header_wraps_without_overlapping_actions() {
        let mut row = runtime(RuntimeRowState::Override);
        row.name = Runtime::Copilot;
        row.display_name = "GitHub Copilot CLI".into();
        row.effective_source = Some(RuntimeCommandSource::Override);
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![row], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        for rem in [16., 20.8] {
            for width in [320., 480., 760.] {
                pane.update(&mut window, |_, window, _| {
                    window.resize(gpui::size(px(width), px(1200.)));
                    window.set_rem_size(px(rem));
                    window.refresh();
                })
                .unwrap();
                window.run_until_parked();
                let header = window.debug_bounds("AGENT_HEADER_copilot").unwrap();
                let identity = window.debug_bounds("AGENT_IDENTITY_copilot").unwrap();
                let actions = window.debug_bounds("AGENT_ACTIONS_copilot").unwrap();
                for bounds in [identity, actions] {
                    assert!(
                        header.left() <= bounds.left()
                            && header.right() >= bounds.right()
                            && header.top() <= bounds.top()
                            && header.bottom() >= bounds.bottom(),
                        "width {width}, rem {rem}: {header:?} {bounds:?}"
                    );
                }
                assert!(
                    identity.right() <= actions.left() || identity.bottom() <= actions.top(),
                    "width {width}, rem {rem}: {identity:?} {actions:?}"
                );
                let button = window.debug_bounds("AGENT_SET_DEFAULT_copilot").unwrap();
                let toggle = window.debug_bounds("AGENT_TOGGLE_copilot").unwrap();
                assert!(
                    identity.left() <= button.left()
                        && identity.right() >= button.right()
                        && identity.top() <= button.top()
                        && identity.bottom() >= button.bottom()
                );
                assert!(button.right() < toggle.left() || button.bottom() <= toggle.top());
            }
        }
    }

    #[test]
    fn agent_preferences_persist_explicit_enable_disable_and_clear_default() {
        let mut settings = AppSettings {
            default_runtime: "codex".into(),
            ..AppSettings::default()
        };
        assert!(update_agent_enabled_preferences(
            &mut settings,
            Runtime::Codex,
            false
        ));
        assert!(settings.disabled_agents.contains(Runtime::Codex.key()));
        assert!(!settings.enabled_agents.contains(Runtime::Codex.key()));
        assert!(settings.default_runtime.is_empty());
        assert!(update_agent_enabled_preferences(
            &mut settings,
            Runtime::Codex,
            true
        ));
        assert!(!settings.disabled_agents.contains(Runtime::Codex.key()));
        assert!(settings.enabled_agents.contains(Runtime::Codex.key()));
        assert!(update_default_runtime(&mut settings, Some(Runtime::Codex)));
        assert_eq!(settings.default_runtime, "codex");
        assert!(!update_default_runtime(&mut settings, Some(Runtime::Codex)));
    }

    #[test]
    fn unavailable_default_waits_for_discovery_then_clears() {
        let catalog = vec![RuntimeCatalogEntry {
            name: Runtime::Codex,
            display_name: "Codex".into(),
            command: "codex".into(),
            native_fork: true,
            description: String::new(),
            install_url: String::new(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: Vec::new(),
            efforts: Vec::new(),
        }];
        let mut settings = AppSettings {
            default_runtime: "codex".into(),
            ..AppSettings::default()
        };
        assert!(!reconcile_default_runtime(
            &mut settings,
            &status(runtime(RuntimeRowState::Checking), true),
            &catalog,
        ));
        assert!(reconcile_default_runtime(
            &mut settings,
            &status(runtime(RuntimeRowState::NotFound), false),
            &catalog,
        ));
        assert!(settings.default_runtime.is_empty());
    }

    #[test]
    fn override_validation_preserves_backend_copy() {
        let error = OverrideValidationError {
            code: "not_executable".into(),
            message: "Codex executable is not executable: /tmp/codex".into(),
        };
        assert_eq!(
            override_validation_error_message(&error),
            "Codex executable is not executable: /tmp/codex"
        );
    }
}

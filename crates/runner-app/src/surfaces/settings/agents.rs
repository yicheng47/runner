use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, Entity, FontWeight, PathPromptOptions, Render,
    SharedString, Subscription, WeakEntity, Window,
};
use runner_app::ui::button::spinner;
use runner_app::ui::{
    BrowseField, Button, ButtonSize, ButtonVariant, FieldValidation, PaneHeader, SelectHandler,
    SelectOption, SettingsCard, SettingsRow, StyledSelect, TextField, Toggle,
};
use runner_backend::ops::runtime::RuntimeCatalogEntry;
use runner_backend::runtime_status::{
    OverrideValidationError, RuntimeCommandSource, RuntimeExecutableStatus, RuntimeRowState,
    RuntimeStatusResponse, ShellDiscoveryStatus,
};
use runner_backend::shell_path::DiscoveryOutcome;

use crate::app_settings::AppSettings;
use crate::app_store::AppStore;
use crate::theme;
use crate::NativeRoot;

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

pub(crate) struct AgentsPane {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    status: Option<RuntimeStatusResponse>,
    catalog: Vec<RuntimeCatalogEntry>,
    error: Option<String>,
    loading: bool,
    refreshing: bool,
    default_runtime: Entity<StyledSelect>,
    overrides: BTreeMap<String, Entity<TextField>>,
    validation: BTreeMap<String, String>,
    validation_drafts: BTreeMap<String, String>,
    saving: HashSet<String>,
    focused: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl AgentsPane {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pane = cx.weak_entity();
        let default_handler: SelectHandler = Rc::new(move |value, _, cx| {
            let _ = pane.update(cx, |this, pane_cx| this.set_default_runtime(value, pane_cx));
        });
        let default_value = app_store.read(cx).settings.default_runtime.clone();
        let default_runtime = cx.new(|select_cx| {
            StyledSelect::new(
                "agents-default-runtime",
                select_cx.focus_handle(),
                default_value,
                vec![SelectOption::new("", "First available")],
                default_handler,
                select_cx,
            )
        });

        let mut overrides = BTreeMap::new();
        let mut subscriptions = Vec::new();
        for runtime in runner_backend::ops::runtime::runtime_list() {
            let runtime_name = runtime.name.clone();
            let enter_shell = shell.clone();
            let escape_pane = cx.weak_entity();
            let escape_name = runtime_name.clone();
            let field = cx.new(|input_cx| {
                TextField::new(
                    input_cx.focus_handle(),
                    "",
                    format!("Auto — {} not found on PATH", runtime.command),
                    true,
                )
                .text_size(11.)
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
                            let runtime = escape_name.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = pane.update(cx, |this, pane_cx| {
                                    this.discard_override(&runtime, pane_cx)
                                });
                            });
                            true
                        }
                        _ => false,
                    }
                }))
            });
            let focus = field.read(cx).focus_handle();
            let focus_runtime = runtime_name.clone();
            subscriptions.push(cx.on_focus_in(&focus, window, move |this, _, cx| {
                this.focused.insert(focus_runtime.clone());
                cx.notify();
            }));
            let blur_runtime = runtime_name.clone();
            subscriptions.push(cx.on_focus_out(&focus, window, move |this, _, _, cx| {
                this.focused.remove(&blur_runtime);
                this.commit_override(&blur_runtime, cx);
            }));
            let draft_runtime = runtime_name.clone();
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
            this.sync_default_control(cx);
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
            default_runtime,
            overrides,
            validation: BTreeMap::new(),
            validation_drafts: BTreeMap::new(),
            saving: HashSet::new(),
            focused: HashSet::new(),
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
            Ok::<_, String>((status, catalog))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
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
        let task = cx.background_spawn(async move {
            let status = runner_backend::ops::runtime::runtime_refresh(&core)
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
                    self.validation.insert(runtime.name.clone(), error);
                    self.validation_drafts.insert(
                        runtime.name.clone(),
                        runtime.override_path.clone().unwrap_or_default(),
                    );
                } else {
                    self.validation.remove(&runtime.name);
                    self.validation_drafts.remove(&runtime.name);
                }
            }
        }
        self.status = Some(status);
        self.sync_default_control(cx);
    }

    fn sync_default_control(&mut self, cx: &mut Context<Self>) {
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
        let settings = self.app_store.read(cx).settings.clone();
        let options = available_runtime_options(status, &self.catalog, &settings);
        self.default_runtime.update(cx, |select, select_cx| {
            select.set_options(options, select_cx);
            select.set_value(settings.default_runtime, select_cx);
        });
    }

    fn set_default_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        let value = value.trim().to_owned();
        let changed = self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| update_default_runtime(settings, &value),
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

    fn set_enabled(&mut self, runtime: String, enabled: bool, cx: &mut Context<Self>) {
        let changed = self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| update_agent_enabled_preferences(settings, &runtime, enabled),
                true,
                store_cx,
            )
        });
        if !changed {
            return;
        }
        self.sync_default_control(cx);
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            if let Some(shell) = shell.upgrade() {
                shell.update(cx, |shell, shell_cx| {
                    shell.refresh_start_chat_runtimes(shell_cx);
                    shell.refresh_runner_form_runtimes(shell_cx);
                });
            }
        });
        cx.notify();
    }

    fn set_validation(
        &mut self,
        runtime: &str,
        validation: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = validation.clone() {
            self.validation.insert(runtime.to_owned(), validation);
            let draft = self
                .overrides
                .get(runtime)
                .map(|field| field.read(cx).text().to_owned())
                .unwrap_or_default();
            self.validation_drafts.insert(runtime.to_owned(), draft);
        } else {
            self.validation.remove(runtime);
            self.validation_drafts.remove(runtime);
        }
        if let Some(field) = self.overrides.get(runtime).cloned() {
            field.update(cx, |field, field_cx| {
                field.set_validation(
                    validation.map(FieldValidation::error).unwrap_or_default(),
                    field_cx,
                )
            });
        }
    }

    fn commit_override(&mut self, runtime: &str, cx: &mut Context<Self>) {
        if self.saving.contains(runtime) {
            return;
        }
        let Some(field) = self.overrides.get(runtime).cloned() else {
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
        self.saving.insert(runtime.to_owned());
        self.set_validation(runtime, None, cx);
        field.update(cx, |field, field_cx| field.set_disabled(true, field_cx));
        let core = self.app_store.read(cx).core.clone();
        let runtime_name = runtime.to_owned();
        let task_runtime = runtime_name.clone();
        let task = cx.background_spawn(async move {
            if draft.is_empty() {
                runner_backend::ops::runtime::runtime_clear_override(&core, &task_runtime)
                    .map_err(|error| error.to_string())
            } else {
                runner_backend::ops::runtime::runtime_set_override(&core, &task_runtime, &draft)
                    .map_err(|error| override_validation_error_message(&error))
            }
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
                    Err(error) => this.set_validation(&runtime_name, Some(error), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn discard_override(&mut self, runtime: &str, cx: &mut Context<Self>) {
        let current = self
            .status
            .as_ref()
            .and_then(|status| status.runtimes.iter().find(|row| row.name == runtime));
        let value = current
            .and_then(|row| row.override_path.clone())
            .unwrap_or_default();
        let validation = current.and_then(|row| row.invalid_reason.clone());
        if let Some(field) = self.overrides.get(runtime).cloned() {
            field.update(cx, |field, field_cx| field.reset(value, field_cx));
        }
        self.set_validation(runtime, validation, cx);
        cx.notify();
    }

    fn browse_override(&mut self, runtime: String, cx: &mut Context<Self>) {
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
            .unwrap_or_else(|| runtime.clone());
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
                            this.commit_override(&runtime, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.set_validation(&runtime, Some(error), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_override(&mut self, runtime: String, cx: &mut Context<Self>) {
        if self.saving.contains(&runtime) {
            return;
        }
        if let Some(field) = self.overrides.get(&runtime).cloned() {
            field.update(cx, |field, field_cx| field.reset("", field_cx));
        }
        self.commit_override(&runtime, cx);
    }

    fn render_shell_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let checking = self.refreshing
            || self
                .status
                .as_ref()
                .is_some_and(|status| status.shell.checking);
        let pane = cx.entity();
        div()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_5()
            .py_4()
            .flex()
            .items_center()
            .justify_between()
            .gap_8()
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Shell environment"),
                    )
                    .child(
                        div()
                            .mt(rems(2. / 16.))
                            .text_size(rems(12. / 16.))
                            .line_height(rems(17.4 / 16.))
                            .text_color(theme::muted())
                            .child(shell_description(
                                self.status.as_ref().map(|status| &status.shell),
                            )),
                    ),
            )
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
            )
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
            runtime_default_enabled(&self.catalog, &runtime.name).is_some_and(|default_enabled| {
                self.app_store
                    .read(cx)
                    .settings
                    .is_agent_enabled(&runtime.name, default_enabled)
            });
        let pane = cx.entity();
        let toggle_pane = pane.clone();
        let browse_pane = pane.clone();
        let reset_pane = pane;
        let toggle_runtime = runtime.name.clone();
        let browse_runtime = runtime.name.clone();
        let reset_runtime = runtime.name.clone();
        let field = self.overrides.get(&runtime.name).cloned();
        let browse_id = SharedString::from(format!("agent-browse-{}", runtime.name));
        let browse_field = field.clone().map(|field| {
            BrowseField::new(
                field,
                saving,
                Rc::new(move |_, cx| {
                    browse_pane.update(cx, |this, pane_cx| {
                        this.browse_override(browse_runtime.clone(), pane_cx)
                    });
                }),
            )
            .browse_id(browse_id)
            .browse_label("Browse")
        });
        presentation.show_reset |= field
            .as_ref()
            .is_some_and(|field| !field.read(cx).text().trim().is_empty());
        div()
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
                        div()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(runtime.display_name.clone()),
                    )
                    .child(runtime_badge(&runtime.name, &presentation))
                    .child(div().min_w(px(0.)).flex_1())
                    .child(
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .child(if enabled { "Enabled" } else { "Disabled" }),
                    )
                    .child(
                        Toggle::new(
                            SharedString::from(format!("agent-toggle-{}", runtime.name)),
                            enabled,
                        )
                        .on_change(move |enabled, _, cx| {
                            toggle_pane.update(cx, |this, pane_cx| {
                                this.set_enabled(toggle_runtime.clone(), enabled, pane_cx)
                            });
                        }),
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
                            "Reset to auto",
                        )
                        .variant(ButtonVariant::Ghost)
                        .disabled(saving)
                        .on_press(move |_, cx| {
                            reset_pane.update(cx, |this, pane_cx| {
                                this.reset_override(reset_runtime.clone(), pane_cx)
                            });
                        })
                    })),
            )
            .children(presentation.caption.map(|caption| {
                div()
                    .font_family("JetBrains Mono")
                    .text_size(rems(11. / 16.))
                    .line_height(rems(15.4 / 16.))
                    .text_color(if validation.is_some() {
                        theme::danger()
                    } else {
                        theme::faint()
                    })
                    .child(caption)
            }))
            .into_any_element()
    }
}

impl Render for AgentsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .status
            .as_ref()
            .map(|status| {
                status
                    .runtimes
                    .iter()
                    .map(|runtime| self.render_runtime_row(runtime, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                runner_backend::ops::runtime::runtime_list()
                    .into_iter()
                    .map(|_| {
                        div()
                            .h(rems(108. / 16.))
                            .bg(theme::with_alpha(theme::raised(), 0.2))
                            .px_5()
                            .py_4()
                            .into_any_element()
                    })
                    .collect()
            });
        let retry = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                "Agents",
                "Discover, enable, and override built-in agent executables.",
            ))
            .child(SettingsCard::new(vec![SettingsRow::new(
                "Default agent",
                self.default_runtime.clone(),
            )
            .subtitle("Pre-selected when starting a direct chat in Direct mode.")
            .into_any_element()]))
            .child(self.render_shell_card(cx))
            .child(SettingsCard::new(rows))
            .child(
                div()
                    .text_size(rems(12. / 16.))
                    .line_height(rems(18. / 16.))
                    .text_color(theme::faint())
                    .child("Disabled agents stay configured but are hidden from agent pickers. Overrides apply to new sessions that use the agent's default command; runners with a custom command keep it."),
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
                    .text_size(rems(12. / 16.))
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

fn runtime_default_enabled(catalog: &[RuntimeCatalogEntry], name: &str) -> Option<bool> {
    catalog
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.default_enabled)
}

fn available_runtime_options(
    status: &RuntimeStatusResponse,
    catalog: &[RuntimeCatalogEntry],
    settings: &AppSettings,
) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new("", "First available")];
    options.extend(status.runtimes.iter().filter_map(|runtime| {
        let available = matches!(
            runtime.effective_source,
            Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
        );
        let enabled = runtime_default_enabled(catalog, &runtime.name)
            .is_some_and(|default| settings.is_agent_enabled(&runtime.name, default));
        (available && enabled)
            .then(|| SelectOption::new(runtime.name.clone(), runtime.display_name.clone()))
    }));
    options
}

fn reconcile_default_runtime(
    settings: &mut AppSettings,
    status: &RuntimeStatusResponse,
    catalog: &[RuntimeCatalogEntry],
) -> bool {
    if settings.default_runtime.is_empty() {
        return false;
    }
    let enabled = runtime_default_enabled(catalog, &settings.default_runtime)
        .is_some_and(|default| settings.is_agent_enabled(&settings.default_runtime, default));
    let available = status.runtimes.iter().any(|runtime| {
        runtime.name == settings.default_runtime
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
    runtime: &str,
    enabled: bool,
) -> bool {
    let before = (
        settings.default_runtime.clone(),
        settings.disabled_agents.clone(),
        settings.enabled_agents.clone(),
    );
    if enabled {
        settings.disabled_agents.remove(runtime);
        settings.enabled_agents.insert(runtime.to_owned());
    } else {
        settings.disabled_agents.insert(runtime.to_owned());
        settings.enabled_agents.remove(runtime);
        if settings.default_runtime == runtime {
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

fn update_default_runtime(settings: &mut AppSettings, runtime: &str) -> bool {
    if settings.default_runtime == runtime {
        return false;
    }
    settings.default_runtime = runtime.to_owned();
    true
}

fn override_validation_error_message(error: &OverrideValidationError) -> String {
    error.message.clone()
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
                let failure = if probe_outcome == Some(DiscoveryOutcome::Timeout) {
                    "Login shell timed out"
                } else {
                    "Shell detection failed"
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

fn runtime_badge(runtime: &str, presentation: &RuntimePresentation) -> AnyElement {
    let (background, foreground) = match presentation.tone {
        BadgeTone::Accent => (theme::with_alpha(theme::accent(), 0.1), theme::accent()),
        BadgeTone::Neutral => (theme::with_alpha(theme::text(), 0.05), theme::text()),
        BadgeTone::Danger => (theme::with_alpha(theme::danger(), 0.1), theme::danger()),
        BadgeTone::Warning => (theme::with_alpha(theme::warning(), 0.1), theme::warning()),
        BadgeTone::Muted => (theme::with_alpha(theme::faint(), 0.1), theme::faint()),
    };
    div()
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_full()
        .bg(background)
        .px_2()
        .py(rems(2. / 16.))
        .text_size(rems(10. / 16.))
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

    fn runtime(state: RuntimeRowState) -> RuntimeExecutableStatus {
        RuntimeExecutableStatus {
            name: "codex".into(),
            display_name: "Codex".into(),
            command: "codex".into(),
            detected_path: None,
            override_path: None,
            effective_command: None,
            effective_source: None,
            state,
            invalid_reason: None,
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
    }

    #[test]
    fn agent_preferences_persist_explicit_enable_disable_and_clear_default() {
        let mut settings = AppSettings {
            default_runtime: "codex".into(),
            ..AppSettings::default()
        };
        assert!(update_agent_enabled_preferences(
            &mut settings,
            "codex",
            false
        ));
        assert!(settings.disabled_agents.contains("codex"));
        assert!(!settings.enabled_agents.contains("codex"));
        assert!(settings.default_runtime.is_empty());
        assert!(update_agent_enabled_preferences(
            &mut settings,
            "codex",
            true
        ));
        assert!(!settings.disabled_agents.contains("codex"));
        assert!(settings.enabled_agents.contains("codex"));
        assert!(update_default_runtime(&mut settings, "codex"));
        assert_eq!(settings.default_runtime, "codex");
        assert!(!update_default_runtime(&mut settings, "codex"));
    }

    #[test]
    fn unavailable_default_waits_for_discovery_then_clears() {
        let catalog = vec![RuntimeCatalogEntry {
            name: "codex".into(),
            display_name: "Codex".into(),
            command: "codex".into(),
            description: String::new(),
            default_enabled: true,
            available: false,
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

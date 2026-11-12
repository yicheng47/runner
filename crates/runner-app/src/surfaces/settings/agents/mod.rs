mod launch;
mod presentation;
mod render;

use presentation::*;

use runner_core::protocol::model::Runtime;

use std::collections::{HashMap, HashSet};

use std::rc::Rc;

use gpui::prelude::*;

use gpui::{
    div, rems, svg, AnyElement, Context, Div, Entity, FontWeight, KeyDownEvent, PathPromptOptions,
    Render, SharedString, Subscription, WeakEntity, Window,
};

use runner_app::ui::button::spinner;

use runner_app::ui::{
    Badge, BrowseField, Button, ButtonSize, ButtonVariant, CopyValueButton, FieldValidation,
    PaneHeader, SettingsCard, TextField, Toggle, Tone, Tooltip,
};

use runner_core::protocol::discovery::DiscoveryOutcome;

use runner_core::protocol::runtime::RuntimeCatalogEntry;

use runner_core::protocol::runtime::{
    OverrideValidationError, RuntimeCommandSource, RuntimeExecutableStatus, RuntimeRowState,
    RuntimeStatusResponse, ShellDiscoveryStatus,
};

use crate::app_settings::AppSettings;

use crate::app_store::AppStore;

use crate::chat_icon::ChatIcon;

use crate::theme;

use crate::NativeRoot;

const NO_INSTALLED_AGENTS: &str =
    "ui.no_agents_installed_install_one_of_the_agents_below_to_get_started";

const ALL_AGENTS_INSTALLED: &str = "ui.all_supported_agents_are_installed";

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
    auto_path: crate::i18n::Text,
    show_reset: bool,
}

/// The Update button and its running-sessions caption, present only while
/// npm has a newer version than the one installed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct UpdateAction {
    caption: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AgentTab {
    #[default]
    General,
    Skills,
    Mcp,
}

pub(crate) fn supports_tab(runtime: Runtime, tab: AgentTab) -> bool {
    match tab {
        AgentTab::General => true,
        AgentTab::Skills => runner_core::protocol::runtime_metadata::catalog_for(runtime)
            .is_some_and(|c| !c.skills_dirs.is_empty()),
        AgentTab::Mcp => runner_core::protocol::mcp::McpClientId::for_runtime(runtime).is_some(),
    }
}

pub(crate) struct AgentsPane {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    selected_runtime: Option<Runtime>,
    pub(crate) tab: AgentTab,
    pub(crate) skills: Entity<super::skills::SkillsPane>,
    pub(crate) mcp: Entity<super::mcp::McpPane>,
    last_skills: Option<Runtime>,
    last_mcp: Option<Runtime>,
    accounts: Entity<super::claude_accounts::AccountsSection>,
    status: Option<RuntimeStatusResponse>,
    pub(crate) tasks: Vec<runner_core::protocol::RuntimeUpdateTask>,
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
    launch_previews:
        HashMap<(Runtime, bool), runner_core::protocol::runtime::RuntimeCommandPreview>,
    mission_preview: bool,
    launch_preview_errors: HashMap<Runtime, String>,
    launch_copy: Entity<CopyValueButton>,
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
        for runtime in runner_core::protocol::runtime_metadata::runtime_list() {
            let runtime_name = runtime.name;
            let enter_shell = shell.clone();
            let escape_pane = cx.weak_entity();
            let escape_name = runtime_name;
            let field = cx.new(|input_cx| {
                TextField::new(
                    input_cx.focus_handle(),
                    "",
                    crate::i18n::messagef(
                        "format.auto_not_found_on_path",
                        &[("command", (runtime.command).to_string())],
                    ),
                    true,
                )
                .text_size(theme::text_meta())
                .key_interceptor(Rc::new(move |event, window, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" => {
                            let shell = enter_shell.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = shell
                                    .update(cx, |root, cx| root.focus_settings_page(window, cx));
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

        let accounts =
            cx.new(|cx| super::claude_accounts::AccountsSection::new(app_store.clone(), cx));
        let skills = cx.new(|cx| super::skills::SkillsPane::new(app_store.clone(), cx));
        let mcp = cx.new(|cx| super::mcp::McpPane::new(app_store.clone(), cx));
        let notice_shell = shell.clone();
        subscriptions.push(cx.subscribe(
            &skills.read(cx).detail.clone(),
            move |_, _, notice: &super::SaveNotice, cx| {
                let _ = notice_shell.update(cx, |root, cx| {
                    root.show_toast(notice.message.clone(), notice.tone, cx)
                });
            },
        ));
        let notice_shell = shell.clone();
        subscriptions.push(cx.subscribe(
            &mcp.read(cx).detail.clone(),
            move |_, _, notice: &super::SaveNotice, cx| {
                let _ = notice_shell.update(cx, |root, cx| {
                    root.show_toast(notice.message.clone(), notice.tone, cx)
                });
            },
        ));
        subscriptions.push(cx.observe(&skills, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&mcp, |_, _, cx| cx.notify()));

        let launch_copy = cx.new(|copy_cx| {
            CopyValueButton::new(
                copy_cx.focus_handle(),
                None,
                crate::i18n::message("ui.copy"),
            )
            .labeled()
            .show_when_empty()
            .debug_selector("AGENT_LAUNCH_COPY")
        });
        let tasks = app_store
            .read(cx)
            .client
            .runtime_task_list()
            .unwrap_or_default();
        Self {
            tasks,
            shell,
            app_store,
            accounts,
            selected_runtime: None,
            tab: AgentTab::General,
            skills,
            mcp,
            last_skills: None,
            last_mcp: None,
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
            launch_previews: HashMap::new(),
            mission_preview: false,
            launch_preview_errors: HashMap::new(),
            launch_copy,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let core = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            let status = core
                .runtime_status_list()
                .map_err(|error| error.to_string())?;
            let catalog = core.runtime_catalog().map_err(|error| error.to_string())?;
            let live = core.live_session_counts().unwrap_or_default();
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
                        if let Some(runtime) = this.selected_runtime {
                            this.refresh_launch_previews(runtime, cx);
                        }
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

    /// Recounts live sessions per agent, which the Update caption reads, as
    /// sessions start and stop while Settings is open.
    pub(crate) fn refresh_live_sessions(&mut self, cx: &mut Context<Self>) {
        let core = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move { core.live_session_counts() });
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
        let name = runtime.name;
        let shell = self.shell.clone();
        window.defer(cx, move |_, cx| {
            let _ = shell.update(cx, |root, cx| root.start_agent_update(name, cx));
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
        let core = self.app_store.read(cx).client.clone();
        let model_runtimes = self.app_store.read(cx).settings.model_runtimes();
        let task = cx.background_spawn(async move {
            let status = core
                .runtime_refresh(&model_runtimes)
                .map_err(|error| error.to_string())?;
            let catalog = core.runtime_catalog().map_err(|error| error.to_string())?;
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
        let core = self.app_store.read(cx).client.clone();
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
                core.runtime_clear_override(task_runtime)
                    .map_err(|error| error.to_string())
            } else {
                core.runtime_set_override(task_runtime, &draft)
                    .map_err(|error| error.to_string())
                    .and_then(|result| {
                        result.map_err(|error| override_validation_error_message(&error))
                    })
            };
            if status.is_ok() && refresh_models {
                let _ = core.runtime_request_models(&[task_runtime]);
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
            prompt: Some(
                crate::i18n::tf(
                    "format.choose_executable",
                    &[("display_name", (display_name).to_string())],
                )
                .into(),
            ),
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

    pub(crate) fn accounts_overlay(&self, cx: &gpui::App) -> gpui::AnyElement {
        self.accounts.read(cx).confirm_overlay().into_any_element()
    }

    pub(crate) fn is_composing(&self, cx: &gpui::App) -> bool {
        self.skills.read(cx).is_composing(cx)
            || self.mcp.read(cx).is_composing(cx)
            || self.accounts.read(cx).is_composing(cx)
            || self
                .overrides
                .values()
                .any(|field| field.read(cx).is_composing())
    }

    fn reset_scroll(&self, cx: &mut Context<Self>) {
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |root, cx| {
                root.reset_settings_scroll(cx);
            });
        });
    }

    pub(crate) fn show_list(&mut self, cx: &mut Context<Self>) {
        self.selected_runtime = None;
        self.sync_launch_copy(true, cx);
        self.reset_scroll(cx);
        cx.notify();
    }

    pub(crate) fn open_tab(&mut self, tab: AgentTab, cx: &mut Context<Self>) {
        let last = match tab {
            AgentTab::Skills => self.last_skills,
            AgentTab::Mcp => self.last_mcp,
            _ => None,
        };
        let runtime = last
            .filter(|r| supports_tab(*r, tab))
            .or_else(|| {
                Runtime::parse(&self.app_store.read(cx).settings.default_runtime)
                    .filter(|r| supports_tab(*r, tab))
            })
            .or_else(|| Runtime::ALL.into_iter().find(|r| supports_tab(*r, tab)));
        if let Some(runtime) = runtime {
            self.open_runtime(runtime, cx);
            self.select_tab(tab, cx);
        }
    }
    fn select_tab(&mut self, tab: AgentTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.launch_copy
                .update(cx, |copy, copy_cx| copy.reset_copied(copy_cx));
        }
        self.tab = tab;
        if let Some(runtime) = self.selected_runtime {
            match tab {
                AgentTab::Skills => self.last_skills = Some(runtime),
                AgentTab::Mcp => self.last_mcp = Some(runtime),
                _ => {}
            }
        }
        self.reset_scroll(cx);
        cx.notify();
    }

    pub(crate) fn open_runtime(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        if runtime == Runtime::ClaudeCode {
            self.accounts
                .update(cx, |accounts, cx| accounts.refresh(cx));
        }
        let changed = self.selected_runtime != Some(runtime);
        self.selected_runtime = Some(runtime);
        self.tab = AgentTab::General;
        self.skills.update(cx, |pane, cx| pane.scope(runtime, cx));
        self.mcp.update(cx, |pane, cx| pane.scope(runtime, cx));
        self.sync_launch_copy(changed, cx);
        self.refresh_launch_previews(runtime, cx);
        self.reset_scroll(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests;

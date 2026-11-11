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
    Badge, BrowseField, Button, ButtonSize, ButtonVariant, FieldValidation, PaneHeader,
    SettingsCard, TextField, Toggle, Tone, Tooltip,
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
        self.selected_runtime = Some(runtime);
        self.tab = AgentTab::General;
        self.skills.update(cx, |pane, cx| pane.scope(runtime, cx));
        self.mcp.update(cx, |pane, cx| pane.scope(runtime, cx));
        self.refresh_launch_previews(runtime, cx);
        self.reset_scroll(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use runner_core::protocol::runtime::ShellDiscoveryStatus;

    #[test]
    fn tabs_follow_catalog_support() {
        for runtime in [Runtime::Codex, Runtime::ClaudeCode] {
            assert!(supports_tab(runtime, AgentTab::General));
            assert!(supports_tab(runtime, AgentTab::Skills));
            assert!(supports_tab(runtime, AgentTab::Mcp));
        }
        assert!(supports_tab(Runtime::Pi, AgentTab::Skills));
        assert!(!supports_tab(Runtime::Pi, AgentTab::Mcp));
        assert!(!supports_tab(Runtime::Shell, AgentTab::Skills));
        assert!(!supports_tab(Runtime::Shell, AgentTab::Mcp));
    }

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
            launch_options: Default::default(),
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
            min_version: Some("0.149.0".into()),
            below_minimum: false,
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
    fn update_button_shows_only_with_a_newer_version_and_allows_running_sessions() {
        let current = versioned(RuntimeRowState::Detected, Some("0.155.0"), None);
        assert_eq!(update_action(&current, 3), None);
        assert_eq!(
            update_action(&versioned(RuntimeRowState::Detected, None, None), 0),
            None
        );
        let mut trae = versioned(RuntimeRowState::Detected, Some("0.1.0"), None);
        trae.name = Runtime::Trae;
        trae.display_name = "TRAE CLI".into();
        assert_eq!(update_action(&trae, 0), None);

        let stale = versioned(RuntimeRowState::Detected, Some("0.153.4"), Some("0.155.0"));
        assert_eq!(
            update_action(&stale, 0),
            Some(UpdateAction { caption: None })
        );
        assert_eq!(
            update_action(&stale, 1),
            Some(UpdateAction {
                caption: Some("1 running Codex session keeps 0.153.4 until it relaunches.".into()),
            })
        );
        assert_eq!(
            update_action(&stale, 3),
            Some(UpdateAction {
                caption: Some("3 running Codex sessions keep 0.153.4 until they relaunch.".into()),
            })
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

        open_test_page(pane, Runtime::Codex, &mut window);
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
            runtime_default_effort_value(runtime.default_effort.as_deref()),
            "xhigh"
        );

        runtime.default_effort = None;
        assert_eq!(
            runtime_default_effort_value(runtime.default_effort.as_deref()),
            "runtime default"
        );
    }

    fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
        use runner_daemon::{db, session, shell_path};
        use std::sync::{Arc, RwLock};
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_fixture_pool(&path.join("runner.db")).unwrap()),
            path.into(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
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
        runner_core::protocol::runtime_metadata::runtime_list()
            .into_iter()
            .map(|entry| RuntimeCatalogEntry {
                installed_version: None,
                available_version: None,
                min_version: None,
                below_minimum: false,
                name: entry.name,
                capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
                    entry.name,
                )
                .map(|entry| entry.capabilities)
                .unwrap_or_default(),
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

    fn open_test_page(
        pane: gpui::WindowHandle<AgentsPane>,
        runtime: Runtime,
        window: &mut gpui::VisualTestContext,
    ) {
        let row = window
            .debug_bounds(Box::leak(format!("AGENT_CARD_{runtime}").into_boxed_str()))
            .unwrap();
        window.simulate_click(
            gpui::point(row.left() + px(24.), row.center().y),
            gpui::Modifiers::default(),
        );
        window.run_until_parked();
        pane.read_with(window, |pane, _| {
            assert_eq!(pane.selected_runtime, Some(runtime))
        })
        .unwrap();
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
        let rows = runner_core::protocol::runtime_metadata::runtime_list()
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
            "AGENT_CARD_antigravity",
            "AGENT_CARD_pi",
            "AGENT_CARD_copilot",
            "AGENT_CARD_trae",
        ] {
            let card = window.debug_bounds(selector).unwrap();
            assert!(card.top() >= previous_bottom, "{selector}");
            previous_bottom = card.bottom();
        }
    }

    #[test]
    fn install_link_shares_the_description_line_and_both_are_omitted_when_empty() {
        let mut cx = gpui::TestAppContext::single();
        let rows = runner_core::protocol::runtime_metadata::runtime_list()
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
        open_test_page(pane, Runtime::Codex, &mut window);
        let description = window.debug_bounds("AGENT_DESCRIPTION_codex").unwrap();
        let install = window.debug_bounds("AGENT_INSTALL_codex").unwrap();
        assert!(install.left() > description.right());
        assert!(install.top() < description.bottom() && install.bottom() > description.top());
        pane.update(&mut window, |pane, _, cx| {
            pane.open_runtime(Runtime::Trae, cx);
        })
        .unwrap();
        window.run_until_parked();
        assert!(window.debug_bounds("AGENT_PAGE_trae").is_some());
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
        open_test_page(pane, Runtime::Codex, &mut window);
        assert!(window.debug_bounds("AGENT_VALIDATION_codex").is_some());
        assert!(window.debug_bounds("AGENT_RESET_codex").is_some());
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
        pane.read_with(&window, |pane, _| {
            assert_eq!(pane.selected_runtime, Some(Runtime::Codex))
        })
        .unwrap();
        pane.update(&mut window, |pane, _, cx| {
            pane.apply_status(status(runtime(RuntimeRowState::NotFound), false), cx);
            cx.notify();
        })
        .unwrap();
        window.run_until_parked();
        assert!(window.debug_bounds("AGENT_PAGE_codex").is_some());
    }

    #[test]
    fn compact_rows_and_agent_defaults_fit_at_two_rem_sizes() {
        for rem in [16., 20.8] {
            let mut cx = gpui::TestAppContext::single();
            let mut claude = runtime(RuntimeRowState::Detected);
            claude.name = Runtime::ClaudeCode;
            claude.display_name = "Claude Code".into();
            claude.default_model = Some("claude-fable-5[1m]".into());
            claude.default_effort = Some("xhigh".into());
            let (_temp, pane) = test_pane(
                vec![claude, runtime(RuntimeRowState::NotFound)],
                false,
                &mut cx,
            );
            let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
            pane.update(&mut window, |_, window, _| {
                window.set_rem_size(px(rem));
                window.refresh();
            })
            .unwrap();
            window.run_until_parked();
            let row = window.debug_bounds("AGENT_CARD_claude-code").unwrap();
            assert!((row.size.height - px(64. * rem / 16.)).abs() <= px(1.));
            assert!(window
                .debug_bounds("AGENT_MODEL_LINE_claude-code")
                .is_none());
            open_test_page(pane, Runtime::ClaudeCode, &mut window);
            let page = window.debug_bounds("AGENT_PAGE_claude-code").unwrap();
            let model = window.debug_bounds("AGENT_MODEL_LINE_claude-code").unwrap();
            assert!(model.bottom() <= page.bottom());
            assert!(window.debug_bounds("AGENT_MCP_LINE_claude-code").is_none());
            assert!(window.debug_bounds("AGENT_MODEL_LINE_codex").is_none());
        }
    }

    #[test]
    fn every_runtime_row_opens_its_page_and_breadcrumb_returns() {
        for installed in [false, true] {
            let mut cx = gpui::TestAppContext::single();
            let rows: Vec<_> = runner_core::protocol::runtime_metadata::runtime_list()
                .into_iter()
                .map(|entry| {
                    let mut row = runtime(if installed {
                        RuntimeRowState::Detected
                    } else {
                        RuntimeRowState::NotFound
                    });
                    row.name = entry.name;
                    row.display_name = entry.display_name;
                    row.command = entry.command;
                    row
                })
                .collect();
            let names: Vec<_> = rows.iter().map(|row| row.name).collect();
            let (_temp, pane) = test_pane(rows, false, &mut cx);
            let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
            for name in names {
                open_test_page(pane, name, &mut window);
                assert!(window
                    .debug_bounds(Box::leak(format!("AGENT_PAGE_{name}").into_boxed_str()))
                    .is_some());
                let back = window.debug_bounds("AGENTS_BREADCRUMB").unwrap();
                window.simulate_click(back.center(), gpui::Modifiers::default());
                window.run_until_parked();
                pane.read_with(&window, |pane, _| assert_eq!(pane.selected_runtime, None))
                    .unwrap();
            }
        }
    }

    #[test]
    fn list_toggle_changes_preference_without_opening_the_agent() {
        let mut cx = gpui::TestAppContext::single();
        let (_temp, pane) = test_pane(vec![runtime(RuntimeRowState::Detected)], false, &mut cx);
        let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
        let toggle = window.debug_bounds("AGENT_TOGGLE_codex").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        window.run_until_parked();
        pane.read_with(&window, |pane, cx| {
            assert_eq!(pane.selected_runtime, None);
            assert!(pane
                .app_store
                .read(cx)
                .settings
                .disabled_agents
                .contains("codex"));
        })
        .unwrap();
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
        assert!(!runtime_defaults_visible(&runtime(
            RuntimeRowState::InvalidOverride
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
        open_test_page(pane, Runtime::Codex, &mut window);
        for (selector, expected) in [
            ("AGENT_SET_DEFAULT_codex", "codex"),
            ("AGENT_SET_DEFAULT_claude-code", "claude-code"),
        ] {
            let runtime = if expected == "codex" {
                Runtime::Codex
            } else {
                Runtime::ClaudeCode
            };
            pane.update(&mut window, |pane, _, cx| pane.open_runtime(runtime, cx))
                .unwrap();
            window.run_until_parked();
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
            open_test_page(pane, Runtime::Codex, &mut window);
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
            open_test_page(pane, Runtime::Codex, &mut window);
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
        open_test_page(pane, Runtime::Copilot, &mut window);
        for rem in [16., 20.8] {
            for width in [320., 480., 760.] {
                pane.update(&mut window, |_, window, _| {
                    window.set_rem_size(px(rem));
                    window.refresh();
                })
                .unwrap();
                window.simulate_resize(gpui::size(px(width), px(1200.)));
                window.run_until_parked();
                let header = window.debug_bounds("AGENT_HEADER_copilot").unwrap();
                let identity = window.debug_bounds("AGENT_IDENTITY_copilot").unwrap();
                let actions = window.debug_bounds("AGENT_ACTIONS_copilot").unwrap();
                assert!(header.left() >= px(0.) && header.right() <= px(width));
                if width == 320. {
                    assert!(identity.size.height > px(26. * rem / 16.));
                    assert!(identity.bottom() <= actions.top());
                }
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
            installed_version: None,
            available_version: None,
            min_version: None,
            below_minimum: false,
            name: Runtime::Codex,
            capabilities: runner_core::protocol::runtime::RuntimeCatalogEntry::for_runtime(
                Runtime::Codex,
            )
            .map(|entry| entry.capabilities)
            .unwrap_or_default(),
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

    #[cfg(debug_assertions)]
    #[test]
    fn mounted_agents_placeholders_follow_locale_without_reloading_drafts() {
        use crate::i18n::{self, Language, Locale};
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        i18n::testing::with_locale(Locale::English, || {
            for (state, path, english, chinese) in [
                (
                    RuntimeRowState::Checking,
                    None,
                    "Auto — detecting…",
                    "自动 — 正在检测…",
                ),
                (
                    RuntimeRowState::Detected,
                    Some("/tmp/{path}/codex"),
                    "Auto — /tmp/{path}/codex",
                    "自动 — /tmp/{path}/codex",
                ),
                (
                    RuntimeRowState::NotFound,
                    None,
                    "Auto — codex not found on PATH",
                    "自动 — PATH 中未找到 codex",
                ),
            ] {
                let mut cx = gpui::TestAppContext::single();
                let mut row = runtime(state);
                row.detected_path = path.map(str::to_owned);
                let (_temp, pane) = test_pane_with_settings(
                    vec![row],
                    false,
                    AppSettings {
                        language: Language::English,
                        ..AppSettings::default()
                    },
                    &mut cx,
                );
                let (store, field) = pane
                    .read_with(&cx, |pane, _| {
                        (
                            pane.app_store.clone(),
                            pane.overrides[&Runtime::Codex].clone(),
                        )
                    })
                    .unwrap();
                let peer = cx.add_window(|window, cx| {
                    AgentsPane::new(WeakEntity::new_invalid(), store.clone(), window, cx)
                });
                field.update(&mut cx, |field, cx| {
                    field.set_text("/tmp/draft {count} 中文", cx);
                    field.select_all(cx);
                });
                let caret = field.read_with(&cx, |field, _| field.caret_offset());
                let mut visual = gpui::VisualTestContext::from_window(pane.into(), &cx);
                open_test_page(pane, Runtime::Codex, &mut visual);
                for (language, expected) in [
                    (Language::SimplifiedChinese, chinese),
                    (Language::English, english),
                ] {
                    peer.update(&mut visual, |root, _, cx| {
                        root.app_store.update(cx, |store, cx| {
                            store.update_settings(
                                |settings| {
                                    settings.language = language;
                                    true
                                },
                                false,
                                cx,
                            )
                        })
                    })
                    .unwrap();
                    visual.run_until_parked();
                    field.read_with(&visual, |field, _| {
                        assert_eq!(field.placeholder(), expected);
                        assert_eq!(field.text(), "/tmp/draft {count} 中文");
                        assert_eq!(field.caret_offset(), caret);
                    });
                    pane.read_with(&visual, |pane, _| {
                        assert_eq!(
                            pane.overrides[&Runtime::Codex].entity_id(),
                            field.entity_id()
                        )
                    })
                    .unwrap();
                }
            }
        });
    }

    #[cfg(debug_assertions)]
    #[test]
    fn chinese_agent_defaults_translate_effort_and_keep_model_names_literal() {
        crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
            let _theme = crate::theme_snapshot::ThemeGuard::new();
            let mut row = runtime(RuntimeRowState::Detected);
            row.default_model = Some("xhigh".into());
            row.default_effort = Some("xhigh".into());
            assert_eq!(runtime_default_value(row.default_model.as_deref()), "xhigh");
            assert_eq!(
                runtime_default_effort_value(row.default_effort.as_deref()),
                "极高"
            );
            assert_eq!(runtime_default_effort_value(None), "运行时默认");
            let mut cx = gpui::TestAppContext::single();
            let (_temp, pane) = test_pane(vec![row], false, &mut cx);
            let mut visual = gpui::VisualTestContext::from_window(pane.into(), &cx);
            open_test_page(pane, Runtime::Codex, &mut visual);
            assert!(visual.debug_bounds("AGENT_PROPERTY_VALUE 极高").is_some());
            assert!(visual.debug_bounds("AGENT_PROPERTY_VALUE xhigh").is_some());
        });
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
    #[test]
    fn launch_card_switches_follow_the_runtime_table() {
        for name in [
            Runtime::Codex,
            Runtime::ClaudeCode,
            Runtime::Pi,
            Runtime::Cursor,
        ] {
            let mut cx = gpui::TestAppContext::single();
            let mut row = runtime(RuntimeRowState::Detected);
            row.name = name;
            let (_temp, pane) = test_pane(vec![row], false, &mut cx);
            pane.update(&mut cx, |pane, _, cx| {
                pane.selected_runtime = Some(name);
                pane.launch_previews.insert(
                    (name, false),
                    runner_core::protocol::runtime::RuntimeCommandPreview {
                        lines: vec![runner_core::protocol::runtime::RuntimeCommandLine {
                            command: "chat command".into(),
                            note: runner_core::protocol::runtime::RuntimeCommandNote::None,
                            option: None,
                        }],
                    },
                );
                cx.notify();
            })
            .unwrap();
            cx.run_until_parked();
            let mut window = gpui::VisualTestContext::from_window(pane.into(), &cx);
            assert!(window
                .debug_bounds(Box::leak(format!("AGENT_LAUNCH_{name}").into_boxed_str()))
                .is_some());
            for option in [
                runner_core::protocol::runtime::RuntimeLaunchOption::Bypass,
                runner_core::protocol::runtime::RuntimeLaunchOption::RemotePluginCatalog,
                runner_core::protocol::runtime::RuntimeLaunchOption::StatusHooks,
            ] {
                assert_eq!(
                    window
                        .debug_bounds(Box::leak(
                            format!("AGENT_LAUNCH_SWITCH_{name}_{option:?}").into_boxed_str()
                        ))
                        .is_some(),
                    option.supported(name)
                );
            }
            assert!(window
                .debug_bounds(Box::leak(
                    format!("AGENT_LAUNCH_COMMAND_{name}").into_boxed_str()
                ))
                .is_some());
        }
    }
}

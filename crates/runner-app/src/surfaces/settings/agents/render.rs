use super::*;

impl AgentsPane {
    pub(super) fn render_refresh(&self, cx: &mut Context<Self>) -> Button {
        let checking = self.refreshing
            || self
                .status
                .as_ref()
                .is_some_and(|status| status.shell.checking);
        let pane = cx.entity();
        Button::new(
            "agents-refresh",
            if checking {
                crate::i18n::text("ui.checking")
            } else {
                crate::i18n::text("ui.refresh")
            },
        )
        .icon("refresh-cw.svg")
        .size(ButtonSize::Sm)
        .variant(ButtonVariant::Secondary)
        .loading(checking)
        .on_press(move |_, cx| {
            pane.update(cx, |this, cx| this.refresh_discovery(cx));
        })
    }

    pub(super) fn render_agent_page(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pane = cx.entity();
        let installed = runtime_defaults_visible(runtime);
        div()
            .debug_selector(|| format!("AGENT_PAGE_{}", runtime.name))
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().debug_selector(|| "AGENTS_BREADCRUMB".into()).child(
                            Button::new("agents-back", crate::i18n::text("ui.agents"))
                                .icon("arrow-left.svg")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_press(move |_, cx| {
                                    pane.update(cx, |this, cx| {
                                        this.show_list(cx);
                                    });
                                }),
                        ),
                    )
                    .child(
                        svg()
                            .path("chevron-right.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(runtime.display_name.clone()),
                    ),
            )
            .child(if installed {
                self.render_runtime_page(runtime, cx)
            } else {
                self.render_not_installed_page(runtime, cx)
            })
            .children((self.tab == AgentTab::General).then(|| self.render_launch(runtime, cx)))
            .children(
                (self.tab == AgentTab::General && installed && runtime.name == Runtime::ClaudeCode)
                    .then(|| self.accounts.clone()),
            )
            .into_any_element()
    }

    pub(super) fn render_agent_row(
        &self,
        runtime: &RuntimeExecutableStatus,
        installed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = runtime.name;
        let pane = cx.entity();
        let key_pane = pane.clone();
        let presentation = runtime_presentation(
            runtime,
            self.status.as_ref().and_then(|status| status.shell.outcome),
            self.validation.get(&name).map(String::as_str),
            self.saving.contains(&name),
        );
        let catalog = self.catalog.iter().find(|entry| entry.name == name);
        let detail = if installed {
            let mut parts = Vec::new();
            if let Some(version) = &runtime.installed_version {
                parts.push(version.clone());
            }
            if let Some(path) = runtime
                .override_path
                .as_ref()
                .or(runtime.detected_path.as_ref())
            {
                parts.push(path.clone());
            }
            if runtime.override_path.is_some() {
                parts.push(crate::i18n::text("ui.custom_path").to_owned());
            }
            if name == Runtime::ClaudeCode {
                let accounts = &self.app_store.read(cx).claude_accounts;
                if accounts.multiple() {
                    if let Some(active) = accounts.active() {
                        parts.push(crate::i18n::tf(
                            "accounts.active_count",
                            &[
                                ("name", active.name.clone()),
                                ("count", accounts.accounts.len().to_string()),
                            ],
                        ));
                    }
                }
            }
            if parts.is_empty() {
                parts.push(presentation.badge.to_owned());
            }
            parts.join(" · ")
        } else {
            catalog
                .map(|entry| {
                    let mut parts = Vec::new();
                    if !entry.description.is_empty() {
                        parts.push(
                            runner_app::ui::select::runtime_description(name, &entry.description)
                                .resolve()
                                .to_string(),
                        );
                    }
                    if !entry.install_url.is_empty() {
                        parts.push(entry.install_url.clone());
                    }
                    parts.join(" · ")
                })
                .unwrap_or_default()
        };
        div()
            .id(SharedString::from(format!("agent-row-{name}")))
            .debug_selector(|| format!("AGENT_CARD_{name}"))
            .tab_index(0)
            .tab_stop(true)
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_3()
            .h(rems(64. / 16.))
            .px_5()
            .hover(|style| style.bg(theme::raised()))
            .focus_visible(|style| style.bg(theme::raised()))
            .on_click(move |_, _, cx| {
                pane.update(cx, |this, cx| this.open_runtime(name, cx));
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_pane.update(cx, |this, cx| this.open_runtime(name, cx));
                }
            })
            .child(runtime_mark(name, 32.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| format!("AGENT_IDENTITY_{name}"))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(runtime.display_name.clone()),
                            )
                            .children(
                                (self.app_store.read(cx).settings.default_runtime == name.key())
                                    .then(|| self.render_default_action(name, false, false, cx))
                                    .flatten(),
                            )
                            .when(
                                installed
                                    && !matches!(
                                        presentation.tone,
                                        BadgeTone::Accent | BadgeTone::Neutral
                                    ),
                                |row| row.child(runtime_badge(name, &presentation)),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(detail),
                    ),
            )
            .when(installed, |row| {
                row.child(
                    div()
                        .id(SharedString::from(format!("agent-row-actions-{name}")))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_3()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .children(self.render_update_button(runtime, cx))
                        .child(self.render_toggle(runtime, cx)),
                )
            })
            .child(
                svg()
                    .flex_none()
                    .path("chevron-right.svg")
                    .size(rems(14. / 16.))
                    .text_color(theme::faint()),
            )
            .into_any_element()
    }

    pub(super) fn render_update_button(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if let Some(task) = self.tasks.iter().find(|task| task.runtime == runtime.name) {
            use runner_core::protocol::RuntimeUpdateState as State;
            let name = task.runtime;
            let answer = task.state == State::MayNeedInput;
            let failed = matches!(task.state, State::Failed | State::BrokenAfterStop);
            let broken = task.state == State::BrokenAfterStop;
            let caption = if answer || failed {
                super::super::super::background_tasks::label(task)
            } else if task.state.active() {
                crate::i18n::tf(
                    "agent_updates.settings_updating",
                    &[(
                        "version",
                        task.to_version.clone().unwrap_or_else(|| {
                            crate::i18n::text("agent_updates.latest_version").into()
                        }),
                    )],
                )
            } else if task.state == State::Succeeded {
                crate::i18n::tf(
                    "agent_updates.settings_updated",
                    &[(
                        "version",
                        task.installed_version.clone().unwrap_or_else(|| {
                            crate::i18n::text("agent_updates.latest_version").into()
                        }),
                    )],
                )
            } else {
                crate::i18n::text("agent_updates.settings_failed").into()
            };
            return Some(
                div()
                    .debug_selector(|| format!("AGENT_UPDATE_{}", runtime.name))
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .debug_selector(|| format!("AGENT_TASK_CAPTION_{name}"))
                            .text_size(theme::text_meta())
                            .text_color(if answer {
                                theme::warning()
                            } else if failed {
                                theme::danger()
                            } else {
                                theme::muted()
                            })
                            .child(caption),
                    )
                    .when(failed, |el| {
                        let shell = self.shell.clone();
                        el.gap_1().child(
                            Button::new(
                                SharedString::from(format!("agent-task-retry-{name}")),
                                if broken {
                                    crate::i18n::text("agent_updates.reinstall")
                                } else {
                                    crate::i18n::text("ui.retry")
                                },
                            )
                            .debug_selector(format!("AGENT_TASK_RETRY_{name}"))
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Secondary)
                            .on_press(move |_, cx| {
                                let _ =
                                    shell.update(cx, |root, cx| root.start_agent_update(name, cx));
                            }),
                        )
                    })
                    .into_any_element(),
            );
        }
        update_action(runtime, 0).map(|_| {
            let pane = cx.entity();
            let status = runtime.clone();
            div()
                .debug_selector(|| format!("AGENT_UPDATE_{}", runtime.name))
                .child(
                    Button::new(
                        SharedString::from(format!("agent-update-{}", runtime.name)),
                        crate::i18n::text("ui.update"),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .on_press(move |window, cx| {
                        pane.update(cx, |this, cx| this.open_update(&status, window, cx));
                    }),
                )
                .into_any_element()
        })
    }

    pub(super) fn render_toggle(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = runtime.name;
        let enabled = runtime_default_enabled(&self.catalog, name).is_some_and(|default_enabled| {
            self.app_store
                .read(cx)
                .settings
                .is_agent_enabled(name, default_enabled)
        });
        let pane = cx.entity();
        div()
            .debug_selector(|| format!("AGENT_TOGGLE_{name}"))
            .child(Tooltip::new(
                SharedString::from(format!("agent-toggle-tip-{name}")),
                format!(
                    "{} {}",
                    if enabled {
                        crate::i18n::text("ui.disable")
                    } else {
                        crate::i18n::text("ui.enable")
                    },
                    runtime.display_name
                ),
                Toggle::new(SharedString::from(format!("agent-toggle-{name}")), enabled).on_change(
                    move |enabled, _, cx| {
                        pane.update(cx, |this, cx| this.set_enabled(name, enabled, cx));
                    },
                ),
            ))
            .into_any_element()
    }

    pub(super) fn render_default_action(
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
                    .child(crate::i18n::text("ui.default"))
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
                            crate::i18n::text("ui.set_as_default"),
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

    pub(super) fn render_browse_field(
        &self,
        runtime: Runtime,
        cx: &mut Context<Self>,
    ) -> Option<BrowseField> {
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
            .browse_label(crate::i18n::text("ui.browse"))
        })
    }

    pub(super) fn render_not_installed_page(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let catalog = self.catalog.iter().find(|entry| entry.name == runtime.name);
        let validation = self.validation.get(&runtime.name).map(String::as_str);
        let pane = cx.entity();
        let name = runtime.name;
        div()
            .debug_selector(|| format!("AGENT_DETAILS_{}", runtime.name))
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .child(runtime_mark(runtime.name, 36.))
                    .child(
                        div()
                            .text_size(theme::text_display())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(runtime.display_name.clone()),
                    ),
            )
            .children(self.render_tabs(name, cx))
            .when(self.tab == AgentTab::Skills, |page| {
                page.child(self.skills.clone())
            })
            .when(self.tab == AgentTab::Mcp, |page| {
                page.child(self.mcp.clone())
            })
            .when(self.tab == AgentTab::General, |page| {
                page.children(
                    catalog
                        .filter(|entry| {
                            !entry.description.is_empty() || !entry.install_url.is_empty()
                        })
                        .map(|entry| {
                            let description = (!entry.description.is_empty()).then(|| {
                                runtime_section_caption(
                                    runner_app::ui::select::runtime_description(
                                        entry.name,
                                        &entry.description,
                                    )
                                    .resolve(),
                                )
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
                                            crate::i18n::text("ui.install_instructions"),
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
                                .flex_wrap()
                                .gap_2()
                                .children(description)
                                .children(install)
                        }),
                )
                .child(runtime_section_header(
                    crate::i18n::text("ui.executable"),
                    None,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .debug_selector(|| format!("AGENT_EXECUTABLE_{name}"))
                                .min_w_0()
                                .flex_1()
                                .children(self.render_browse_field(name, cx)),
                        )
                        .children(runtime.override_path.is_some().then(|| {
                            div()
                                .debug_selector(|| format!("AGENT_RESET_{name}"))
                                .child(
                                    Button::new(
                                        SharedString::from(format!("agent-reset-{name}")),
                                        crate::i18n::text("ui.reset"),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(self.saving.contains(&name))
                                    .on_press(move |_, cx| {
                                        pane.update(cx, |this, cx| this.reset_override(name, cx));
                                    }),
                                )
                        })),
                )
                .children(validation.map(|message| {
                    runtime_caption(message.to_owned(), true)
                        .debug_selector(|| format!("AGENT_VALIDATION_{}", runtime.name))
                }))
            })
            .into_any_element()
    }

    fn render_tabs(&self, runtime: Runtime, cx: &Context<Self>) -> Option<AnyElement> {
        let tabs: Vec<_> = [AgentTab::General, AgentTab::Skills, AgentTab::Mcp]
            .into_iter()
            .filter(|tab| supports_tab(runtime, *tab))
            .collect();
        if tabs.len() == 1 {
            return None;
        }
        Some(
            div()
                .flex()
                .gap_4()
                .border_b_1()
                .border_color(theme::border())
                .children(tabs.into_iter().map(|tab| {
                    let count = match tab {
                        AgentTab::Skills => Some(self.skills.read(cx).count()),
                        AgentTab::Mcp => Some(self.mcp.read(cx).count(cx)),
                        _ => None,
                    };
                    let label = match tab {
                        AgentTab::General => crate::i18n::text("ui.general"),
                        AgentTab::Skills => crate::i18n::text("ui.skills"),
                        AgentTab::Mcp => "MCP",
                    };
                    let owner = cx.entity();
                    let key_owner = owner.clone();
                    div()
                        .id(gpui::SharedString::from(format!("agent-tab-{tab:?}")))
                        .debug_selector(move || format!("AGENT_TAB_{tab:?}"))
                        .tab_index(0)
                        .pb_2()
                        .cursor_pointer()
                        .text_size(theme::text_body())
                        .text_color(if self.tab == tab {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .when(self.tab == tab, |tab| {
                            tab.border_b_2().border_color(theme::accent())
                        })
                        .child(
                            count.map_or_else(
                                || label.to_owned(),
                                |count| format!("{label} {count}"),
                            ),
                        )
                        .on_click(move |_, _, cx| {
                            owner.update(cx, |this, cx| this.select_tab(tab, cx))
                        })
                        .on_key_down(move |event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_owner.update(cx, |this, cx| this.select_tab(tab, cx));
                            }
                        })
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_runtime_page(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = runtime.name;
        let validation = self.validation.get(&name).map(String::as_str);
        let saving = self.saving.contains(&name);
        let mut presentation = runtime_presentation(
            runtime,
            self.status.as_ref().and_then(|status| status.shell.outcome),
            validation,
            saving,
        );
        presentation.show_reset |= self
            .overrides
            .get(&name)
            .is_some_and(|field| !field.read(cx).text().trim().is_empty());
        let reset_pane = cx.entity();
        let default_action = self.render_default_action(
            name,
            runtime_default_eligible(runtime, &self.catalog, &self.app_store.read(cx).settings),
            runtime.state == RuntimeRowState::Checking,
            cx,
        );
        let update_caption =
            update_action(runtime, self.live_sessions.get(&name).copied().unwrap_or(0))
                .and_then(|update| update.caption);
        div()
            .debug_selector(|| format!("AGENT_DETAILS_{name}"))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| format!("AGENT_HEADER_{name}"))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .debug_selector(|| format!("AGENT_IDENTITY_{name}"))
                                    .min_w_0()
                                    .max_w_full()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_3()
                                    .child(runtime_mark(name, 36.))
                                    .child(
                                        div()
                                            .text_size(theme::text_display())
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
                                            identity.child(runtime_badge(name, &presentation))
                                        },
                                    ),
                            )
                            .child(
                                div()
                                    .debug_selector(|| format!("AGENT_ACTIONS_{name}"))
                                    .ml_auto()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap_3()
                                    .children(self.render_update_button(runtime, cx))
                                    .child(self.render_toggle(runtime, cx)),
                            ),
                    )
                    .children(
                        presentation
                            .caption
                            .map(|caption| runtime_caption(caption, validation.is_some())),
                    )
                    .children(update_caption.map(|caption| {
                        runtime_caption(caption, false)
                            .debug_selector(|| format!("AGENT_UPDATE_CAPTION_{name}"))
                    })),
            )
            .children(self.render_tabs(name, cx))
            .when(self.tab == AgentTab::Skills, |page| {
                page.child(self.skills.clone())
            })
            .when(self.tab == AgentTab::Mcp, |page| {
                page.child(self.mcp.clone())
            })
            .when(self.tab == AgentTab::General, |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(runtime_section_header(
                            crate::i18n::text("ui.executable"),
                            None,
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .debug_selector(|| format!("AGENT_EXECUTABLE_{name}"))
                                        .min_w_0()
                                        .flex_1()
                                        .children(self.render_browse_field(name, cx)),
                                )
                                .children(presentation.show_reset.then(|| {
                                    Button::new(
                                        SharedString::from(format!("agent-reset-{name}")),
                                        crate::i18n::text("ui.reset"),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(saving)
                                    .on_press(move |_, cx| {
                                        reset_pane
                                            .update(cx, |this, cx| this.reset_override(name, cx));
                                    })
                                })),
                        )
                        .child(runtime_section_caption(crate::i18n::text(
                            "ui.agent_executable_hint",
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(runtime_section_header(
                            crate::i18n::text("ui.defaults"),
                            None,
                        ))
                        .child(
                            runtime_property_line(name, "MODEL", crate::i18n::text("ui.model"))
                                .child(runtime_property_value(runtime_default_value(
                                    runtime.default_model.as_deref(),
                                )))
                                .child(div().text_color(theme::muted()).child("·"))
                                .child(
                                    div()
                                        .text_color(theme::muted())
                                        .child(crate::i18n::text("ui.effort")),
                                )
                                .child(runtime_property_value(runtime_default_effort_value(
                                    runtime.default_effort.as_deref(),
                                ))),
                        )
                        .child(runtime_section_caption(crate::i18n::text(
                            "ui.agent_defaults_hint",
                        ))),
                )
            })
            .into_any_element()
    }
}

pub(super) fn runtime_mark(runtime: Runtime, size: f32) -> Div {
    let mark = ChatIcon::for_runtime(runtime.key());
    div()
        .flex_none()
        .size(rems(size / 16.))
        .rounded(rems(8. / 16.))
        .bg(theme::raised())
        .flex()
        .items_center()
        .justify_center()
        .child(mark.render(rems(1.), mark.color(theme::text(), true), true))
}

pub(super) fn runtime_section_header(label: &'static str, count: Option<usize>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(theme::text_body())
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
        .children(count.map(|count| Badge::new(count.to_string(), Tone::Muted)))
}

pub(super) fn runtime_section_caption(caption: impl Into<SharedString>) -> Div {
    div()
        .text_size(theme::text_ui())
        .line_height(rems(17.4 / 16.))
        .text_color(theme::muted())
        .child(caption.into())
}

pub(super) fn runtime_property_line(
    runtime: Runtime,
    key: &'static str,
    label: &'static str,
) -> Div {
    let line_selector = format!("AGENT_{key}_LINE_{runtime}");
    let label_selector = format!("AGENT_{key}_LABEL_{runtime}");
    div()
        .debug_selector(|| line_selector)
        .flex()
        .items_baseline()
        .flex_wrap()
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

pub(super) fn runtime_property_value(value: String) -> Div {
    div()
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_color(theme::text())
        .when(cfg!(test), |property| {
            let value = value.clone();
            property.debug_selector(move || format!("AGENT_PROPERTY_VALUE {value}"))
        })
        .child(value)
}

pub(super) fn runtime_caption(caption: String, danger: bool) -> Div {
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
        let page = self.selected_runtime.and_then(|name| {
            self.status
                .as_ref()?
                .runtimes
                .iter()
                .find(|runtime| runtime.name == name)
        });
        let sections = if let Some(runtime) = page {
            self.render_agent_page(runtime, cx)
        } else {
            let lists = if let Some(status) = &self.status {
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
                            .gap_3()
                            .child(
                                runtime_section_header(
                                    crate::i18n::text("ui.installed"),
                                    Some(installed.len()),
                                )
                                .debug_selector(|| "AGENTS_INSTALLED_HEADER".into()),
                            )
                            .when(installed.is_empty(), |section| {
                                section.child(
                                    runtime_section_caption(crate::i18n::text(NO_INSTALLED_AGENTS))
                                        .debug_selector(|| "AGENTS_INSTALLED_EMPTY".into()),
                                )
                            })
                            .when(!installed.is_empty(), |section| {
                                section.child(SettingsCard::new(
                                    installed
                                        .into_iter()
                                        .map(|runtime| self.render_agent_row(runtime, true, cx)),
                                ))
                            }),
                    )
                    .children(if not_installed.is_empty() {
                        (!status.shell.checking).then(|| {
                            runtime_section_caption(crate::i18n::text(ALL_AGENTS_INSTALLED))
                                .debug_selector(|| "AGENTS_NOT_INSTALLED_EMPTY".into())
                        })
                    } else {
                        Some(
                            div()
                                .debug_selector(|| "AGENTS_NOT_INSTALLED".into())
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    runtime_section_header(
                                        crate::i18n::text("ui.not_installed"),
                                        Some(not_installed.len()),
                                    )
                                    .debug_selector(|| "AGENTS_NOT_INSTALLED_HEADER".into()),
                                )
                                .child(SettingsCard::new(
                                    not_installed
                                        .into_iter()
                                        .map(|runtime| self.render_agent_row(runtime, false, cx)),
                                )),
                        )
                    })
                    .into_any_element()
            } else {
                SettingsCard::new(
                    runner_core::protocol::runtime_metadata::runtime_list()
                        .into_iter()
                        .map(|_| {
                            div()
                                .h(rems(64. / 16.))
                                .bg(theme::with_alpha(theme::raised(), 0.2))
                                .into_any_element()
                        }),
                )
                .into_any_element()
            };
            div().flex().flex_col().gap_5()
                .child(PaneHeader::new(crate::i18n::text("ui.agents"), shell_description(self.status.as_ref().map(|status| &status.shell))).action(self.render_refresh(cx)))
                .child(runtime_section_caption(crate::i18n::text("ui.open_agent_settings_hint")))
                .child(lists)
                .child(runtime_section_caption(crate::i18n::text("ui.disabled_agents_stay_configured_but_are_hidden_from_agent_pickers_overrides_apply_to_new_s")))
                .into_any_element()
        };
        let retry = cx.entity();
        div().flex().flex_col().gap_5().child(sections).children(
            self.error
                .clone()
                .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                .map(|error| {
                    runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger).child(
                        Button::new("agents-retry", crate::i18n::text("ui.retry"))
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_press(move |_, cx| {
                                retry.update(cx, |this, pane_cx| this.refresh(pane_cx));
                            }),
                    )
                }),
        )
    }
}

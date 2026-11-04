use super::*;

pub(crate) fn settings_section_heading(label: impl Into<SharedString>) -> AnyElement {
    div()
        .px_1()
        .text_size(theme::text_meta())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label.into())
        .into_any_element()
}

pub(super) fn command_line_settings_row(
    title: AnyElement,
    description: &'static str,
    status_lines: Vec<AnyElement>,
    control: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(24. / 16.))
        .px_4()
        .py_3()
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .child(title)
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(description),
                )
                .children(status_lines),
        )
        .children(control.map(|control| div().flex_none().child(control)))
        .into_any_element()
}

pub(super) fn command_status_text(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
    icon: Option<&'static str>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(theme::text_caption())
        .text_color(color)
        .children(icon.map(|icon| svg().path(icon).size(px(12.)).text_color(color)))
        .child(text.into())
        .into_any_element()
}

pub(super) fn command_installed_line(path: &Path) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(theme::text_caption())
        .text_color(theme::accent())
        .child(
            svg()
                .path("check.svg")
                .size(px(12.))
                .text_color(theme::accent()),
        )
        .child(crate::i18n::text("ui.installed_at"))
        .child(
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .child(path.display().to_string()),
        )
        .into_any_element()
}

pub(super) fn runner_skill_status_line(
    enabled: bool,
    status: &crate::app_store::RunnerSkillStatus,
) -> Option<(String, bool)> {
    if let Some(folder) = &status.foreign {
        return Some((
            crate::i18n::tf(
                "format.was_not_installed_by_runner_and_is_left_alone",
                &[("path", (folder.display()).to_string())],
            ),
            true,
        ));
    }
    (enabled && !status.detected).then(|| {
        (
            crate::i18n::text(
                "ui.no_supported_agent_detected_yet_the_skill_is_added_when_one_is_installed",
            )
            .into(),
            false,
        )
    })
}

impl NativeRoot {
    pub(super) fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let zoom_value = normalize_zoom(self.settings(cx).app_zoom);
        let zoom_index = ZOOM_STEPS
            .iter()
            .position(|step| *step == zoom_value)
            .unwrap_or(2);
        let decrement_root = cx.entity();
        let increment_root = decrement_root.clone();
        let decrement: StepHandler = Rc::new(move |window, cx| {
            decrement_root.update(cx, |this, root_cx| {
                this.set_zoom(nudge_zoom(zoom_value, -1), window, root_cx)
            });
        });
        let increment: StepHandler = Rc::new(move |window, cx| {
            increment_root.update(cx, |this, root_cx| {
                this.set_zoom(nudge_zoom(zoom_value, 1), window, root_cx)
            });
        });
        let zoom = Stepper::new(
            "settings-app-zoom",
            56.,
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_ui())
                .font_weight(FontWeight::MEDIUM)
                .child(format!(
                    "{}%",
                    (ZOOM_STEPS[zoom_index] * 100.).round() as u16
                )),
            decrement,
            increment,
        )
        .decrement_disabled(zoom_index == 0)
        .increment_disabled(zoom_index == ZOOM_STEPS.len() - 1);
        let browse_root = cx.entity();
        let working_dir = WorkingDirField::new(
            self.settings_page.default_working_dir.clone(),
            false,
            Rc::new(move |_, cx| {
                browse_root.update(cx, |this, root_cx| {
                    this.browse_settings_working_dir(root_cx)
                });
            }),
        )
        .browse_focus(self.settings_page.working_dir_browse_focus.clone());
        let file_link_editor = self.settings(cx).file_link_editor;
        // The file viewer's Open in menu sets the same choice.
        if self.settings_page.file_link_editor.read(cx).value() != file_link_editor.key() {
            self.settings_page
                .file_link_editor
                .update(cx, |select, cx| {
                    select.set_value(file_link_editor.key(), cx)
                });
        }
        let found = file_link_editor.cli().map(|command| {
            self.app_store
                .read(cx)
                .file_link_environment
                .1
                .get(command)
                .copied()
                .unwrap_or(false)
        });
        let (file_link_hint, file_link_warns) = file_link_hint(file_link_editor, found);
        let file_link_row = SettingsRow::new(
            crate::i18n::text("ui.open_file_links_in"),
            self.settings_page.file_link_editor.clone(),
        )
        .subtitle(file_link_hint);
        let file_link_row = if file_link_warns {
            file_link_row.subtitle_color(theme::warning())
        } else {
            file_link_row
        };
        let toggle_root = cx.entity();
        let (command_status, command_requires_escalation, command_action_target, skill_status) = {
            let store = self.app_store.read(cx);
            (
                store.runner_command_status().cloned(),
                store.command_install_requires_escalation(),
                store.command_action_target().cloned(),
                store.runner_skill_status().clone(),
            )
        };
        let skill_enabled = self.settings(cx).runner_skill_enabled;
        let command_row = self.render_command_install_row(
            command_status.as_ref(),
            command_requires_escalation,
            command_action_target.as_deref(),
            cx,
        );
        let skill_toggle_root = cx.entity();
        let skill_row = command_line_settings_row(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(crate::i18n::text("ui.runner_skill_for_agents"))
                .into_any_element(),
            crate::i18n::text("ui.installs_a_skill_for_every_detected_agent_so_it_can_find_runner_and_drive_it_through_the_r"),
            runner_skill_status_line(skill_enabled, &skill_status)
                .into_iter()
                .map(|(text, warning)| {
                    div()
                        .text_size(theme::text_caption())
                        .text_color(if warning { theme::warning() } else { theme::faint() })
                        .child(text)
                        .into_any_element()
                })
                .collect(),
            Some(
                Toggle::new("settings-runner-skill", skill_enabled)
                    .on_change(move |enabled, _, cx| {
                        skill_toggle_root.update(cx, |this, root_cx| {
                            this.set_runner_skill_enabled(enabled, root_cx)
                        });
                    })
                    .into_any_element(),
            ),
        );

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(crate::i18n::text("ui.general"), crate::i18n::text("ui.defaults_and_startup_behavior")))
            .child(SettingsCard::new(vec![
                SettingsRow::new(
                    i18n::t("language.title"),
                    div().when(cfg!(test), |select| select.debug_selector(|| "SETTINGS_LANGUAGE_SELECT".into()))
                        .child(self.settings_page.language.clone()),
                ).subtitle(i18n::t("language.description")).wrap_control().into_any_element(),
                SettingsRow::new(
                    crate::i18n::text("ui.default_working_directory"),
                    div().w(rems(280. / 16.)).child(working_dir),
                )
                .subtitle(crate::i18n::text("ui.default_for_new_chats_leave_blank_to_use_your_home_directory"))
                .into_any_element(),
                file_link_row.into_any_element(),
                SettingsRow::new(crate::i18n::text("ui.app_zoom"), zoom)
                    .subtitle(
                        crate::i18n::text("ui.whole_app_scale_doesn_t_apply_to_the_terminal_canvas_see_terminal_pane"),
                    )
                    .into_any_element(),
            ]))
            .child(
                div().flex().flex_col().gap_2()
                    .child(settings_section_heading(crate::i18n::text("ui.sessions")))
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("ui.when_runner_quits"), self.settings_page.quit_behavior.clone(),
                    ).subtitle(crate::i18n::tnf("format.keep_sessions_running_in_the_background_or_stop_them_until_you_next_open_runne", self.app_store.read(cx).live_session_count, &[])).into_any_element()])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text("ui.command_line")))
                    .child(SettingsCard::new(vec![command_row, skill_row])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_1()
                            .text_size(theme::text_meta())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child(crate::i18n::text("heading.startup")),
                    )
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("ui.resume_running_agents_on_launch"),
                        Toggle::new(
                            "settings-resume-on-launch",
                            self.settings(cx).resume_on_launch,
                        )
                        .on_change(move |enabled, _, cx| {
                            toggle_root.update(cx, |this, root_cx| {
                                this.set_resume_on_launch(enabled, root_cx)
                            });
                        }),
                    )
                    .subtitle(
                        crate::i18n::text("ui.reopen_chats_and_mission_agents_that_were_live_when_runner_quit"),
                    )
                    .into_any_element()])),
            )
            .into_any_element()
    }

    pub(super) fn render_command_install_row(
        &self,
        status: Option<&RunnerCommandStatus>,
        requires_escalation: bool,
        action_target: Option<&Path>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.settings_page.command_install_activity;
        let install_root = cx.entity();
        let uninstall_root = install_root.clone();
        let mut lines = Vec::new();
        let mut control = None;
        match status.map(|status| &status.state) {
            Some(RunnerCommandState::Installed | RunnerCommandState::Shadowed) => {
                if cfg!(any(windows, target_os = "linux")) {
                    lines.push(command_status_text(
                        crate::i18n::text("ui.on_your_path_new_terminals_pick_it_up"),
                        theme::accent(),
                        Some("check.svg"),
                    ));
                } else if let Some(path) = status.and_then(|status| status.path.as_ref()) {
                    lines.push(command_installed_line(path));
                }
                if let Some(path) = status.and_then(|status| status.shadowed_by.as_ref()) {
                    let command_name =
                        runner_core::command_install::runner_command_name(cfg!(debug_assertions));
                    lines.push(command_status_text(
                        crate::i18n::tf(
                            "format.comes_first_on_your_path_so_typing_starts_that_one",
                            &[
                                ("path", (path.display()).to_string()),
                                ("command", (command_name).to_string()),
                            ],
                        ),
                        theme::warning(),
                        Some("triangle-alert.svg"),
                    ));
                }
                let label = if busy == Some(CommandInstallActivity::Uninstall) {
                    crate::i18n::text("ui.uninstalling")
                } else {
                    crate::i18n::text("ui.uninstall")
                };
                control = Some(
                    Button::new("settings-uninstall-runner-command", label)
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(busy.is_some())
                        .on_press(move |_, cx| {
                            uninstall_root
                                .update(cx, |this, root_cx| this.uninstall_runner_command(root_cx));
                        })
                        .into_any_element(),
                );
            }
            Some(RunnerCommandState::Managed) => {
                lines.push(command_status_text(
                    crate::i18n::text("ui.linux_package_command_removal"),
                    theme::accent(),
                    Some("check.svg"),
                ));
                if let Some(path) = status.and_then(|status| status.shadowed_by.as_ref()) {
                    lines.push(command_status_text(
                        crate::i18n::tf(
                            "format.comes_first_on_your_path_so_typing_starts_that_one",
                            &[
                                ("path", path.display().to_string()),
                                (
                                    "command",
                                    runner_core::command_install::runner_command_name(cfg!(
                                        debug_assertions
                                    ))
                                    .to_string(),
                                ),
                            ],
                        ),
                        theme::warning(),
                        Some("triangle-alert.svg"),
                    ));
                }
            }
            Some(RunnerCommandState::Foreign) => {
                if let Some(path) = status.and_then(|status| status.path.as_ref()) {
                    lines.push(command_status_text(
                        crate::i18n::tf(
                            "format.belongs_to_another_program_runner_will_not_replace_it",
                            &[("path", (path.display()).to_string())],
                        ),
                        theme::warning(),
                        Some("triangle-alert.svg"),
                    ));
                }
            }
            Some(RunnerCommandState::Unsupported) => {
                lines.push(command_status_text(
                    crate::i18n::text(
                        "ui.the_development_build_does_not_install_a_command_on_windows",
                    ),
                    theme::faint(),
                    None,
                ));
            }
            Some(RunnerCommandState::NotInstalled) | None => {
                let text = if requires_escalation && busy == Some(CommandInstallActivity::Install) {
                    crate::i18n::text("ui.waiting_for_your_password").to_owned()
                } else if requires_escalation {
                    let target = action_target
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| {
                            format!(
                                "/usr/local/bin/{}",
                                runner_core::command_install::runner_command_name(cfg!(
                                    debug_assertions
                                ))
                            )
                        });
                    if runner_core::protocol::command::force_escalated_command_install() {
                        crate::i18n::tf(
                            "format.not_installed_install_links_and_asks_for_your_password_once",
                            &[("target", (target).to_string())],
                        )
                    } else {
                        crate::i18n::tf("format.not_installed_local_bin_is_not_on_your_path_so_install_links_and_asks_for_your", &[("target", (target).to_string())])
                    }
                } else {
                    crate::i18n::text("ui.not_installed").to_owned()
                };
                lines.push(command_status_text(text, theme::faint(), None));
                let label = if busy == Some(CommandInstallActivity::Install) {
                    crate::i18n::text("ui.installing")
                } else if requires_escalation {
                    crate::i18n::text("ui.install")
                } else {
                    crate::i18n::text("ui.install_command_action")
                };
                control = Some(
                    Button::new("settings-install-runner-command", label)
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(busy.is_some())
                        .on_press(move |_, cx| {
                            install_root
                                .update(cx, |this, root_cx| this.install_runner_command(root_cx));
                        })
                        .into_any_element(),
                );
            }
        }
        if let Some((activity, path, reason)) = &self.settings_page.command_install_error {
            let verb = match activity {
                CommandInstallActivity::Install => crate::i18n::text("word.link"),
                CommandInstallActivity::Uninstall => crate::i18n::text("word.remove"),
            };
            lines.push(command_status_text(
                crate::i18n::tf(
                    "format.could_not",
                    &[
                        ("verb", (verb).to_string()),
                        ("path", (path.display()).to_string()),
                        ("reason", (reason.trim_end_matches('.')).to_string()),
                    ],
                ),
                theme::danger(),
                None,
            ));
        }
        command_line_settings_row(
            div()
                .flex()
                .items_baseline()
                .gap_1()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(div().font_family(theme::UI_MONOSPACE_FONT).child(
                    runner_core::command_install::runner_command_name(cfg!(debug_assertions)),
                ))
                .child(crate::i18n::text("word.command_suffix"))
                .into_any_element(),
            crate::i18n::text("ui.use_runner_from_any_terminal_or_script"),
            lines,
            control,
        )
    }
}

/// The row subtitle for the current editor choice, and whether it warns.
pub(super) fn file_link_hint(editor: FileLinkEditor, cli_found: Option<bool>) -> (String, bool) {
    match (editor, cli_found) {
        (FileLinkEditor::DefaultApp, _) => (
            crate::i18n::tf(
                "format.click_opens_the_file_in_its_default_app_the_cited_line_is_lost",
                &[(
                    "shortcut",
                    (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                )],
            ),
            false,
        ),
        (editor, Some(false)) => (
            crate::i18n::tf(
                "format.not_found_on_path",
                &[
                    ("command", (editor.cli().unwrap_or_default()).to_string()),
                    (
                        "install_hint",
                        (editor.install_hint().unwrap_or_default()).to_string(),
                    ),
                ],
            ),
            true,
        ),
        (editor, _) => (
            crate::i18n::tf(
                "format.click_opens_the_file_at_the_cited_line_via",
                &[
                    (
                        "shortcut",
                        (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                    ),
                    ("editor", (editor.cli().unwrap_or_default()).to_string()),
                ],
            ),
            false,
        ),
    }
}

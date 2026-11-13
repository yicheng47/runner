use super::render::{runtime_section_caption, runtime_section_header};
use super::*;
use runner_core::protocol::runtime::{
    RuntimeCommandNote, RuntimeCommandPreview, RuntimeLaunchOption,
};

fn switch_copy(
    runtime: Runtime,
    option: RuntimeLaunchOption,
) -> (&'static str, &'static str, &'static str) {
    match option {
        RuntimeLaunchOption::Bypass if runtime == Runtime::Codex => (
            "ui.launch_codex_bypass",
            "ui.launch_codex_bypass_description",
            "On: --ask-for-approval never --sandbox danger-full-access",
        ),
        RuntimeLaunchOption::Bypass => (
            "ui.launch_claude_bypass",
            "ui.launch_claude_bypass_description",
            "On: --permission-mode bypassPermissions",
        ),
        RuntimeLaunchOption::RemotePluginCatalog => (
            "ui.launch_catalog",
            "ui.launch_catalog_description",
            "Off: -c features.remote_plugin=false",
        ),
        RuntimeLaunchOption::StatusHooks => (
            "ui.launch_status_hooks",
            match runtime {
                Runtime::Codex => "ui.launch_codex_hooks_description",
                Runtime::ClaudeCode => "ui.launch_claude_hooks_description",
                Runtime::Pi => "ui.launch_pi_hooks_description",
                _ => "ui.launch_activity_hooks_description",
            },
            match runtime {
                Runtime::Codex => "On: --enable hooks --dangerously-bypass-hook-trust -c hooks.…",
                Runtime::ClaudeCode => "On: --settings {\"hooks\":{…}}",
                Runtime::Pi => "On: -e <Runner status extension>",
                Runtime::Copilot => "On: --plugin-dir <Runner status plugin>",
                Runtime::OpenCode => "On: OPENCODE_CLI_CONFIG_CONTENT=<Runner status plugin>",
                _ => "On: <Runner status hooks>",
            },
        ),
    }
}

impl AgentsPane {
    pub(super) fn sync_launch_copy(&self, reset_copied: bool, cx: &mut Context<Self>) {
        let value = self
            .selected_runtime
            .and_then(|runtime| self.launch_previews.get(&(runtime, self.mission_preview)))
            .map(preview_clipboard_text);
        self.launch_copy.update(cx, |copy, copy_cx| {
            if reset_copied {
                copy.reset_copied(copy_cx);
            }
            copy.set_value(value, copy_cx);
        });
    }

    pub(super) fn refresh_launch_previews(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        let core = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            Ok::<_, String>((
                core.runtime_command_preview(runtime, false)
                    .map_err(|e| e.to_string())?,
                core.runtime_command_preview(runtime, true)
                    .map_err(|e| e.to_string())?,
            ))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((chat, mission)) => {
                        this.launch_preview_errors.remove(&runtime);
                        this.launch_previews.insert((runtime, false), chat);
                        this.launch_previews.insert((runtime, true), mission);
                    }
                    Err(error) => {
                        this.launch_previews.remove(&(runtime, false));
                        this.launch_previews.remove(&(runtime, true));
                        this.launch_preview_errors.insert(runtime, error);
                    }
                }
                this.sync_launch_copy(false, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_launch_option(
        &mut self,
        runtime: Runtime,
        option: RuntimeLaunchOption,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.saving.insert(runtime) {
            return;
        }
        let core = self.app_store.read(cx).client.clone();
        let task =
            cx.background_spawn(
                async move { core.runtime_set_launch_option(runtime, option, enabled) },
            );
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.saving.remove(&runtime);
                match result {
                    Ok(status) => {
                        this.apply_status(status, cx);
                        this.error = None;
                        this.refresh_launch_previews(runtime, cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_launch(
        &self,
        runtime: &RuntimeExecutableStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = runtime.name;
        let mut rows = Vec::new();
        for option in [
            RuntimeLaunchOption::Bypass,
            RuntimeLaunchOption::RemotePluginCatalog,
            RuntimeLaunchOption::StatusHooks,
        ] {
            if !option.supported(name) {
                continue;
            }
            let (title, description, flags) = switch_copy(name, option);
            let pane = cx.entity();
            rows.push(
                div()
                    .debug_selector(|| format!("AGENT_LAUNCH_SWITCH_{name}_{option:?}"))
                    .flex()
                    .items_center()
                    .gap_6()
                    .px_5()
                    .py(rems(14. / 16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme::text_body())
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(crate::i18n::text(title)),
                                    )
                                    .child(Badge::new(
                                        crate::i18n::text(
                                            if option == RuntimeLaunchOption::RemotePluginCatalog {
                                                "ui.launch_chats_and_missions"
                                            } else {
                                                "ui.launch_chats"
                                            },
                                        ),
                                        Tone::Muted,
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text(description)),
                            )
                            .child(
                                div()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(flag_copy(flags)),
                            ),
                    )
                    .child(
                        Toggle::new(
                            SharedString::from(format!("launch-{name}-{option:?}")),
                            runtime.launch_options.get(option),
                        )
                        .disabled(self.saving.contains(&name))
                        .on_change(move |enabled, _, cx| {
                            pane.update(cx, |this, cx| {
                                this.set_launch_option(name, option, enabled, cx)
                            })
                        }),
                    )
                    .into_any_element(),
            );
        }
        let preview = self.launch_previews.get(&(name, self.mission_preview));
        self.launch_copy.update(cx, |copy, copy_cx| {
            copy.set_value(preview.map(preview_clipboard_text), copy_cx);
        });
        let tabs = div()
            .flex()
            .gap(rems(2. / 16.))
            .p(rems(2. / 16.))
            .rounded(rems(7. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .children([false, true].into_iter().map(|mission| {
                let pane = cx.entity();
                Button::new(
                    SharedString::from(format!("launch-tab-{name}-{mission}")),
                    crate::i18n::text(if mission {
                        "ui.launch_mission"
                    } else {
                        "ui.chat"
                    }),
                )
                .debug_selector(format!(
                    "AGENT_LAUNCH_TAB_{name}_{}",
                    if mission { "mission" } else { "chat" }
                ))
                .size(ButtonSize::Sm)
                .variant(if mission == self.mission_preview {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .on_press(move |_, cx| {
                    pane.update(cx, |this, cx| {
                        let changed = this.mission_preview != mission;
                        this.mission_preview = mission;
                        this.sync_launch_copy(changed, cx);
                        cx.notify();
                    })
                })
            }));
        let command = div()
            .debug_selector(|| format!("AGENT_LAUNCH_COMMAND_{name}"))
            .flex()
            .flex_col()
            .gap(rems(10. / 16.))
            .px_5()
            .py(rems(14. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("ui.command")),
                    )
                    .child(tabs)
                    .child(
                        div()
                            .ml_auto()
                            .debug_selector(|| format!("AGENT_LAUNCH_COPY_{name}"))
                            .child(self.launch_copy.clone()),
                    ),
            )
            .child(render_command(preview))
            .children(
                self.launch_preview_errors
                    .get(&name)
                    .map(|error| runtime_section_caption(error.clone())),
            );
        rows.push(command.into_any_element());
        div()
            .debug_selector(|| format!("AGENT_LAUNCH_{name}"))
            .flex()
            .flex_col()
            .gap_2()
            .child(runtime_section_header(crate::i18n::text("ui.launch"), None))
            .child(SettingsCard::new(rows))
            .child(runtime_section_caption(crate::i18n::text(
                "ui.launch_caption",
            )))
            .into_any_element()
    }
}

fn preview_clipboard_text(preview: &RuntimeCommandPreview) -> String {
    preview
        .lines
        .iter()
        .map(|line| line.command.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_command(preview: Option<&RuntimeCommandPreview>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rems(5. / 16.))
        .px_4()
        .py_3()
        .rounded(rems(10. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .children(
            preview
                .into_iter()
                .flat_map(|preview| &preview.lines)
                .map(|line| {
                    div()
                        .flex()
                        .items_start()
                        .gap_4()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_meta())
                                .child(line.command.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(theme::text_meta())
                                .text_color(if line.option.is_some() {
                                    theme::accent()
                                } else {
                                    theme::faint()
                                })
                                .child(preview_note(&line.note)),
                        )
                }),
        )
}

fn flag_copy(flags: &str) -> String {
    if let Some(flags) = flags.strip_prefix("On: ") {
        format!("{}: {flags}", crate::i18n::text("ui.launch_on"))
    } else if let Some(flags) = flags.strip_prefix("Off: ") {
        format!("{}: {flags}", crate::i18n::text("ui.launch_off"))
    } else {
        flags.into()
    }
}

fn preview_note(note: &RuntimeCommandNote) -> SharedString {
    use RuntimeCommandNote as Note;
    let key = match note {
        Note::None => return "".into(),
        Note::Always => "ui.launch_always",
        Note::AlwaysInMissions => "ui.launch_always_missions",
        Note::NamesChat => "ui.launch_names_chat",
        Note::FromChat => "ui.launch_from_chat",
        Note::FromRole => "ui.launch_from_role",
        Note::FirstTurn => "ui.launch_first_turn",
        Note::MissionFiles => "ui.launch_mission_files",
        Note::CodexBypass => "ui.launch_codex_bypass",
        Note::ClaudeBypass => "ui.launch_claude_bypass",
        Note::CatalogOff => "ui.launch_catalog_off",
        Note::StatusHooks => "ui.launch_status_hooks_chats",
        Note::HooksBypass => "ui.launch_hooks_bypass",
        Note::IdentityBypass => "ui.launch_identity_bypass",
        Note::Identity => "ui.launch_identity_hook",
        Note::ActiveAccount { name } => {
            return crate::i18n::tf("format.launch_active_account", &[("account", name.clone())])
                .into()
        }
    };
    crate::i18n::text(key).into()
}

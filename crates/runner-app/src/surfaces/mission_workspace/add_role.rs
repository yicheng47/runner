use std::collections::HashSet;

use runner_core::{protocol::mission::MissionAddRoleInput, slot_handle::suggest_slot_handle};

use super::*;

pub(super) struct MissionRoleForm {
    mission_id: String,
    title: String,
    cwd: String,
    current: HashSet<String>,
    removed: HashSet<String>,
    handle: Entity<TextField>,
    task: Entity<TextField>,
    _handle_subscription: gpui::Subscription,
    _task_subscription: gpui::Subscription,
}

fn handle_error(form: &MissionRoleForm, cx: &App) -> Option<crate::i18n::Text> {
    runner_app::ui::field::slot_handle_error(
        form.handle.read(cx).text(),
        &form.current,
        &form.removed,
        "mission",
    )
}

fn can_add(modal: &StartChatModal, cx: &App) -> bool {
    modal.can_submit()
        && !modal.is_composing(cx)
        && modal.mission_role.as_ref().is_some_and(|form| {
            !form.handle.read(cx).text().is_empty()
                && handle_error(form, cx).is_none()
                && !form.handle.read(cx).is_composing()
                && !form.task.read(cx).is_composing()
        })
}

impl NativeRoot {
    pub(crate) fn open_add_role_modal(
        &mut self,
        mission: runner_core::protocol::model::Mission,
        roster: Vec<runner_core::protocol::model::SlotWithRole>,
        events: Vec<runner_core::Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        self.open_start_chat_modal(ChatTarget::NewTab, None, None, window, cx);
        let handle = cx.new(|cx| TextField::new(cx.focus_handle(), "", "handle", true));
        handle.update(cx, |field, cx| field.set_bare(true, cx));
        let task = cx.new(|cx| {
            TextField::textarea(
                cx.focus_handle(),
                "",
                crate::i18n::message("ui.what_should_this_agent_do"),
                3,
                false,
            )
        });
        let handle_subscription = cx.observe(&handle, |this, field, cx| {
            if let Some(form) = this
                .start_chat_modal
                .as_ref()
                .and_then(|modal| modal.mission_role.as_ref())
            {
                let error = handle_error(form, cx);
                field.update(cx, |field, cx| {
                    field.set_validation(
                        error.map_or(
                            runner_app::ui::FieldValidation::Valid,
                            runner_app::ui::FieldValidation::error,
                        ),
                        cx,
                    )
                });
            }
            cx.notify();
        });
        let task_subscription = cx.observe(&task, |_, _, cx| cx.notify());
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.mode = ChatMode::Role;
            modal.mission_role = Some(MissionRoleForm {
                mission_id: mission.id,
                title: mission.title,
                cwd: mission.cwd.unwrap_or_default(),
                current: roster
                    .into_iter()
                    .map(|member| member.slot.slot_handle)
                    .collect(),
                removed: events
                    .into_iter()
                    .filter(|event| {
                        event.from == "human"
                            && event.kind == runner_core::EventKind::Signal
                            && event
                                .signal_type
                                .as_ref()
                                .is_some_and(|kind| kind.as_str() == "role_removed")
                    })
                    .filter_map(|event| event.payload["handle"].as_str().map(str::to_owned))
                    .collect(),
                handle,
                task,
                _handle_subscription: handle_subscription,
                _task_subscription: task_subscription,
            });
            clear_role_overrides(modal, cx);
            modal.role_select.read(cx).focus_handle().focus(window, cx);
        }
        self.suggest_add_role_handle(cx);
        self.refresh_start_chat_models(cx);
        cx.notify();
    }

    pub(super) fn suggest_add_role_handle(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.start_chat_modal.as_ref() else {
            return;
        };
        let Some(form) = &modal.mission_role else {
            return;
        };
        if let Some(role) = modal.selected_role() {
            let taken = form.current.union(&form.removed).cloned().collect();
            let suggestion = suggest_slot_handle(&role.handle, &taken);
            form.handle
                .update(cx, |field, cx| field.reset(suggestion, cx));
        }
    }

    pub(super) fn submit_mission_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self
            .start_chat_modal
            .as_ref()
            .filter(|modal| can_add(modal, cx))
        else {
            return;
        };
        let form = modal.mission_role.as_ref().unwrap();
        let input = MissionAddRoleInput {
            mission_id: form.mission_id.clone(),
            role_id: modal.selected_role().unwrap().id.clone(),
            handle: Some(form.handle.read(cx).text().to_owned()),
            runtime: modal
                .role_runtime_override
                .as_deref()
                .and_then(runner_core::Runtime::parse),
            model: modal.model_override(cx),
            effort: modal.effort_override(),
            speed: modal.speed_override(),
            task: normalized_value(form.task.read(cx).text()),
        };
        let modal = self.start_chat_modal.as_mut().unwrap();
        modal.submitting = true;
        modal.error = None;
        set_start_chat_controls_disabled(modal, true, cx);
        if let Some(form) = &modal.mission_role {
            form.handle
                .update(cx, |field, cx| field.set_disabled(true, cx));
            form.task
                .update(cx, |field, cx| field.set_disabled(true, cx));
        }
        let core = self.core(cx).clone();
        let add = cx.background_spawn(async move {
            core.mission_add_role(input)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = add.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.start_chat_modal = None;
                        this.focus_active_terminal(window, cx);
                    }
                    Err(error) => {
                        if let Some(modal) = this.start_chat_modal.as_mut() {
                            modal.submitting = false;
                            modal.error = Some(error.into());
                            set_start_chat_controls_disabled(modal, false, cx);
                            if let Some(form) = &modal.mission_role {
                                form.handle
                                    .update(cx, |field, cx| field.set_disabled(false, cx));
                                form.task
                                    .update(cx, |field, cx| field.set_disabled(false, cx));
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_add_role_modal(
        &self,
        modal: &StartChatModal,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let form = modal.mission_role.as_ref().unwrap();
        let layout = CardLayout::new(OverlayWidth::Custom(MODAL_WIDTH).body_width(window));
        let error = handle_error(form, cx);
        let handle = form.handle.read(cx).text().to_owned();
        let root = cx.entity();
        let close = root.clone();
        let cancel = root.clone();
        let submit = root.clone();
        let dismiss = root;
        let mut handle_field = Field::new(
            "add-role-handle",
            crate::i18n::text("ui.handle"),
            div()
                .flex()
                .items_center()
                .gap_1()
                .rounded(rems(6. / 16.))
                .border_1()
                .border_color(if error.is_some() {
                    theme::danger()
                } else {
                    theme::border()
                })
                .bg(theme::bg())
                .pl_3()
                .child(div().text_color(theme::muted()).child("@"))
                .child(div().flex_1().child(form.handle.clone())),
        )
        .focus_target(form.handle.read(cx).focus_handle())
        .emphasized(true);
        if let Some(error) = error {
            handle_field = handle_field.error(error.resolve());
        } else {
            handle_field = handle_field.subtitle(crate::i18n::text("ui.how_the_team_addresses_it"));
        }
        let content = div().debug_selector(|| "ADD_ROLE_FORM".into()).flex().flex_col().gap_5()
            .on_key_down(cx.listener(Self::on_start_chat_key_down))
            .children(modal.error.as_ref().map(|error| runner_app::ui::notice_banner(error.resolve(), runner_app::ui::Tone::Danger)))
            .child(self.render_role_fields(modal, layout, cx))
            .child(handle_field)
            .child(Field::new("add-role-task", crate::i18n::text("ui.task"), form.task.clone()).tag(crate::i18n::text("word.optional"))
                .focus_target(form.task.read(cx).focus_handle()).emphasized(true)
                .subtitle(crate::i18n::tf("format.sent_to_as_a_message_from_you_left_blank_it_waits_until_someone_messages_it", &[("handle", (handle).to_string())])));
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::text("ui.add_role")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "format.a_new_worker_joins",
                                &[("mission", (form.title).to_string())],
                            )),
                    ),
            )
            .child(
                IconButton::new("close-add-role", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip(crate::i18n::text("ui.close_add_role"))
                    .disabled(modal.submitting)
                    .on_press(move |window, cx| {
                        close.update(cx, |this, cx| this.close_start_chat_modal(window, cx))
                    }),
            );
        let footer = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(crate::i18n::tf(
                        "format.works_in",
                        &[("directory", (form.cwd).to_string())],
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("cancel-add-role", crate::i18n::text("ui.cancel"))
                            .shortcut("esc")
                            .focus_handle(modal.cancel_focus.clone())
                            .disabled(modal.submitting)
                            .on_press(move |window, cx| {
                                cancel
                                    .update(cx, |this, cx| this.close_start_chat_modal(window, cx))
                            }),
                    )
                    .child(
                        Button::new(
                            "submit-add-role",
                            if modal.submitting {
                                crate::i18n::text("ui.adding_worker")
                            } else {
                                crate::i18n::text("ui.add_role")
                            },
                        )
                        .shortcut(keymap::fixed_shortcut("cmd-enter"))
                        .focus_handle(modal.submit_focus.clone())
                        .variant(ButtonVariant::Primary)
                        .disabled(!can_add(modal, cx))
                        .on_press(move |window, cx| {
                            submit.update(cx, |this, cx| this.submit_mission_role(window, cx))
                        }),
                    ),
            );
        let mut focus = start_chat_focus_order(modal, cx);
        focus.retain(|item| {
            *item != modal.title.read(cx).focus_handle()
                && *item != modal.cwd.read(cx).focus_handle()
                && *item != modal.browse_focus
                && *item != modal.role_mode_focus
        });
        focus.insert(
            focus.len().saturating_sub(2),
            form.handle.read(cx).focus_handle(),
        );
        focus.insert(
            focus.len().saturating_sub(2),
            form.task.read(cx).focus_handle(),
        );
        div()
            .absolute()
            .inset_0()
            .on_action(cx.listener(Self::confirm_start_chat))
            .child(
                Modal::new(
                    title,
                    content,
                    Rc::new(move |window, cx| {
                        dismiss.update(cx, |this, cx| this.close_start_chat_modal(window, cx))
                    }),
                )
                .key_context("StartChat")
                .width(OverlayWidth::Custom(MODAL_WIDTH))
                .busy(modal.submitting)
                .focus_order(if modal.submitting { Vec::new() } else { focus })
                .scrollbar(modal.scroll_handle.clone(), modal.scrollbar.clone())
                .footer(footer),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_suggests_handles_and_disables_invalid_taken_and_removed_names() {
        let now = chrono::Utc::now();
        let role = Role {
            id: "role-coder".into(),
            handle: "coder".into(),
            display_name: "Coder".into(),
            runtime: "codex".into(),
            command: "codex".into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            avatar_path: None,
            avatar_style: Default::default(),
            avatar_color: None,
            created_at: now,
            updated_at: now,
        };
        let mut harness =
            super::super::tests::modal_harness(900., 1000., vec![role], Vec::new(), ChatMode::Role);
        harness.act(|root, _, cx| {
            let handle = cx.new(|cx| TextField::new(cx.focus_handle(), "", "", false));
            let task = cx.new(|cx| TextField::textarea(cx.focus_handle(), "", "", 3, false));
            let handle_subscription = cx.observe(&handle, |_, _, cx| cx.notify());
            let task_subscription = cx.observe(&task, |_, _, cx| cx.notify());
            root.start_chat_modal.as_mut().unwrap().mission_role = Some(MissionRoleForm {
                mission_id: "m".into(),
                title: "Test mission".into(),
                cwd: "/tmp".into(),
                current: HashSet::from(["coder".into()]),
                removed: HashSet::from(["coder-2".into()]),
                handle,
                task,
                _handle_subscription: handle_subscription,
                _task_subscription: task_subscription,
            });
            root.suggest_add_role_handle(cx);
            let modal = root.start_chat_modal.as_ref().unwrap();
            assert_eq!(
                modal.mission_role.as_ref().unwrap().handle.read(cx).text(),
                "coder-3"
            );
            assert!(can_add(modal, cx));
            for value in ["", "coder", "coder-2", "Bad"] {
                let form = modal.mission_role.as_ref().unwrap();
                form.handle.update(cx, |field, cx| field.reset(value, cx));
                assert!(!can_add(modal, cx), "{value}");
            }
            assert!(handle_error(modal.mission_role.as_ref().unwrap(), cx).is_some());
        });
    }
}

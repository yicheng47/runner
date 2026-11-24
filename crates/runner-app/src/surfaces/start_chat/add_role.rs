use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use runner_core::{protocol::mission::MissionAddRoleInput, slot_handle::suggest_slot_handle};

use super::*;
use crate::surfaces::front_page::{FrontPageClient, FrontPageTarget};

static NEXT_ADD_ROLE_MODAL_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct AddRoleContext {
    pub(crate) mission_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) current: HashSet<String>,
    pub(crate) removed: HashSet<String>,
    pub(crate) client: FrontPageClient,
    pub(crate) viewport_size: Option<(u16, u16)>,
}

pub(super) struct MissionRoleForm {
    modal_id: u64,
    mission_id: String,
    title: String,
    cwd: String,
    current: HashSet<String>,
    removed: HashSet<String>,
    client: FrontPageClient,
    viewport_size: Option<(u16, u16)>,
    remote_epoch: Option<u64>,
    loading_catalog: bool,
    catalog_failed: bool,
    #[cfg(test)]
    submit_gate: Option<crate::test_support::StagedReplyGate>,
    handle: Entity<TextField>,
    task: Entity<TextField>,
    _handle_subscription: gpui::Subscription,
    _task_subscription: gpui::Subscription,
}

impl MissionRoleForm {
    pub(super) fn is_remote(&self) -> bool {
        self.client.is_remote()
    }

    pub(super) fn loading_catalog(&self) -> bool {
        self.loading_catalog
    }

    pub(super) fn catalog_failed(&self) -> bool {
        self.catalog_failed
    }
}

#[cfg(test)]
impl MissionRoleForm {
    pub(super) fn fixture(client: FrontPageClient, cx: &mut Context<NativeRoot>) -> Self {
        let handle = cx.new(|cx| TextField::new(cx.focus_handle(), "test", "handle", true));
        let task = cx.new(|cx| TextField::new(cx.focus_handle(), "", "task", false));
        Self {
            modal_id: NEXT_ADD_ROLE_MODAL_ID.fetch_add(1, Ordering::Relaxed),
            mission_id: "test-mission".into(),
            title: "Test mission".into(),
            cwd: "/tmp".into(),
            current: HashSet::new(),
            removed: HashSet::new(),
            client,
            viewport_size: None,
            remote_epoch: None,
            loading_catalog: false,
            catalog_failed: false,
            submit_gate: None,
            _handle_subscription: cx.observe(&handle, |_, _, cx| cx.notify()),
            _task_subscription: cx.observe(&task, |_, _, cx| cx.notify()),
            handle,
            task,
        }
    }
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
            !form.loading_catalog
                && !form.catalog_failed
                && !form.handle.read(cx).text().is_empty()
                && handle_error(form, cx).is_none()
                && !form.handle.read(cx).is_composing()
                && !form.task.read(cx).is_composing()
        })
}

impl NativeRoot {
    fn is_current_add_role_modal(
        &self,
        modal_id: u64,
        target: &FrontPageTarget,
        remote_epoch: Option<u64>,
        cx: &App,
    ) -> bool {
        let Some(form) = self
            .start_chat_modal
            .as_ref()
            .and_then(|modal| modal.mission_role.as_ref())
        else {
            return false;
        };
        if form.modal_id != modal_id || form.client.target != *target || !form.client.valid() {
            return false;
        }
        match remote_epoch {
            Some(epoch) => {
                self.remote_client.is_active()
                    && self.remote_client.ui.epoch == epoch
                    && self.remote_client.client(cx).target == *target
            }
            None => self.remote_client.is_local(),
        }
    }

    pub(super) fn focus_after_add_role_modal(&self, window: &mut Window, cx: &mut App) {
        if self.remote_client.is_active() {
            if matches!(
                self.remote_client.active_item(),
                Some(crate::surfaces::remote_client::RemoteItem::Mission(_))
            ) {
                if let Some(workspace) = self.remote_client.mission_workspace() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.focus_active_mission_terminal(window, cx)
                    });
                    return;
                }
            }
        } else if matches!(self.route, crate::surfaces::AppRoute::Mission(_)) {
            self.mission_workspace.update(cx, |workspace, cx| {
                workspace.focus_active_mission_terminal(window, cx)
            });
            return;
        }
        self.focus_active_terminal(window, cx);
    }

    fn remote_add_role_mission_is_active_and_running(
        &self,
        mission_id: &str,
        target: &FrontPageTarget,
        cx: &App,
    ) -> bool {
        self.remote_client.is_active()
            && self.remote_client.client(cx).target == *target
            && self.remote_client.client(cx).valid()
            && matches!(
                self.remote_client.active_item(),
                Some(crate::surfaces::remote_client::RemoteItem::Mission(active))
                    if active == mission_id
            )
            && self.remote_client.overview(cx).is_some_and(|overview| {
                overview.missions.iter().any(|mission| {
                    mission.id == mission_id
                        && mission.state == runner_core::companion::MissionState::Running
                })
            })
    }

    pub(crate) fn prune_remote_add_role_modal(&mut self, cx: &mut Context<Self>) {
        let Some((mission_id, target)) = self.start_chat_modal.as_ref().and_then(|modal| {
            if modal.submitting {
                return None;
            }
            let form = modal.mission_role.as_ref()?;
            form.is_remote()
                .then(|| (form.mission_id.clone(), form.client.target.clone()))
        }) else {
            return;
        };
        if !self.remote_add_role_mission_is_active_and_running(&mission_id, &target, cx) {
            self.start_chat_modal = None;
            if let Some(workspace) = self.remote_client.mission_workspace() {
                workspace.update(cx, |workspace, _| workspace.reset_remote_dialog_focus());
            }
            cx.notify();
        }
    }

    pub(crate) fn close_stale_remote_add_role_modal(
        &mut self,
        target: &FrontPageTarget,
        active_running_mission: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let should_close = self.start_chat_modal.as_ref().is_some_and(|modal| {
            !modal.submitting
                && modal.mission_role.as_ref().is_some_and(|form| {
                    form.is_remote()
                        && form.client.target == *target
                        && active_running_mission != Some(form.mission_id.as_str())
                })
        });
        if should_close {
            self.start_chat_modal = None;
            if let Some(workspace) = self.remote_client.mission_workspace() {
                workspace.update(cx, |workspace, _| workspace.reset_remote_dialog_focus());
            }
            cx.notify();
        }
    }

    pub(crate) fn open_add_role_modal(
        &mut self,
        context: AddRoleContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        let remote = context.client.is_remote();
        if remote && !context.client.supports(&["catalog", "addMissionRole"]) {
            return;
        }
        if remote {
            self.build_add_role_form(Vec::new(), Vec::new(), window, cx);
        } else {
            self.open_add_role_form(window, cx);
        }
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
                if !form.loading_catalog && !form.catalog_failed {
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
            }
            cx.notify();
        });
        let task_subscription = cx.observe(&task, |_, _, cx| cx.notify());
        let modal_id = NEXT_ADD_ROLE_MODAL_ID.fetch_add(1, Ordering::Relaxed);
        let remote_epoch = remote.then_some(self.remote_client.ui.epoch);
        let client = context.client.clone();
        let mission_id = context.mission_id.clone();
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.mission_role = Some(MissionRoleForm {
                modal_id,
                mission_id: context.mission_id,
                title: context.title,
                cwd: context.cwd,
                current: context.current,
                removed: context.removed,
                client: context.client,
                viewport_size: context.viewport_size,
                remote_epoch,
                loading_catalog: remote,
                catalog_failed: false,
                #[cfg(test)]
                submit_gate: None,
                handle,
                task,
                _handle_subscription: handle_subscription,
                _task_subscription: task_subscription,
            });
            if remote {
                modal.role_select.update(cx, |select, select_cx| {
                    select.set_placeholder(crate::i18n::message("ui.loading"), select_cx);
                });
                set_start_chat_controls_disabled(modal, true, cx);
                if let Some(form) = &modal.mission_role {
                    form.handle
                        .update(cx, |field, cx| field.set_disabled(true, cx));
                    form.task
                        .update(cx, |field, cx| field.set_disabled(true, cx));
                }
                modal.cancel_focus.focus(window, cx);
            } else {
                clear_role_overrides(modal, cx);
                modal.role_select.read(cx).focus_handle().focus(window, cx);
            }
        }
        if remote {
            let target = client.target.clone();
            let load =
                cx.background_spawn(async move { client.catalog().map_err(|e| e.to_string()) });
            cx.spawn_in(window, async move |weak, cx| {
                let result = load.await;
                let _ = weak.update_in(cx, |this, window, cx| {
                    if !this.is_current_add_role_modal(modal_id, &target, remote_epoch, cx) {
                        return;
                    }
                    if !this.remote_add_role_mission_is_active_and_running(&mission_id, &target, cx)
                    {
                        this.start_chat_modal = None;
                        this.focus_after_add_role_modal(window, cx);
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(catalog) => {
                            let roles: Vec<RoleChoice> =
                                catalog.roles.into_iter().map(RoleChoice::from).collect();
                            let runtimes: Vec<RuntimeChoice> = catalog
                                .runtimes
                                .iter()
                                .filter_map(RuntimeChoice::remote)
                                .collect();
                            if let Some(modal) = this.start_chat_modal.as_mut() {
                                modal.roles = roles;
                                modal.runtimes = runtimes;
                                modal.role_id = modal.roles.first().map(|role| role.id.clone());
                                if let Some(form) = modal.mission_role.as_mut() {
                                    form.loading_catalog = false;
                                    form.catalog_failed = false;
                                }
                                let options = role_options(&modal.roles);
                                let value = modal.role_id.clone().unwrap_or_default();
                                let empty = modal.roles.is_empty();
                                modal.role_select.update(cx, |select, select_cx| {
                                    select.set_placeholder(
                                        crate::i18n::message("ui.no_roles_yet"),
                                        select_cx,
                                    );
                                    select.set_options(options, select_cx);
                                    select.set_value(value, select_cx);
                                    select.set_disabled(empty, select_cx);
                                });
                                clear_role_overrides(modal, cx);
                                set_start_chat_controls_disabled(modal, false, cx);
                                if let Some(form) = &modal.mission_role {
                                    form.handle
                                        .update(cx, |field, cx| field.set_disabled(false, cx));
                                    form.task
                                        .update(cx, |field, cx| field.set_disabled(false, cx));
                                }
                                modal.role_select.read(cx).focus_handle().focus(window, cx);
                            }
                            this.suggest_add_role_handle(cx);
                        }
                        Err(error) => {
                            if let Some(modal) = this.start_chat_modal.as_mut() {
                                if let Some(form) = modal.mission_role.as_mut() {
                                    form.loading_catalog = false;
                                    form.catalog_failed = true;
                                }
                                modal.role_select.update(cx, |select, select_cx| {
                                    select.set_placeholder(
                                        crate::i18n::message("ui.no_roles_yet"),
                                        select_cx,
                                    );
                                    select.set_disabled(true, select_cx);
                                });
                                set_start_chat_controls_disabled(modal, true, cx);
                                modal.error = Some(error.into());
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.suggest_add_role_handle(cx);
            self.refresh_start_chat_models(cx);
        }
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
        let modal_id = form.modal_id;
        let mission_id = form.mission_id.clone();
        let client = form.client.clone();
        let target = client.target.clone();
        let viewport_size = form.viewport_size;
        let remote_epoch = form.remote_epoch;
        let input = MissionAddRoleInput {
            mission_id: mission_id.clone(),
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
        #[cfg(test)]
        let submit_gate = modal
            .mission_role
            .as_mut()
            .and_then(|form| form.submit_gate.take());
        if let Some(form) = &modal.mission_role {
            form.handle
                .update(cx, |field, cx| field.set_disabled(true, cx));
            form.task
                .update(cx, |field, cx| field.set_disabled(true, cx));
        }
        let add = cx.background_spawn(async move {
            client
                .mission_add_role(input, viewport_size)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = add.await;
            #[cfg(test)]
            let _ack = match submit_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.is_current_add_role_modal(modal_id, &target, remote_epoch, cx) {
                    return;
                }
                match result {
                    Ok(_) => {
                        this.start_chat_modal = None;
                        this.focus_after_add_role_modal(window, cx);
                    }
                    Err(error) => {
                        let stale_remote = remote_epoch.is_some()
                            && !this.remote_add_role_mission_is_active_and_running(
                                &mission_id,
                                &target,
                                cx,
                            );
                        if stale_remote {
                            this.start_chat_modal = None;
                            this.focus_after_add_role_modal(window, cx);
                        } else if let Some(modal) = this.start_chat_modal.as_mut() {
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
        let error = (!form.loading_catalog && !form.catalog_failed)
            .then(|| handle_error(form, cx))
            .flatten();
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
            .children(modal.error.as_ref().map(|error| {
                runner_app::ui::notice_banner(error.resolve(), runner_app::ui::Tone::Danger)
                    .when(cfg!(test), |el| el.debug_selector(|| "ADD_ROLE_ERROR".into()))
            }))
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
        if !form.loading_catalog && !form.catalog_failed {
            focus.insert(
                focus.len().saturating_sub(2),
                form.handle.read(cx).focus_handle(),
            );
            focus.insert(
                focus.len().saturating_sub(2),
                form.task.read(cx).focus_handle(),
            );
        }
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
impl NativeRoot {
    pub(crate) fn add_role_loading_catalog_for_test(&self) -> bool {
        self.start_chat_modal
            .as_ref()
            .is_some_and(StartChatModal::loading_catalog)
    }

    pub(crate) fn add_role_catalog_failed_for_test(&self) -> bool {
        self.start_chat_modal
            .as_ref()
            .is_some_and(StartChatModal::catalog_failed)
    }

    pub(crate) fn add_role_error_for_test(&self) -> Option<String> {
        self.start_chat_modal
            .as_ref()
            .and_then(|modal| modal.error.as_ref().map(|text| text.resolve().to_string()))
    }

    pub(crate) fn add_role_placeholder_for_test(&self, cx: &App) -> Option<String> {
        self.start_chat_modal
            .as_ref()
            .map(|modal| modal.role_select.read(cx).placeholder_text().to_owned())
    }

    pub(crate) fn add_role_controls_disabled_for_test(
        &self,
        cx: &App,
    ) -> Option<(bool, bool, bool)> {
        let modal = self.start_chat_modal.as_ref()?;
        let form = modal.mission_role.as_ref()?;
        Some((
            modal.role_select.read(cx).is_disabled(),
            form.handle.read(cx).is_disabled(),
            form.task.read(cx).is_disabled(),
        ))
    }

    pub(crate) fn add_role_handle_for_test(&self, cx: &App) -> Option<String> {
        self.start_chat_modal
            .as_ref()?
            .mission_role
            .as_ref()
            .map(|form| form.handle.read(cx).text().to_owned())
    }

    pub(crate) fn set_add_role_handle_for_test(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(form) = self
            .start_chat_modal
            .as_ref()
            .and_then(|modal| modal.mission_role.as_ref())
        {
            form.handle.update(cx, |field, cx| field.reset(value, cx));
            cx.notify();
        }
    }

    pub(crate) fn set_add_role_task_for_test(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(form) = self
            .start_chat_modal
            .as_ref()
            .and_then(|modal| modal.mission_role.as_ref())
        {
            form.task.update(cx, |field, cx| field.reset(value, cx));
            cx.notify();
        }
    }

    pub(crate) fn set_add_role_overrides_for_test(
        &mut self,
        runtime: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        speed: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if let Some(runtime) = runtime {
            self.select_start_chat_choice(StartChatSelection::RoleRuntime, runtime, cx);
        }
        if let Some(model) = model {
            if let Some(modal) = self.start_chat_modal.as_ref() {
                modal
                    .model
                    .update(cx, |field, cx| field.set_text(model, cx));
            }
        }
        if let Some(effort) = effort {
            self.select_start_chat_choice(StartChatSelection::Effort, effort, cx);
        }
        if let Some(speed) = speed {
            self.select_start_chat_choice(StartChatSelection::Speed, speed, cx);
        }
    }

    pub(crate) fn add_role_can_submit_for_test(&self, cx: &App) -> bool {
        self.start_chat_modal
            .as_ref()
            .is_some_and(|modal| can_add(modal, cx))
    }

    pub(crate) fn submit_add_role_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_mission_role(window, cx);
    }

    pub(crate) fn close_add_role_for_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_start_chat_modal(window, cx);
    }

    pub(crate) fn stage_add_role_submit_gate_for_test(
        &mut self,
        gate: crate::test_support::StagedReplyGate,
    ) {
        if let Some(form) = self
            .start_chat_modal
            .as_mut()
            .and_then(|modal| modal.mission_role.as_mut())
        {
            form.submit_gate = Some(gate);
        }
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
            created_at: now,
            updated_at: now,
        };
        let mut harness =
            super::super::tests::modal_harness(900., 1000., vec![role], Vec::new(), ChatMode::Role);
        harness.act(|root, _, cx| {
            let client = FrontPageClient::local(root.core(cx).clone());
            let handle = cx.new(|cx| TextField::new(cx.focus_handle(), "", "", false));
            let task = cx.new(|cx| TextField::textarea(cx.focus_handle(), "", "", 3, false));
            let handle_subscription = cx.observe(&handle, |_, _, cx| cx.notify());
            let task_subscription = cx.observe(&task, |_, _, cx| cx.notify());
            root.start_chat_modal.as_mut().unwrap().mission_role = Some(MissionRoleForm {
                modal_id: NEXT_ADD_ROLE_MODAL_ID.fetch_add(1, Ordering::Relaxed),
                mission_id: "m".into(),
                title: "Test mission".into(),
                cwd: "/tmp".into(),
                current: HashSet::from(["coder".into()]),
                removed: HashSet::from(["coder-2".into()]),
                client,
                viewport_size: None,
                remote_epoch: None,
                loading_catalog: false,
                catalog_failed: false,
                submit_gate: None,
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

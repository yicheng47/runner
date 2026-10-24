use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rems, svg, AnyElement, Entity, FontWeight, PathPromptOptions, Window};
use runner_app::ui::{
    working_dir_text_field, Button, ButtonVariant, Field, IconButton, Modal, OverlayWidth,
    RoleAvatar, SelectLeading, SelectOption, StyledSelect, TextField, WorkingDirField,
};
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::model::SlotWithRole;
use runner_core::protocol::project::ProjectRow;
use runner_core::protocol::project::ProjectScope;

use crate::surfaces::crews::{crew_picture, lead_badge, slot_setup, slot_setup_line};
use crate::*;

pub(crate) struct StartMissionModalState {
    initial_crew_id: Option<String>,
    project: Option<ProjectRow>,
    crews: Vec<CrewListItem>,
    crew_id: String,
    roster: Vec<SlotWithRole>,
    crew_select: Entity<StyledSelect>,
    title: Entity<TextField>,
    goal: Entity<TextField>,
    cwd: Entity<TextField>,
    loading: bool,
    submitting: bool,
    error: Option<String>,
    close_focus: FocusHandle,
    browse_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl StartMissionModalState {
    fn can_submit(&self, cx: &App) -> bool {
        !self.submitting
            && !self.loading
            && !self.title.read(cx).text().trim().is_empty()
            && (self.crew_id.is_empty()
                || self
                    .crews
                    .iter()
                    .any(|crew| crew.crew.id == self.crew_id && crew.role_count > 0))
    }

    fn is_composing(&self, cx: &App) -> bool {
        [&self.title, &self.goal, &self.cwd]
            .into_iter()
            .any(|field| field.read(cx).is_composing())
    }
}

impl NativeRoot {
    pub(crate) fn open_start_mission_modal(
        &mut self,
        initial_crew_id: Option<String>,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_window.is_some() {
            return;
        }
        let project = scope
            .project_id()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| project.id == id)
            })
            .cloned();
        let cwd = project
            .as_ref()
            .map(|project| project.cwd.clone())
            .unwrap_or_else(|| self.settings(cx).default_working_dir.clone());
        let title = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.e_g_wire_up_event_bus_watcher"),
                false,
            )
            .text_size(theme::text_body())
        });
        let goal = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.describe_what_to_do"),
                5,
                true,
            )
            .text_size(theme::text_body())
        });
        let cwd_input = cx.new(|input_cx| {
            working_dir_text_field(
                input_cx.focus_handle(),
                cwd,
                crate::i18n::message("ui.role_default_or_home_directory"),
            )
            .text_size(theme::text_ui())
        });
        let root = cx.entity();
        let select_root = root.clone();
        let crew_select = cx.new(move |select_cx| {
            StyledSelect::new(
                "start-mission-crew",
                select_cx.focus_handle(),
                "",
                Vec::new(),
                Rc::new(move |crew_id, _, cx| {
                    select_root.update(cx, |this, cx| this.select_start_mission_crew(crew_id, cx));
                }),
                select_cx,
            )
            .picker(true)
            .placeholder(crate::i18n::message("ui.no_crews_yet"))
        });
        let crew_focus = crew_select.read(cx).focus_handle();
        let subscriptions = [&title, &goal, &cwd_input]
            .into_iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        self.start_mission_modal = Some(StartMissionModalState {
            initial_crew_id,
            project,
            crews: Vec::new(),
            crew_id: String::new(),
            roster: Vec::new(),
            crew_select,
            title,
            goal,
            cwd: cwd_input,
            loading: true,
            submitting: false,
            error: None,
            close_focus: cx.focus_handle(),
            browse_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        });
        crew_focus.focus(window, cx);
        cx.notify();

        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.crew_list(1, 10_000, "")
                .map(|page| page.items)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this.start_mission_modal.as_mut() else {
                    return;
                };
                modal.loading = false;
                match result {
                    Ok(crews) => {
                        let preferred = modal
                            .initial_crew_id
                            .as_ref()
                            .and_then(|id| crews.iter().find(|crew| &crew.crew.id == id))
                            .or_else(|| crews.iter().find(|crew| crew.role_count > 0))
                            .or_else(|| crews.first())
                            .map(|crew| crew.crew.id.clone())
                            .unwrap_or_default();
                        modal.crews = crews;
                        modal.crew_id = preferred.clone();
                        modal.crew_select.update(cx, |select, select_cx| {
                            select.set_options(start_mission_crew_options(&modal.crews), select_cx);
                            select.set_value(preferred.clone(), select_cx);
                        });
                        if !preferred.is_empty() {
                            this.load_start_mission_roster(preferred, cx);
                        }
                    }
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_start_mission_crew(&mut self, crew_id: String, cx: &mut Context<Self>) {
        let Some(modal) = self.start_mission_modal.as_mut() else {
            return;
        };
        if modal.submitting || modal.crew_id == crew_id {
            return;
        }
        modal.crew_id = crew_id.clone();
        modal.roster.clear();
        modal.error = None;
        modal.loading = !crew_id.is_empty();
        if !crew_id.is_empty() {
            self.load_start_mission_roster(crew_id, cx);
        }
        cx.notify();
    }

    fn load_start_mission_roster(&mut self, crew_id: String, cx: &mut Context<Self>) {
        let core = self.core(cx).clone();
        let load_id = crew_id.clone();
        let task = cx.background_spawn(async move {
            core.slot_list(&load_id).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this
                    .start_mission_modal
                    .as_mut()
                    .filter(|modal| modal.crew_id == crew_id)
                else {
                    return;
                };
                modal.loading = false;
                match result {
                    Ok(roster) => {
                        modal.roster = roster;
                        let options = start_mission_crew_options(&modal.crews);
                        modal.crew_select.update(cx, |select, select_cx| {
                            select.set_options(options, select_cx)
                        });
                    }
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_start_mission_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .start_mission_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.start_mission_modal = None;
        window.focus(&self.root_focus, cx);
        cx.notify();
    }

    fn browse_start_mission_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self
            .start_mission_modal
            .as_ref()
            .filter(|modal| !modal.submitting)
            .map(|modal| modal.cwd.clone())
        else {
            return;
        };
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.pick_a_working_directory").into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this
                    .start_mission_modal
                    .as_mut()
                    .filter(|modal| modal.cwd == cwd)
                else {
                    return;
                };
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

    fn confirm_start_mission(
        &mut self,
        _: &ConfirmStartMission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self
            .start_mission_modal
            .as_ref()
            .filter(|modal| modal.can_submit(cx) && !modal.is_composing(cx))
        else {
            return;
        };
        modal.crew_select.update(cx, |select, cx| select.close(cx));
        self.submit_start_mission(window, cx);
    }

    fn submit_start_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.start_mission_modal.as_mut() else {
            return;
        };
        if !modal.can_submit(cx) || modal.is_composing(cx) {
            return;
        }
        let input = runner_core::protocol::mission::MissionStart {
            crew_id: nonempty(&modal.crew_id),
            scope: ProjectScope::or_root(modal.project.as_ref().map(|project| project.id.clone())),
            title: modal.title.read(cx).text().trim().to_owned(),
            goal_override: nonempty(modal.goal.read(cx).text()),
            cwd: nonempty(modal.cwd.read(cx).text()),
        };
        modal.submitting = true;
        modal.error = None;
        set_start_mission_fields_disabled(modal, true, cx);
        let size = self.estimated_mission_terminal_size(window, cx);
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.mission_start_impl_with_size(input, Some(size))
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(output) => {
                        this.start_mission_modal = None;
                        this.refresh_store(StoreRefreshKind::All, cx);
                        this.open_mission(output.mission.id, window, cx);
                    }
                    Err(error) => {
                        if let Some(modal) = this.start_mission_modal.as_mut() {
                            modal.submitting = false;
                            modal.error = Some(error);
                            set_start_mission_fields_disabled(modal, false, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_start_mission_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self
            .start_mission_modal
            .as_ref()
            .expect("start mission modal");
        let selected = modal
            .crews
            .iter()
            .find(|crew| crew.crew.id == modal.crew_id);
        let launchable = selected.is_some_and(|crew| crew.role_count > 0);
        let can_submit = modal.can_submit(cx);
        let lead = modal.roster.iter().find(|member| member.slot.lead);
        let root = cx.entity();
        let close_root = root.clone();
        let browse_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::text("ui.start_mission")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(crate::i18n::text(
                                "ui.a_team_of_agents_on_one_goal_with_a_shared_feed",
                            )),
                    ),
            )
            .child(
                IconButton::new("close-start-mission", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip(crate::i18n::text("ui.close_start_mission"))
                    .disabled(modal.submitting)
                    .on_press(move |window, cx| {
                        close_root
                            .update(cx, |this, cx| this.close_start_mission_modal(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .children(modal.error.clone().filter(|_| !self.app_store.read(cx).daemon_disconnected).map(|error| runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)))
            .child(
                Field::new(
                    "start-mission-crew-field",
                    crate::i18n::text("ui.crew"),
                    start_mission_crew_card(modal),
                )
                .emphasized(true),
            )
            .children((selected.is_some() && !launchable).then(|| {
                div()
                    .mt(rems(-14. / 16.))
                    .text_size(theme::text_meta())
                    .text_color(theme::warning())
                    .child(crate::i18n::text("ui.this_crew_has_no_roles_add_at_least_one_before_starting_a_mission"))
            }))
            .child(
                Field::new(
                    "start-mission-title-field",
                    crate::i18n::text("ui.mission_title"),
                    modal.title.clone(),
                )
                .emphasized(true)
                .subtitle(crate::i18n::text("ui.short_label_shown_in_the_missions_list_and_event_log")),
            )
            .child(
                Field::new("start-mission-goal-field", crate::i18n::text("ui.goal"), modal.goal.clone()).tag(crate::i18n::text("word.optional"))
                    .emphasized(true)
                    .subtitle(
                        lead.map(|lead| {
                            crate::i18n::tf("format.delivered_to_lead_on_mission_start", &[("handle", (lead.slot.slot_handle).to_string())])
                        })
                        .unwrap_or_else(|| if modal.crew_id.is_empty() { crate::i18n::text("ui.shared_with_every_role_that_joins_give_each_one_its_own_task_when_you_add_it").into() } else { crate::i18n::text("ui.delivered_to_the_crew_lead_on_mission_start").into() }),
                    ),
            )
            .child(
                Field::new(
                    "start-mission-cwd-field",
                    crate::i18n::text("ui.working_directory"),
                    WorkingDirField::new(
                        modal.cwd.clone(),
                        modal.submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| this.browse_start_mission_cwd(cx));
                        }),
                    )
                    .browse_id("browse-start-mission")
                    .browse_focus(modal.browse_focus.clone()),
                )
                .emphasized(true)
                .subtitle(if modal.crew_id.is_empty() { crate::i18n::text("ui.every_role_you_add_starts_here_leave_blank_to_use_its_default_directory_or_your_home_direc") } else { crate::i18n::text("ui.each_role_starts_here_leave_blank_to_use_its_default_directory_or_your_home_directory") }),
            )
            ;
        let footer = div().w_full().flex().items_center().justify_end().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("cancel-start-mission", crate::i18n::text("ui.cancel"))
                        .shortcut("esc")
                        .focus_handle(modal.cancel_focus.clone())
                        .disabled(modal.submitting)
                        .on_press(move |window, cx| {
                            cancel_root
                                .update(cx, |this, cx| this.close_start_mission_modal(window, cx));
                        }),
                )
                .child(
                    Button::new(
                        "submit-start-mission",
                        if modal.submitting {
                            crate::i18n::text("ui.starting")
                        } else {
                            crate::i18n::text("ui.start_mission")
                        },
                    )
                    .shortcut(keymap::fixed_shortcut("cmd-enter"))
                    .focus_handle(modal.submit_focus.clone())
                    .variant(ButtonVariant::Primary)
                    .disabled(!can_submit)
                    .on_press(move |window, cx| {
                        submit_root.update(cx, |this, cx| this.submit_start_mission(window, cx));
                    }),
                ),
        );
        let close_modal_root = root;
        let modal_element = Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                close_modal_root.update(cx, |this, cx| this.close_start_mission_modal(window, cx));
            }),
        )
        .key_context("StartMission")
        .width(OverlayWidth::Custom(560.))
        .busy(modal.submitting)
        .focus_order(if modal.submitting {
            Vec::new()
        } else {
            vec![
                modal.close_focus.clone(),
                modal.crew_select.read(cx).focus_handle(),
                modal.title.read(cx).focus_handle(),
                modal.goal.read(cx).focus_handle(),
                modal.cwd.read(cx).focus_handle(),
                modal.browse_focus.clone(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
            ]
        })
        .footer(footer);
        div()
            .debug_selector(|| "START_MISSION_MODAL".into())
            .absolute()
            .inset_0()
            .on_action(cx.listener(Self::confirm_start_mission))
            .child(modal_element)
            .into_any_element()
    }
}

fn set_start_mission_fields_disabled(
    modal: &StartMissionModalState,
    disabled: bool,
    cx: &mut Context<NativeRoot>,
) {
    modal.crew_select.update(cx, |select, select_cx| {
        select.set_disabled(disabled, select_cx)
    });
    for input in [&modal.title, &modal.goal, &modal.cwd] {
        input.update(cx, |input, input_cx| input.set_disabled(disabled, input_cx));
    }
}

fn start_mission_crew_card(modal: &StartMissionModalState) -> gpui::Div {
    div()
        .w_full()
        .min_w(px(0.))
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .child(modal.crew_select.clone())
        .children(modal.crew_id.is_empty().then(|| div().border_t_1().border_color(theme::border()).px_4().py_3()
            .text_size(theme::text_ui()).text_color(theme::muted())
            .child(crate::i18n::text("ui.no_roles_at_start_add_them_one_at_a_time_from_the_mission_once_it_starts_each_joins_as_a_w"))))
        .children((!modal.roster.is_empty()).then(|| {
            div()
                .border_t_1()
                .border_color(theme::border())
                .px_4()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .children(modal.roster.iter().map(|member| {
                    div()
                        .debug_selector({
                            let handle = member.slot.slot_handle.clone();
                            move || format!("START_MISSION_SLOT {handle}")
                        })
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(RoleAvatar::new(member.slot.slot_handle.clone(), 24.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap_2()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .font_family(theme::UI_MONOSPACE_FONT)
                                        .text_size(theme::text_ui())
                                        .text_color(
                                            runner_app::ui::avatar::hue_for_seed(
                                                &member.slot.slot_handle,
                                            )
                                            .color(),
                                        )
                                        .child(format!("@{}", member.slot.slot_handle)),
                                )
                                .children(member.slot.lead.then(lead_badge))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::muted())
                                        .child(member.role.display_name.clone()),
                                ),
                        )
                        .child(
                            slot_setup_line(&slot_setup(member))
                                .max_w(rems(250. / 16.))
                                .min_w(px(0.)),
                        )
                }))
        }))
}

fn crew_summary_message(count: usize, lead: Option<&str>) -> i18n::Text {
    if let Some(lead) = lead {
        i18n::message_count("count.slots_lead", count, &[("lead", lead.to_owned())])
    } else if count == 0 {
        i18n::message("ui.no_slots_yet")
    } else {
        i18n::message_count("count.slots", count, &[])
    }
}

fn start_mission_crew_options(crews: &[CrewListItem]) -> Vec<SelectOption> {
    let empty = SelectOption::new("", crate::i18n::message("ui.no_crew"))
        .description(crate::i18n::message(
            "ui.start_empty_add_roles_from_the_mission",
        ))
        .trigger_description(crate::i18n::message("ui.start_empty"))
        .leading(SelectLeading::new("no-crew", |size| {
            div()
                .flex_none()
                .size(rems(size / 16.))
                .rounded(rems(8. / 16.))
                .bg(theme::raised())
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path("user-plus.svg")
                        .size(rems(size * 0.5 / 16.))
                        .text_color(theme::muted()),
                )
                .into_any_element()
        }));
    std::iter::once(empty)
        .chain(crews.iter().enumerate().map(|(index, crew)| {
            let handles: Vec<SharedString> = crew
                .members
                .iter()
                .map(|member| member.slot_handle.clone().into())
                .collect();
            let key = format!("crew:{}:{handles:?}", crew.crew.id);
            SelectOption::new(crew.crew.id.clone(), crew.crew.name.clone())
                .separator_before(index == 0)
                .description(crew_summary_message(
                    crew.role_count as usize,
                    crew.members
                        .iter()
                        .find(|member| member.lead)
                        .map(|member| member.slot_handle.as_str()),
                ))
                .leading(SelectLeading::new(key, move |size| {
                    crew_picture(&handles, size)
                }))
        }))
        .collect()
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_matches_start_mission_optional_fields() {
        assert_eq!(nonempty("  "), None);
        assert_eq!(nonempty("  goal  ").as_deref(), Some("goal"));
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use crate::surfaces::start_chat::{
        tests::{modal_harness, ModalHarness},
        ChatMode,
    };
    use gpui::EntityInputHandler;

    fn mission_harness() -> ModalHarness {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.new_mission_action(&NewMission, window, cx);
        });
        harness.act(|root, _, cx| {
            let crew = root
                .core(cx)
                .crew_create(runner_core::protocol::crew::CreateCrewInput {
                    name: "First".into(),
                    ..Default::default()
                })
                .unwrap();
            let other = root
                .core(cx)
                .crew_create(runner_core::protocol::crew::CreateCrewInput {
                    name: "Second".into(),
                    ..Default::default()
                })
                .unwrap();
            let form = root.start_mission_modal.as_mut().unwrap();
            form.loading = false;
            form.crews = vec![
                CrewListItem {
                    crew: crew.clone(),
                    role_count: 1,
                    members: Vec::new(),
                },
                CrewListItem {
                    crew: other,
                    role_count: 1,
                    members: Vec::new(),
                },
            ];
            form.crew_id = crew.id.clone();
            form.crew_select.update(cx, |select, cx| {
                select.set_options(start_mission_crew_options(&form.crews), cx);
                select.set_value(crew.id, cx);
            });
            form.title
                .update(cx, |field, cx| field.set_text("Mission title", cx));
            form.goal.update(cx, |field, cx| {
                field.set_text("First line\nSecond line", cx)
            });
        });
        harness
    }

    #[test]
    fn crew_picker_and_five_read_only_roster_rows_use_the_crew_setup() {
        let mut harness = mission_harness();
        harness.act(|root, _, cx| {
            let modal = root.start_mission_modal.as_mut().unwrap();
            modal.roster = (0..5).map(|index| serde_json::from_value::<SlotWithRole>(serde_json::json!({
                "id": format!("slot-{index}"), "crew_id": modal.crew_id, "role_id": "r", "slot_handle": format!("worker-{index}"), "position": index, "lead": index == 0, "runtime_override": "codex", "model_override": "gpt-5.5", "effort_override": "high", "added_at": "2026-10-15T00:00:00Z",
                "role": {"id": "r", "handle": "role", "display_name": "Implementation", "runtime": "claude-code", "command": "claude", "args": [], "env": {}, "created_at": "2026-10-15T00:00:00Z", "updated_at": "2026-10-15T00:00:00Z"}
            })).unwrap()).collect();
            modal.crews[0].role_count = 5;
            modal.crews[0].members = modal.roster.iter().map(|member| runner_core::protocol::crew::CrewMemberPreview {
                slot_handle: member.slot.slot_handle.clone(), role_handle: member.role.handle.clone(), runtime: "codex".into(), lead: member.slot.lead,
            }).collect();
            let options = start_mission_crew_options(&modal.crews);
            assert_eq!(options[1].description.as_deref(), Some("5 slots · lead @worker-0"));
            assert!(options[1].leading.is_some());
            modal.crew_select.update(cx, |select, cx| select.set_options(options, cx));
            cx.notify();
        });
        let modal = harness.visual.debug_bounds("START_MISSION_MODAL").unwrap();
        for selector in [
            "START_MISSION_SLOT worker-0",
            "START_MISSION_SLOT worker-1",
            "START_MISSION_SLOT worker-2",
            "START_MISSION_SLOT worker-3",
            "START_MISSION_SLOT worker-4",
        ] {
            let row = harness.visual.debug_bounds(selector).unwrap();
            assert!(row.left() >= modal.left() && row.right() <= modal.right());
        }
    }

    #[test]
    fn no_crew_is_first_and_starts_an_empty_workspace_at_minimum_size() {
        let mut harness = modal_harness(640., 480., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            for crew in root.core(cx).crew_list(1, 10_000, "").unwrap().items {
                root.core(cx).crew_delete(&crew.crew.id).unwrap();
            }
            root.new_mission_action(&NewMission, window, cx);
        });
        harness.act(|root, _, cx| {
            let form = root.start_mission_modal.as_ref().unwrap();
            assert!(form.crews.is_empty());
            assert!(form.crew_id.is_empty());
            let options = start_mission_crew_options(&form.crews);
            assert_eq!(options[0].label, "No crew");
            assert_eq!(
                options[0].description.as_deref(),
                Some("start empty · add roles from the mission")
            );
            form.title
                .update(cx, |field, cx| field.set_text("Empty mission", cx));
            form.goal
                .update(cx, |field, cx| field.set_text("Shared goal", cx));
            assert!(form.can_submit(cx));
        });
        harness
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
        harness.act(|root, _, cx| {
            assert!(root.start_mission_modal.is_none());
            let AppRoute::Mission(id) = &root.route else {
                panic!("mission not opened");
            };
            assert!(root.core(cx).mission_get(id).unwrap().crew_id.is_none());
            assert!(root.core(cx).session_list(id).unwrap().is_empty());
        });
    }

    #[test]
    fn mission_confirm_dispatches_from_every_control_including_an_open_select_and_textarea() {
        for control in 0..8 {
            let mut harness = mission_harness();
            let mut original_crew = String::new();
            harness.act(|root, window, cx| {
                let form = root.start_mission_modal.as_ref().unwrap();
                original_crew = form.crew_id.clone();
                let focus = match control {
                    0 => form.crew_select.read(cx).focus_handle(),
                    1 => form.title.read(cx).focus_handle(),
                    2 => form.goal.read(cx).focus_handle(),
                    3 => form.cwd.read(cx).focus_handle(),
                    4 => form.browse_focus.clone(),
                    5 => form.cancel_focus.clone(),
                    6 => form.submit_focus.clone(),
                    _ => form.close_focus.clone(),
                };
                focus.focus(window, cx);
            });
            if control == 0 {
                harness.visual.simulate_keystrokes("enter down");
            }
            harness
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            harness.act(|root, _, cx| {
                let form = root.start_mission_modal.as_ref().unwrap();
                // The fixture crews have no real slots, so the existing submit returns a validation error without spawning agents.
                assert!(
                    form.error.is_some(),
                    "confirm did not submit at control {control}"
                );
                assert_eq!(form.crew_id, original_crew);
                assert_eq!(form.goal.read(cx).text(), "First line\nSecond line");
            });
        }
    }

    #[test]
    fn mission_confirm_is_inert_while_unavailable_starting_or_composing_and_create_keys_are_guarded(
    ) {
        for condition in 0..6 {
            let mut harness = mission_harness();
            harness.act(|root, window, cx| {
                let form = root.start_mission_modal.as_mut().unwrap();
                match condition {
                    0 => form.loading = true,
                    1 => form.submitting = true,
                    2 => form.crew_id = "missing-crew".into(),
                    3 => form.title.update(cx, |field, cx| field.set_text("", cx)),
                    4 => form.crews[0].role_count = 0,
                    _ => form.goal.update(cx, |field, cx| {
                        field.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
                    }),
                }
            });
            harness
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            for key in ["cmd-n", "cmd-t", "shift-cmd-m"] {
                harness
                    .visual
                    .simulate_keystrokes(&keymap::platform_default(key));
            }
            harness.act(|root, _, _| {
                assert!(root.start_chat_modal.is_none());
                assert!(root.tabs.tabs().is_empty());
                let form = root.start_mission_modal.as_ref().unwrap();
                assert!(form.error.is_none());
                assert_eq!(form.submitting, condition == 1);
            });
        }
    }

    #[test]
    fn mission_goal_plain_enter_inserts_newlines_and_escape_closes_the_menu_first() {
        let mut harness = mission_harness();
        harness.act(|root, window, cx| {
            root.start_mission_modal
                .as_ref()
                .unwrap()
                .goal
                .read(cx)
                .focus_handle()
                .focus(window, cx)
        });
        harness.visual.simulate_keystrokes("enter");
        harness.act(|root, window, cx| {
            let form = root.start_mission_modal.as_ref().unwrap();
            assert!(form.goal.read(cx).text().contains('\n'));
            assert!(form.error.is_none());
            form.crew_select.read(cx).focus_handle().focus(window, cx);
        });
        harness.visual.simulate_keystrokes("enter escape");
        harness.act(|root, _, _| assert!(root.start_mission_modal.is_some()));
        harness.visual.simulate_keystrokes("escape");
        harness.act(|root, _, _| assert!(root.start_mission_modal.is_none()));
    }

    #[test]
    fn mission_key_displays_the_modal_above_the_production_settings_surface() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.enter_settings_route(Some("general"), window, cx);
        });
        assert!(harness.visual.debug_bounds("START_MISSION_MODAL").is_none());
        harness
            .visual
            .simulate_keystrokes(&keymap::platform_default("shift-cmd-m"));
        assert!(harness.visual.debug_bounds("START_MISSION_MODAL").is_some());
        harness.act(|root, window, cx| {
            assert_eq!(root.route, AppRoute::Settings);
            assert!(root
                .start_mission_modal
                .as_ref()
                .unwrap()
                .crew_select
                .read(cx)
                .focus_handle()
                .is_focused(window));
        });
        harness.visual.simulate_keystrokes("escape");
        harness.act(|root, _, _| {
            assert_eq!(root.route, AppRoute::Settings);
            assert!(root.start_mission_modal.is_none());
        });
    }

    #[test]
    fn user_confirm_collision_cannot_shadow_mission_confirm_from_a_picker_or_textarea() {
        for textarea in [false, true] {
            let mut harness = mission_harness();
            let key = gpui::Keystroke::parse(&keymap::platform_default("cmd-enter")).unwrap();
            let overrides = keymap::KeymapOverrides::from([(
                "new-terminal".into(),
                keymap::combo_from_keystroke(&key),
            )]);
            harness.act(|root, window, cx| {
                keymap::install_bindings(cx, &overrides, false);
                let form = root.start_mission_modal.as_ref().unwrap();
                let focus = if textarea {
                    form.goal.read(cx).focus_handle()
                } else {
                    form.crew_select.read(cx).focus_handle()
                };
                focus.focus(window, cx);
            });
            harness
                .visual
                .simulate_keystrokes(&keymap::platform_default("cmd-enter"));
            harness.act(|root, _, _| {
                assert!(root.start_mission_modal.as_ref().unwrap().error.is_some())
            });
        }
    }

    #[test]
    fn mission_key_uses_the_active_project_from_other_routes() {
        let mut harness = mission_harness();
        let mut project_id = String::new();
        harness.act(|root, window, cx| {
            root.close_start_mission_modal(window, cx);
            let project = root
                .core(cx)
                .project_create("Project".into(), "/tmp".into())
                .unwrap();
            project_id = project.id.clone();
            let project_node = runner_daemon::repo::node::ensure_project_node(
                &root.app_store.read(cx).test_core.db.get().unwrap(),
                &project.id,
            )
            .unwrap();
            let mut layout = PaneLayout::single(None, &[]);
            layout.id = "01M3VD00000000000000000003".into();
            layout.parent_id = Some(project_node.id);
            root.core(cx)
                .node_tab_upsert(layout.upsert_input().unwrap())
                .unwrap();
            root.app_store.update(cx, |store, cx| {
                store.projects = vec![project];
                cx.notify();
            });
            root.reload_tabs(cx).unwrap();
            root.set_route(AppRoute::Crews, cx);
        });
        harness.act(|root, window, cx| root.root_focus.focus(window, cx));
        harness
            .visual
            .simulate_keystrokes(&keymap::platform_default("shift-cmd-m"));
        harness.act(|root, _, cx| {
            let form = root.start_mission_modal.as_ref().unwrap();
            assert_eq!(form.project.as_ref().unwrap().id, project_id);
            assert_eq!(form.cwd.read(cx).text(), "/tmp");
        });
    }
}

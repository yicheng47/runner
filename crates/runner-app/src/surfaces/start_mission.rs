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
use crate::surfaces::front_page::folder_picker::FolderPicker;
use crate::surfaces::front_page::{FrontPageClient, FrontPageTarget};
use crate::*;

pub(crate) struct StartMissionModalState {
    remote: Option<RemoteMissionForm>,
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

struct RemoteMissionForm {
    client: FrontPageClient,
    navigation: u64,
    form: u64,
    project: Option<String>,
    catalog: Option<runner_core::companion::Catalog>,
    folder_picker: Option<Entity<FolderPicker>>,
}

struct StartMissionSlotDisplay {
    handle: String,
    lead: bool,
    role_name: String,
    setup: Option<crate::surfaces::crews::SlotSetup>,
}

impl StartMissionModalState {
    fn can_submit(&self, cx: &App) -> bool {
        !self.submitting
            && !self.loading
            && !self.title.read(cx).text().trim().is_empty()
            && self
                .remote
                .as_ref()
                .is_none_or(|remote| remote.client.supports(&["catalog", "startMission"]))
            && (if self.remote.is_some() {
                self.host_crew().is_some_and(|crew| !crew.slots.is_empty())
            } else {
                self.crew_id.is_empty()
                    || self
                        .crews
                        .iter()
                        .any(|crew| crew.crew.id == self.crew_id && crew.role_count > 0)
            })
    }

    fn host_crew(&self) -> Option<&runner_core::companion::CatalogCrew> {
        self.remote
            .as_ref()?
            .catalog
            .as_ref()?
            .crews
            .iter()
            .find(|crew| crew.id == self.crew_id)
    }

    fn lead_handle(&self) -> Option<&str> {
        if self.remote.is_some() {
            return self
                .host_crew()?
                .slots
                .iter()
                .find(|slot| slot.lead)
                .map(|slot| slot.handle.as_str());
        }
        self.roster
            .iter()
            .find(|member| member.slot.lead)
            .map(|member| member.slot.slot_handle.as_str())
    }

    fn slot_display(&self) -> Vec<StartMissionSlotDisplay> {
        if let Some(remote) = &self.remote {
            let Some(catalog) = &remote.catalog else {
                return Vec::new();
            };
            return self
                .host_crew()
                .into_iter()
                .flat_map(|crew| &crew.slots)
                .map(|slot| StartMissionSlotDisplay {
                    handle: slot.handle.clone(),
                    lead: slot.lead,
                    role_name: catalog
                        .roles
                        .iter()
                        .find(|role| role.id == slot.role)
                        .map(|role| role.name.clone())
                        .unwrap_or_default(),
                    setup: None,
                })
                .collect();
        }
        self.roster
            .iter()
            .map(|member| StartMissionSlotDisplay {
                handle: member.slot.slot_handle.clone(),
                lead: member.slot.lead,
                role_name: member.role.display_name.clone(),
                setup: Some(slot_setup(member)),
            })
            .collect()
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
        let remote_form = if let Some(remote) = self.remote_window.as_mut() {
            if !remote.front_page.supports(&["catalog", "startMission"]) {
                return;
            }
            remote.form_generation = remote.form_generation.wrapping_add(1);
            Some(RemoteMissionForm {
                client: remote.front_page.clone(),
                navigation: remote.navigation_generation,
                form: remote.form_generation,
                project: scope.project_id().map(str::to_owned),
                catalog: None,
                folder_picker: None,
            })
        } else {
            None
        };
        let project = remote_form
            .is_none()
            .then(|| {
                scope
                    .project_id()
                    .and_then(|id| {
                        self.app_store
                            .read(cx)
                            .projects
                            .iter()
                            .find(|project| project.id == id)
                    })
                    .cloned()
            })
            .flatten();
        let cwd = if remote_form.is_some() {
            String::new()
        } else {
            project
                .as_ref()
                .map(|project| project.cwd.clone())
                .unwrap_or_else(|| self.settings(cx).default_working_dir.clone())
        };
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
            remote: remote_form,
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

        if self.remote_window.is_some() {
            self.load_remote_mission_catalog(window, cx);
            return;
        }
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
        if modal.remote.is_some() {
            if modal
                .remote
                .as_ref()
                .and_then(|remote| remote.catalog.as_ref())
                .is_some_and(|catalog| {
                    catalog
                        .crews
                        .iter()
                        .any(|crew| crew.id == crew_id && !crew.slots.is_empty())
                })
            {
                modal.crew_id = crew_id;
                modal.error = None;
                cx.notify();
            }
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
        if self.remote_window.is_some() {
            return;
        }
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
        if self.remote_window.is_some() {
            return;
        }
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
        if self.remote_window.is_some() {
            self.submit_remote_mission(window, cx);
            return;
        }
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
        let launchable = if modal.remote.is_some() {
            modal.host_crew().is_some_and(|crew| !crew.slots.is_empty())
        } else {
            selected.is_some_and(|crew| crew.role_count > 0)
        };
        let can_submit = modal.can_submit(cx);
        let lead = modal.lead_handle();
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
            .children((modal.remote.as_ref().is_some_and(|remote| remote.catalog.is_none()) && !modal.loading && modal.error.is_some()).then(|| {
                let retry = cx.entity();
                Button::new("retry-mission-catalog", crate::i18n::text("ui.retry")).on_press(move |window,cx| retry.update(cx,|this,cx| this.load_remote_mission_catalog(window,cx)))
            }))
            .child(
                Field::new(
                    "start-mission-crew-field",
                    crate::i18n::text("ui.crew"),
                    start_mission_crew_card(modal),
                )
                .emphasized(true),
            )
            .children((modal.remote.is_some() && !modal.loading && !launchable).then(|| div().text_size(theme::text_meta()).text_color(theme::warning()).child(crate::i18n::text("client.no_host_crews"))))
            .children((modal.remote.is_none() && selected.is_some() && !launchable).then(|| {
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
                            crate::i18n::tf("format.delivered_to_lead_on_mission_start", &[("handle", lead.to_owned())])
                        })
                        .unwrap_or_else(|| if modal.remote.is_none() && modal.crew_id.is_empty() { crate::i18n::text("ui.shared_with_every_role_that_joins_give_each_one_its_own_task_when_you_add_it").into() } else { crate::i18n::text("ui.delivered_to_the_crew_lead_on_mission_start").into() }),
                    ),
            )
            .child(
                Field::new(
                    "start-mission-cwd-field",
                    crate::i18n::text("ui.working_directory"),
                    div().debug_selector(|| "START_MISSION_WORKING_DIR_FIELD".into()).w_full().relative().child(WorkingDirField::new(
                        modal.cwd.clone(),
                        modal.submitting,
                        Rc::new(move |window, cx| {
                            browse_root.update(cx, |this, cx| {
                                if let Some(picker) = this.start_mission_modal.as_ref().and_then(|modal| modal.remote.as_ref()).and_then(|remote| remote.folder_picker.clone()) { picker.update(cx, |picker,cx| picker.toggle(window,cx)); }
                                else { this.browse_start_mission_cwd(cx); }
                            });
                        }),
                    )
                    .browse_id("browse-start-mission")
                    .browse_focus(modal.browse_focus.clone())
                    .show_browse(modal.remote.as_ref().is_none_or(|remote| remote.client.supports(&["folders"]))))
                    .children(modal.remote.as_ref().and_then(|remote| remote.folder_picker.clone()).map(FolderPicker::anchor)),
                )
                .emphasized(true)
                .subtitle(if modal.crew_id.is_empty() { crate::i18n::text("ui.every_role_you_add_starts_here_leave_blank_to_use_its_default_directory_or_your_home_direc") } else { crate::i18n::text("ui.each_role_starts_here_leave_blank_to_use_its_default_directory_or_your_home_directory") }).map(|field| if let Some(error) = modal.remote.as_ref().and_then(|remote| remote.folder_picker.as_ref()).and_then(|picker| picker.read(cx).path_error.clone()) { field.error(error) } else { field }),
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
            let mut order = vec![
                modal.close_focus.clone(),
                modal.crew_select.read(cx).focus_handle(),
                modal.title.read(cx).focus_handle(),
                modal.goal.read(cx).focus_handle(),
                modal.cwd.read(cx).focus_handle(),
            ];
            if modal
                .remote
                .as_ref()
                .is_none_or(|remote| remote.client.supports(&["folders"]))
            {
                order.push(modal.browse_focus.clone());
            }
            order.extend([modal.cancel_focus.clone(), modal.submit_focus.clone()]);
            order
        })
        .footer(footer);
        div()
            .debug_selector(|| "START_MISSION_MODAL".into())
            .absolute()
            .inset_0()
            .on_action(cx.listener(Self::confirm_start_mission))
            .child(modal_element)
            .children(
                modal
                    .remote
                    .as_ref()
                    .and_then(|remote| remote.folder_picker.clone()),
            )
            .into_any_element()
    }
}

impl NativeRoot {
    fn remote_mission_form_current(
        &self,
        target: &FrontPageTarget,
        navigation: u64,
        form: u64,
    ) -> bool {
        self.remote_window.as_ref().is_some_and(|remote| {
            remote.front_page.target == *target
                && remote.front_page.valid()
                && remote.navigation_generation == navigation
                && remote.form_generation == form
        }) && self
            .start_mission_modal
            .as_ref()
            .and_then(|modal| modal.remote.as_ref())
            .is_some_and(|remote| {
                remote.client.target == *target
                    && remote.navigation == navigation
                    && remote.form == form
            })
    }

    fn load_remote_mission_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(remote) = self
            .start_mission_modal
            .as_ref()
            .and_then(|modal| modal.remote.as_ref())
        else {
            return;
        };
        if !remote.client.supports(&["catalog", "startMission"]) {
            return;
        }
        let client = remote.client.clone();
        let target = client.target.clone();
        let navigation = remote.navigation;
        let form = remote.form;
        let modal = self.start_mission_modal.as_mut().unwrap();
        modal.loading = true;
        modal.error = None;
        let task =
            cx.background_spawn(async move { client.catalog().map_err(|error| error.to_string()) });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if !this.remote_mission_form_current(&target, navigation, form) {
                    return;
                }
                let host = this.remote_window.as_ref().unwrap();
                let computer = host.computer.name.clone();
                let platform = host.computer.info.platform.clone();
                let modal = this.start_mission_modal.as_mut().unwrap();
                modal.loading = false;
                match result {
                    Ok(catalog) => {
                        if modal.initial_crew_id.as_ref().is_some_and(|id| {
                            !catalog
                                .crews
                                .iter()
                                .any(|crew| &crew.id == id && !crew.slots.is_empty())
                        }) {
                            modal.error =
                                Some(crate::i18n::text("client.crew_unavailable").to_owned());
                            cx.notify();
                            return;
                        }
                        let preferred = modal
                            .initial_crew_id
                            .as_ref()
                            .and_then(|id| {
                                catalog
                                    .crews
                                    .iter()
                                    .find(|crew| &crew.id == id && !crew.slots.is_empty())
                            })
                            .or_else(|| catalog.crews.iter().find(|crew| !crew.slots.is_empty()))
                            .map(|crew| crew.id.clone())
                            .unwrap_or_default();
                        modal.crew_id = preferred.clone();
                        modal.crew_select.update(cx, |select, cx| {
                            select.set_options(remote_mission_crew_options(&catalog), cx);
                            select.set_value(preferred, cx);
                        });
                        let input = modal.cwd.clone();
                        let client = modal.remote.as_ref().unwrap().client.clone();
                        let choices = crate::surfaces::front_page::folder_picker::choices(&catalog);
                        let picker = cx.new(|cx| {
                            FolderPicker::new(
                                client,
                                computer,
                                platform,
                                input.clone(),
                                choices,
                                cx,
                            )
                        });
                        let subscription = cx.observe(&picker, |_, _, cx| cx.notify());
                        let blur_picker = picker.clone();
                        let blur = cx.on_focus_out(
                            &input.read(cx).focus_handle(),
                            window,
                            move |_, _, _, cx| {
                                blur_picker.update(cx, |picker, cx| picker.validate(cx))
                            },
                        );
                        let remote = modal.remote.as_mut().unwrap();
                        remote.catalog = Some(catalog);
                        remote.folder_picker = Some(picker);
                        modal._subscriptions.extend([subscription, blur]);
                    }
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn submit_remote_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.start_mission_modal.as_ref() else {
            return;
        };
        if !modal.can_submit(cx) || modal.is_composing(cx) {
            return;
        }
        let Some(remote) = &modal.remote else {
            return;
        };
        let client = remote.client.clone();
        let target = client.target.clone();
        let navigation = remote.navigation;
        let form = remote.form;
        if !self.remote_mission_form_current(&target, navigation, form) {
            return;
        }
        let crew = modal.crew_id.clone();
        let title = modal.title.read(cx).text().trim().to_owned();
        let goal = nonempty(modal.goal.read(cx).text());
        let cwd = nonempty(modal.cwd.read(cx).text());
        let project = remote.project.clone();
        let size = self
            .remote_window
            .as_ref()
            .unwrap()
            .mission_workspace
            .read(cx)
            .estimated_mission_terminal_size(window, cx);
        let size = (size.0.max(20), size.1.max(8));
        let modal = self.start_mission_modal.as_mut().unwrap();
        modal.submitting = true;
        modal.error = None;
        set_start_mission_fields_disabled(modal, true, cx);
        let task = cx.background_spawn(async move {
            client
                .start_mission(crew, title, goal, project, cwd, size)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if !this.remote_mission_form_current(&target, navigation, form) {
                    return;
                }
                match result {
                    Ok(mission) => {
                        this.start_mission_modal = None;
                        this.remote_window.as_mut().unwrap().activate_mission = Some(mission);
                        this.sync_remote_state(cx);
                    }
                    Err(error) => {
                        let computer = this.remote_window.as_ref().unwrap().computer.name.clone();
                        let modal = this.start_mission_modal.as_mut().unwrap();
                        modal.submitting = false;
                        if error == "Choose an accessible folder on the host." {
                            if let Some(picker) = modal
                                .remote
                                .as_ref()
                                .and_then(|remote| remote.folder_picker.clone())
                            {
                                picker.update(cx, |picker, cx| {
                                    picker.path_error =
                                        Some(crate::surfaces::remote_client::label(
                                            "client.folder_invalid",
                                            &computer,
                                        ));
                                    cx.notify();
                                });
                            }
                        }
                        modal.error = Some(error);
                        set_start_mission_fields_disabled(modal, false, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn remote_mission_crew_options(catalog: &runner_core::companion::Catalog) -> Vec<SelectOption> {
    catalog
        .crews
        .iter()
        .map(|crew| {
            let handles = crew
                .slots
                .iter()
                .map(|slot| SharedString::from(slot.handle.clone()))
                .collect::<Vec<_>>();
            SelectOption::new(crew.id.clone(), crew.name.clone())
                .disabled(crew.slots.is_empty())
                .description(crew_summary_message(
                    crew.slots.len(),
                    crew.slots
                        .iter()
                        .find(|slot| slot.lead)
                        .map(|slot| slot.handle.as_str()),
                ))
                .leading(SelectLeading::new(
                    format!("host-crew:{}:{handles:?}", crew.id),
                    move |size| crew_picture(&handles, size),
                ))
        })
        .collect()
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
    let roster = modal.slot_display();
    div()
        .w_full()
        .min_w(px(0.))
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .child(modal.crew_select.clone())
        .children((modal.remote.is_none() && modal.crew_id.is_empty()).then(|| div().border_t_1().border_color(theme::border()).px_4().py_3()
            .text_size(theme::text_ui()).text_color(theme::muted())
            .child(crate::i18n::text("ui.no_roles_at_start_add_them_one_at_a_time_from_the_mission_once_it_starts_each_joins_as_a_w"))))
        .children((!roster.is_empty()).then(|| {
            div()
                .border_t_1()
                .border_color(theme::border())
                .px_4()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .children(roster.iter().map(|member| {
                    div()
                        .debug_selector({
                            let handle = member.handle.clone();
                            move || format!("START_MISSION_SLOT {handle}")
                        })
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(RoleAvatar::new(member.handle.clone(), 24.))
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
                                                &member.handle,
                                            )
                                            .color(),
                                        )
                                        .child(format!("@{}", member.handle)),
                                )
                                .children(member.lead.then(lead_badge))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::muted())
                                        .child(member.role_name.clone()),
                                ),
                        )
                        .children(member.setup.as_ref().map(|setup|
                            slot_setup_line(setup)
                                .max_w(rems(250. / 16.))
                                .min_w(px(0.))),
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

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::surfaces::remote_client::{
        tests::{self as remote_test, Fake},
        RemoteItem,
    };
    use gpui::{TestAppContext, VisualTestContext};
    use runner_app::i18n::{testing::with_locale, Locale};
    use runner_core::companion::Overview;
    use std::sync::Arc;

    fn catalog() -> serde_json::Value {
        serde_json::json!({"roles":[{"id":"host-role","name":"Host coder","handle":"host-coder","runtime":"codex","model":"host-role-model","effort":"high","speed":null}],"runtimes":[],"accounts":[],"activeAccount":"","crews":[{"id":"empty-host-crew","name":"Empty host crew","slots":[]},{"id":"host-crew","name":"Host duo","slots":[{"handle":"lead","role":"host-role","lead":true},{"handle":"reviewer","role":"host-role","lead":false}]}],"projects":[{"id":"host-project","name":"Host project","path":"\\\\nas\\share"}],"recentFolders":["C:\\host\\recent"]})
    }
    fn overview(created: bool) -> Overview {
        serde_json::from_value(serde_json::json!({"projects":[{"id":"host-project","name":"Host project"}],"tabs":[{"id":"first","project":null,"name":"First","pinned":false,"panes":{"session":null},"shells":[]},{"id":"other","project":null,"name":"Other","pinned":false,"panes":{"session":null},"shells":[]}],"missions":if created {serde_json::json!([{"id":"created-mission","title":"Host mission","project":"host-project","crew":"Host duo","state":"running","pinned":false,"members":[],"asks":[],"shells":[]}])} else {serde_json::json!([])},"sessions":{}})).unwrap()
    }
    fn setup(
        features: Option<&[&str]>,
    ) -> (
        tempfile::TempDir,
        TestAppContext,
        gpui::WindowHandle<NativeRoot>,
        Entity<AppStore>,
        Arc<Fake>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        if let Some(features) = features {
            fake.set_features(features);
        }
        fake.set_overview(overview(false));
        store.update(&mut cx, |store, _| {
            store.settings.default_working_dir = "/local/unchanged".into()
        });
        let window = remote_test::add_window(&mut cx, &store, temp.path(), "start-host-mission");
        window
            .update(&mut cx, |root, window, cx| {
                remote_test::choose_host(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        (temp, cx, window, store, fake)
    }
    fn open(
        cx: &mut TestAppContext,
        window: gpui::WindowHandle<NativeRoot>,
        fake: &Fake,
        scope: ProjectScope,
    ) {
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(cx, |root, window, cx| {
                root.open_start_mission_modal(None, scope, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |root, _, cx| {
                let form = root.start_mission_modal.as_ref().unwrap();
                form.title
                    .update(cx, |field, cx| field.reset("Host mission", cx));
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[test]
    fn host_catalog_existing_crew_defaults_scope_size_and_activation_come_from_the_target() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for push_before_result in [true, false] {
            let (_temp, mut cx, window, store, fake) = setup(None);
            let catalogs = fake.request_count("catalog");
            let local = fake.local_call_count();
            open(
                &mut cx,
                window,
                &fake,
                ProjectScope::Project("host-project".into()),
            );
            assert_eq!(fake.request_count("catalog"), catalogs + 1);
            let mut size = (0, 0);
            window
                .update(&mut cx, |root, window, cx| {
                    size = root
                        .remote_window
                        .as_ref()
                        .unwrap()
                        .mission_workspace
                        .read(cx)
                        .estimated_mission_terminal_size(window, cx);
                    let modal = root.start_mission_modal.as_ref().unwrap();
                    assert!(modal.crews.is_empty());
                    assert!(modal.roster.is_empty());
                    assert_eq!(modal.crew_id, "host-crew");
                    assert_eq!(modal.lead_handle(), Some("lead"));
                    assert!(modal.cwd.read(cx).text().is_empty());
                    assert!(modal.slot_display().iter().all(|slot| slot.setup.is_none()));
                    assert!(modal.can_submit(cx));
                    modal
                        .goal
                        .update(cx, |field, cx| field.reset("Host goal", cx));
                    root.confirm_start_mission(&ConfirmStartMission, window, cx);
                    assert!(root.start_mission_modal.as_ref().unwrap().submitting);
                    assert!(root
                        .remote_window
                        .as_ref()
                        .unwrap()
                        .activate_mission
                        .is_none());
                    if push_before_result {
                        let clients = root.app_store.read(cx).remote_clients.clone();
                        clients.update(cx, |clients, cx| {
                            clients.overviews.insert("host".into(), overview(true));
                            cx.notify();
                        });
                        root.sync_remote_state(cx);
                        assert!(matches!(
                            root.remote_window.as_ref().unwrap().active,
                            Some(RemoteItem::Tab(_))
                        ));
                    }
                })
                .unwrap();
            cx.run_until_parked();
            let requests = remote_test::action_trace(&fake, "startMission");
            let request = &requests[0];
            assert_eq!(request["crew"], "host-crew");
            assert_eq!(request["title"], "Host mission");
            assert_eq!(request["goal"], "Host goal");
            assert_eq!(request["project"], "host-project");
            assert!(request["cwd"].is_null());
            assert_eq!(request["cols"], size.0.max(20));
            assert_eq!(request["rows"], size.1.max(8));
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(root.start_mission_modal.is_none());
                    if push_before_result {
                        assert!(
                            remote.active == Some(RemoteItem::Mission("created-mission".into()))
                        );
                    } else {
                        assert_eq!(remote.activate_mission.as_deref(), Some("created-mission"));
                        assert!(matches!(remote.active, Some(RemoteItem::Tab(_))));
                    }
                })
                .unwrap();
            assert_eq!(fake.local_call_count(), local);
            fake.set_overview(overview(true));
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(remote.active == Some(RemoteItem::Mission("created-mission".into())));
                    assert!(remote.activate_mission.is_none());
                })
                .unwrap();
            store.read_with(&cx, |store, _| {
                assert_eq!(store.settings.default_working_dir, "/local/unchanged")
            });
        }
    }

    #[test]
    fn host_folder_menu_uses_the_existing_field_and_typed_windows_paths_survive_failure() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                let (_temp, mut cx, window, _, fake) = setup(None);
                open(&mut cx, window, &fake, ProjectScope::Root);
                cx.simulate_window_resize(window.into(), size(px(760.), px(900.)));
                cx.run_until_parked();
                window
                    .update(&mut cx, |root, window, cx| {
                        let modal = root.start_mission_modal.as_ref().unwrap();
                        modal.browse_focus.focus(window, cx);
                    })
                    .unwrap();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                visual.simulate_keystrokes("space");
                visual.run_until_parked();
                let field = visual
                    .debug_bounds("START_MISSION_WORKING_DIR_FIELD")
                    .unwrap();
                let menu = visual.debug_bounds("HOST_FOLDER_MENU").unwrap();
                assert_eq!(field.size.width, menu.size.width);
                assert!(menu.bottom() <= field.top());
                let choice = visual.debug_bounds("HOST_FOLDER_ROW:choice-0").unwrap();
                visual.simulate_click(choice.center(), Default::default());
                visual.run_until_parked();
                window
                    .read_with(&visual, |root, cx| {
                        assert_eq!(
                            root.start_mission_modal
                                .as_ref()
                                .unwrap()
                                .cwd
                                .read(cx)
                                .text(),
                            "\\\\nas\\share"
                        )
                    })
                    .unwrap();
                fake.queue_result(
                    "startMission",
                    Err(runner_core::protocol::ClientError::msg(
                        "Choose an accessible folder on the host.",
                    )),
                );
                window
                    .update(&mut visual, |root, window, cx| {
                        root.confirm_start_mission(&ConfirmStartMission, window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                window
                    .read_with(&visual, |root, cx| {
                        let modal = root.start_mission_modal.as_ref().unwrap();
                        assert_eq!(modal.cwd.read(cx).text(), "\\\\nas\\share");
                        assert_eq!(modal.title.read(cx).text(), "Host mission");
                        assert!(!modal.submitting);
                        assert!(modal
                            .remote
                            .as_ref()
                            .unwrap()
                            .folder_picker
                            .as_ref()
                            .unwrap()
                            .read(cx)
                            .path_error
                            .is_some());
                    })
                    .unwrap();
                assert_eq!(
                    remote_test::action_trace(&fake, "startMission")[0]["cwd"],
                    "\\\\nas\\share"
                );
                assert!(remote_test::action_trace(&fake, "startMission")[0]["project"].is_null());
            });
        }
    }

    #[test]
    fn missing_catalog_or_start_capability_and_empty_host_crews_never_fall_back_locally() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for features in [vec!["catalog"], vec!["startMission"], vec![]] {
            let (_temp, mut cx, window, _, fake) = setup(Some(&features));
            let local = fake.local_call_count();
            let catalogs = fake.request_count("catalog");
            window
                .update(&mut cx, |root, window, cx| {
                    root.new_mission_action(&NewMission, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| assert!(root.start_mission_modal.is_none()))
                .unwrap();
            assert_eq!(fake.request_count("catalog"), catalogs);
            assert_eq!(fake.request_count("startMission"), 0);
            assert_eq!(fake.local_call_count(), local);
        }
        let (_temp, mut cx, window, _, fake) = setup(None);
        let mut empty = catalog();
        empty["crews"] = serde_json::json!([]);
        fake.queue_result("catalog", Ok(empty));
        window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let modal = root.start_mission_modal.as_ref().unwrap();
                modal
                    .title
                    .update(cx, |field, cx| field.reset("No host crew", cx));
                assert!(!modal.can_submit(cx));
                root.confirm_start_mission(&ConfirmStartMission, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("startMission"), 0);
    }

    #[test]
    fn navigating_during_catalog_or_start_cancels_old_forms_and_activation_intents() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for before_result in [true, false] {
            let (_temp, mut cx, window, _, fake) = setup(None);
            fake.queue_result("catalog", Ok(catalog()));
            window
                .update(&mut cx, |root, window, cx| {
                    root.new_mission_action(&NewMission, window, cx);
                    root.select_remote_item(RemoteItem::Tab("other".into()), cx);
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| assert!(root.start_mission_modal.is_none()))
                .unwrap();
            open(&mut cx, window, &fake, ProjectScope::Root);
            window
                .update(&mut cx, |root, window, cx| {
                    root.confirm_start_mission(&ConfirmStartMission, window, cx);
                    if before_result {
                        root.select_remote_item(RemoteItem::Tab("first".into()), cx);
                    }
                })
                .unwrap();
            cx.run_until_parked();
            if !before_result {
                window
                    .update(&mut cx, |root, _, cx| {
                        root.select_remote_item(RemoteItem::Tab("first".into()), cx)
                    })
                    .unwrap();
                cx.run_until_parked();
            }
            fake.set_overview(overview(true));
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    let remote = root.remote_window.as_ref().unwrap();
                    assert!(remote.active == Some(RemoteItem::Tab("first".into())));
                    assert!(remote.activate_mission.is_none());
                    assert!(root.start_mission_modal.is_none());
                })
                .unwrap();
            assert_eq!(fake.request_count("startMission"), 1);
        }
    }
    #[test]
    fn missing_folders_skips_browse_in_the_tab_order_and_preserves_typed_host_defaults() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, _, fake) = setup(Some(&["catalog", "startMission"]));
        open(&mut cx, window, &fake, ProjectScope::Root);
        window
            .update(&mut cx, |root, window, cx| {
                let modal = root.start_mission_modal.as_ref().unwrap();
                modal
                    .cwd
                    .update(cx, |field, cx| field.reset("C:\\host\\typed", cx));
                modal.cwd.read(cx).focus_handle().focus(window, cx);
                window.activate_window();
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                assert!(root
                    .start_mission_modal
                    .as_ref()
                    .unwrap()
                    .cancel_focus
                    .is_focused(window));
                root.confirm_start_mission(&ConfirmStartMission, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(fake.request_count("folders"), 0);
        let request = &remote_test::action_trace(&fake, "startMission")[0];
        assert_eq!(request["cwd"], "C:\\host\\typed");
        assert!(request["project"].is_null());
    }

    #[test]
    fn reopening_refreshes_host_crews_and_catalog_failure_retries_in_the_same_form() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, _, fake) = setup(None);
        let before = fake.request_count("catalog");
        open(&mut cx, window, &fake, ProjectScope::Root);
        window
            .update(&mut cx, |root, window, cx| {
                root.close_start_mission_modal(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        fake.queue_result(
            "catalog",
            Err(runner_core::protocol::ClientError::msg("Catalog refused")),
        );
        window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .read_with(&cx, |root, cx| {
                let modal = root.start_mission_modal.as_ref().unwrap();
                assert!(modal.error.is_some());
                assert!(modal.remote.as_ref().unwrap().catalog.is_none());
                assert!(!modal.can_submit(cx));
            })
            .unwrap();
        let mut changed = catalog();
        changed["crews"][1]["id"] = serde_json::json!("refreshed-crew");
        fake.queue_result("catalog", Ok(changed));
        window
            .update(&mut cx, |root, window, cx| {
                root.load_remote_mission_catalog(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                assert_eq!(
                    root.start_mission_modal.as_ref().unwrap().crew_id,
                    "refreshed-crew"
                )
            })
            .unwrap();
        assert_eq!(fake.request_count("catalog"), before + 3);
        assert_eq!(fake.request_count("startMission"), 0);
    }
    #[test]
    fn remote_crew_library_launch_refreshes_and_preselects_the_clicked_crew() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, _store, fake) = setup(None);
        let mut listing = catalog();
        listing["crews"].as_array_mut().unwrap().push(serde_json::json!({"id":"chosen-host-crew","name":"Chosen crew","slots":[{"handle":"captain","role":"host-role","lead":true}]}));
        fake.queue_result("catalog", Ok(listing.clone()));
        window
            .update(&mut cx, |root, window, cx| root.open_crews(window, cx))
            .unwrap();
        cx.run_until_parked();
        let reads = fake.request_count("catalog");
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        fake.queue_result("catalog", Ok(listing));
        let start = visual
            .debug_bounds("CREW_ROW_START:chosen-host-crew")
            .unwrap();
        visual.simulate_click(start.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("START_MISSION_SLOT captain").is_some());
        window
            .update(&mut visual, |root, window, cx| {
                let form = root.start_mission_modal.as_ref().unwrap();
                assert_eq!(form.crew_id, "chosen-host-crew");
                form.title
                    .update(cx, |title, cx| title.reset("From the host library", cx));
                root.confirm_start_mission(&ConfirmStartMission, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        let starts = remote_test::action_trace(&fake, "startMission");
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["crew"], "chosen-host-crew");
        assert!(starts[0]["cwd"].is_null());
        assert!(starts[0]["project"].is_null());
        assert_eq!(fake.request_count("catalog"), reads + 1);
    }

    #[test]
    fn remote_crew_library_does_not_replace_a_removed_crew_with_another() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, _store, fake) = setup(None);
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| root.open_crews(window, cx))
            .unwrap();
        cx.run_until_parked();
        let mut fresh = catalog();
        fresh["crews"][1]["id"] = "another-crew".into();
        fake.queue_result("catalog", Ok(fresh));
        window
            .update(&mut cx, |root, window, cx| {
                root.open_start_mission_modal(
                    Some("host-crew".into()),
                    ProjectScope::Root,
                    window,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let form = root.start_mission_modal.as_ref().unwrap();
                assert_eq!(
                    form.error.as_deref(),
                    Some(crate::i18n::text("client.crew_unavailable"))
                );
                assert!(form.crew_id.is_empty());
                root.confirm_start_mission(&ConfirmStartMission, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("startMission"), 0);
    }
}

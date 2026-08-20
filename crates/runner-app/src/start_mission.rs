use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, rems, svg, AnyElement, Entity, FontWeight, PathPromptOptions, Window};
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, OverlayWidth, SelectOption, StyledSelect,
    TextField,
};
use runner_backend::model::SlotWithRunner;
use runner_backend::ops::crew::CrewListItem;
use runner_backend::repo::project::ProjectRow;

use super::*;

pub(crate) struct StartMissionModalState {
    initial_crew_id: Option<String>,
    project: Option<ProjectRow>,
    crews: Vec<CrewListItem>,
    crew_id: String,
    roster: Vec<SlotWithRunner>,
    crew_select: Entity<StyledSelect>,
    title: Entity<TextField>,
    goal: Entity<TextField>,
    cwd: Entity<TextField>,
    advanced_open: bool,
    loading: bool,
    submitting: bool,
    error: Option<String>,
    close_focus: FocusHandle,
    browse_focus: FocusHandle,
    advanced_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl NativeRoot {
    pub(crate) fn open_start_mission_modal(
        &mut self,
        initial_crew_id: Option<String>,
        project_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = project_id
            .as_deref()
            .and_then(|id| self.projects.iter().find(|project| project.id == id))
            .cloned();
        let cwd = project
            .as_ref()
            .map(|project| project.cwd.clone())
            .unwrap_or_else(|| self.settings.default_working_dir.clone());
        let title = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                "",
                "e.g. Wire up event bus watcher",
                false,
            )
            .text_size(13.)
        });
        let goal = cx.new(|input_cx| {
            TextField::textarea(input_cx.focus_handle(), "", "Describe what to do…", 5, true)
                .text_size(13.)
        });
        let cwd_input = cx.new(|input_cx| {
            TextField::new(
                input_cx.focus_handle(),
                cwd,
                "/Users/you/projects/foo (optional)",
                true,
            )
            .text_size(12.)
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
            .width(px(632.))
            .min_menu_width(px(0.))
            .detailed(true)
            .placeholder("No crews yet")
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
            advanced_open: false,
            loading: true,
            submitting: false,
            error: None,
            close_focus: cx.focus_handle(),
            browse_focus: cx.focus_handle(),
            advanced_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        });
        crew_focus.focus(window);
        cx.notify();

        let core = self.core.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::crew::crew_list(&core, 1, 10_000, "")
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
                            .or_else(|| crews.iter().find(|crew| crew.runner_count > 0))
                            .or_else(|| crews.first())
                            .map(|crew| crew.crew.id.clone())
                            .unwrap_or_default();
                        modal.crews = crews;
                        modal.crew_id = preferred.clone();
                        modal.crew_select.update(cx, |select, select_cx| {
                            select.set_options(
                                start_mission_crew_options(&modal.crews, "", &[]),
                                select_cx,
                            );
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
        modal.loading = true;
        let placeholder = modal
            .crews
            .iter()
            .find(|crew| crew.crew.id == crew_id)
            .and_then(|crew| crew.crew.goal.clone())
            .unwrap_or_else(|| "Describe what to do…".into());
        modal.goal.update(cx, |goal, goal_cx| {
            goal.set_placeholder(placeholder, goal_cx)
        });
        self.load_start_mission_roster(crew_id, cx);
        cx.notify();
    }

    fn load_start_mission_roster(&mut self, crew_id: String, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let load_id = crew_id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::slot::slot_list(&core, &load_id).map_err(|error| error.to_string())
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
                        let options =
                            start_mission_crew_options(&modal.crews, &modal.crew_id, &modal.roster);
                        modal.crew_select.update(cx, |select, select_cx| {
                            select.set_options(options, select_cx)
                        });
                        let placeholder = modal
                            .crews
                            .iter()
                            .find(|crew| crew.crew.id == crew_id)
                            .and_then(|crew| crew.crew.goal.clone())
                            .unwrap_or_else(|| "Describe what to do…".into());
                        modal.goal.update(cx, |goal, goal_cx| {
                            goal.set_placeholder(placeholder, goal_cx)
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
        window.focus(&self.root_focus);
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
            prompt: Some("Pick a working directory".into()),
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

    fn submit_start_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.start_mission_modal.as_mut() else {
            return;
        };
        if modal.submitting
            || modal.loading
            || modal.crew_id.is_empty()
            || modal.title.read(cx).text().trim().is_empty()
        {
            return;
        }
        let launchable = modal
            .crews
            .iter()
            .find(|crew| crew.crew.id == modal.crew_id)
            .is_some_and(|crew| crew.runner_count > 0);
        if !launchable {
            return;
        }
        let input = runner_backend::ops::mission::StartMissionInput {
            crew_id: modal.crew_id.clone(),
            project_id: modal.project.as_ref().map(|project| project.id.clone()),
            title: modal.title.read(cx).text().trim().to_owned(),
            goal_override: nonempty(modal.goal.read(cx).text()),
            cwd: nonempty(modal.cwd.read(cx).text()),
        };
        modal.submitting = true;
        modal.error = None;
        set_start_mission_fields_disabled(modal, true, cx);
        let size = self.estimated_mission_terminal_size(window);
        let core = self.core.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::mission::mission_start_impl_with_size(&core, input, Some(size))
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(output) => {
                        this.start_mission_modal = None;
                        this.refresh_sidebar(SidebarRefreshKind::All, cx);
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
        let launchable = selected.is_some_and(|crew| crew.runner_count > 0);
        let runner_count = selected.map_or(0, |crew| crew.runner_count);
        let title_empty = modal.title.read(cx).text().trim().is_empty();
        let can_submit = !modal.submitting
            && !modal.loading
            && !modal.crew_id.is_empty()
            && !title_empty
            && launchable;
        let lead = modal.roster.iter().find(|member| member.slot.lead);
        let root = cx.entity();
        let close_root = root.clone();
        let browse_root = root.clone();
        let advanced_root = root.clone();
        let advanced_key_root = root.clone();
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
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Start mission"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Spawns a session per slot and opens the mission workspace."),
                    ),
            )
            .child(
                IconButton::new("close-start-mission", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close start mission")
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
            .children(modal.error.clone().map(error_banner))
            .child(
                Field::new(
                    "start-mission-crew-field",
                    "Crew",
                    modal.crew_select.clone(),
                )
                .emphasized(true),
            )
            .children((selected.is_some() && !launchable).then(|| {
                div()
                    .mt(rems(-14. / 16.))
                    .text_size(rems(11. / 16.))
                    .text_color(theme::warning())
                    .child("This crew has no runners. Add at least one before starting a mission.")
            }))
            .child(
                Field::new(
                    "start-mission-title-field",
                    "Mission title",
                    modal.title.clone(),
                )
                .emphasized(true)
                .subtitle("Short label shown in the missions list and event log."),
            )
            .child(
                Field::new("start-mission-goal-field", "Goal", modal.goal.clone())
                    .emphasized(true)
                    .subtitle(
                        lead.map(|lead| {
                            format!(
                                "Delivered to @{} (lead) on mission start.",
                                lead.slot.slot_handle
                            )
                        })
                        .unwrap_or_else(|| "Delivered to the crew lead on mission start.".into()),
                    ),
            )
            .child(
                Field::new(
                    "start-mission-cwd-field",
                    "Working directory",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().min_w(px(0.)).flex_1().child(modal.cwd.clone()))
                        .child(
                            Button::new("browse-start-mission", "Browse…")
                                .focus_handle(modal.browse_focus.clone())
                                .disabled(modal.submitting)
                                .on_press(move |_, cx| {
                                    browse_root
                                        .update(cx, |this, cx| this.browse_start_mission_cwd(cx));
                                }),
                        ),
                )
                .emphasized(true)
                .subtitle("Each runner's PTY starts in this directory. Exposed as $MISSION_CWD."),
            )
            .child(
                div()
                    .id("start-mission-advanced")
                    .rounded_md()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .px_3()
                    .py_3()
                    .child(
                        div()
                            .id("start-mission-advanced-toggle")
                            .track_focus(&modal.advanced_focus)
                            .tab_index(0)
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .on_click(move |_, _, cx| {
                                advanced_root.update(cx, |this, cx| {
                                    if let Some(modal) = this.start_mission_modal.as_mut() {
                                        modal.advanced_open = !modal.advanced_open;
                                        cx.notify();
                                    }
                                });
                            })
                            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    advanced_key_root.update(cx, |this, cx| {
                                        if let Some(modal) = this.start_mission_modal.as_mut() {
                                            modal.advanced_open = !modal.advanced_open;
                                            cx.notify();
                                        }
                                    });
                                }
                            })
                            .child(
                                svg()
                                    .path(if modal.advanced_open {
                                        "chevron-down.svg"
                                    } else {
                                        "chevron-right.svg"
                                    })
                                    .size(rems(14. / 16.))
                                    .text_color(theme::muted()),
                            )
                            .child(div().flex_1().child("Advanced"))
                            .child(
                                div()
                                    .text_size(rems(11. / 16.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(theme::faint())
                                    .child("env overrides · per-runner args · attach files"),
                            ),
                    )
                    .children(modal.advanced_open.then(|| {
                        div()
                            .mt_3()
                            .rounded_sm()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .px_3()
                            .py_2()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .child("Reserved for v0.x — custom env, dry-run mode. Inert in v0 MVP.")
                    })),
            );
        let footer = div()
            .w_full()
            .flex()
            .items_center()
            .child(
                div()
                    .mr_auto()
                    .text_size(rems(11. / 16.))
                    .text_color(theme::faint())
                    .child(if selected.is_some() {
                        format!(
                            "{runner_count} session{} will spawn",
                            if runner_count == 1 { "" } else { "s" }
                        )
                    } else {
                        String::new()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("cancel-start-mission", "Cancel")
                            .focus_handle(modal.cancel_focus.clone())
                            .disabled(modal.submitting)
                            .on_press(move |window, cx| {
                                cancel_root.update(cx, |this, cx| {
                                    this.close_start_mission_modal(window, cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(
                            "submit-start-mission",
                            if modal.submitting {
                                "Starting…"
                            } else {
                                "Start mission"
                            },
                        )
                        .focus_handle(modal.submit_focus.clone())
                        .variant(ButtonVariant::Primary)
                        .disabled(!can_submit)
                        .on_press(move |window, cx| {
                            submit_root
                                .update(cx, |this, cx| this.submit_start_mission(window, cx));
                        }),
                    ),
            );
        let close_modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                close_modal_root.update(cx, |this, cx| this.close_start_mission_modal(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(680.))
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
                modal.advanced_focus.clone(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
            ]
        })
        .footer(footer)
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

fn start_mission_crew_options(
    crews: &[CrewListItem],
    selected_id: &str,
    roster: &[SlotWithRunner],
) -> Vec<SelectOption> {
    crews
        .iter()
        .map(|crew| {
            let description = if crew.crew.id == selected_id && !roster.is_empty() {
                summarize_crew(crew, roster)
            } else if crew.runner_count == 0 {
                "No runners in this crew.".into()
            } else {
                format!(
                    "{} runner{}",
                    crew.runner_count,
                    if crew.runner_count == 1 { "" } else { "s" }
                )
            };
            SelectOption::new(crew.crew.id.clone(), crew.crew.name.clone()).description(description)
        })
        .collect()
}

fn summarize_crew(crew: &CrewListItem, roster: &[SlotWithRunner]) -> String {
    let Some(lead) = roster.iter().find(|member| member.slot.lead) else {
        return format!(
            "{} slot{}",
            crew.runner_count,
            if crew.runner_count == 1 { "" } else { "s" }
        );
    };
    let workers = roster
        .iter()
        .filter(|member| !member.slot.lead)
        .collect::<Vec<_>>();
    if workers.is_empty() {
        return format!("lead: @{}", lead.slot.slot_handle);
    }
    let shown = workers
        .iter()
        .take(3)
        .map(|member| format!("@{}", member.slot.slot_handle))
        .collect::<Vec<_>>()
        .join(", ");
    let tail = if workers.len() > 3 {
        format!(", +{}", workers.len() - 3)
    } else {
        String::new()
    };
    format!(
        "lead: @{} · {} worker{}: {shown}{tail}",
        lead.slot.slot_handle,
        workers.len(),
        if workers.len() == 1 { "" } else { "s" }
    )
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn error_banner(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(rems(12. / 16.))
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
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

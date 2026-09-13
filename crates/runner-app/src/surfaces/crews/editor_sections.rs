use super::logic::section_label;
use super::logic::text_action;
use super::logic::trimmed_option;

use gpui::prelude::*;
use gpui::{div, px, rems, AnyElement, Context, Window};
use runner_app::ui::{Button, TextField};
use runner_backend::model::Crew;
use runner_backend::ops::crew::UpdateCrewInput;

use crate::*;

impl NativeRoot {
    pub(super) fn render_crew_goal_section(
        &self,
        crew: &Crew,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let root = cx.entity();
        let existing_edit_root = root.clone();
        let add_edit_root = root.clone();
        let save_root = root.clone();
        let cancel_root = root;
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(section_label("Default goal"))
                    .children(
                        (editor.goal_edit.is_none() && crew.goal.is_some()).then(|| {
                            text_action("edit-crew-goal", "Edit", move |window, cx| {
                                existing_edit_root
                                    .update(cx, |this, cx| this.start_crew_goal_edit(window, cx));
                            })
                        }),
                    ),
            )
            .child(if let Some(input) = editor.goal_edit.clone() {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(input)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(
                                    "save-crew-goal",
                                    if editor.saving_goal {
                                        "Saving..."
                                    } else {
                                        "Save"
                                    },
                                )
                                .disabled(editor.saving_goal)
                                .on_press(move |_, cx| {
                                    save_root.update(cx, |this, cx| this.save_crew_goal(cx));
                                }),
                            )
                            .child(
                                Button::new("cancel-crew-goal", "Cancel")
                                    .disabled(editor.saving_goal)
                                    .on_press(move |_, cx| {
                                        cancel_root
                                            .update(cx, |this, cx| this.cancel_crew_goal_edit(cx));
                                    }),
                            ),
                    )
                    .into_any_element()
            } else if let Some(goal) = crew.goal.clone() {
                div()
                    .w_full()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .text_size(theme::text_title())
                    .line_height(rems(20. / 16.))
                    .text_color(theme::text())
                    .child(goal)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(text_action(
                        "add-crew-goal",
                        "+ Add default goal",
                        move |window, cx| {
                            add_edit_root
                                .update(cx, |this, cx| this.start_crew_goal_edit(window, cx));
                        },
                    ))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::faint())
                            .child("Pre-fills the Start Mission goal. Optional."),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    pub(super) fn render_crew_conventions_section(
        &self,
        crew: &Crew,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let root = cx.entity();
        let existing_edit_root = root.clone();
        let add_edit_root = root.clone();
        let save_root = root.clone();
        let cancel_root = root;
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(section_label("Team conventions"))
                    .children(
                        (editor.conventions_edit.is_none()
                            && crew.system_prompt_addendum.is_some())
                        .then(|| {
                            text_action("edit-crew-conventions", "Edit", move |window, cx| {
                                existing_edit_root.update(cx, |this, cx| {
                                    this.start_crew_conventions_edit(window, cx)
                                });
                            })
                        }),
                    ),
            )
            .child(if let Some(input) = editor.conventions_edit.clone() {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(input)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(
                                    "save-crew-conventions",
                                    if editor.saving_conventions {
                                        "Saving..."
                                    } else {
                                        "Save"
                                    },
                                )
                                .disabled(editor.saving_conventions)
                                .on_press(move |_, cx| {
                                    save_root.update(cx, |this, cx| {
                                        this.save_crew_conventions(cx)
                                    });
                                }),
                            )
                            .child(
                                Button::new("cancel-crew-conventions", "Cancel")
                                    .disabled(editor.saving_conventions)
                                    .on_press(move |_, cx| {
                                        cancel_root.update(cx, |this, cx| {
                                            this.cancel_crew_conventions_edit(cx)
                                        });
                                    }),
                            ),
                    )
                    .into_any_element()
            } else if let Some(conventions) = crew.system_prompt_addendum.clone() {
                div()
                    .w_full()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .text_size(theme::text_title())
                    .line_height(rems(20. / 16.))
                    .text_color(theme::text())
                    .child(conventions)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(text_action(
                        "add-crew-conventions",
                        "+ Add team conventions",
                        move |window, cx| {
                            add_edit_root.update(cx, |this, cx| {
                                this.start_crew_conventions_edit(window, cx)
                            });
                        },
                    ))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::faint())
                            .child("Optional team-level guidance applied to all mission spawns. Leave blank for crews that need no team-level layer."),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn start_crew_goal_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .crew_surfaces
            .editor
            .crew
            .as_ref()
            .and_then(|crew| crew.goal.clone())
            .unwrap_or_default();
        let input = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                value,
                "Pre-fills the Start Mission goal.",
                4,
                false,
            )
            .auto_grow(12)
        });
        let focus = input.read(cx).focus_handle();
        self.crew_surfaces.editor.goal_edit = Some(input);
        focus.focus(window);
        cx.notify();
    }

    fn cancel_crew_goal_edit(&mut self, cx: &mut Context<Self>) {
        if !self.crew_surfaces.editor.saving_goal {
            self.crew_surfaces.editor.goal_edit = None;
            cx.notify();
        }
    }

    fn save_crew_goal(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(input)) = (editor.crew.as_ref(), editor.goal_edit.as_ref()) else {
            return;
        };
        if editor.saving_goal {
            return;
        }
        editor.saving_goal = true;
        let crew_id = crew.id.clone();
        let value = trimmed_option(input.read(cx).text());
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    goal: Some(value),
                    ..Default::default()
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }

    fn start_crew_conventions_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .crew_surfaces
            .editor
            .crew
            .as_ref()
            .and_then(|crew| crew.system_prompt_addendum.clone())
            .unwrap_or_default();
        let input = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                value,
                "Optional team-level guidance applied to all mission spawns.",
                6,
                true,
            )
            .auto_grow(24)
        });
        let focus = input.read(cx).focus_handle();
        self.crew_surfaces.editor.conventions_edit = Some(input);
        focus.focus(window);
        cx.notify();
    }

    fn cancel_crew_conventions_edit(&mut self, cx: &mut Context<Self>) {
        if !self.crew_surfaces.editor.saving_conventions {
            self.crew_surfaces.editor.conventions_edit = None;
            cx.notify();
        }
    }

    fn save_crew_conventions(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(input)) = (editor.crew.as_ref(), editor.conventions_edit.as_ref())
        else {
            return;
        };
        if editor.saving_conventions {
            return;
        }
        editor.saving_conventions = true;
        let crew_id = crew.id.clone();
        let value = trimmed_option(input.read(cx).text());
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    system_prompt_addendum: Some(value),
                    ..Default::default()
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }
}

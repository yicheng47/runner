use super::logic::create_crew_form_is_composing;
use super::logic::error_banner;
use super::logic::trimmed_option;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, rems, AnyElement, Context, FontWeight, KeyDownEvent, Window};
use runner_app::ui::{
    Button, ButtonVariant, ConfirmDialog, Field, IconButton, Modal, OverlayWidth, TextField,
};
use runner_backend::ops::crew::CreateCrewInput;

use super::*;
use crate::*;

impl NativeRoot {
    pub(super) fn open_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.crew_surfaces.create.is_some() {
            return;
        }
        let name = cx
            .new(|input_cx| TextField::new(input_cx.focus_handle(), "", "runners-feature", false));
        let purpose = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "What does this crew exist to do?",
                2,
                false,
            )
        });
        let goal = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "Pre-fills the Start Mission goal.",
                3,
                false,
            )
        });
        let focus = name.read(cx).focus_handle();
        self.crew_surfaces.create = Some(CreateCrewForm {
            name,
            purpose,
            goal,
            purpose_hint_focus: cx.focus_handle(),
            goal_hint_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
        });
        focus.focus(window);
        cx.notify();
    }

    fn close_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.create = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_create_crew_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.crew_surfaces.create.as_ref().is_some_and(|form| {
                let multiline_focused = [&form.purpose, &form.goal]
                    .into_iter()
                    .any(|field| field.read(cx).focus_handle().is_focused(window));
                !multiline_focused && !create_crew_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_create_crew(window, cx);
        }
    }

    fn submit_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.create.as_mut() else {
            return;
        };
        let name = form.name.read(cx).text().trim().to_owned();
        if name.is_empty() {
            form.error = Some("Name is required".into());
            cx.notify();
            return;
        }
        if form.submitting {
            return;
        }
        form.submitting = true;
        form.error = None;
        let input = CreateCrewInput {
            name,
            purpose: trimmed_option(form.purpose.read(cx).text()),
            goal: trimmed_option(form.goal.read(cx).text()),
            system_prompt_addendum: None,
        };
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::crew::crew_create(&core, input).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(crew) => {
                        this.crew_surfaces.create = None;
                        this.load_crew_page(cx);
                        this.open_crew_editor(crew.id, window, cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.crew_surfaces.create.as_mut() {
                            form.submitting = false;
                            form.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_create_crew_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .crew_surfaces
            .create
            .as_ref()
            .expect("create crew form");
        let submitting = form.submitting;
        let can_submit = !submitting;
        let root = cx.entity();
        let close_root = root.clone();
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
                            .child("New crew"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Group of runners that work missions together."),
                    ),
            )
            .child(
                IconButton::new("close-create-crew", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close new crew")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_create_crew(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .on_key_down(cx.listener(Self::on_create_crew_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                Field::new("crew-name", "Name", form.name.clone())
                    .focus_target(form.name.read(cx).focus_handle()),
            )
            .child(
                Field::new("crew-purpose", "Purpose", form.purpose.clone())
                    .focus_target(form.purpose.read(cx).focus_handle())
                    .hint("optional", form.purpose_hint_focus.clone()),
            )
            .child(
                Field::new("crew-goal", "Default goal", form.goal.clone())
                    .focus_target(form.goal.read(cx).focus_handle())
                    .hint("optional", form.goal_hint_focus.clone()),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-create-crew", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_create_crew(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-create-crew",
                    if submitting {
                        "Creating…"
                    } else {
                        "Create crew"
                    },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_create_crew(window, cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_create_crew(window, cx));
            }),
        )
        .width(OverlayWidth::Md)
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                form.close_focus.clone(),
                form.name.read(cx).focus_handle(),
                form.purpose_hint_focus.clone(),
                form.purpose.read(cx).focus_handle(),
                form.goal_hint_focus.clone(),
                form.goal.read(cx).focus_handle(),
                form.cancel_focus.clone(),
                form.submit_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }

    pub(super) fn render_crew_delete_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .crew_surfaces
            .delete_confirm
            .as_ref()
            .expect("crew delete confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        ConfirmDialog::new(
            format!("Delete crew \"{}\" permanently?", confirm.name),
            "This removes all its slots and deletes its archived missions and session history. Crews with non-archived missions cannot be deleted until those missions are archived.",
            "Delete crew",
            "Deleting…",
            self.crew_surfaces.delete_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.advance_crew_delete(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.crew_surfaces.delete_busy {
                        this.crew_surfaces.delete_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }
}

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Context, KeyDownEvent, Window};
use runner_app::ui::{ConfirmDialog, TextField};
use runner_backend::ops::crew::CreateCrewInput;

use super::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.crew_surfaces.create.is_some() {
            self.enter_entity_route(AppRoute::NewCrew, window, cx);
            return;
        }
        let return_route = self.route.clone();
        self.enter_entity_route(AppRoute::NewCrew, window, cx);
        let name = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Crew name", false)
                .text_size(theme::text_display())
        });
        let conventions = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                "",
                "How this crew works together: branches, reviews, reporting. Markdown works.",
                6,
                true,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input.fill_height().with_scrollbar(input_cx)
        });
        let subscriptions = [&name, &conventions]
            .into_iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        self.crew_surfaces.editor.conventions_preview = false;
        let focus = name.read(cx).focus_handle();
        self.crew_surfaces.create = Some(CreateCrewForm {
            name,
            conventions,
            mode_focus: [cx.focus_handle(), cx.focus_handle()],
            action_focus: [cx.focus_handle(), cx.focus_handle()],
            return_route,
            _subscriptions: subscriptions,
            submitting: false,
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        let route = self
            .crew_surfaces
            .create
            .take()
            .map(|form| form.return_route)
            .unwrap_or(AppRoute::Crews);
        self.open_page_route(route, window, cx);
        cx.notify();
    }

    pub(super) fn on_create_crew_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.crew_surfaces.create.as_ref().is_some_and(|form| {
                !form.name.read(cx).is_composing()
                    && !form.conventions.read(cx).is_composing()
                    && form.name.read(cx).focus_handle().is_focused(window)
            })
        {
            cx.stop_propagation();
            self.submit_create_crew(window, cx);
        }
    }

    pub(super) fn submit_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            system_prompt_addendum: super::logic::trimmed_option(form.conventions.read(cx).text()),
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

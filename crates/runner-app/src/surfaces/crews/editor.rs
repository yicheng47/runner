use super::logic::crew_picture;
use super::logic::crew_summary;
use super::logic::error_panel;
use super::logic::selected_add_slot_role;
use super::logic::short_date;
use super::logic::slot_handle_error;
use super::logic::suggest_slot_handle;
use super::logic::trimmed_option;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, App, Context, FontWeight, KeyDownEvent, SharedString, Window,
};
use runner_app::ui::{Button, ButtonVariant, TextField};
use runner_backend::model::{Crew, SlotWithRole};
use runner_backend::ops::crew::UpdateCrewInput;

use super::*;
use crate::surfaces::profile_page::{
    breadcrumb, column_text, dot_note, editing_tag, page_columns, page_container, profile_column,
    section, short_id, ClickHandler, PROFILE_COLUMN_WIDTH,
};
use crate::surfaces::*;
use crate::*;

/// The crew picture on the page.
const PICTURE_SIZE: f32 = 95.;

impl NativeRoot {
    pub(crate) fn load_crew_editor(&mut self, crew_id: String, cx: &mut Context<Self>) {
        if self.crew_surfaces.editor.crew_id == crew_id {
            let editor = &mut self.crew_surfaces.editor;
            editor.loading = !editor.loaded;
            editor.error = None;
        } else {
            self.crew_surfaces.editor = CrewEditorState {
                crew_id: crew_id.clone(),
                loading: true,
                ..Default::default()
            };
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested = crew_id.clone();
            let result = (|| {
                let crew = runner_backend::ops::crew::crew_get(&core, &crew_id)?;
                let slots = runner_backend::ops::slot::slot_list(&core, &crew_id)?;
                Ok::<_, runner_backend::error::Error>((crew, slots))
            })();
            result
                .map(|(crew, slots)| (requested.clone(), crew, slots))
                .map_err(|error| (requested, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ =
                weak.update(cx, |this, cx| {
                    match result {
                        Ok((crew_id, crew, slots))
                            if matches!(
                                &this.route,
                                AppRoute::CrewEditor(active) if active == &crew_id
                            ) =>
                        {
                            let crew_name = crew.name.clone();
                            let existing_handles = slots
                                .iter()
                                .map(|slot| slot.slot.slot_handle.clone())
                                .collect::<HashSet<_>>();
                            let editor = &mut this.crew_surfaces.editor;
                            if editor.popup.as_ref().is_some_and(|popup| {
                                !slots.iter().any(|s| s.slot.id == popup.slot_id)
                            }) {
                                editor.popup = None;
                            }
                            editor.crew = Some(crew);
                            editor.slots = slots;
                            editor.loaded = true;
                            editor.loading = false;
                            editor.error = None;
                            if let Some(form) = this
                                .crew_surfaces
                                .add_slot
                                .as_mut()
                                .filter(|form| form.crew_id == crew_id)
                            {
                                form.crew_name = crew_name;
                                form.existing_handles = existing_handles;
                                if !form.slot_handle.read(cx).edited() {
                                    let suggestion = selected_add_slot_role(form)
                                        .map(|role| {
                                            suggest_slot_handle(
                                                &role.role.handle,
                                                &form.existing_handles,
                                            )
                                        })
                                        .unwrap_or_default();
                                    if form.slot_handle.read(cx).text() != suggestion {
                                        form.slot_handle.update(cx, |input, input_cx| {
                                            input.reset(suggestion, input_cx)
                                        });
                                    }
                                } else {
                                    let handle = form.slot_handle.read(cx).text();
                                    form.slot_handle_empty = handle.is_empty();
                                    form.slot_handle_error =
                                        slot_handle_error(handle, &form.existing_handles);
                                }
                            }
                        }
                        Ok(_) => {}
                        Err((crew_id, error))
                            if matches!(
                                &this.route,
                                AppRoute::CrewEditor(active) if active == &crew_id
                            ) =>
                        {
                            let editor = &mut this.crew_surfaces.editor;
                            editor.loading = false;
                            if error.to_lowercase().contains("not found") {
                                editor.crew = None;
                                editor.slots.clear();
                                editor.edit = None;
                                editor.popup = None;
                                editor.loaded = true;
                            }
                            editor.error = Some(error);
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                });
        })
        .detach();
    }

    /// The page's edit in place and slot popup live only on their own crew's
    /// page, so a route that leaves it discards them, as Cancel does.
    pub(crate) fn drop_crew_edit_for_route(&mut self, route: &AppRoute) {
        let editor = &mut self.crew_surfaces.editor;
        if matches!(route, AppRoute::Settings)
            || matches!(route, AppRoute::CrewEditor(id) if id == &editor.crew_id)
        {
            return;
        }
        if !editor.edit.as_ref().is_some_and(|form| form.saving) {
            editor.edit = None;
        }
        editor.popup = None;
    }

    pub(super) fn render_crew_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = self.profile_page_column_width(window, cx);
        let editor = &self.crew_surfaces.editor;
        let editing = editor.edit.is_some();
        let (loading, loaded) = (editor.loading, editor.loaded);
        let crew = editor.crew.clone();
        let slots = editor.slots.clone();
        let editor_error = editor.error.clone();
        let back_root = cx.entity();
        let on_back: ClickHandler = Rc::new(move |window, cx| {
            back_root.update(cx, |this, cx| this.open_crews(window, cx));
        });
        let current = crew
            .as_ref()
            .map(|crew| crew.name.clone())
            .unwrap_or_else(|| "…".into());
        let body = if loading && !loaded {
            div()
                .text_size(theme::text_title())
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if !loaded {
            error_panel(
                editor_error
                    .clone()
                    .unwrap_or_else(|| "Failed to load crew.".into()),
            )
        } else if let Some(crew) = crew {
            self.render_crew_page_body(crew, slots, column, cx)
        } else {
            div()
                .text_size(theme::text_title())
                .text_color(theme::danger())
                .child("Crew not found.")
                .into_any_element()
        };
        let error = editor_error.filter(|_| loaded);
        div()
            .id("crew-editor-scroll")
            .when(cfg!(test), |scroll| {
                scroll.debug_selector(|| "CREW_EDITOR_SCROLL".into())
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .when(editing, |page| {
                page.on_key_down(cx.listener(Self::on_crew_page_key_down))
            })
            .child(
                page_container()
                    .when(cfg!(test), |container| {
                        container.debug_selector(|| "CREW_EDITOR_CONTAINER".into())
                    })
                    .child(
                        breadcrumb(
                            "crew-editor-back",
                            "Crews",
                            on_back,
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                                .child(current),
                        )
                        .when(cfg!(test), |header| {
                            header.debug_selector(|| "CREW_EDITOR_HEADER".into())
                        })
                        .children(editing.then(|| {
                            editing_tag().when(cfg!(test), |tag| {
                                tag.debug_selector(|| "CREW_EDITING_TAG".into())
                            })
                        })),
                    )
                    .children(error.map(error_panel))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_crew_page_body(
        &mut self,
        crew: Crew,
        slots: Vec<SlotWithRole>,
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = self.crew_surfaces.editor.edit.is_some();
        let left = profile_column(column)
            .when(cfg!(test), |column| {
                column.debug_selector(|| "CREW_PAGE_PROFILE".into())
            })
            .child(self.render_crew_profile(&crew, &slots, column, cx))
            .child(self.render_slot_section(slots, column, cx))
            .child(crew_details(&crew, editing));
        let right = self.render_crew_cards(&crew, cx);
        page_columns(left, right, false)
            .when(cfg!(test), |body| {
                body.debug_selector(|| "CREW_PAGE_BODY".into())
            })
            .into_any_element()
    }

    /// The picture, name and actions. The actions keep the fixed column's
    /// width when the column spans a stacked page.
    fn render_crew_profile(
        &self,
        crew: &Crew,
        slots: &[SlotWithRole],
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let actions = rems(column.min(PROFILE_COLUMN_WIDTH) / 16.);
        let handles = slots
            .iter()
            .map(|slot| SharedString::from(slot.slot.slot_handle.clone()))
            .collect::<Vec<_>>();
        let lead = slots
            .iter()
            .find(|slot| slot.slot.lead)
            .map(|slot| slot.slot.slot_handle.as_str());
        let summary = column_text(crew_summary(slots.len(), lead), column)
            .text_size(theme::text_body())
            .text_color(theme::muted());
        let root = cx.entity();
        let profile = div().flex().flex_col().gap_4().child(
            div()
                .when(cfg!(test), |picture| {
                    picture.debug_selector(|| "CREW_PICTURE".into())
                })
                .child(crew_picture(&handles, PICTURE_SIZE)),
        );
        match self.crew_surfaces.editor.edit.as_ref() {
            None => {
                let start_root = root.clone();
                let start_crew_id = crew.id.clone();
                let edit_root = root;
                profile
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                column_text(crew.name.clone(), column)
                                    .text_size(theme::text_display())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text()),
                            )
                            .child(summary),
                    )
                    .child(
                        div()
                            .when(cfg!(test), |actions| {
                                actions.debug_selector(|| "CREW_PAGE_ACTIONS".into())
                            })
                            .w(actions)
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div().flex_1().min_w(px(0.)).child(
                                    Button::new("start-crew-mission", "Start mission")
                                        .icon("play.svg")
                                        .variant(ButtonVariant::Primary)
                                        .full_width(true)
                                        .tooltip(if slots.is_empty() {
                                            "Add at least one slot before starting a mission"
                                        } else {
                                            "Start a mission with this crew"
                                        })
                                        .disabled(slots.is_empty())
                                        .on_press(move |window, cx| {
                                            let crew_id = start_crew_id.clone();
                                            start_root.update(cx, |this, cx| {
                                                this.open_start_mission_modal(
                                                    Some(crew_id),
                                                    runner_backend::ops::project::ProjectScope::Root,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                ),
                            )
                            .child(
                                div()
                                    .when(cfg!(test), |edit| {
                                        edit.debug_selector(|| "CREW_EDIT".into())
                                    })
                                    .child(
                                        Button::new("edit-crew", "Edit")
                                            .icon("pencil.svg")
                                            .tooltip("Edit the name and conventions")
                                            .on_press(move |window, cx| {
                                                edit_root.update(cx, |this, cx| {
                                                    this.start_crew_edit(window, cx)
                                                });
                                            }),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            Some(form) => {
                let saving = form.saving;
                let name_empty = form.name.read(cx).text().trim().is_empty();
                let dirty = crew_edit_is_dirty(form, crew, cx);
                let save_root = root.clone();
                let cancel_root = root;
                profile
                    .when(cfg!(test), |profile| {
                        profile.debug_selector(|| "CREW_EDIT_IN_PLACE".into())
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().w(actions).child(form.name.clone()))
                            .child(summary.mt_1()),
                    )
                    .child(
                        div()
                            .w(actions)
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .when(cfg!(test), |save| {
                                                save.debug_selector(|| "CREW_EDIT_SAVE".into())
                                            })
                                            .flex_1()
                                            .min_w(px(0.))
                                            .child(
                                                Button::new(
                                                    "crew-page-save",
                                                    if saving { "Saving…" } else { "Save" },
                                                )
                                                .icon("check.svg")
                                                .variant(ButtonVariant::Primary)
                                                .full_width(true)
                                                .tooltip(if name_empty {
                                                    "The crew needs a name"
                                                } else {
                                                    "Save the name and conventions"
                                                })
                                                .disabled(saving || name_empty)
                                                .on_press(move |_, cx| {
                                                    save_root.update(cx, |this, cx| {
                                                        this.save_crew_edit(cx)
                                                    });
                                                }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .when(cfg!(test), |cancel| {
                                                cancel.debug_selector(|| "CREW_EDIT_CANCEL".into())
                                            })
                                            .child(
                                                Button::new("crew-page-cancel", "Cancel")
                                                    .disabled(saving)
                                                    .on_press(move |window, cx| {
                                                        cancel_root.update(cx, |this, cx| {
                                                            this.cancel_crew_edit(window, cx)
                                                        });
                                                    }),
                                            ),
                                    ),
                            )
                            // Always laid out, so the slots below never jump.
                            .child(
                                dot_note("Unsaved changes. Slots save on their own.")
                                    .when(!dirty, |line| line.opacity(0.))
                                    .when(cfg!(test) && dirty, |line| {
                                        line.debug_selector(|| "CREW_EDIT_DIRTY".into())
                                    }),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    pub(super) fn start_crew_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let Some(crew) = editor.crew.as_ref() else {
            return;
        };
        if editor.edit.is_some() {
            return;
        }
        let name_value = crew.name.clone();
        let conventions_value = crew.system_prompt_addendum.clone().unwrap_or_default();
        let name = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), name_value, "Crew name", false)
                .text_size(theme::text_display())
        });
        let conventions = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                conventions_value,
                "How this crew works together: who leads, how work is handed off and reviewed. Every slot gets it when the crew runs a mission. Markdown works.",
                6,
                true,
            )
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input.fill_height().with_scrollbar(input_cx)
        });
        // The page redraws on every keystroke to keep "Unsaved changes" honest.
        let subscriptions = [&name, &conventions]
            .into_iter()
            .map(|input| cx.observe(input, |_, _, cx| cx.notify()))
            .collect();
        let focus = name.read(cx).focus_handle();
        let editor = &mut self.crew_surfaces.editor;
        editor.conventions_preview = false;
        editor.popup = None;
        editor.edit = Some(CrewEditForm {
            name,
            conventions,
            saving: false,
            _subscriptions: subscriptions,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_crew_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        if editor.edit.as_ref().is_some_and(|form| form.saving) {
            return;
        }
        editor.edit = None;
        window.focus(&self.root_focus, cx);
        cx.notify();
    }

    pub(super) fn save_crew_edit(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(form)) = (editor.crew.as_ref(), editor.edit.as_mut()) else {
            return;
        };
        let name = form.name.read(cx).text().trim().to_owned();
        if form.saving || name.is_empty() {
            return;
        }
        form.saving = true;
        editor.error = None;
        let input = UpdateCrewInput {
            name: Some(name),
            system_prompt_addendum: Some(trimmed_option(form.conventions.read(cx).text())),
        };
        let crew_id = crew.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(&core, &crew_id, input)
                .map(|_| ())
                .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }

    fn on_crew_page_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.crew_surfaces.editor.edit.as_ref() else {
            return;
        };
        let composing =
            form.name.read(cx).is_composing() || form.conventions.read(cx).is_composing();
        if composing || form.saving {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.cancel_crew_edit(window, cx);
            }
            "enter" if form.name.read(cx).focus_handle().is_focused(window) => {
                cx.stop_propagation();
                self.save_crew_edit(cx);
            }
            _ => {}
        }
    }
}

/// Whether the edit holds anything a save would write.
pub(super) fn crew_edit_is_dirty(form: &CrewEditForm, crew: &Crew, cx: &App) -> bool {
    form.name.read(cx).text().trim() != crew.name.trim()
        || trimmed_option(form.conventions.read(cx).text())
            != crew
                .system_prompt_addendum
                .as_deref()
                .and_then(trimmed_option)
}

fn crew_details(crew: &Crew, editing: bool) -> AnyElement {
    let now = chrono::Local::now();
    let date = |timestamp: runner_backend::model::Timestamp| {
        short_date(&timestamp.with_timezone(&chrono::Local), &now)
    };
    let (created, updated) = (date(crew.created_at), date(crew.updated_at));
    let dates = if updated == created {
        format!("Created {created}")
    } else {
        format!("Created {created} · updated {updated}")
    };
    section()
        .gap(rems(6. / 16.))
        .when(editing, |section| section.opacity(0.4))
        .text_size(theme::text_ui())
        .child(div().text_color(theme::faint()).child(dates))
        .child(
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(short_id(&crew.id)),
        )
        .into_any_element()
}

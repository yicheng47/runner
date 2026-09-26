use super::logic::crew_name_refresh;
use super::logic::crew_name_state;
use super::logic::error_panel;
use super::logic::section_label;
use super::logic::selected_add_slot_role;
use super::logic::slot_handle_error;
use super::logic::slot_section_description;
use super::logic::suggest_slot_handle;
use super::logic::CrewNameRefresh;
use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{div, px, rems, AnyElement, Context, FontWeight, KeyDownEvent, Window};
use runner_app::ui::{Button, ButtonVariant, TextField, Tooltip};
use runner_backend::ops::crew::UpdateCrewInput;

use super::*;
use crate::surfaces::*;
use crate::*;

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
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((crew_id, crew, slots))
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) =>
                    {
                        let active_crew_id = crew_id.clone();
                        let crew_name = crew.name.clone();
                        let existing_handles = slots
                            .iter()
                            .map(|slot| slot.slot.slot_handle.clone())
                            .collect::<HashSet<_>>();
                        if this.crew_surfaces.editor.name.is_none() {
                            let name = cx.new(|input_cx| {
                                TextField::new(
                                    input_cx.focus_handle(),
                                    crew.name.clone(),
                                    "",
                                    false,
                                )
                                .text_size(theme::text_title())
                            });
                            let subscription = cx.observe(&name, move |this, input, cx| {
                                let value = input.read(cx).text().to_owned();
                                let editor = &mut this.crew_surfaces.editor;
                                let (changed, dirty, empty) =
                                    crew_name_state(&value, &editor.original_name);
                                if editor.name_changed != changed
                                    || editor.name_dirty != dirty
                                    || editor.name_empty != empty
                                {
                                    editor.name_changed = changed;
                                    editor.name_dirty = dirty;
                                    editor.name_empty = empty;
                                    cx.notify();
                                }
                            });
                            let editor = &mut this.crew_surfaces.editor;
                            editor.name = Some(name);
                            editor._name_subscription = Some(subscription);
                        }
                        let name = this
                            .crew_surfaces
                            .editor
                            .name
                            .as_ref()
                            .cloned()
                            .expect("crew editor name field");
                        let current_name = name.read(cx).text().to_owned();
                        match crew_name_refresh(&current_name, &crew.name, name.read(cx).edited()) {
                            CrewNameRefresh::MarkClean => {
                                name.update(cx, |input, _| input.mark_clean());
                            }
                            CrewNameRefresh::Reset => {
                                name.update(cx, |input, input_cx| {
                                    input.reset(crew.name.clone(), input_cx)
                                });
                            }
                            CrewNameRefresh::Preserve => {}
                        }
                        let current_name = name.read(cx).text().to_owned();
                        let (name_changed, name_dirty, name_empty) =
                            crew_name_state(&current_name, &crew.name);
                        let editor = &mut this.crew_surfaces.editor;
                        editor.crew_id = crew_id;
                        editor.original_name = crew.name.clone();
                        editor.crew = Some(crew);
                        editor.slots = slots;
                        editor.loaded = true;
                        editor.loading = false;
                        editor.error = None;
                        editor.name_changed = name_changed;
                        editor.name_dirty = name_dirty;
                        editor.name_empty = name_empty;
                        if let Some(form) = this
                            .crew_surfaces
                            .add_slot
                            .as_mut()
                            .filter(|form| form.crew_id == active_crew_id)
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
                            editor.name = None;
                            editor._name_subscription = None;
                            editor.original_name.clear();
                            editor.goal_edit = None;
                            editor.conventions_edit = None;
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

    pub(super) fn render_crew_editor(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let crew = editor.crew.clone();
        let slots = editor.slots.clone();
        let name = editor.name.clone();
        let name_changed = editor.name_changed;
        let name_dirty = editor.name_dirty;
        let name_empty = editor.name_empty;
        let root = cx.entity();
        let back_root = root.clone();
        let back_key_root = root.clone();
        let save_name_root = root.clone();
        let start_mission_root = root.clone();
        let start_mission_crew_id = editor.crew_id.clone();
        let add_slot_root = root.clone();
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_8()
            .pb_4()
            .pt(rems(36. / 16.))
            .on_key_down(cx.listener(Self::on_crew_name_key_down))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("crew-editor-back")
                            .tab_index(0)
                            .flex_none()
                            .cursor_pointer()
                            .text_size(theme::text_title())
                            .text_color(theme::muted())
                            .hover(|text| text.text_color(theme::text()))
                            .focus_visible(|text| text.text_color(theme::text()).underline())
                            .on_click(move |_, window, cx| {
                                back_root.update(cx, |this, cx| this.open_crews(window, cx));
                            })
                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    back_key_root
                                        .update(cx, |this, cx| this.open_crews(window, cx));
                                }
                            })
                            .child("‹ Crews"),
                    )
                    .child(div().text_color(theme::border_strong()).child("›"))
                    .child(if let Some(name) = name.clone() {
                        div()
                            .w_full()
                            .max_w(rems(384. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name)
                            .into_any_element()
                    } else {
                        div()
                            .text_size(theme::text_title())
                            .text_color(theme::faint())
                            .child("…")
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if name_changed || editor.saving_name {
                        Button::new(
                            "save-crew-name",
                            if editor.saving_name {
                                "Saving..."
                            } else {
                                "Save"
                            },
                        )
                        .disabled(editor.saving_name || !name_dirty)
                        .tooltip(if name_empty {
                            "Crew name cannot be empty"
                        } else if name_dirty {
                            "Save crew name"
                        } else {
                            "No persisted change after trimming"
                        })
                        .on_press(move |_, cx| {
                            save_name_root.update(cx, |this, cx| this.save_crew_name(cx));
                        })
                        .into_any_element()
                    } else {
                        Tooltip::new(
                            "crew-name-saved-tooltip",
                            "Crew name is saved. Slot changes save immediately.",
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .border_1()
                                .border_color(theme::border())
                                .bg(theme::raised())
                                .px_3()
                                .py(rems(6. / 16.))
                                .text_size(theme::text_title())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::faint())
                                .child("Saved"),
                        )
                        .into_any_element()
                    })
                    .child(
                        Button::new("start-crew-mission", "Start mission")
                            .variant(ButtonVariant::Primary)
                            .tooltip(if slots.is_empty() {
                                "Add at least one slot before starting a mission"
                            } else {
                                "Start a mission with this crew"
                            })
                            .disabled(slots.is_empty())
                            .on_press(move |window, cx| {
                                start_mission_root.update(cx, |this, cx| {
                                    this.open_start_mission_modal(
                                        Some(start_mission_crew_id.clone()),
                                        runner_backend::ops::project::ProjectScope::Root,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    ),
            );
        let content = if editor.loading {
            div()
                .p_8()
                .text_size(theme::text_title())
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if !editor.loaded {
            div()
                .m_8()
                .child(error_panel(
                    editor
                        .error
                        .clone()
                        .unwrap_or_else(|| "Failed to load crew.".into()),
                ))
                .into_any_element()
        } else if crew.is_none() {
            div()
                .p_8()
                .text_size(theme::text_title())
                .text_color(theme::danger())
                .child("Crew not found.")
                .into_any_element()
        } else {
            let crew = match crew {
                Some(crew) => crew,
                None => unreachable!("crew presence checked above"),
            };
            let sections = div()
                .when(cfg!(test), |sections| {
                    sections.debug_selector(|| "CREW_EDITOR_SECTIONS".into())
                })
                .w_full()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_8()
                .children(editor.error.clone().map(error_panel))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(rems(6. / 16.))
                        .child(section_label("Purpose"))
                        .child(if let Some(purpose) = crew.purpose.clone() {
                            div()
                                .w_full()
                                .min_w(px(0.))
                                .whitespace_normal()
                                .text_size(theme::text_title())
                                .line_height(rems(20. / 16.))
                                .text_color(theme::text())
                                .child(purpose)
                        } else {
                            div()
                                .text_size(theme::text_title())
                                .text_color(theme::faint())
                                .italic()
                                .child("No purpose set.")
                        }),
                )
                .child(self.render_crew_goal_section(&crew, cx))
                .child(self.render_crew_conventions_section(&crew, cx))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.))
                                .flex()
                                .items_end()
                                .justify_between()
                                .gap_4()
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap(rems(2. / 16.))
                                        .child(
                                            div()
                                                .text_size(theme::text_display())
                                                .font_weight(FontWeight::BOLD)
                                                .child("Slots"),
                                        )
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w(px(0.))
                                                .whitespace_normal()
                                                .text_size(theme::text_ui())
                                                .line_height(rems(1.))
                                                .text_color(theme::muted())
                                                .child(slot_section_description()),
                                        ),
                                )
                                .child(
                                    div().flex_none().child(
                                        Button::new("add-crew-slot", "+ Add slot")
                                            .variant(ButtonVariant::Primary)
                                            .on_press(move |window, cx| {
                                                add_slot_root.update(cx, |this, cx| {
                                                    this.open_add_slot(window, cx)
                                                });
                                            }),
                                    ),
                                ),
                        )
                        .child(self.render_slot_list(slots, cx)),
                );
            div()
                .when(cfg!(test), |container| {
                    container.debug_selector(|| "CREW_EDITOR_CONTAINER".into())
                })
                .mx_auto()
                .w_full()
                .min_w(px(0.))
                .max_w(rems(896. / 16.))
                .px_8()
                .py_8()
                .child(sections)
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("crew-editor-scroll")
                    .when(cfg!(test), |scroll| {
                        scroll.debug_selector(|| "CREW_EDITOR_SCROLL".into())
                    })
                    .min_h(px(0.))
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
            .into_any_element()
    }

    fn on_crew_name_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = &self.crew_surfaces.editor;
        let Some(name) = editor.name.as_ref() else {
            return;
        };
        if !name.read(cx).focus_handle().is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" if !name.read(cx).is_composing() => {
                cx.stop_propagation();
                self.save_crew_name(cx);
            }
            "escape" => {
                cx.stop_propagation();
                if let Some(value) = self
                    .crew_surfaces
                    .editor
                    .crew
                    .as_ref()
                    .map(|crew| crew.name.clone())
                {
                    name.update(cx, |field, field_cx| field.reset(value, field_cx));
                }
                window.focus(&self.root_focus);
                cx.notify();
            }
            _ => {}
        }
    }

    fn save_crew_name(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(name)) = (editor.crew.as_ref(), editor.name.as_ref()) else {
            return;
        };
        let next = name.read(cx).text().trim().to_owned();
        if next.is_empty() || next == crew.name || editor.saving_name {
            return;
        }
        editor.saving_name = true;
        let crew_id = crew.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    name: Some(next),
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

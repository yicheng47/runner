use super::logic::add_slot_can_submit;
use super::logic::add_slot_focus_order;
use super::logic::add_slot_form_is_composing;
use super::logic::add_slot_runtime_options;
use super::logic::crew_usage_label;
use super::logic::error_banner;
use super::logic::role_activity_label;
use super::logic::role_matches;
use super::logic::runtime_models;
use super::logic::selected_add_slot_role;
use super::logic::slot_handle_error;
use super::logic::suggest_slot_handle;
use super::logic::trimmed_option;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, FontWeight, KeyDownEvent, ScrollHandle, Window,
};
use runner_app::ui::{
    Button, ButtonVariant, Field, IconButton, Modal, ModelField, OverlayWidth, Scrollbar,
    StyledSelect, TextField, Tooltip,
};

use super::*;
use crate::*;

impl NativeRoot {
    pub(super) fn open_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.crew_surfaces.add_slot.is_some() {
            return;
        }
        let Some(crew) = self.crew_surfaces.editor.crew.clone() else {
            return;
        };
        let existing_handles = self
            .crew_surfaces
            .editor
            .slots
            .iter()
            .map(|slot| slot.slot.slot_handle.clone())
            .collect::<HashSet<_>>();
        let runtimes =
            runner_backend::ops::runtime::runtime_catalog(self.core(cx)).unwrap_or_default();
        let query = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Search roles...", false)
                .text_size(theme::text_body())
        });
        let slot_handle = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "architect", true)
                .text_size(theme::text_title())
        });
        slot_handle.update(cx, |input, input_cx| input.set_bare(true, input_cx));
        let model_override = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", true).placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| ModelField::new(model_override.clone(), &[], model_cx));
        let runtime_root = cx.entity();
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "add-slot-runtime",
                select_cx.focus_handle(),
                "",
                add_slot_runtime_options(&runtimes, None),
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_add_slot_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe(&query, |this, _, cx| {
            this.sync_add_slot_filter(cx);
        }));
        subscriptions.push(cx.observe(&slot_handle, |this, input, cx| {
            let text = input.read(cx).text().to_owned();
            let lowercase = text.to_lowercase();
            if text != lowercase {
                input.update(cx, |input, input_cx| input.set_text(lowercase, input_cx));
                return;
            }
            let Some(form) = this.crew_surfaces.add_slot.as_mut() else {
                return;
            };
            let empty = text.is_empty();
            let error = slot_handle_error(&text, &form.existing_handles);
            if form.slot_handle_empty != empty || form.slot_handle_error != error {
                form.slot_handle_empty = empty;
                form.slot_handle_error = error;
                cx.notify();
            }
        }));
        let focus = query.read(cx).focus_handle();
        self.crew_surfaces.add_slot = Some(AddSlotForm {
            crew_id: crew.id,
            crew_name: crew.name,
            existing_handles,
            roles: Vec::new(),
            runtimes,
            query,
            last_synced_query: String::new(),
            selected_role_id: None,
            slot_handle,
            runtime_override: String::new(),
            model_override,
            model_field,
            runtime_select,
            scroll,
            scrollbar,
            slot_handle_hint_focus: cx.focus_handle(),
            runtime_hint_focus: cx.focus_handle(),
            model_hint_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            slot_handle_empty: true,
            slot_handle_error: None,
            loading: true,
            submitting: false,
            error: None,
            _subscriptions: subscriptions,
        });
        focus.focus(window);
        self.load_add_slot_roles(cx);
        cx.notify();
    }

    fn load_add_slot_roles(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_ref() else {
            return;
        };
        let crew_id = form.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::role::role_list_with_activity(&core, 1, 1_000_000, "")
                .map(|page| page.items)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(form) = this.crew_surfaces.add_slot.as_mut() else {
                    return;
                };
                if form.crew_id != crew_id {
                    return;
                }
                form.loading = false;
                match result {
                    Ok(roles) => {
                        form.roles = roles;
                        let query = form.query.read(cx).text().trim().to_lowercase();
                        form.last_synced_query = query.clone();
                        form.selected_role_id = form
                            .roles
                            .iter()
                            .find(|role| role_matches(role, &query))
                            .map(|role| role.role.id.clone());
                        if !form.slot_handle.read(cx).edited() {
                            if let Some(role) = selected_add_slot_role(form) {
                                let suggestion =
                                    suggest_slot_handle(&role.role.handle, &form.existing_handles);
                                if form.slot_handle.read(cx).text() != suggestion {
                                    form.slot_handle.update(cx, |input, input_cx| {
                                        input.reset(suggestion, input_cx)
                                    });
                                }
                            }
                        }
                        let options =
                            add_slot_runtime_options(&form.runtimes, selected_add_slot_role(form));
                        form.runtime_select.update(cx, |select, select_cx| {
                            select.set_options(options, select_cx)
                        });
                        this.sync_add_slot_catalog(cx);
                    }
                    Err(error) => form.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_add_slot_filter(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        let query = form.query.read(cx).text().trim().to_lowercase();
        let query_changed = query != form.last_synced_query;
        if query_changed {
            form.last_synced_query = query.clone();
        }
        let selected_visible = form.selected_role_id.as_ref().is_some_and(|selected| {
            form.roles
                .iter()
                .any(|role| role.role.id == *selected && role_matches(role, &query))
        });
        if !query_changed && selected_visible {
            return;
        }
        let mut selection_changed = false;
        if !selected_visible {
            let next = form
                .roles
                .iter()
                .find(|role| role_matches(role, &query))
                .map(|role| role.role.id.clone());
            selection_changed = next != form.selected_role_id;
            form.selected_role_id = next;
            if selection_changed && !form.slot_handle.read(cx).edited() {
                let suggestion = selected_add_slot_role(form)
                    .map(|role| suggest_slot_handle(&role.role.handle, &form.existing_handles))
                    .unwrap_or_default();
                if form.slot_handle.read(cx).text() != suggestion {
                    form.slot_handle
                        .update(cx, |input, input_cx| input.reset(suggestion, input_cx));
                }
            }
            if selection_changed {
                let options =
                    add_slot_runtime_options(&form.runtimes, selected_add_slot_role(form));
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx)
                });
            }
        }
        if selection_changed {
            self.sync_add_slot_catalog(cx);
        }
        if query_changed || selection_changed {
            cx.notify();
        }
    }

    fn select_add_slot_role(&mut self, role_id: String, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        form.selected_role_id = Some(role_id);
        let suggestion = selected_add_slot_role(form)
            .map(|role| suggest_slot_handle(&role.role.handle, &form.existing_handles))
            .unwrap_or_default();
        form.slot_handle
            .update(cx, |input, input_cx| input.reset(suggestion, input_cx));
        let options = add_slot_runtime_options(&form.runtimes, selected_add_slot_role(form));
        form.runtime_select.update(cx, |select, select_cx| {
            select.set_options(options, select_cx)
        });
        self.sync_add_slot_catalog(cx);
    }

    fn select_add_slot_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        form.runtime_override = value;
        form.model_override
            .update(cx, |input, input_cx| input.reset("", input_cx));
        self.sync_add_slot_catalog(cx);
    }

    fn sync_add_slot_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        let runtime = if form.runtime_override.is_empty() {
            selected_add_slot_role(form)
                .map(|role| role.role.runtime.clone())
                .unwrap_or_default()
        } else {
            form.runtime_override.clone()
        };
        form.model_field.update(cx, |field, cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &runtime), cx)
        });
        self.request_model_catalog(&runtime, cx);
        cx.notify();
    }

    pub(crate) fn refresh_add_slot_runtimes(&mut self, cx: &mut Context<Self>) {
        let Ok(catalog) = runner_backend::ops::runtime::runtime_catalog(self.core(cx)) else {
            return;
        };
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        form.runtimes = catalog;
        let options = add_slot_runtime_options(&form.runtimes, selected_add_slot_role(form));
        form.runtime_select.update(cx, |select, select_cx| {
            select.set_options(options, select_cx)
        });
        self.sync_add_slot_catalog(cx);
    }

    fn close_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .add_slot
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.add_slot = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_add_slot_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self
                .crew_surfaces
                .add_slot
                .as_ref()
                .is_some_and(|form| !add_slot_form_is_composing(form, cx))
        {
            cx.stop_propagation();
            self.submit_add_slot(cx);
        }
    }

    fn create_role_from_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .add_slot
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.add_slot = None;
        self.open_roles(window, cx);
        self.open_create_role(window, cx);
    }

    fn submit_add_slot(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        let handle = form.slot_handle.read(cx).text().to_owned();
        if !add_slot_can_submit(form) {
            return;
        }
        let role_id = form.selected_role_id.clone().expect("validated role");
        form.submitting = true;
        form.error = None;
        let crew_id = form.crew_id.clone();
        let input = runner_backend::ops::slot::CreateSlotInput {
            crew_id: crew_id.clone(),
            role_id,
            slot_handle: handle,
            runtime_override: runner_backend::model::Runtime::parse(&form.runtime_override),
            model_override: (!form.runtime_override.is_empty())
                .then(|| trimmed_option(form.model_override.read(cx).text()))
                .flatten(),
        };
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::slot::slot_create(&core, input).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.crew_surfaces.add_slot = None;
                        this.load_crew_editor(crew_id, cx);
                        this.load_crew_page(cx);
                        this.load_role_page(cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.crew_surfaces.add_slot.as_mut() {
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

    pub(super) fn render_add_slot_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self.crew_surfaces.add_slot.as_ref().expect("add slot form");
        let query = form.query.read(cx).text().trim().to_lowercase();
        let filtered = form
            .roles
            .iter()
            .filter(|role| role_matches(role, &query))
            .cloned()
            .collect::<Vec<_>>();
        let submitting = form.submitting;
        let can_submit = add_slot_can_submit(form);
        let handle_error = form.slot_handle_error.clone();
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let create_root = root.clone();
        let create_key_root = root.clone();
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
                            .child("Add slot"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(format!("crew: {}", form.crew_name)),
                    ),
            )
            .child(
                IconButton::new("close-add-slot", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close add slot")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_add_slot(window, cx));
                    }),
            );
        let role_rows = if form.loading {
            vec![div()
                .px_3()
                .py_3()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child("Loading roles...")
                .into_any_element()]
        } else if filtered.is_empty() {
            vec![div()
                .px_3()
                .py_3()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(if form.roles.is_empty() {
                    "No roles yet. Create one first, then add it here."
                } else {
                    "No roles match this search."
                })
                .into_any_element()]
        } else {
            filtered
                .into_iter()
                .enumerate()
                .map(|(index, role)| {
                    let selected = form.selected_role_id.as_deref() == Some(role.role.id.as_str());
                    let role_id = role.role.id.clone();
                    let key_role_id = role_id.clone();
                    let select_root = cx.entity();
                    let key_root = select_root.clone();
                    div()
                        .id(("add-slot-role", index))
                        .tab_index(0)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .border_b_1()
                        .border_color(theme::border())
                        .px_3()
                        .py(rems(10. / 16.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(theme::raised()))
                        .hover(|row| row.bg(theme::raised()))
                        .child(
                            div()
                                .w(rems(160. / 16.))
                                .truncate()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::accent())
                                .child(format!("@{}", role.role.handle)),
                        )
                        .child(
                            div()
                                .w_20()
                                .truncate()
                                .text_size(theme::text_meta())
                                .text_color(theme::muted())
                                .child(role.role.runtime.clone()),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .truncate()
                                .text_size(theme::text_ui())
                                .text_color(theme::muted())
                                .child(format!(
                                    "{} · {}",
                                    crew_usage_label(&role),
                                    role_activity_label(&role)
                                )),
                        )
                        .on_click(move |_, _, cx| {
                            select_root.update(cx, |this, cx| {
                                this.select_add_slot_role(role_id.clone(), cx)
                            });
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root.update(cx, |this, cx| {
                                    this.select_add_slot_role(key_role_id.clone(), cx)
                                });
                            }
                        })
                        .into_any_element()
                })
                .collect()
        };
        let handle_input = div()
            .w_full()
            .flex()
            .items_center()
            .rounded_sm()
            .border_1()
            .border_color(if handle_error.is_some() {
                theme::danger()
            } else {
                theme::border_strong()
            })
            .bg(theme::bg())
            .px(rems(10. / 16.))
            .py(rems(6. / 16.))
            .text_size(theme::text_title())
            .child(
                div()
                    .pr_1()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child("@"),
            )
            .child(div().min_w(px(0.)).flex_1().child(form.slot_handle.clone()));
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_add_slot_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(6. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Role"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(rems(6. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .px_3()
                            .py_2()
                            .child(div().min_w(px(0.)).flex_1().child(form.query.clone()))
                            .child(
                                svg()
                                    .flex_none()
                                    .path("chevron-down.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::faint()),
                            ),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .rounded(rems(6. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .id("add-slot-create-role")
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .border_b_1()
                                    .border_color(theme::border())
                                    .px_3()
                                    .py(rems(10. / 16.))
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::accent())
                                    .hover(|row| row.bg(theme::raised()))
                                    .focus_visible(|row| row.bg(theme::raised()))
                                    .on_click(move |_, window, cx| {
                                        create_root.update(cx, |this, cx| {
                                            this.create_role_from_add_slot(window, cx)
                                        });
                                    })
                                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                        if matches!(
                                            event.keystroke.key.as_str(),
                                            "enter" | "space"
                                        ) {
                                            cx.stop_propagation();
                                            create_key_root.update(cx, |this, cx| {
                                                this.create_role_from_add_slot(window, cx)
                                            });
                                        }
                                    })
                                    .child("+ Create new role..."),
                            )
                            .child(
                                div()
                                    .id("add-slot-role-scroll")
                                    .max_h(rems(224. / 16.))
                                    .overflow_y_scroll()
                                    .children(role_rows),
                            ),
                    ),
            )
            .child(
                Field::new("add-slot-handle", "Slot handle", handle_input)
                    .focus_target(form.slot_handle.read(cx).focus_handle())
                    .hint(
                        "in-crew identity used by mission events and stdin routing",
                        form.slot_handle_hint_focus.clone(),
                    )
                    .when_some(handle_error, |field, error| field.error(error)),
            )
            .child(
                Field::new("add-slot-runtime", "Runtime", form.runtime_select.clone())
                    .focus_target(form.runtime_select.read(cx).focus_handle())
                    .hint(
                        "engine this slot runs — overriding keeps the role's prompt but uses the runtime's default command and flags",
                        form.runtime_hint_focus.clone(),
                    ),
            )
            .children((!form.runtime_override.is_empty()
                && form.selected_role_id.is_some())
            .then(|| {
                Field::new("add-slot-model", "Model", form.model_field.clone())
                    .focus_target(form.model_override.read(cx).focus_handle())
                    .hint(
                        "optional · default uses the selected runtime's own model",
                        form.model_hint_focus.clone(),
                    )
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .opacity(0.7)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme::text_ui())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("System prompt override"),
                                    )
                                    .child(
                                        div()
                                            .rounded_sm()
                                            .bg(theme::raised())
                                            .px(rems(6. / 16.))
                                            .py(rems(2. / 16.))
                                            .text_size(theme::text_caption())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::faint())
                                            .child("V0.X"),
                                    ),
                            )
                            .child(Tooltip::new(
                                "add-slot-system-prompt-tooltip",
                                "Per-slot prompt overrides land in v0.x",
                                div()
                                    .w_8()
                                    .h(rems(18. / 16.))
                                    .flex()
                                    .items_center()
                                    .rounded_full()
                                    .bg(theme::raised())
                                    .p(rems(2. / 16.))
                                    .child(
                                        div()
                                            .size(rems(14. / 16.))
                                            .rounded_full()
                                            .bg(theme::panel()),
                                    ),
                            )),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child("Uses the selected role's default prompt. Per-slot overrides are not editable in the MVP."),
                    ),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-add-slot", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_add_slot(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-add-slot",
                    if submitting { "Adding..." } else { "Add slot" },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |_, cx| {
                    submit_root.update(cx, |this, cx| this.submit_add_slot(cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_add_slot(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(FORM_WIDTH))
        .busy(submitting)
        .focus_order(add_slot_focus_order(form, cx))
        .scrollbar(form.scroll.clone(), form.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }
}

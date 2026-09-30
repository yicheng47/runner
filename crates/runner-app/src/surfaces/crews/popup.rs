use super::logic::slot_command_summary;
use super::logic::slot_runtime_options;
use super::logic::slot_setup;
use super::logic::trimmed_option;
use super::slots::lead_badge;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, div, linear_color_stop, linear_gradient, point, px, rems, svg, Anchor,
    AnchoredPositionMode, AnyElement, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize, ModelField, RoleAvatar,
    SelectOption, StyledSelect, TextField,
};
use runner_backend::model::{CodexSpeed, Runtime, SlotWithRole};
use runner_backend::ops::slot::UpdateSlotInput;

use super::*;
use crate::chat_icon::ChatIcon;
use crate::surfaces::profile_page::{override_dot, plural, text_action};
use crate::surfaces::roles::logic::{
    effort_options, ensure_runtime_present, runtime_display_name, runtime_models,
};
use crate::surfaces::*;
use crate::*;

const POPUP_WIDTH: f32 = 384.;
const POPUP_LABEL_WIDTH: f32 = 82.;
/// A popup field: the popup less its padding and the label column.
const POPUP_FIELD_WIDTH: f32 = POPUP_WIDTH - 2. * 16. - POPUP_LABEL_WIDTH;
const RUNTIME_NOTE: &str =
    "A different runtime starts from its own agent defaults; set overrides here.";

fn speed_value(speed: Option<CodexSpeed>) -> &'static str {
    match speed {
        None => "",
        Some(CodexSpeed::Standard) => "standard",
        Some(CodexSpeed::Fast) => "fast",
    }
}

fn speed_label(speed: Option<CodexSpeed>) -> &'static str {
    match speed {
        None => "Inherit",
        Some(CodexSpeed::Standard) => "Standard",
        Some(CodexSpeed::Fast) => "Fast",
    }
}

/// A model or effort the slot does not set: the role's value on the role's
/// own runtime, else the runtime's own default.
pub(super) fn inherited_label(
    runtime: &str,
    own_runtime: bool,
    role_value: Option<&str>,
) -> String {
    match role_value.filter(|value| own_runtime && !value.trim().is_empty()) {
        Some(value) => value.to_owned(),
        None => format!("{} default", runtime_display_name(runtime)),
    }
}

impl NativeRoot {
    pub(super) fn render_slot_popup(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let editor = &self.crew_surfaces.editor;
        let popup = editor.popup.as_ref()?;
        if !matches!(&self.route, AppRoute::CrewEditor(id) if id == &editor.crew_id) {
            return None;
        }
        let slot = editor
            .slots
            .iter()
            .find(|slot| slot.slot.id == popup.slot_id)?
            .clone();
        let viewport = window.viewport_size();
        let rem = window.rem_size();
        let anchor = popup.anchor.get();
        let root = cx.entity();
        let dismiss_root = root.clone();
        let dismiss_right_root = root.clone();
        let key_root = root;
        let panel = div()
            .id("crew-slot-popup")
            .when(cfg!(test), |panel| {
                panel.debug_selector(|| "CREW_SLOT_POPUP".into())
            })
            .track_focus(&popup.focus)
            .w(rems(POPUP_WIDTH / 16.))
            // The window's height less the snap margins; the setup scrolls
            // between the fixed header and footer when it runs taller.
            .max_h(viewport.height - rem)
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::panel())
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    key_root.update(cx, |this, cx| this.close_slot_popup(window, cx));
                }
            })
            .child(div().flex_none().child(self.render_popup_header(&slot, cx)))
            .child(
                div()
                    .id("crew-slot-popup-body")
                    .when(cfg!(test), |body| {
                        body.debug_selector(|| "CREW_SLOT_POPUP_BODY".into())
                    })
                    .min_h(px(0.))
                    .flex_shrink(1.)
                    .overflow_y_scroll()
                    .child(match popup.edit.as_ref() {
                        Some(form) => self.render_popup_override_rows(&slot, form, cx),
                        None => popup_view_rows(&slot),
                    })
                    .child(popup_command_and_prompt(&slot)),
            )
            .child(div().flex_none().child(match popup.edit.as_ref() {
                Some(form) => self.render_popup_edit_footer(form, cx),
                None => self.render_popup_view_footer(&slot, cx),
            }));
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(
                    anchored()
                        .position(point(px(0.), px(0.)))
                        .position_mode(AnchoredPositionMode::Window)
                        .child(
                            div()
                                .w(viewport.width)
                                .h(viewport.height)
                                .occlude()
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    dismiss_root
                                        .update(cx, |this, cx| this.close_slot_popup(window, cx));
                                })
                                .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                    dismiss_right_root
                                        .update(cx, |this, cx| this.close_slot_popup(window, cx));
                                }),
                        ),
                )
                .child(
                    anchored()
                        .position(point(anchor.right() + rem, anchor.top() - rem * 0.5))
                        .anchor(Anchor::TopLeft)
                        .position_mode(AnchoredPositionMode::Window)
                        .snap_to_window_with_margin(rem * 0.5)
                        .child(panel),
                )
                .into_any_element(),
        )
    }

    fn render_popup_header(&self, slot: &SlotWithRole, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let open_root = root.clone();
        let close_root = root;
        let handle = slot.role.handle.clone();
        let open_role_focus = self
            .crew_surfaces
            .editor
            .popup
            .as_ref()
            .map(|popup| popup.open_role_focus.clone())
            .unwrap_or_else(|| cx.focus_handle());
        div()
            .flex()
            .items_start()
            .gap_3()
            .px_4()
            .pt_4()
            .pb_3()
            .border_b_1()
            .border_color(theme::border())
            .child(RoleAvatar::new(slot.slot.slot_handle.clone(), 40.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_title())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(format!("@{}", slot.slot.slot_handle)),
                            )
                            .children(slot.slot.lead.then(lead_badge)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_ui())
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_color(theme::muted())
                                    .child(slot.role.display_name.clone()),
                            )
                            .child(
                                text_action(
                                    "crew-slot-popup-open-role",
                                    &open_role_focus,
                                    move |window, cx| {
                                        let handle = handle.clone();
                                        open_root.update(cx, |this, cx| {
                                            this.crew_surfaces.editor.popup = None;
                                            this.open_role_detail(handle, window, cx)
                                        });
                                    },
                                )
                                .when(cfg!(test), |link| {
                                    link.debug_selector(|| "CREW_SLOT_OPEN_ROLE".into())
                                })
                                .flex_none()
                                .gap_1()
                                .text_color(theme::text())
                                .hover(|link| link.underline())
                                .child("Open role")
                                .child(
                                    svg()
                                        .flex_none()
                                        .path("arrow-up-right.svg")
                                        .size(rems(12. / 16.))
                                        .text_color(theme::muted()),
                                ),
                            ),
                    ),
            )
            .child(
                IconButton::new("crew-slot-popup-close", "close.svg")
                    .size(IconButtonSize::Sm)
                    .tooltip("Close")
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_slot_popup(window, cx));
                    }),
            )
            .into_any_element()
    }

    fn render_popup_override_rows(
        &self,
        slot: &SlotWithRole,
        form: &SlotOverrideForm,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let runtime = form
            .runtime
            .clone()
            .unwrap_or_else(|| slot.role.runtime.clone());
        let own_runtime = runtime == slot.role.runtime;
        let model_set = trimmed_option(form.model.read(cx).text()).is_some();
        let reset = |id: &'static str,
                     focus: &FocusHandle,
                     action: fn(&mut NativeRoot, &mut Context<NativeRoot>)| {
            let root = root.clone();
            text_action(id, focus, move |_, cx| root.update(cx, action))
                .when(cfg!(test), move |reset| {
                    reset.debug_selector(move || id.to_uppercase().replace('-', "_"))
                })
                .flex_none()
                .gap_1()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .hover(|reset| reset.text_color(theme::text()))
                .child(
                    svg()
                        .flex_none()
                        .path("rotate-ccw.svg")
                        .size(rems(11. / 16.))
                        .text_color(theme::muted()),
                )
                .child("Reset")
        };
        div()
            .flex()
            .flex_col()
            .px_4()
            .py_2()
            .child(
                div()
                    .when(cfg!(test), |row| {
                        row.debug_selector(|| "CREW_SLOT_RUNTIME_EDIT".into())
                    })
                    .child(override_row(
                        "Runtime",
                        form.runtime_select.clone().into_any_element(),
                        format!("role: {}", runtime_display_name(&slot.role.runtime)),
                        form.runtime.is_some().then(|| {
                            reset(
                                "slot-reset-runtime",
                                &form.reset_focus[0],
                                Self::reset_slot_override_runtime,
                            )
                        }),
                        true,
                    )),
            )
            .child(override_row(
                "Model",
                form.model_field.clone().into_any_element(),
                format!("role: {}", role_value(slot.role.model.as_deref())),
                model_set.then(|| {
                    reset(
                        "slot-reset-model",
                        &form.reset_focus[1],
                        Self::reset_slot_override_model,
                    )
                }),
                true,
            ))
            .child(override_row(
                "Effort",
                form.effort_select.clone().into_any_element(),
                format!("role: {}", role_value(slot.role.effort.as_deref())),
                (!form.effort.is_empty()).then(|| {
                    reset(
                        "slot-reset-effort",
                        &form.reset_focus[2],
                        Self::reset_slot_override_effort,
                    )
                }),
                runtime == "codex",
            ))
            .when(runtime == "codex", |rows| {
                rows.child(
                    div()
                        .when(cfg!(test), |row| {
                            row.debug_selector(|| "CREW_SLOT_SPEED_EDIT".into())
                        })
                        .child(override_row(
                            "Speed",
                            form.speed_select.clone().into_any_element(),
                            if own_runtime {
                                format!("role: {}", speed_label(slot.role.codex_speed))
                            } else {
                                "Codex default".into()
                            },
                            form.speed.map(|_| {
                                reset(
                                    "slot-reset-speed",
                                    &form.reset_focus[3],
                                    Self::reset_slot_override_speed,
                                )
                            }),
                            false,
                        )),
                )
                .children(
                    (form
                        .speed
                        .or(own_runtime.then_some(slot.role.codex_speed).flatten())
                        == Some(CodexSpeed::Fast))
                    .then(|| {
                        div()
                            .when(cfg!(test), |note| {
                                note.debug_selector(|| "CREW_SLOT_SPEED_NOTE".into())
                            })
                            .pl_4()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child("Fast uses more credits.")
                    }),
                )
            })
            .children((!own_runtime).then(runtime_note))
            .children(form.error.clone().map(|error| {
                div()
                    .pt_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(error)
            }))
            .into_any_element()
    }

    fn render_popup_view_footer(&self, slot: &SlotWithRole, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let edit_root = root.clone();
        let lead_root = root.clone();
        let remove_root = root;
        let lead_id = slot.slot.id.clone();
        let remove_slot = slot.clone();
        let remove_focus = self
            .crew_surfaces
            .editor
            .popup
            .as_ref()
            .map(|popup| popup.remove_focus.clone())
            .unwrap_or_else(|| cx.focus_handle());
        popup_footer()
            .child(
                div()
                    .when(cfg!(test), |edit| {
                        edit.debug_selector(|| "CREW_SLOT_EDIT_OVERRIDES".into())
                    })
                    .child(
                        Button::new("crew-slot-edit-overrides", "Edit overrides")
                            .icon("sliders-horizontal.svg")
                            .size(ButtonSize::Sm)
                            .on_press(move |window, cx| {
                                edit_root.update(cx, |this, cx| {
                                    this.start_slot_override_edit(window, cx)
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .when(cfg!(test), |lead| {
                        lead.debug_selector(|| "CREW_SLOT_SET_LEAD".into())
                    })
                    .child(
                        Button::new(
                            "crew-slot-set-lead",
                            if slot.slot.lead {
                                "Lead"
                            } else {
                                "Set as lead"
                            },
                        )
                        .icon("star.svg")
                        .size(ButtonSize::Sm)
                        .disabled(slot.slot.lead)
                        .tooltip(if slot.slot.lead {
                            "This slot leads the crew"
                        } else {
                            "Make this slot the crew's lead"
                        })
                        .on_press(move |window, cx| {
                            let slot_id = lead_id.clone();
                            lead_root.update(cx, |this, cx| {
                                this.close_slot_popup(window, cx);
                                this.set_crew_lead(slot_id, cx);
                            });
                        }),
                    ),
            )
            .child(div().flex_1())
            .child(
                text_action("crew-slot-remove", &remove_focus, move |window, cx| {
                    let slot = remove_slot.clone();
                    remove_root.update(cx, |this, cx| {
                        this.close_slot_popup(window, cx);
                        this.crew_surfaces.slot_remove_confirm = Some(SlotRemoveConfirm { slot });
                        cx.notify();
                    });
                })
                .when(cfg!(test), |remove| {
                    remove.debug_selector(|| "CREW_SLOT_REMOVE".into())
                })
                .flex_none()
                .gap(rems(6. / 16.))
                .px_2()
                .py_1()
                .text_size(theme::text_ui())
                .text_color(theme::danger())
                .hover(|remove| remove.bg(theme::with_alpha(theme::danger(), 0.1)))
                .child(
                    svg()
                        .flex_none()
                        .path("trash.svg")
                        .size(rems(12. / 16.))
                        .text_color(theme::danger()),
                )
                .child("Remove"),
            )
            .into_any_element()
    }

    fn render_popup_edit_footer(
        &self,
        form: &SlotOverrideForm,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let cancel_root = root.clone();
        let save_root = root;
        popup_footer()
            .child(
                div()
                    .flex_1()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child("Saves to this slot only."),
            )
            .child(
                Button::new("crew-slot-override-cancel", "Cancel")
                    .size(ButtonSize::Sm)
                    .disabled(form.saving)
                    .on_press(move |_, cx| {
                        cancel_root.update(cx, |this, cx| this.cancel_slot_override_edit(cx));
                    }),
            )
            .child(
                div()
                    .when(cfg!(test), |save| {
                        save.debug_selector(|| "CREW_SLOT_OVERRIDE_SAVE".into())
                    })
                    .child(
                        Button::new(
                            "crew-slot-override-save",
                            if form.saving { "Saving…" } else { "Save" },
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Primary)
                        .disabled(form.saving)
                        .on_press(move |_, cx| {
                            save_root.update(cx, |this, cx| this.save_slot_overrides(cx));
                        }),
                    ),
            )
            .into_any_element()
    }

    fn popup_slot(&self) -> Option<SlotWithRole> {
        let editor = &self.crew_surfaces.editor;
        let popup = editor.popup.as_ref()?;
        editor
            .slots
            .iter()
            .find(|slot| slot.slot.id == popup.slot_id)
            .cloned()
    }

    pub(super) fn start_slot_override_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        let (mut runtimes, agents_checking, _) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        let runtime_override = slot
            .slot
            .runtime_override
            .as_deref()
            .and_then(trimmed_option);
        let runtime = runtime_override
            .clone()
            .unwrap_or_else(|| slot.role.runtime.clone());
        ensure_runtime_present(self.core(cx), &mut runtimes, &slot.role.runtime);
        ensure_runtime_present(self.core(cx), &mut runtimes, &runtime);
        self.request_model_catalog(&runtime, cx);
        let root = cx.entity();
        let runtime_root = root.clone();
        let runtime_options = slot_runtime_options(&runtimes, &slot.role.runtime, &runtime);
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "slot-override-runtime",
                select_cx.focus_handle(),
                runtime_override.clone().unwrap_or_default(),
                runtime_options,
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| {
                        this.set_slot_override_runtime(value, false, cx)
                    });
                }),
                select_cx,
            )
            .width(px(POPUP_FIELD_WIDTH))
            .min_menu_width(px(POPUP_FIELD_WIDTH))
            .placeholder(if agents_checking {
                "Detecting agents…"
            } else {
                "No enabled agents detected"
            })
        });
        let model_value = slot.slot.model_override.clone().unwrap_or_default();
        let model = cx.new(move |input_cx| {
            TextField::new(input_cx.focus_handle(), model_value, "", true)
                .placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| {
            ModelField::new(model.clone(), runtime_models(&runtimes, &runtime), model_cx)
        });
        let effort = slot.slot.effort_override.clone().unwrap_or_default();
        let effort_root = root;
        let effort_select = cx.new(|select_cx| {
            StyledSelect::new(
                "slot-override-effort",
                select_cx.focus_handle(),
                effort.clone(),
                Vec::new(),
                Rc::new(move |value, _, cx| {
                    effort_root.update(cx, |this, cx| {
                        if let Some(form) = this.slot_override_form() {
                            form.effort = value;
                            cx.notify();
                        }
                    });
                }),
                select_cx,
            )
            .width(px(POPUP_FIELD_WIDTH))
            .min_menu_width(px(POPUP_FIELD_WIDTH))
        });
        let speed = slot.slot.codex_speed_override;
        let speed_root = cx.entity();
        let speed_select = cx.new(|select_cx| {
            StyledSelect::new(
                "slot-override-speed",
                select_cx.focus_handle(),
                speed_value(speed),
                vec![
                    SelectOption::new("", "Inherit"),
                    SelectOption::new("standard", "Standard"),
                    SelectOption::new("fast", "Fast"),
                ],
                Rc::new(move |value, _, cx| {
                    speed_root.update(cx, |this, cx| {
                        this.set_slot_override_speed(&value, false, cx);
                    });
                }),
                select_cx,
            )
            .width(px(POPUP_FIELD_WIDTH))
            .min_menu_width(px(POPUP_FIELD_WIDTH))
        });
        let subscriptions = vec![cx.observe(&model, |this, _, cx| {
            this.sync_slot_effort_choices(cx);
        })];
        let focus = runtime_select.read(cx).focus_handle();
        let Some(popup) = self.crew_surfaces.editor.popup.as_mut() else {
            return;
        };
        popup.edit = Some(SlotOverrideForm {
            runtimes,
            runtime: runtime_override,
            runtime_select,
            model,
            model_field,
            effort,
            effort_select,
            speed,
            speed_select,
            reset_focus: [
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
            ],
            saving: false,
            error: None,
            _subscriptions: subscriptions,
        });
        self.sync_slot_model_choices(cx);
        self.sync_slot_effort_choices(cx);
        focus.focus(window, cx);
        cx.notify();
    }

    fn slot_override_form(&mut self) -> Option<&mut SlotOverrideForm> {
        self.crew_surfaces
            .editor
            .popup
            .as_mut()
            .and_then(|popup| popup.edit.as_mut())
    }

    /// Keeps the model's placeholder and suggestions in step with the picked
    /// runtime.
    fn sync_slot_model_choices(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        let Some(form) = self.slot_override_form() else {
            return;
        };
        let runtime = form
            .runtime
            .clone()
            .unwrap_or_else(|| slot.role.runtime.clone());
        let own_runtime = runtime == slot.role.runtime;
        let placeholder = inherited_label(&runtime, own_runtime, slot.role.model.as_deref());
        form.model.update(cx, |input, input_cx| {
            input.set_placeholder(placeholder, input_cx)
        });
        form.model_field.update(cx, |field, field_cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &runtime), field_cx)
        });
    }

    /// Keeps the effort choices in step with the picked runtime and model.
    /// Runs from the model field's observer, so it must not touch the model.
    fn sync_slot_effort_choices(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        let Some(form) = self.slot_override_form() else {
            return;
        };
        let runtime = form
            .runtime
            .clone()
            .unwrap_or_else(|| slot.role.runtime.clone());
        let own_runtime = runtime == slot.role.runtime;
        let mut options = effort_options(
            &form.runtimes,
            &runtime,
            &slot.role,
            true,
            form.model.read(cx).text(),
        );
        let inherited = inherited_label(&runtime, own_runtime, slot.role.effort.as_deref());
        match options.iter_mut().find(|option| option.value.is_empty()) {
            Some(blank) => blank.label = inherited.into(),
            None => options.insert(0, SelectOption::new("", inherited)),
        }
        // A stored effort the catalog does not list stays chosen rather than
        // being dropped on the next save.
        if !options.iter().any(|option| option.value == form.effort) {
            options.push(SelectOption::new(form.effort.clone(), form.effort.clone()));
        }
        let effort = form.effort.clone();
        form.effort_select.update(cx, |select, select_cx| {
            select.set_disabled(options.len() <= 1, select_cx);
            select.set_options(options, select_cx);
            select.set_value(effort, select_cx);
        });
        cx.notify();
    }

    pub(super) fn select_slot_override_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        self.set_slot_override_runtime(value, true, cx);
    }

    fn set_slot_override_runtime(
        &mut self,
        value: String,
        sync_select: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        let Some(form) = self.slot_override_form() else {
            return;
        };
        let next = trimmed_option(&value);
        if form.saving || next == form.runtime {
            return;
        }
        let effective =
            |pin: &Option<String>| pin.clone().unwrap_or_else(|| slot.role.runtime.clone());
        let runtime = effective(&next);
        // A new runtime starts from its own model and effort; pinning or
        // unpinning the role's own runtime keeps them.
        if runtime != effective(&form.runtime) {
            form.effort.clear();
            form.speed = None;
            form.speed_select
                .update(cx, |select, select_cx| select.set_value("", select_cx));
            form.model
                .update(cx, |input, input_cx| input.reset("", input_cx));
        }
        form.runtime = next;
        let value = form.runtime.clone().unwrap_or_default();
        if sync_select {
            form.runtime_select
                .update(cx, |select, select_cx| select.set_value(value, select_cx));
        }
        self.request_model_catalog(&runtime, cx);
        self.sync_slot_model_choices(cx);
        self.sync_slot_effort_choices(cx);
    }

    fn reset_slot_override_runtime(&mut self, cx: &mut Context<Self>) {
        self.select_slot_override_runtime(String::new(), cx);
    }

    fn reset_slot_override_model(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.slot_override_form() {
            form.model
                .update(cx, |input, input_cx| input.reset("", input_cx));
        }
        cx.notify();
    }

    fn reset_slot_override_effort(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.slot_override_form() {
            form.effort.clear();
        }
        self.sync_slot_effort_choices(cx);
    }

    fn reset_slot_override_speed(&mut self, cx: &mut Context<Self>) {
        self.select_slot_override_speed("", cx);
    }

    pub(super) fn select_slot_override_speed(&mut self, value: &str, cx: &mut Context<Self>) {
        self.set_slot_override_speed(value, true, cx);
    }

    fn set_slot_override_speed(&mut self, value: &str, sync_select: bool, cx: &mut Context<Self>) {
        if let Some(form) = self.slot_override_form() {
            form.speed = match value {
                "standard" => Some(CodexSpeed::Standard),
                "fast" => Some(CodexSpeed::Fast),
                _ => None,
            };
            if sync_select {
                form.speed_select
                    .update(cx, |select, select_cx| select.set_value(value, select_cx));
            }
        }
        cx.notify();
    }

    pub(crate) fn refresh_slot_override_runtimes(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        if self.slot_override_form().is_none() {
            return;
        }
        let (mut runtimes, _, error) =
            crate::surfaces::start_chat::load_selectable_runtimes(self.core(cx), self.settings(cx));
        if error.is_some() {
            return;
        }
        let core = self.core(cx).clone();
        let Some(form) = self.slot_override_form() else {
            return;
        };
        let runtime = form
            .runtime
            .clone()
            .unwrap_or_else(|| slot.role.runtime.clone());
        ensure_runtime_present(&core, &mut runtimes, &slot.role.runtime);
        ensure_runtime_present(&core, &mut runtimes, &runtime);
        let options = slot_runtime_options(&runtimes, &slot.role.runtime, &runtime);
        form.runtimes = runtimes;
        form.runtime_select.update(cx, |select, select_cx| {
            select.set_options(options, select_cx)
        });
        self.sync_slot_model_choices(cx);
        self.sync_slot_effort_choices(cx);
    }

    fn cancel_slot_override_edit(&mut self, cx: &mut Context<Self>) {
        let Some(popup) = self.crew_surfaces.editor.popup.as_mut() else {
            return;
        };
        if popup.edit.as_ref().is_some_and(|form| form.saving) {
            return;
        }
        popup.edit = None;
        cx.notify();
    }

    /// Writes the overrides to this slot only; the role never changes
    /// here. A runtime left as it was is not sent, so a stored pin the
    /// catalog no longer knows survives an edit of the model or effort.
    pub(super) fn save_slot_overrides(&mut self, cx: &mut Context<Self>) {
        let Some(slot) = self.popup_slot() else {
            return;
        };
        let Some(form) = self.slot_override_form() else {
            return;
        };
        if form.saving {
            return;
        }
        let stored = slot
            .slot
            .runtime_override
            .as_deref()
            .and_then(trimmed_option);
        let runtime_override = if form.runtime == stored {
            None
        } else {
            match form.runtime.as_deref().map(Runtime::parse) {
                None => Some(None),
                Some(Some(runtime)) => Some(Some(runtime)),
                Some(None) => {
                    form.error = Some("Pick a runtime Runner knows.".into());
                    cx.notify();
                    return;
                }
            }
        };
        let input = UpdateSlotInput {
            slot_handle: None,
            runtime_override,
            model_override: Some(trimmed_option(form.model.read(cx).text())),
            effort_override: Some(trimmed_option(&form.effort)),
            codex_speed_override: Some(form.speed),
        };
        form.saving = true;
        form.error = None;
        let slot_id = slot.slot.id.clone();
        let crew_id = slot.slot.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::slot::slot_update(&core, &slot_id, input)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if let Some(popup) = this.crew_surfaces.editor.popup.as_mut() {
                            popup.edit = None;
                        }
                        if matches!(&this.route, AppRoute::CrewEditor(active) if active == &crew_id)
                        {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.load_crew_page(cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.slot_override_form() {
                            form.saving = false;
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
}

fn role_value(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned()
}

fn popup_footer() -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .border_t_1()
        .border_color(theme::border())
}

fn runtime_note() -> gpui::Div {
    div()
        .pt_2()
        .text_size(theme::text_meta())
        .line_height(rems(1.))
        .text_color(theme::faint())
        .child(RUNTIME_NOTE)
}

/// A setup row while editing: the label, the control, and under it the
/// role's value with Reset while the slot overrides it.
fn override_row(
    label: &'static str,
    control: AnyElement,
    role_hint: String,
    reset: Option<gpui::Stateful<gpui::Div>>,
    divider: bool,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .py_2()
        .when(divider, |row| {
            row.border_b_1().border_color(theme::border())
        })
        .child(
            div()
                .w(rems(POPUP_LABEL_WIDTH / 16.))
                .flex_none()
                .pt(rems(7. / 16.))
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(label),
        )
        .child(
            div()
                .w(rems(POPUP_FIELD_WIDTH / 16.))
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .child(control)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::faint())
                                .child(role_hint),
                        )
                        .children(reset),
                ),
        )
        .into_any_element()
}

/// The setup rows at rest: each value beside the role's.
fn popup_view_rows(slot: &SlotWithRole) -> AnyElement {
    let setup = slot_setup(slot);
    let own_runtime = setup.own_runtime;
    let icon = ChatIcon::for_runtime(&setup.runtime);
    let row = |label: &'static str, value: AnyElement, role_hint: String, divider: bool| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_2()
            .when(divider, |row| {
                row.border_b_1().border_color(theme::border())
            })
            .child(
                div()
                    .w(rems(POPUP_LABEL_WIDTH / 16.))
                    .flex_none()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(label),
            )
            .child(div().min_w(px(0.)).flex_1().child(value))
            .child(
                div()
                    .flex_none()
                    .max_w(rems(120. / 16.))
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_caption())
                    .text_color(theme::faint())
                    .child(role_hint),
            )
    };
    let value = |text: String, overridden: bool| {
        div()
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .text_size(theme::text_body())
            .text_color(theme::text())
            .child(div().min_w(px(0.)).truncate().child(text))
            .children(overridden.then(override_dot))
            .into_any_element()
    };
    let runtime_value = div()
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .text_size(theme::text_body())
        .text_color(theme::text())
        .child(icon.render(rems(13. / 16.), icon.color(theme::muted(), true), true))
        .child(
            div()
                .min_w(px(0.))
                .truncate()
                .child(runtime_display_name(&setup.runtime)),
        )
        .children(setup.runtime_overridden.then(override_dot))
        .into_any_element();
    div()
        .when(cfg!(test), |rows| {
            rows.debug_selector(|| "CREW_SLOT_POPUP_SETUP".into())
        })
        .flex()
        .flex_col()
        .px_4()
        .py_1()
        .child(row(
            "Runtime",
            runtime_value,
            format!("role: {}", runtime_display_name(&slot.role.runtime)),
            true,
        ))
        .child(row(
            "Model",
            value(
                setup.model.clone().unwrap_or_else(|| {
                    inherited_label(&setup.runtime, own_runtime, slot.role.model.as_deref())
                }),
                setup.model_overridden,
            ),
            format!("role: {}", role_value(slot.role.model.as_deref())),
            true,
        ))
        .child(row(
            "Effort",
            value(
                setup.effort.clone().unwrap_or_else(|| {
                    inherited_label(&setup.runtime, own_runtime, slot.role.effort.as_deref())
                }),
                setup.effort_overridden,
            ),
            format!("role: {}", role_value(slot.role.effort.as_deref())),
            setup.runtime == "codex",
        ))
        .when(setup.runtime == "codex", |rows| {
            rows.child(
                div()
                    .when(cfg!(test), |row| {
                        row.debug_selector(|| "CREW_SLOT_SPEED_VIEW".into())
                    })
                    .child(row(
                        "Speed",
                        value(speed_label(setup.speed).into(), setup.speed_overridden),
                        if own_runtime {
                            format!("role: {}", speed_label(slot.role.codex_speed))
                        } else {
                            "Codex default".into()
                        },
                        false,
                    )),
            )
            .children((setup.speed == Some(CodexSpeed::Fast)).then(|| {
                div()
                    .when(cfg!(test), |note| {
                        note.debug_selector(|| "CREW_SLOT_SPEED_VIEW_NOTE".into())
                    })
                    .pl_4()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child("Fast uses more credits.")
            }))
        })
        .children((!own_runtime).then(runtime_note))
        .into_any_element()
}

/// The command the slot runs and the start of its role's prompt.
fn popup_command_and_prompt(slot: &SlotWithRole) -> AnyElement {
    let prompt = slot
        .role
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty());
    let label = |text: String| {
        div()
            .text_size(theme::text_meta())
            .text_color(theme::faint())
            .child(text)
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .px_4()
        .pt_2()
        .pb_4()
        .child(label("Command".into()))
        .child(
            div()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .px_3()
                .py(rems(7. / 16.))
                .truncate()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_ui())
                .text_color(theme::text())
                .child(format!("$ {}", slot_command_summary(slot))),
        )
        .child(
            div()
                .mt_1()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(label(format!("Prompt · from @{}", slot.role.handle)))
                .children(prompt.map(|prompt| {
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child(plural(prompt.lines().count() as i64, "line", "lines"))
                })),
        )
        .child(match prompt {
            None => div()
                .text_size(theme::text_ui())
                .italic()
                .text_color(theme::faint())
                .child("The role has no system prompt.")
                .into_any_element(),
            Some(prompt) => div()
                .relative()
                .max_h(rems(57. / 16.))
                .overflow_hidden()
                .text_size(theme::text_ui())
                .line_height(rems(19. / 16.))
                .text_color(theme::muted())
                .child(SharedString::from(
                    prompt.split_whitespace().collect::<Vec<_>>().join(" "),
                ))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(rems(24. / 16.))
                        .bg(linear_gradient(
                            180.,
                            linear_color_stop(theme::with_alpha(theme::panel(), 0.), 0.),
                            linear_color_stop(theme::panel(), 1.),
                        )),
                )
                .into_any_element(),
        })
        .into_any_element()
}

use super::logic::move_item;
use super::logic::slot_command_summary;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, Context, CursorStyle, DragMoveEvent, FontWeight, SharedString,
    Window,
};
use runner_app::ui::{
    ConfirmDialog, ContextMenu, IconButton, IconButtonSize, MenuItem as UiMenuItem, RuntimeBadge,
    Tooltip,
};
use runner_backend::model::SlotWithRunner;

use super::*;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn render_slot_list(
        &mut self,
        slots: Vec<SlotWithRunner>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if slots.is_empty() {
            return div()
                .rounded_lg()
                .border_1()
                .border_dashed()
                .border_color(theme::border_strong())
                .bg(theme::with_alpha(theme::panel(), 0.4))
                .px_5()
                .py_8()
                .text_center()
                .child(
                    div()
                        .text_size(theme::text_title())
                        .text_color(theme::text())
                        .child("No slots yet."),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child("Use + Add slot above — the first slot auto-assigns as LEAD."),
                )
                .into_any_element();
        }
        let total = slots.len();
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .children(
                slots
                    .into_iter()
                    .enumerate()
                    .map(|(index, slot)| self.render_slot_row(slot, index, total, cx)),
            )
            .into_any_element()
    }

    fn render_slot_row(
        &self,
        slot: SlotWithRunner,
        index: usize,
        total: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let effective_runtime = slot
            .slot
            .runtime_override
            .as_deref()
            .unwrap_or(&slot.runner.runtime)
            .to_owned();
        let runtime_overridden =
            slot.slot.runtime_override.is_some() && effective_runtime != slot.runner.runtime;
        let summary = slot_command_summary(&slot);
        let draggable = total > 1 && !self.crew_surfaces.editor.reordering;
        let active_drop = self.crew_surfaces.editor.drop_target == Some(index)
            && self.crew_surfaces.editor.dragged_slot_id.as_deref() != Some(slot.slot.id.as_str());
        let drag_handle = div()
            .flex_none()
            .text_size(theme::text_title())
            .text_color(theme::faint())
            .opacity(if draggable { 1. } else { 0.4 })
            .cursor(if draggable {
                CursorStyle::OpenHand
            } else {
                CursorStyle::Arrow
            })
            .child("⋮⋮");
        let drag_handle = if draggable {
            Tooltip::new(
                SharedString::from(format!("crew-slot-drag-tooltip-{}", slot.slot.id)),
                "Drag to reorder",
                drag_handle,
            )
            .into_any_element()
        } else {
            drag_handle.into_any_element()
        };
        let menu_slot = slot.clone();
        let menu_root = cx.entity();
        let mut row = div()
            .id(SharedString::from(format!("crew-slot-{}", slot.slot.id)))
            .group("slot-row")
            .w_full()
            .min_w(px(0.))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_4()
            .rounded_lg()
            .border_1()
            .border_color(if active_drop {
                theme::with_alpha(theme::accent(), 0.5)
            } else {
                theme::border()
            })
            .bg(if active_drop {
                theme::with_alpha(theme::accent(), 0.05)
            } else {
                theme::panel()
            })
            .p_4()
            .hover(|row| row.border_color(theme::border_strong()))
            .child(drag_handle)
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("@{}", slot.slot.slot_handle)),
                            )
                            .children(slot.slot.lead.then(|| {
                                div()
                                    .rounded_sm()
                                    .bg(theme::with_alpha(theme::accent(), 0.1))
                                    .px(rems(6. / 16.))
                                    .py(rems(2. / 16.))
                                    .text_size(theme::text_caption())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::accent())
                                    .child("LEAD")
                            }))
                            .child(Tooltip::new(
                                SharedString::from(format!(
                                    "slot-runtime-tooltip-{}",
                                    slot.slot.id
                                )),
                                if runtime_overridden {
                                    format!(
                                        "Runtime override — runner default is {}",
                                        slot.runner.runtime
                                    )
                                } else {
                                    "Runtime (runner default)".to_owned()
                                },
                                RuntimeBadge::new(effective_runtime).overridden(runtime_overridden),
                            ))
                            .child(
                                div()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format!("from @{}", slot.runner.handle)),
                            ),
                    )
                    .children(slot.runner.system_prompt.clone().map(|prompt| {
                        let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .mt_1()
                            .truncate()
                            .text_size(theme::text_ui())
                            .line_height(rems(1.))
                            .text_color(theme::muted())
                            .child(prompt)
                    }))
                    .children((!summary.is_empty()).then(|| {
                        div()
                            .mt_1()
                            .truncate()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(format!("$ {summary}"))
                    })),
            )
            .child(
                div().flex_none().child(
                    IconButton::new(
                        SharedString::from(format!("slot-actions-{}", slot.slot.id)),
                        "more-horizontal.svg",
                    )
                    .size(IconButtonSize::Md)
                    .tooltip("Slot actions")
                    .on_press(move |window, cx| {
                        let position = window.mouse_position();
                        let slot = menu_slot.clone();
                        menu_root.update(cx, |this, cx| {
                            this.open_slot_menu(slot, position, window, cx)
                        });
                    }),
                ),
            );
        if draggable {
            let drag = SlotDrag {
                slot_id: slot.slot.id.clone(),
                label: format!("@{}", slot.slot.slot_handle),
            };
            let drag_root = cx.entity();
            row = row
                .cursor_move()
                .on_drag(drag, move |drag: &SlotDrag, _, _, cx| {
                    drag_root.update(cx, |this, cx| {
                        this.crew_surfaces.editor.dragged_slot_id = Some(drag.slot_id.clone());
                        cx.notify();
                    });
                    cx.new(|_| drag.clone())
                })
                .on_drag_move::<SlotDrag>(cx.listener(
                    move |this, event: &DragMoveEvent<SlotDrag>, _, cx| {
                        if event.bounds.contains(&event.event.position) {
                            this.crew_surfaces.editor.drop_target = Some(index);
                            cx.notify();
                        }
                    },
                ))
                .on_drop(cx.listener(move |this, drag: &SlotDrag, _, cx| {
                    this.commit_slot_reorder(&drag.slot_id, index, cx);
                }));
        }
        row.into_any_element()
    }

    fn open_slot_menu(
        &mut self,
        slot: SlotWithRunner,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            SlotMenuAction::SetLead(slot.slot.id.clone()),
            SlotMenuAction::Edit(slot.clone()),
            SlotMenuAction::Remove(slot.clone()),
        ];
        let items = vec![
            UiMenuItem::new(if slot.slot.lead {
                "Current lead"
            } else {
                "Set as lead"
            })
            .icon("star.svg")
            .disabled(slot.slot.lead),
            UiMenuItem::new("Edit runner").icon("square-pen.svg"),
            UiMenuItem::new("Remove from crew")
                .icon("trash.svg")
                .separator_before(true)
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "slot-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_slot_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.crew_surfaces.context_menu = None;
                        cx.notify();
                    });
                }),
            )
            .width(px(208.))
        });
        let focus = menu.read(cx).focus_handle();
        self.crew_surfaces.context_menu = Some(menu);
        focus.focus(window);
        cx.notify();
    }

    fn handle_slot_menu_action(
        &mut self,
        action: SlotMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            SlotMenuAction::SetLead(slot_id) => self.set_crew_lead(slot_id, cx),
            SlotMenuAction::Edit(slot) => {
                self.open_runner_edit(slot.runner.clone(), Some(slot), window, cx)
            }
            SlotMenuAction::Remove(slot) => {
                self.crew_surfaces.slot_remove_confirm = Some(SlotRemoveConfirm { slot });
                cx.notify();
            }
        }
    }

    fn set_crew_lead(&mut self, slot_id: String, cx: &mut Context<Self>) {
        let crew_id = self.crew_surfaces.editor.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::slot::slot_set_lead(&core, &slot_id)
                .map_err(|error| error.to_string());
            (crew_id, result)
        });
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if !matches!(
                    &this.route,
                    AppRoute::CrewEditor(active) if active == &crew_id
                ) {
                    return;
                }
                match result {
                    Ok(_) => this.load_crew_editor(crew_id, cx),
                    Err(error) => this.crew_surfaces.editor.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn clear_crew_slot_drag(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        if editor.dragged_slot_id.is_none() && editor.drop_target.is_none() {
            return;
        }
        editor.dragged_slot_id = None;
        editor.drop_target = None;
        cx.notify();
    }

    fn commit_slot_reorder(&mut self, slot_id: &str, to: usize, cx: &mut Context<Self>) {
        if self.crew_surfaces.editor.reordering {
            return;
        }
        let Some(from) = self
            .crew_surfaces
            .editor
            .slots
            .iter()
            .position(|slot| slot.slot.id == slot_id)
        else {
            return;
        };
        self.crew_surfaces.editor.dragged_slot_id = None;
        self.crew_surfaces.editor.drop_target = None;
        if from == to {
            cx.notify();
            return;
        }
        let reordered = move_item(&self.crew_surfaces.editor.slots, from, to);
        let ordered_ids = reordered
            .iter()
            .map(|slot| slot.slot.id.clone())
            .collect::<Vec<_>>();
        let crew_id = self.crew_surfaces.editor.crew_id.clone();
        self.crew_surfaces.editor.slots = reordered;
        self.crew_surfaces.editor.reordering = true;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested_crew_id = crew_id.clone();
            runner_backend::ops::slot::slot_reorder(&core, &crew_id, ordered_ids)
                .map(|slots| (crew_id, slots))
                .map_err(|error| (requested_crew_id, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((crew_id, slots)) => {
                        if this.crew_surfaces.editor.crew_id != crew_id {
                            return;
                        }
                        this.crew_surfaces.editor.reordering = false;
                        this.crew_surfaces.editor.slots = slots;
                        this.load_crew_page(cx);
                    }
                    Err((crew_id, error)) => {
                        if this.crew_surfaces.editor.crew_id != crew_id {
                            return;
                        }
                        this.crew_surfaces.editor.reordering = false;
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.crew_surfaces.editor.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_slot_remove_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .crew_surfaces
            .slot_remove_confirm
            .as_ref()
            .expect("slot remove confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        let body = if confirm.slot.slot.lead {
            "As the LEAD, leadership will pass to the next slot by position."
        } else {
            ""
        };
        ConfirmDialog::new(
            format!(
                "Remove slot @{} from this crew?",
                confirm.slot.slot.slot_handle,
            ),
            body,
            "Remove from crew",
            "Removing…",
            self.crew_surfaces.slot_remove_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_slot_remove(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.crew_surfaces.slot_remove_busy {
                        this.crew_surfaces.slot_remove_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }
}

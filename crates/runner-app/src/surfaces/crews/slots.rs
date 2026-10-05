use super::logic::move_item;
use super::logic::slot_setup;
use super::logic::text_action;
use super::logic::SlotSetup;
use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, svg, AnyElement, Bounds, Context, DragMoveEvent, FontWeight,
    KeyDownEvent, Pixels, SharedString,
};
use runner_app::ui::{ConfirmDialog, RoleAvatar};
use runner_core::protocol::model::SlotWithRole;

use super::*;
use crate::chat_icon::ChatIcon;
use crate::surfaces::profile_page::{column_text, dot_note, override_dot, section, section_label};
use crate::surfaces::roles::logic::runtime_display_name;
use crate::surfaces::*;
use crate::*;

/// A slot row's inner padding, avatar and gap.
const SLOT_PADDING: f32 = 6.;
const SLOT_AVATAR: f32 = 26.;
/// A slot row's text beside its padding, avatar and gap.
fn slot_text_width(column: f32) -> f32 {
    column - 2. * SLOT_PADDING - SLOT_AVATAR - 10.
}

impl NativeRoot {
    pub(super) fn render_slot_section(
        &mut self,
        slots: Vec<SlotWithRole>,
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let creating = self.route == AppRoute::NewCrew
            || (self.route == AppRoute::Settings
                && self.settings_return_route == AppRoute::NewCrew);
        let add_root = cx.entity();
        let any_override = slots.iter().any(|slot| slot_setup(slot).overrides_any());
        section()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(section_label(format!("Slots · {}", slots.len())))
                    .child(if creating {
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::faint())
                            .child("+ Add slot")
                            .into_any_element()
                    } else {
                        text_action("add-crew-slot", "+ Add slot", move |window, cx| {
                            add_root.update(cx, |this, cx| this.open_add_slot(window, cx));
                        })
                    }),
            )
            .child(if creating {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child("Add roles as slots once the crew exists.")
                    .child(
                        div()
                            .text_color(theme::faint())
                            .child("The first slot leads."),
                    )
                    .into_any_element()
            } else {
                self.render_slot_list(slots, slot_text_width(column), cx)
            })
            .children(any_override.then(|| {
                dot_note("overridden for this slot").when(cfg!(test), |legend| {
                    legend.debug_selector(|| "CREW_SLOT_LEGEND".into())
                })
            }))
            .into_any_element()
    }

    fn render_slot_list(
        &mut self,
        slots: Vec<SlotWithRole>,
        text_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if slots.is_empty() {
            return div()
                .rounded_lg()
                .border_1()
                .border_dashed()
                .border_color(theme::border_strong())
                .px_4()
                .py_5()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child("No slots yet. The first slot you add leads the crew.")
                .into_any_element();
        }
        let total = slots.len();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .children(
                slots
                    .into_iter()
                    .enumerate()
                    .map(|(index, slot)| self.render_slot_row(slot, index, total, text_width, cx)),
            )
            .into_any_element()
    }

    fn render_slot_row(
        &self,
        slot: SlotWithRole,
        index: usize,
        total: usize,
        text_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let setup = slot_setup(&slot);
        let slot_id = slot.slot.id.clone();
        let popup = editor
            .popup
            .as_ref()
            .filter(|popup| popup.slot_id == slot_id);
        let selected = popup.is_some();
        // The open popup follows its row's bounds; any other row measures
        // itself for the click that would open one.
        let anchor = popup
            .map(|popup| popup.anchor.clone())
            .unwrap_or_else(|| Rc::new(Cell::new(Bounds::default())));
        let draggable = total > 1 && !editor.reordering;
        let active_drop = editor.drop_target == Some(index)
            && editor.dragged_slot_id.as_deref() != Some(slot_id.as_str());
        let group = SharedString::from(format!("crew-slot-{slot_id}"));
        let root = cx.entity();
        let click_root = root.clone();
        let key_root = root;
        let click_anchor = anchor.clone();
        let key_anchor = anchor.clone();
        let click_id = slot_id.clone();
        let key_id = slot_id.clone();
        let mut row = div()
            .id(group.clone())
            .group(group.clone())
            .when(cfg!(test) && index == 0, |row| {
                row.debug_selector(|| "CREW_SLOT_ROW".into())
            })
            .relative()
            .w_full()
            .flex()
            .items_start()
            .gap(rems(10. / 16.))
            .rounded(rems(6. / 16.))
            .p(rems(SLOT_PADDING / 16.))
            .tab_index(0)
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::raised()))
            .when(active_drop, |row| {
                row.bg(theme::with_alpha(theme::accent(), 0.08))
            })
            .when(!selected && !active_drop, |row| {
                row.hover(|row| row.bg(theme::with_alpha(theme::raised(), 0.5)))
            })
            .focus_visible(|row| row.bg(theme::raised()))
            .on_click(move |_, window, cx| {
                let anchor = click_anchor.get();
                let slot_id = click_id.clone();
                click_root.update(cx, |this, cx| {
                    this.toggle_slot_popup(slot_id, anchor, window, cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let anchor = key_anchor.get();
                    let slot_id = key_id.clone();
                    key_root.update(cx, |this, cx| {
                        this.toggle_slot_popup(slot_id, anchor, window, cx)
                    });
                }
            })
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .children(draggable.then(|| {
                div()
                    .absolute()
                    .left(rems(-20. / 16.))
                    .top(rems(12. / 16.))
                    .child(
                        svg()
                            .path("grip-vertical.svg")
                            .size(rems(14. / 16.))
                            .text_color(gpui::transparent_black())
                            .group_hover(group.clone(), |grip| grip.text_color(theme::faint())),
                    )
            }))
            .child(RoleAvatar::new(slot.slot.slot_handle.clone(), SLOT_AVATAR))
            .child(slot_text(&slot, &setup, text_width));
        if draggable {
            let drag = SlotDrag {
                slot_id: slot_id.clone(),
                label: format!("@{}", slot.slot.slot_handle),
            };
            let drag_root = cx.entity();
            row = row
                .on_drag(drag, move |drag: &SlotDrag, _, _, cx| {
                    drag_root.update(cx, |this, cx| {
                        let editor = &mut this.crew_surfaces.editor;
                        editor.dragged_slot_id = Some(drag.slot_id.clone());
                        editor.popup = None;
                        cx.notify();
                    });
                    cx.new(|_| drag.clone())
                })
                .on_drag_move::<SlotDrag>(cx.listener(
                    move |this, event: &DragMoveEvent<SlotDrag>, _, cx| {
                        if event.bounds.contains(&event.event.position)
                            && this.crew_surfaces.editor.drop_target != Some(index)
                        {
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

    pub(super) fn toggle_slot_popup(
        &mut self,
        slot_id: String,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = &mut self.crew_surfaces.editor;
        if editor
            .popup
            .as_ref()
            .is_some_and(|popup| popup.slot_id == slot_id)
        {
            self.close_slot_popup(window, cx);
            return;
        }
        let focus = cx.focus_handle();
        editor.popup = Some(SlotPopup {
            slot_id,
            anchor: Rc::new(Cell::new(anchor)),
            focus: focus.clone(),
            open_role_focus: cx.focus_handle(),
            remove_focus: cx.focus_handle(),
            edit: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_slot_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        if editor
            .popup
            .as_ref()
            .and_then(|popup| popup.edit.as_ref())
            .is_some_and(|form| form.saving)
        {
            return;
        }
        if editor.popup.take().is_some() {
            window.focus(&self.root_focus, cx);
            cx.notify();
        }
    }

    pub(super) fn set_crew_lead(&mut self, slot_id: String, cx: &mut Context<Self>) {
        let crew_id = self.crew_surfaces.editor.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = core
                .slot_set_lead(&slot_id)
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
                    Ok(_) => {
                        this.load_crew_editor(crew_id, cx);
                        this.load_crew_page(cx);
                    }
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

    pub(super) fn commit_slot_reorder(&mut self, slot_id: &str, to: usize, cx: &mut Context<Self>) {
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
            core.slot_reorder(&crew_id, ordered_ids)
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

/// The LEAD badge beside a slot handle.
pub(super) fn lead_badge() -> AnyElement {
    div()
        .flex_none()
        .rounded(rems(3. / 16.))
        .border_1()
        .border_color(theme::border_strong())
        .px(rems(4. / 16.))
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_size(theme::text_micro())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child("LEAD")
        .into_any_element()
}

/// A slot's effective runtime with its mark, then its model, effort, and Speed, each
/// with a dot when the slot overrides it.
pub(super) fn slot_setup_line(setup: &SlotSetup) -> gpui::Div {
    let icon = ChatIcon::for_runtime(&setup.runtime);
    let separator = || div().flex_none().text_color(theme::faint()).child("·");
    let value = |text: String, overridden: bool| {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .font_family(theme::UI_MONOSPACE_FONT)
            .text_color(theme::muted())
            .child(text)
            .children(overridden.then(override_dot))
    };
    let mut line = div()
        .flex()
        .items_center()
        .gap(rems(5. / 16.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(theme::text_ui())
        .child(icon.render(rems(12. / 16.), icon.color(theme::muted(), true), true))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .text_color(theme::muted())
                .child(runtime_display_name(&setup.runtime))
                .children(setup.runtime_overridden.then(override_dot)),
        );
    if setup.model.is_none() && setup.effort.is_none() && setup.speed.is_none() {
        return line.child(separator()).child(
            div()
                .flex_none()
                .text_color(theme::faint())
                .child("defaults"),
        );
    }
    for (text, overridden) in [
        (setup.model.clone(), setup.model_overridden),
        (setup.effort.clone(), setup.effort_overridden),
    ] {
        if let Some(text) = text {
            line = line.child(separator()).child(value(text, overridden));
        }
    }
    if let Some(speed) = setup.speed {
        let label = match speed {
            runner_core::protocol::model::CodexSpeed::Standard => "Standard",
            runner_core::protocol::model::CodexSpeed::Fast => "Fast",
        };
        line = line.child(separator()).child(
            value(label.into(), setup.speed_overridden).when(cfg!(test), |item| {
                item.debug_selector(|| "CREW_SLOT_SPEED_ROW".into())
            }),
        );
    }
    line
}

fn slot_text(slot: &SlotWithRole, setup: &SlotSetup, width: f32) -> AnyElement {
    div()
        .w(rems(width / 16.))
        .flex_none()
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
                        .text_size(theme::text_body())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(format!("@{}", slot.slot.slot_handle)),
                )
                .children(slot.slot.lead.then(lead_badge)),
        )
        .child(slot_setup_line(setup))
        .child(
            column_text(format!("role @{}", slot.role.handle), width)
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::faint()),
        )
        .into_any_element()
}

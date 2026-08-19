use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, relative, rems, AnyElement, App, BoxShadow, CursorStyle, Entity, FocusHandle,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, RenderOnce, ScrollHandle, SharedString,
    Window,
};

use crate::theme;
use crate::ui::button::PressHandler;
use crate::ui::scrollbar::Scrollbar;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum OverlayWidth {
    Sm,
    #[default]
    Md,
    Lg,
    Custom(f32),
}

impl OverlayWidth {
    fn pixels(self) -> f32 {
        match self {
            Self::Sm => 448.,
            Self::Md => 512.,
            Self::Lg => 576.,
            Self::Custom(width) => width,
        }
    }
}

#[derive(IntoElement)]
pub struct Modal {
    title: AnyElement,
    body: AnyElement,
    footer: Option<AnyElement>,
    width: OverlayWidth,
    busy: bool,
    focus_order: Vec<FocusHandle>,
    scroll: Option<(ScrollHandle, Entity<Scrollbar>)>,
    on_close: PressHandler,
}

impl Modal {
    pub fn new(title: impl IntoElement, body: impl IntoElement, on_close: PressHandler) -> Self {
        Self {
            title: title.into_any_element(),
            body: body.into_any_element(),
            footer: None,
            width: OverlayWidth::Md,
            busy: false,
            focus_order: Vec::new(),
            scroll: None,
            on_close,
        }
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub fn width(mut self, width: OverlayWidth) -> Self {
        self.width = width;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn focus_order(mut self, focus_order: Vec<FocusHandle>) -> Self {
        self.focus_order = focus_order;
        self
    }

    pub fn scrollbar(mut self, handle: ScrollHandle, scrollbar: Entity<Scrollbar>) -> Self {
        self.scroll = Some((handle, scrollbar));
        self
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let close_backdrop = Rc::clone(&self.on_close);
        let close_key = Rc::clone(&self.on_close);
        let focus_order = self.focus_order;
        let busy = self.busy;
        let scroll_handle = self.scroll.as_ref().map(|(handle, _)| handle.clone());
        let scrollbar = self.scroll.map(|(_, scrollbar)| scrollbar);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x00000099))
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !busy {
                    close_backdrop(window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !busy => {
                        cx.stop_propagation();
                        close_key(window, cx);
                    }
                    "tab" if !focus_order.is_empty() => {
                        cx.stop_propagation();
                        let current = focus_order
                            .iter()
                            .position(|handle| handle.is_focused(window));
                        let index = focus_target_index(
                            focus_order.len(),
                            current,
                            event.keystroke.modifiers.shift,
                        );
                        focus_order[index].focus(window);
                    }
                    _ => {}
                }
            })
            .child(
                div()
                    .w_full()
                    .max_w(rems(self.width.pixels() / 16.))
                    .max_h(relative(0.85))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(rems(8. / 16.))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::panel())
                    .shadow_2xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex_none()
                            .px(rems(24. / 16.))
                            .py(rems(1.))
                            .border_b_1()
                            .border_color(theme::border())
                            .text_size(rems(14. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .id("modal-scroll-content")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .when_some(scroll_handle, |body, handle| {
                                        body.scrollbar_width(px(0.)).track_scroll(&handle)
                                    })
                                    .px(rems(24. / 16.))
                                    .py(rems(20. / 16.))
                                    .child(self.body),
                            )
                            .children(scrollbar),
                    )
                    .children(self.footer.map(|footer| {
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme::border())
                            .bg(theme::with_alpha(theme::bg(), 0.4))
                            .px(rems(24. / 16.))
                            .py(rems(1.))
                            .child(footer)
                    })),
            )
    }
}

#[derive(IntoElement)]
pub struct Drawer {
    title: AnyElement,
    body: AnyElement,
    footer: Option<AnyElement>,
    width: OverlayWidth,
    busy: bool,
    focus_order: Vec<FocusHandle>,
    scroll: Option<(ScrollHandle, Entity<Scrollbar>)>,
    on_close: PressHandler,
}

impl Drawer {
    pub fn new(title: impl IntoElement, body: impl IntoElement, on_close: PressHandler) -> Self {
        Self {
            title: title.into_any_element(),
            body: body.into_any_element(),
            footer: None,
            width: OverlayWidth::Sm,
            busy: false,
            focus_order: Vec::new(),
            scroll: None,
            on_close,
        }
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub fn width(mut self, width: OverlayWidth) -> Self {
        self.width = width;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn focus_order(mut self, focus_order: Vec<FocusHandle>) -> Self {
        self.focus_order = focus_order;
        self
    }

    pub fn scrollbar(mut self, handle: ScrollHandle, scrollbar: Entity<Scrollbar>) -> Self {
        self.scroll = Some((handle, scrollbar));
        self
    }
}

impl RenderOnce for Drawer {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let close_backdrop = Rc::clone(&self.on_close);
        let close_key = Rc::clone(&self.on_close);
        let focus_order = self.focus_order;
        let busy = self.busy;
        let scroll_handle = self.scroll.as_ref().map(|(handle, _)| handle.clone());
        let scrollbar = self.scroll.map(|(_, scrollbar)| scrollbar);
        div()
            .absolute()
            .inset_0()
            .flex()
            .justify_end()
            .bg(gpui::rgba(0x00000099))
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !busy {
                    close_backdrop(window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !busy => {
                        cx.stop_propagation();
                        close_key(window, cx);
                    }
                    "tab" if !focus_order.is_empty() => {
                        cx.stop_propagation();
                        let current = focus_order
                            .iter()
                            .position(|handle| handle.is_focused(window));
                        let index = focus_target_index(
                            focus_order.len(),
                            current,
                            event.keystroke.modifiers.shift,
                        );
                        focus_order[index].focus(window);
                    }
                    _ => {}
                }
            })
            .child(
                div()
                    .w_full()
                    .max_w(rems(self.width.pixels() / 16.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_l_1()
                    .border_color(theme::border_strong())
                    .bg(theme::panel())
                    .shadow_2xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px(rems(24. / 16.))
                            .py(rems(1.))
                            .border_b_1()
                            .border_color(theme::border())
                            .text_size(rems(14. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .id("drawer-scroll-content")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .when_some(scroll_handle, |body, handle| {
                                        body.scrollbar_width(px(0.)).track_scroll(&handle)
                                    })
                                    .px(rems(24. / 16.))
                                    .py(rems(20. / 16.))
                                    .child(self.body),
                            )
                            .children(scrollbar),
                    )
                    .children(self.footer.map(|footer| {
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme::border())
                            .bg(theme::with_alpha(theme::bg(), 0.4))
                            .px(rems(24. / 16.))
                            .py(rems(1.))
                            .child(footer)
                    })),
            )
    }
}

#[derive(IntoElement)]
pub struct ConfirmDialog {
    title: SharedString,
    body: SharedString,
    confirm_label: SharedString,
    busy_label: SharedString,
    busy: bool,
    on_confirm: PressHandler,
    on_cancel: PressHandler,
}

impl ConfirmDialog {
    pub fn new(
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        confirm_label: impl Into<SharedString>,
        busy_label: impl Into<SharedString>,
        busy: bool,
        on_confirm: PressHandler,
        on_cancel: PressHandler,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            confirm_label: confirm_label.into(),
            busy_label: busy_label.into(),
            busy,
            on_confirm,
            on_cancel,
        }
    }
}

impl RenderOnce for ConfirmDialog {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let cancel = Rc::clone(&self.on_cancel);
        let cancel_key = Rc::clone(&self.on_cancel);
        let confirm = Rc::clone(&self.on_confirm);
        let busy = self.busy;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x0000008c))
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !busy {
                    cancel(window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !busy {
                    cx.stop_propagation();
                    cancel_key(window, cx);
                }
            })
            .child(
                div()
                    .w_full()
                    .max_w(rems(420. / 16.))
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px(rems(22. / 16.))
                    .py(rems(20. / 16.))
                    .shadow_2xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .child(
                                gpui::svg()
                                    .path("trash.svg")
                                    .size(rems(15. / 16.))
                                    .flex_none()
                                    .text_color(theme::danger()),
                            )
                            .child(
                                div()
                                    .text_size(rems(15. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(self.title),
                            ),
                    )
                    .children((!self.body.is_empty()).then(|| {
                        div()
                            .text_size(rems(13. / 16.))
                            .line_height(rems(20. / 16.))
                            .text_color(theme::muted())
                            .child(self.body)
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(confirm_action_button(
                                "confirm-cancel",
                                "Cancel".into(),
                                false,
                                busy,
                                self.on_cancel,
                            ))
                            .child(confirm_action_button(
                                "confirm-submit",
                                if busy {
                                    self.busy_label
                                } else {
                                    self.confirm_label
                                },
                                true,
                                busy,
                                confirm,
                            )),
                    ),
            )
    }
}

fn confirm_action_button(
    id: &'static str,
    label: SharedString,
    destructive: bool,
    disabled: bool,
    on_press: PressHandler,
) -> AnyElement {
    let click = Rc::clone(&on_press);
    let mut button = div()
        .id(id)
        .tab_index(0)
        .tab_stop(!disabled)
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(8. / 16.))
        .when(destructive, |button| {
            button
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
        })
        .bg(if destructive {
            theme::with_alpha(theme::danger(), 0.1)
        } else {
            theme::raised()
        })
        .px(rems(14. / 16.))
        .py(rems(6. / 16.))
        .font_weight(FontWeight::MEDIUM)
        .text_size(rems(12. / 16.))
        .text_color(if destructive {
            theme::danger()
        } else {
            theme::text()
        })
        .opacity(if disabled { 0.6 } else { 1. })
        .cursor(if disabled {
            CursorStyle::Arrow
        } else {
            CursorStyle::PointingHand
        })
        .focus_visible(|style| {
            style.shadow(vec![BoxShadow {
                color: theme::with_alpha(
                    if destructive {
                        theme::danger()
                    } else {
                        theme::border_strong()
                    },
                    0.65,
                ),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
            }])
        })
        .child(label);
    if !disabled {
        button = button
            .hover(move |button| {
                button.bg(if destructive {
                    theme::with_alpha(theme::danger(), 0.2)
                } else {
                    theme::with_alpha(theme::raised(), 0.8)
                })
            })
            .on_click(move |_, window, cx| click(window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    on_press(window, cx);
                }
            });
    }
    button.into_any_element()
}

fn focus_target_index(len: usize, current: Option<usize>, backwards: bool) -> usize {
    if backwards {
        current.map_or(len - 1, |index| index.checked_sub(1).unwrap_or(len - 1))
    } else {
        current.map_or(0, |index| (index + 1) % len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_trap_wraps_in_both_directions() {
        assert_eq!(focus_target_index(4, None, false), 0);
        assert_eq!(focus_target_index(4, Some(3), false), 0);
        assert_eq!(focus_target_index(4, None, true), 3);
        assert_eq!(focus_target_index(4, Some(0), true), 3);
    }
}

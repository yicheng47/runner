use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, relative, rems, AnyElement, App, BoxShadow, CursorStyle, Entity, FocusHandle,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, RenderOnce, ScrollHandle, SharedString,
    Window,
};

use crate::theme;
use crate::ui::button::{ButtonVariant, PressHandler};
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
    icon: SharedString,
    variant: ButtonVariant,
    confirm_label: SharedString,
    busy_label: SharedString,
    busy: bool,
    on_confirm: PressHandler,
    on_cancel: PressHandler,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConfirmDialogState {
    #[default]
    Closed,
    Open,
    Submitting,
}

impl ConfirmDialogState {
    pub fn open(&mut self) {
        if *self == Self::Closed {
            *self = Self::Open;
        }
    }

    pub fn cancel(&mut self) {
        if *self == Self::Open {
            *self = Self::Closed;
        }
    }

    pub fn submit(&mut self) -> bool {
        if *self != Self::Open {
            return false;
        }
        *self = Self::Submitting;
        true
    }

    pub fn finish(&mut self) {
        if *self == Self::Submitting {
            *self = Self::Closed;
        }
    }

    pub fn is_open(self) -> bool {
        self != Self::Closed
    }

    pub fn is_submitting(self) -> bool {
        self == Self::Submitting
    }
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
            icon: "trash.svg".into(),
            variant: ButtonVariant::Danger,
            confirm_label: confirm_label.into(),
            busy_label: busy_label.into(),
            busy,
            on_confirm,
            on_cancel,
        }
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl RenderOnce for ConfirmDialog {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let cancel = Rc::clone(&self.on_cancel);
        let cancel_key = Rc::clone(&self.on_cancel);
        let confirm = Rc::clone(&self.on_confirm);
        let busy = self.busy;
        let icon_color = match self.variant {
            ButtonVariant::Danger => theme::danger(),
            ButtonVariant::Primary => theme::accent(),
            ButtonVariant::Warning => theme::warning(),
            ButtonVariant::Secondary | ButtonVariant::Ghost => theme::text(),
        };
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
                    .debug_selector(|| "CONFIRM_DIALOG_PANEL".into())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .child(
                                gpui::svg()
                                    .path(self.icon)
                                    .size(rems(15. / 16.))
                                    .flex_none()
                                    .text_color(icon_color),
                            )
                            .child(
                                // Panel width minus its padding, the icon, and
                                // the gap. Text wraps by default, and the centering
                                // row above probes this panel at min-content, where
                                // `w_full().min_w_0()` becomes a known width of 0 and
                                // the text is shaped one glyph per line — a height
                                // gpui's layout cache then keeps. Widths are spelled
                                // out here and on the body for that reason.
                                div()
                                    .w(rems((420. - 2. * 22. - 15. - 10.) / 16.))
                                    .whitespace_normal()
                                    .debug_selector(|| "CONFIRM_DIALOG_TITLE".into())
                                    .text_size(rems(15. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(self.title),
                            ),
                    )
                    .children((!self.body.is_empty()).then(|| {
                        div()
                            .w(rems((420. - 2. * 22.) / 16.))
                            .whitespace_normal()
                            .text_size(rems(13. / 16.))
                            .line_height(rems(20. / 16.))
                            .text_color(theme::muted())
                            .debug_selector(|| "CONFIRM_DIALOG_BODY".into())
                            .child(self.body)
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .debug_selector(|| "CONFIRM_DIALOG_FOOTER".into())
                            .child(confirm_action_button(
                                "confirm-cancel",
                                "Cancel".into(),
                                ButtonVariant::Secondary,
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
                                self.variant,
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
    variant: ButtonVariant,
    disabled: bool,
    on_press: PressHandler,
) -> AnyElement {
    let click = Rc::clone(&on_press);
    let bordered = matches!(variant, ButtonVariant::Danger);
    let (background, foreground, border) = match variant {
        ButtonVariant::Primary => (theme::accent(), theme::accent_ink(), theme::accent()),
        ButtonVariant::Warning => (theme::warning(), theme::bg(), theme::warning()),
        ButtonVariant::Secondary => (theme::raised(), theme::text(), theme::border_strong()),
        ButtonVariant::Ghost => (
            gpui::transparent_black(),
            theme::muted(),
            gpui::transparent_black(),
        ),
        ButtonVariant::Danger => (
            theme::with_alpha(theme::danger(), 0.1),
            theme::danger(),
            theme::with_alpha(theme::danger(), 0.4),
        ),
    };
    let mut button = div()
        .id(id)
        .tab_index(0)
        .tab_stop(!disabled)
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(8. / 16.))
        .when(bordered, |button| button.border_1().border_color(border))
        .bg(background)
        .px(rems(14. / 16.))
        .py(rems(6. / 16.))
        .font_weight(FontWeight::MEDIUM)
        .text_size(rems(12. / 16.))
        .text_color(foreground)
        .opacity(if disabled { 0.6 } else { 1. })
        .cursor(if disabled {
            CursorStyle::Arrow
        } else {
            CursorStyle::PointingHand
        })
        .focus_visible(|style| {
            style.shadow(vec![BoxShadow {
                color: theme::with_alpha(
                    match variant {
                        ButtonVariant::Primary => theme::accent(),
                        ButtonVariant::Warning => theme::warning(),
                        ButtonVariant::Secondary | ButtonVariant::Ghost => theme::border_strong(),
                        ButtonVariant::Danger => theme::danger(),
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
                button.bg(match variant {
                    ButtonVariant::Primary => theme::with_alpha(theme::accent(), 0.8),
                    ButtonVariant::Warning => theme::with_alpha(theme::warning(), 0.8),
                    ButtonVariant::Secondary => theme::with_alpha(theme::raised(), 0.8),
                    ButtonVariant::Ghost => theme::with_alpha(theme::raised(), 0.8),
                    ButtonVariant::Danger => theme::with_alpha(theme::danger(), 0.2),
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

#[cfg(test)]
mod confirm_tests {
    use super::{ConfirmDialog, ConfirmDialogState};
    use gpui::{
        div, px, Context, IntoElement, ParentElement, Render, Styled, TestAppContext,
        VisualTestContext, Window,
    };
    use std::rc::Rc;

    struct CrewDeleteConfirmTest;

    impl Render for CrewDeleteConfirmTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(ConfirmDialog::new(
                "Delete crew \"Backorder Unread Cleanup Trae\" permanently?",
                "This removes all its slots and deletes its archived missions and session history. Crews with non-archived missions cannot be deleted until those missions are archived.",
                "Delete crew",
                "Deleting…",
                false,
                Rc::new(|_, _| {}),
                Rc::new(|_, _| {}),
            ))
        }
    }

    #[test]
    fn confirm_dialog_grows_with_its_wrapped_body_and_keeps_the_footer_inside() {
        for rem in [16., 20.8] {
            let mut cx = TestAppContext::single();
            let window = cx.add_window(|window, _| {
                window.set_rem_size(px(rem));
                CrewDeleteConfirmTest
            });
            cx.run_until_parked();
            let mut window = VisualTestContext::from_window(window.into(), &cx);
            for size in [
                gpui::size(px(700.), px(500.)),
                gpui::size(px(1440.), px(900.)),
                gpui::size(px(2560.), px(1440.)),
            ] {
                window.simulate_resize(size);
                cx.run_until_parked();
                let panel = window.debug_bounds("CONFIRM_DIALOG_PANEL").expect("panel");
                let body = window.debug_bounds("CONFIRM_DIALOG_BODY").expect("body");
                let footer = window
                    .debug_bounds("CONFIRM_DIALOG_FOOTER")
                    .expect("footer");
                let line = px(rem * 20. / 16.);
                assert!(
                    body.size.height >= line * 2.,
                    "rem {rem} {size:?}: body did not wrap: {body:?}"
                );
                assert!(
                    footer.bottom() <= panel.bottom() && footer.top() >= body.bottom(),
                    "rem {rem} {size:?}: footer outside the panel: panel={panel:?} body={body:?} footer={footer:?}"
                );
                assert!(
                    panel.size.height < size.height
                        && panel.size.width <= px(rem * 420. / 16.) + px(1.),
                    "rem {rem} {size:?}: panel does not fit: {panel:?}"
                );
            }
        }
    }

    #[test]
    fn submitting_confirmation_ignores_cancel_and_duplicate_submit() {
        let mut state = ConfirmDialogState::Closed;
        state.open();
        assert_eq!(state, ConfirmDialogState::Open);
        assert!(state.submit());
        assert!(!state.submit());
        state.cancel();
        assert_eq!(state, ConfirmDialogState::Submitting);
        state.finish();
        assert_eq!(state, ConfirmDialogState::Closed);
    }
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

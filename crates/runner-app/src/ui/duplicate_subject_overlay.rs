use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, rems, svg, App, FontWeight, KeyDownEvent, RenderOnce, SharedString, Window};

use crate::theme;
use crate::ui::button::{focus_ring, is_activation_key};
use crate::ui::PressHandler;

#[derive(Clone, Copy)]
pub enum DuplicateSubjectKind {
    Mission,
    Chat,
}

#[derive(IntoElement)]
pub struct DuplicateSubjectOverlay {
    id: SharedString,
    kind: DuplicateSubjectKind,
    can_focus: bool,
    on_focus: PressHandler,
    on_stay: PressHandler,
}

impl DuplicateSubjectOverlay {
    pub fn new(
        id: impl Into<SharedString>,
        kind: DuplicateSubjectKind,
        can_focus: bool,
        on_focus: impl Fn(&mut Window, &mut App) + 'static,
        on_stay: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            can_focus,
            on_focus: Rc::new(on_focus),
            on_stay: Rc::new(on_stay),
        }
    }
}

impl RenderOnce for DuplicateSubjectOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let noun = match self.kind {
            DuplicateSubjectKind::Mission => "mission",
            DuplicateSubjectKind::Chat => "chat",
        };
        let focus = Rc::clone(&self.on_focus);
        let focus_key = Rc::clone(&self.on_focus);
        let stay = Rc::clone(&self.on_stay);
        let stay_key = Rc::clone(&self.on_stay);
        let focus_id = SharedString::from(format!("focus-{}", self.id));
        let stay_id = SharedString::from(format!("stay-{}", self.id));
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .bg(theme::with_alpha(theme::bg(), 0.85))
            .child(
                div()
                    .w_full()
                    .max_w(rems(448. / 16.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_6()
                    .text_center()
                    .shadow_2xl()
                    .flex_none()
                    .debug_selector(|| "DUPLICATE_SUBJECT_CARD".into())
                    .child(
                        div()
                            .size(rems(44. / 16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .flex_none()
                            .child(
                                svg()
                                    .path("app-window.svg")
                                    .size(rems(20. / 16.))
                                    .text_color(theme::accent()),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .debug_selector(|| "DUPLICATE_SUBJECT_TITLE".into())
                                    .whitespace_normal()
                                    .text_center()
                                    .text_size(rems(15. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Open in another window"),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .debug_selector(|| "DUPLICATE_SUBJECT_SUBTITLE".into())
                                    .whitespace_normal()
                                    .text_center()
                                    .text_size(rems(13. / 16.))
                                    .line_height(rems(20. / 16.))
                                    .text_color(theme::muted())
                                    .child(format!("Another window is already driving this {noun}. Only one window can own the terminal at a time, so this view is read-only until you focus it here.")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .pt(rems(2. / 16.))
                            .flex_none()
                            .debug_selector(|| "DUPLICATE_SUBJECT_ACTIONS".into())
                            .child(
                                div()
                                    .id(focus_id)
                                    .tab_index(0)
                                    .tab_stop(self.can_focus)
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(rems(6. / 16.))
                                    .bg(theme::accent())
                                    .px(rems(14. / 16.))
                                    .py(rems(8. / 16.))
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::accent_ink())
                                    .opacity(if self.can_focus { 1. } else { 0.5 })
                                    .focus_visible(|button| {
                                        button.shadow(focus_ring(theme::accent()))
                                    })
                                    .when(self.can_focus, |button| {
                                        button
                                            .cursor_pointer()
                                            .hover(|button| {
                                                button.bg(theme::with_alpha(theme::accent(), 0.9))
                                            })
                                            .on_click(move |_, window, cx| focus(window, cx))
                                            .on_key_down(
                                                move |event: &KeyDownEvent, window, cx| {
                                                    if is_activation_key(event) {
                                                        cx.stop_propagation();
                                                        focus_key(window, cx);
                                                    }
                                                },
                                            )
                                    })
                                    .child(
                                        svg()
                                            .path("app-window.svg")
                                            .size(rems(14. / 16.))
                                            .text_color(theme::accent_ink()),
                                    )
                                    .child("Focus that window"),
                            )
                            .child(
                                div()
                                    .id(stay_id)
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .text_size(rems(13. / 16.))
                                    .text_color(theme::muted())
                                    .hover(|button| {
                                        button.text_color(theme::text()).underline()
                                    })
                                    .focus_visible(|button| {
                                        button.text_color(theme::text()).underline()
                                    })
                                    .on_click(move |_, window, cx| stay(window, cx))
                                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                        if is_activation_key(event) {
                                            cx.stop_propagation();
                                            stay_key(window, cx);
                                        }
                                    })
                                    .child("Stay here"),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, Context, Render, TestAppContext, VisualTestContext};

    struct DuplicateOverlayTest;

    impl Render for DuplicateOverlayTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(DuplicateSubjectOverlay::new(
                "test-duplicate",
                DuplicateSubjectKind::Chat,
                true,
                |_, _| {},
                |_, _| {},
            ))
        }
    }

    #[test]
    fn wrapped_duplicate_overlay_preserves_bottom_padding_at_zoom() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            DuplicateOverlayTest
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("DUPLICATE_SUBJECT_CARD")
            .expect("card bounds");
        let actions = window
            .debug_bounds("DUPLICATE_SUBJECT_ACTIONS")
            .expect("actions bounds");
        let title = window
            .debug_bounds("DUPLICATE_SUBJECT_TITLE")
            .expect("title bounds");
        let subtitle = window
            .debug_bounds("DUPLICATE_SUBJECT_SUBTITLE")
            .expect("subtitle bounds");
        assert_eq!(card.bottom() - actions.bottom(), px(31.5));
        assert_eq!(title.size.width, subtitle.size.width);
        assert!(subtitle.size.height > px(26.));
        assert!(subtitle.right() < card.right());
    }
}

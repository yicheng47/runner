use crate::theme;
use gpui::prelude::*;
use gpui::{
    canvas, div, point, px, rems, size, Bounds, Context, ElementInputHandler, EntityInputHandler,
    FocusHandle, KeyDownEvent, MouseButton, Pixels, Point, TextInputConfiguration, UTF16Selection,
    Window,
};
use runner_core::remote::Secret;
use std::ops::Range;

// A PIN never enters the general text field's undo history, clipboard or shaped text.
pub(crate) struct PinField {
    value: Secret,
    pub(crate) focus: FocusHandle,
    painted: Option<Bounds<Pixels>>,
    pub(crate) invalid: bool,
    pub(crate) disabled: bool,
}

impl PinField {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            value: Secret::default(),
            focus: cx.focus_handle(),
            painted: None,
            invalid: false,
            disabled: false,
        }
    }

    pub(crate) fn value(&self) -> &Secret {
        &self.value
    }

    fn selection(&self) -> Range<usize> {
        self.value.0.len()..self.value.0.len()
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> bool {
        if self.disabled {
            return false;
        }
        let range = range.unwrap_or_else(|| self.selection());
        if !text.bytes().all(|b| b.is_ascii_digit())
            || range.start > range.end
            || range.end > self.value.0.len()
            || self.value.0.len() - range.len() + text.len() > 6
        {
            return false;
        }
        self.value.0.replace_range(range, text);
        true
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .filter(|text| text.len() == 6)
        {
            self.replace(Some(0..self.value.0.len()), &text);
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            cx.stop_propagation();
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let command = if cfg!(any(windows, target_os = "linux")) {
            modifiers.control
        } else {
            modifiers.platform
        };
        if command {
            match key {
                "v" => self.paste(cx),
                "a" | "c" | "x" | "z" | "y" => {}
                _ => return,
            }
        } else {
            match key {
                "backspace" | "delete" => {
                    self.value.0.pop();
                }
                "left" | "right" | "home" | "end" => {}
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn point_index(&self, point: Point<Pixels>) -> usize {
        self.painted
            .map(|bounds| ((point.x - bounds.left()) / (bounds.size.width / 6.)).max(0.) as usize)
            .unwrap_or(self.value.0.len())
            .min(self.value.0.len())
    }
}

impl Render for PinField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let length = self.value.0.len();
        let owner = cx.entity();
        div()
            .relative()
            .w_full()
            .h(rems(48. / 16.))
            .grid()
            .grid_cols(6)
            .gap_2()
            .track_focus(&self.focus)
            .key_context("TextInput")
            .tab_index(0)
            .cursor(gpui::CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|_, _: &runner_app::Copy, _, cx| cx.stop_propagation()))
            .on_action(cx.listener(|_, _: &runner_app::Cut, _, cx| cx.stop_propagation()))
            .on_action(cx.listener(|this, _: &runner_app::Paste, _, cx| {
                this.paste(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|_, _: &runner_app::SelectAll, _, cx| cx.stop_propagation()))
            .on_action(cx.listener(|_, _: &runner_app::Undo, _, cx| cx.stop_propagation()))
            .on_action(cx.listener(|_, _: &runner_app::Redo, _, cx| cx.stop_propagation()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .children((0..6).map(|index| {
                let current = focused && index == length.min(5);
                div()
                    .debug_selector(move || format!("REMOTE_PIN_BOX_{index}"))
                    .w_full()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(rems(6. / 16.))
                    .border(rems(if current { 1.5 / 16. } else { 1. / 16. }))
                    .border_color(if self.invalid {
                        theme::danger()
                    } else if current {
                        theme::accent()
                    } else {
                        theme::border_strong()
                    })
                    .bg(theme::bg())
                    .text_color(theme::text())
                    .text_size(rems(20. / 16.))
                    .when(index < length, |cell| cell.child("•"))
                    .when(current, |cell| {
                        cell.child(
                            div()
                                .w(rems(1.5 / 16.))
                                .h(rems(20. / 16.))
                                .bg(theme::accent()),
                        )
                    })
            }))
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, window, cx| {
                        let focus = owner.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, owner.clone()),
                            cx,
                        );
                        owner.update(cx, |field, _| field.painted = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

impl EntityInputHandler for PinField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = range.start.min(self.value.0.len())..range.end.min(self.value.0.len());
        *adjusted = Some(range.clone());
        Some("•".repeat(range.len()))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selection(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.replace(range, text) {
            cx.notify();
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.painted.map(|bounds| {
            let gap = rems(8. / 16.).to_pixels(window.rem_size());
            let width = (bounds.size.width - gap * 5.) / 6.;
            let x = (width + gap) * range.start.min(5) as f32;
            Bounds::new(
                bounds.origin + point(x, px(0.)),
                size(width, bounds.size.height),
            )
        })
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.point_index(point))
    }
    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_input_caps_six_boxes_masks_native_reads_and_never_exports_clipboard() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            let field = PinField::new(cx);
            field.focus.focus(window, cx);
            field
        });
        window
            .update(&mut cx, |field, window, cx| {
                field.replace_text_in_range(None, "001234", window, cx);
                assert!(field.value().0 == "001234");
                for invalid in ["5", "a", "１２３", "12 34", "123\n"] {
                    assert!(!field.replace(None, invalid));
                }
                let mut adjusted = None;
                assert_eq!(
                    field
                        .text_for_range(0..6, &mut adjusted, window, cx)
                        .as_deref(),
                    Some("••••••")
                );
                assert_eq!(adjusted, Some(0..6));
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("clipboard marker".into()));
                let command = if cfg!(any(windows, target_os = "linux")) {
                    "ctrl"
                } else {
                    "cmd"
                };
                for key in ["a", "c", "x", "z", "y"] {
                    field.key_down(
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(&format!("{command}-{key}")).unwrap(),
                            is_held: false,
                            prefer_character_input: false,
                        },
                        window,
                        cx,
                    );
                    assert!(field.value().0 == "001234");
                    assert_eq!(
                        cx.read_from_clipboard().unwrap().text().as_deref(),
                        Some("clipboard marker")
                    );
                }
                for invalid in ["12345", "1234567", "12345x", "１２３４５６"] {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(invalid.into()));
                    field.paste(cx);
                    assert!(field.value().0 == "001234");
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("009876".into()));
                field.key_down(
                    &KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(&format!("{command}-v")).unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
                assert!(field.value().0 == "009876");
                field.disabled = true;
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("123456".into()));
                field.paste(cx);
                assert!(field.value().0 == "009876");
                field.disabled = false;
                field.key_down(
                    &KeyDownEvent {
                        keystroke: gpui::Keystroke::parse("backspace").unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
                assert!(field.value().0 == "00987");
                assert_eq!(field.selection(), 5..5);
                field.replace_and_mark_text_in_range(None, "uncommitted", None, window, cx);
                assert!(field.value().0 == "00987");
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        for selector in [
            "REMOTE_PIN_BOX_0",
            "REMOTE_PIN_BOX_1",
            "REMOTE_PIN_BOX_2",
            "REMOTE_PIN_BOX_3",
            "REMOTE_PIN_BOX_4",
            "REMOTE_PIN_BOX_5",
        ] {
            assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(48.));
        }
        window
            .read_with(&cx, |field, _| {
                assert!(!format!("{:?}", field.value()).contains('0'))
            })
            .unwrap();
    }
}

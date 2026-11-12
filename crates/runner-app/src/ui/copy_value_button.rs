use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, rems, svg, ClipboardItem, Context, CursorStyle, FocusHandle, Focusable, KeyDownEvent,
    MouseButton, Render, SharedString, Window,
};

use crate::theme;
use crate::ui::tooltip::Tooltip;

pub struct CopyValueButton {
    value: Option<String>,
    label: SharedString,
    label_source: crate::i18n::Text,
    labeled: bool,
    show_when_empty: bool,
    copied: bool,
    generation: u64,
    focus_handle: FocusHandle,
    debug_selector: Option<SharedString>,
}

impl CopyValueButton {
    pub fn new(
        focus_handle: FocusHandle,
        value: Option<String>,
        label: impl Into<crate::i18n::Text>,
    ) -> Self {
        let label_source = label.into();
        Self {
            value: value.filter(|value| !value.is_empty()),
            label: label_source.resolve(),
            label_source,
            labeled: false,
            show_when_empty: false,
            copied: false,
            generation: 0,
            focus_handle,
            debug_selector: None,
        }
    }

    pub fn labeled(mut self) -> Self {
        self.labeled = true;
        self
    }

    pub fn show_when_empty(mut self) -> Self {
        self.show_when_empty = true;
        self
    }

    pub fn debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    pub fn is_copied(&self) -> bool {
        self.copied
    }

    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    pub fn reset_copied(&mut self, cx: &mut Context<Self>) {
        if !self.copied {
            return;
        }
        self.copied = false;
        self.generation += 1;
        cx.notify();
    }

    pub fn set_value(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        let value = value.filter(|value| !value.is_empty());
        if self.value == value {
            return;
        }
        self.value = value;
        self.copied = false;
        self.generation += 1;
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(value) = self.value.clone() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.copied = true;
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_500))
                .await;
            let _ = weak.update(cx, |button, cx| {
                if button.generation == generation {
                    button.copied = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            cx.stop_propagation();
            self.copy(cx);
        }
    }
}

impl Focusable for CopyValueButton {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CopyValueButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.label = self.label_source.resolve();
        let available = self.value.is_some();
        if !available && !self.show_when_empty {
            return div().into_any_element();
        }
        let copied = self.copied;
        let labeled = self.labeled;
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        let button = div()
            .when_some(self.debug_selector.clone(), |button, selector| {
                button.debug_selector(move || selector.to_string())
            })
            .id("copy-value-button")
            .group("copy-value-button")
            .track_focus(&self.focus_handle.clone().tab_stop(available))
            .tab_index(0)
            .tab_stop(available)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(6. / 16.))
            .when(!labeled, |button| button.size(rems(20. / 16.)))
            .when(labeled, |button| {
                button
                    .h(rems(26. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::raised())
                    .px(rems(10. / 16.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
            })
            .when(!labeled, |button| {
                button.rounded(rems(4. / 16.)).text_color(theme::faint())
            })
            .cursor(if available {
                CursorStyle::PointingHand
            } else {
                CursorStyle::OperationNotAllowed
            })
            .opacity(if available {
                1.
            } else if labeled {
                0.5
            } else {
                0.4
            })
            .when(available, |button| {
                button.hover(move |button| {
                    if labeled {
                        button
                            .border_color(theme::border_strong())
                            .text_color(theme::text())
                    } else {
                        button
                            .bg(theme::with_alpha(theme::border(), 0.6))
                            .text_color(theme::text())
                    }
                })
            })
            .focus_visible(move |button| {
                if labeled {
                    button
                        .border_color(theme::border_strong())
                        .text_color(theme::text())
                        .shadow(crate::ui::button::focus_ring(theme::border_strong()))
                } else {
                    button
                        .bg(theme::with_alpha(theme::border(), 0.6))
                        .text_color(theme::text())
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(available, |button| {
                button
                    .on_click(move |event, _, cx| {
                        cx.stop_propagation();
                        if !event.is_keyboard() {
                            entity.update(cx, |button, cx| button.copy(cx));
                        }
                    })
                    .on_key_down(cx.listener(Self::on_key_down))
            })
            .child(
                svg()
                    .flex_none()
                    .path(if copied { "check.svg" } else { "copy.svg" })
                    .size(rems(12. / 16.))
                    .text_color(if focused {
                        theme::text()
                    } else {
                        theme::faint()
                    })
                    .when(available, |icon| {
                        icon.group_hover("copy-value-button", |icon| icon.text_color(theme::text()))
                    }),
            )
            .children(labeled.then(|| {
                if copied {
                    crate::i18n::text("ui.copied").into()
                } else {
                    self.label.clone()
                }
            }));
        if labeled {
            return button.into_any_element();
        }
        Tooltip::new(
            SharedString::from(format!(
                "copy-value-tooltip-{}",
                self.label_source.identity()
            )),
            if copied {
                SharedString::from(crate::i18n::text("ui.copied"))
            } else {
                self.label.clone()
            },
            button,
        )
        .focus_handle(self.focus_handle.clone())
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        InputEvent as _, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext,
    };

    #[test]
    fn labeled_copy_button_handles_keyboard_repeats_and_empty_state() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("prior".into()));
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            CopyValueButton::new(focus, None, crate::i18n::message("ui.copy"))
                .labeled()
                .show_when_empty()
                .debug_selector("TEST_COPY")
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();

        let bounds = visual.debug_bounds("TEST_COPY").unwrap();
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        host.read_with(&visual, |button, cx| {
            assert!(!button.is_copied());
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("prior")
            );
        })
        .unwrap();

        host.update(&mut visual, |button, window, cx| {
            button.set_value(Some("codex -m <model>".into()), cx);
            button.focus_handle.focus(window, cx);
        })
        .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        let keystroke = Keystroke::parse("enter").unwrap();
        visual.update(|window, cx| {
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        });
        visual.run_until_parked();
        host.read_with(&visual, |button, cx| {
            assert!(button.is_copied());
            assert_eq!(button.generation, 2);
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("codex -m <model>")
            );
        })
        .unwrap();

        visual
            .executor()
            .advance_clock(Duration::from_millis(1_000));
        visual.run_until_parked();
        visual.update(|_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("overwritten".into()));
        });
        let bounds = visual.debug_bounds("TEST_COPY").unwrap();
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
        host.read_with(&visual, |button, cx| {
            assert!(button.is_copied());
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("codex -m <model>")
            );
        })
        .unwrap();

        visual.executor().advance_clock(Duration::from_millis(600));
        visual.run_until_parked();
        host.read_with(&visual, |button, _| assert!(button.is_copied()))
            .unwrap();

        visual.executor().advance_clock(Duration::from_millis(900));
        visual.run_until_parked();
        host.read_with(&visual, |button, _| assert!(!button.is_copied()))
            .unwrap();
    }
}

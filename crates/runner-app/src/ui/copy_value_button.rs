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
    labeled: bool,
    show_when_empty: bool,
    copied: bool,
    generation: u64,
    focus_handle: FocusHandle,
}

impl CopyValueButton {
    pub fn new(
        focus_handle: FocusHandle,
        value: Option<String>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            value: value.filter(|value| !value.is_empty()),
            label: label.into(),
            labeled: false,
            show_when_empty: false,
            copied: false,
            generation: 0,
            focus_handle,
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
        let available = self.value.is_some();
        if !available && !self.show_when_empty {
            return div().into_any_element();
        }
        let copied = self.copied;
        let labeled = self.labeled;
        let focused = self.focus_handle.is_focused(window);
        let entity = cx.entity();
        let button = div()
            .id("copy-value-button")
            .group("copy-value-button")
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .tab_stop(available || labeled)
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
                    .text_size(rems(12. / 16.))
                    .text_color(theme::muted())
            })
            .when(!labeled, |button| {
                button.rounded(rems(4. / 16.)).text_color(theme::faint())
            })
            .cursor(if available || labeled {
                CursorStyle::PointingHand
            } else {
                CursorStyle::OperationNotAllowed
            })
            .opacity(if !available && !labeled { 0.4 } else { 1. })
            .hover(|button| {
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
            .focus_visible(|button| {
                button
                    .bg(theme::with_alpha(theme::border(), 0.6))
                    .text_color(theme::text())
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity.update(cx, |button, cx| button.copy(cx));
            })
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                svg()
                    .path(if copied { "check.svg" } else { "copy.svg" })
                    .size(rems(12. / 16.))
                    .text_color(if focused {
                        theme::text()
                    } else {
                        theme::faint()
                    })
                    .group_hover("copy-value-button", |icon| icon.text_color(theme::text())),
            )
            .children(labeled.then(|| {
                if copied {
                    "Copied".into()
                } else {
                    self.label.clone()
                }
            }));
        Tooltip::new(
            SharedString::from(format!("copy-value-tooltip-{}", self.label)),
            if copied {
                SharedString::from("Copied")
            } else {
                self.label.clone()
            },
            button,
        )
        .focus_handle(self.focus_handle.clone())
        .into_any_element()
    }
}

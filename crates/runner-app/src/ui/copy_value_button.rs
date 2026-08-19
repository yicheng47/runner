use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, svg, ClipboardItem, Context, CursorStyle, FocusHandle, Focusable, KeyDownEvent,
    MouseButton, Render, SharedString, Window,
};

use crate::theme;
use crate::ui::tooltip::Tooltip;

pub struct CopyValueButton {
    value: Option<String>,
    label: SharedString,
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
            copied: false,
            generation: 0,
            focus_handle,
        }
    }

    pub fn set_value(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.value = value.filter(|value| !value.is_empty());
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(_) = self.value else {
            return div().into_any_element();
        };
        let copied = self.copied;
        let entity = cx.entity();
        let button = div()
            .id("copy-value-button")
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(px(4.))
            .text_color(theme::faint())
            .cursor(CursorStyle::PointingHand)
            .hover(|button| {
                button
                    .bg(theme::with_alpha(theme::border(), 0.6))
                    .text_color(theme::text())
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
                    .size(px(12.)),
            );
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

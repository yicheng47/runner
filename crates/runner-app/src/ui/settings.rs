use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, svg, AnyElement, App, BoxShadow, CursorStyle, ElementId, FontWeight, KeyDownEvent,
    RenderOnce, SharedString, Window,
};

use crate::theme;

#[derive(IntoElement)]
pub struct SettingsHeader {
    title: SharedString,
    subtitle: SharedString,
    action: Option<AnyElement>,
}

pub type PaneHeader = SettingsHeader;

impl SettingsHeader {
    pub fn new(title: impl Into<SharedString>, subtitle: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            action: None,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for SettingsHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .children(self.action.map(|action| div().flex_none().child(action))),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child(self.subtitle),
            )
    }
}

#[derive(IntoElement)]
pub struct SettingsCard {
    rows: Vec<AnyElement>,
}

impl SettingsCard {
    pub fn new(rows: impl IntoIterator<Item = AnyElement>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}

impl RenderOnce for SettingsCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(theme::border())
                    })
                    .child(row)
            }))
    }
}

#[derive(IntoElement)]
pub struct SettingsRow {
    label: SharedString,
    subtitle: Option<SharedString>,
    control: AnyElement,
}

impl SettingsRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            subtitle: None,
            control: control.into_any_element(),
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .px_4()
            .py_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(self.label),
                    )
                    .children(self.subtitle.map(|subtitle| {
                        div()
                            .text_size(px(11.))
                            .text_color(theme::muted())
                            .child(subtitle)
                    })),
            )
            .child(div().flex_none().child(self.control))
    }
}

pub type StepHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Stepper {
    id: ElementId,
    value_width: f32,
    value: AnyElement,
    decrement_disabled: bool,
    increment_disabled: bool,
    on_decrement: StepHandler,
    on_increment: StepHandler,
}

impl Stepper {
    pub fn new(
        id: impl Into<ElementId>,
        value_width: f32,
        value: impl IntoElement,
        on_decrement: StepHandler,
        on_increment: StepHandler,
    ) -> Self {
        Self {
            id: id.into(),
            value_width,
            value: value.into_any_element(),
            decrement_disabled: false,
            increment_disabled: false,
            on_decrement,
            on_increment,
        }
    }

    pub fn decrement_disabled(mut self, disabled: bool) -> Self {
        self.decrement_disabled = disabled;
        self
    }

    pub fn increment_disabled(mut self, disabled: bool) -> Self {
        self.increment_disabled = disabled;
        self
    }
}

impl RenderOnce for Stepper {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .h(px(30.))
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .child(stepper_button(
                (self.id.clone(), "decrement"),
                "minus.svg",
                self.decrement_disabled,
                self.on_decrement,
            ))
            .child(
                div()
                    .w(px(self.value_width))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_r_1()
                    .border_color(theme::border())
                    .child(self.value),
            )
            .child(stepper_button(
                (self.id, "increment"),
                "plus.svg",
                self.increment_disabled,
                self.on_increment,
            ))
    }
}

fn stepper_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    disabled: bool,
    handler: StepHandler,
) -> AnyElement {
    let key_handler = Rc::clone(&handler);
    let mut button = div()
        .id(id)
        .tab_index(0)
        .tab_stop(!disabled)
        .size(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::faint())
        .opacity(if disabled { 0.4 } else { 1. })
        .cursor(if disabled {
            CursorStyle::OperationNotAllowed
        } else {
            CursorStyle::PointingHand
        })
        .focus_visible(|button| {
            button.shadow(vec![BoxShadow {
                color: theme::border_strong(),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
            }])
        })
        .child(svg().path(icon).size(px(14.)));
    if !disabled {
        button = button
            .hover(|button| button.text_color(theme::text()))
            .on_click(move |_, window, cx| handler(window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_handler(window, cx);
                }
            });
    }
    button.into_any_element()
}

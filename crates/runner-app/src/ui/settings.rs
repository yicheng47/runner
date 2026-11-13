use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, BoxShadow, CursorStyle, ElementId, FontWeight, Hsla,
    KeyDownEvent, RenderOnce, SharedString, Window,
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
                    .gap(rems(24. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_display())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .children(self.action.map(|action| div().flex_none().child(action))),
            )
            .child(
                div()
                    .text_size(theme::text_body())
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
            .rounded(rems(12. / 16.))
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
    label_icon: Option<(SharedString, Hsla)>,
    subtitle: Option<SharedString>,
    subtitle_color: Option<Hsla>,
    wrap_control: bool,
    #[cfg(test)]
    debug_prefix: Option<SharedString>,
    control: AnyElement,
}

impl SettingsRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            label_icon: None,
            subtitle: None,
            subtitle_color: None,
            wrap_control: false,
            #[cfg(test)]
            debug_prefix: None,
            control: control.into_any_element(),
        }
    }

    pub fn label_icon(mut self, path: impl Into<SharedString>, color: Hsla) -> Self {
        self.label_icon = Some((path.into(), color));
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn subtitle_color(mut self, color: Hsla) -> Self {
        self.subtitle_color = Some(color);
        self
    }

    pub fn wrap_control(mut self) -> Self {
        self.wrap_control = true;
        self
    }

    #[cfg(test)]
    pub fn debug_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.debug_prefix = Some(prefix.into());
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        #[cfg(test)]
        let label_prefix = self.debug_prefix.clone();
        #[cfg(test)]
        let subtitle_prefix = self.debug_prefix.clone();
        #[cfg(test)]
        let control_prefix = self.debug_prefix;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(24. / 16.))
            .when(self.wrap_control, |row| {
                row.flex_wrap()
                    .gap_y(rems(12. / 16.))
                    .debug_selector(|| "SETTINGS_WRAPPED_ROW".into())
            })
            .px_4()
            .py_3()
            .child(
                div()
                    .min_w(px(0.))
                    .when(self.wrap_control, |text| {
                        text.flex_1()
                            .flex_basis(rems(200. / 16.))
                            .debug_selector(|| "SETTINGS_WRAPPED_ROW_TEXT".into())
                    })
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child({
                        let label = div()
                            .when_some(
                                {
                                    #[cfg(test)]
                                    {
                                        label_prefix
                                    }
                                    #[cfg(not(test))]
                                    {
                                        None::<SharedString>
                                    }
                                },
                                |label, prefix| {
                                    label.debug_selector(move || format!("{prefix}_LABEL"))
                                },
                            )
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text());
                        match self.label_icon {
                            Some((path, color)) => label
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .child(
                                    svg()
                                        .flex_none()
                                        .path(path)
                                        .size(rems(13. / 16.))
                                        .text_color(color),
                                )
                                .child(self.label),
                            None => label.child(self.label),
                        }
                    })
                    .children(self.subtitle.map(|subtitle| {
                        div()
                            .when(self.wrap_control, |description| {
                                description
                                    .debug_selector(|| "SETTINGS_WRAPPED_ROW_DESCRIPTION".into())
                            })
                            .when_some(
                                {
                                    #[cfg(test)]
                                    {
                                        subtitle_prefix
                                    }
                                    #[cfg(not(test))]
                                    {
                                        None::<SharedString>
                                    }
                                },
                                |el, prefix| {
                                    el.debug_selector(move || format!("{prefix}_SUBTITLE"))
                                },
                            )
                            .text_size(theme::text_meta())
                            .text_color(self.subtitle_color.unwrap_or_else(theme::muted))
                            .child(subtitle)
                    })),
            )
            .child(
                div()
                    .when_some(
                        {
                            #[cfg(test)]
                            {
                                control_prefix
                            }
                            #[cfg(not(test))]
                            {
                                None::<SharedString>
                            }
                        },
                        |el, prefix| el.debug_selector(move || format!("{prefix}_CONTROL")),
                    )
                    .flex_none()
                    .child(self.control),
            )
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
            .h(rems(30. / 16.))
            .items_center()
            .rounded(rems(6. / 16.))
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
                    .w(rems(self.value_width / 16.))
                    .h(rems(30. / 16.))
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
    const HOVER_GROUP: &str = "settings-stepper-button";

    let key_handler = Rc::clone(&handler);
    let icon = svg()
        .flex_none()
        .path(icon)
        .size(rems(14. / 16.))
        .text_color(theme::faint())
        .when(!disabled, |icon| {
            icon.group_hover(HOVER_GROUP, |icon| icon.text_color(theme::text()))
        });
    let mut button = div()
        .id(id)
        .when(!disabled, |button| button.group(HOVER_GROUP))
        .tab_index(0)
        .tab_stop(!disabled)
        .size(rems(30. / 16.))
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
                inset: false,
            }])
        })
        .child(icon);
    if !disabled {
        button = button
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

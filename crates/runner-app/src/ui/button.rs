use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, percentage, px, rems, svg, Animation, AnimationExt as _, AnyElement, App, BoxShadow,
    CursorStyle, ElementId, FocusHandle, FontWeight, IntoElement, KeyDownEvent, MouseButton,
    RenderOnce, SharedString, Transformation, Window,
};

use crate::theme;
use crate::ui::tooltip::Tooltip;

pub type PressHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Warning,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonSize {
    Sm,
    #[default]
    Md,
}

impl ButtonSize {
    pub const fn height(self, bordered: bool) -> f32 {
        let height = match self {
            Self::Sm => 24.,
            Self::Md => 32.,
        };
        if bordered {
            height + 2.
        } else {
            height
        }
    }
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<SharedString>,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    loading: bool,
    focus_handle: Option<FocusHandle>,
    tooltip: Option<SharedString>,
    on_press: Option<PressHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            disabled: false,
            loading: false,
            focus_handle: None,
            tooltip: None,
            on_press: None,
        }
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_press(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let inactive = self.disabled || self.loading;
        let has_border = matches!(
            self.variant,
            ButtonVariant::Secondary | ButtonVariant::Danger
        );
        let (background, foreground, border) = match self.variant {
            ButtonVariant::Primary => (theme::accent(), theme::accent_ink(), theme::accent()),
            ButtonVariant::Warning => (theme::warning(), theme::bg(), theme::warning()),
            ButtonVariant::Secondary => (theme::raised(), theme::text(), theme::border_strong()),
            ButtonVariant::Ghost => (
                gpui::transparent_black(),
                theme::muted(),
                gpui::transparent_black(),
            ),
            ButtonVariant::Danger => (
                gpui::transparent_black(),
                theme::danger(),
                theme::with_alpha(theme::danger(), 0.4),
            ),
        };
        let height = self.size.height(has_border);
        let (horizontal_padding, text_size, icon_size) = match self.size {
            ButtonSize::Sm => (10., 12., 12.),
            ButtonSize::Md => (12., 14., 14.),
        };
        let mouse_focus = self.focus_handle.clone();
        let tooltip_focus = self.focus_handle.clone();
        let tooltip_id = (self.id.clone(), "tooltip");
        let spinner_id = (self.id.clone(), "loading");
        let icon = (!self.loading).then_some(self.icon).flatten();
        let mut button = div()
            .id(self.id)
            .when_some(self.focus_handle, |button, handle| {
                button.track_focus(&handle)
            })
            .tab_index(0)
            .tab_stop(!inactive)
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(6. / 16.))
            .h(rems(height / 16.))
            .px(rems(horizontal_padding / 16.))
            .rounded(rems(4. / 16.))
            .when(has_border, |button| button.border_1().border_color(border))
            .bg(background)
            .font_weight(FontWeight::MEDIUM)
            .text_size(rems(text_size / 16.))
            .text_color(foreground)
            .opacity(if inactive { 0.5 } else { 1. })
            .cursor(if inactive {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::PointingHand
            })
            .when(!inactive, |button| {
                button.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if let Some(focus) = &mouse_focus {
                        focus.focus(window);
                    }
                    cx.stop_propagation();
                })
            })
            .focus_visible(|style| {
                style.shadow(focus_ring(match self.variant {
                    ButtonVariant::Primary => theme::accent(),
                    ButtonVariant::Warning => theme::warning(),
                    ButtonVariant::Secondary | ButtonVariant::Ghost => theme::border_strong(),
                    ButtonVariant::Danger => theme::danger(),
                }))
            });

        if !inactive {
            if let Some(on_press) = self.on_press {
                let click = Rc::clone(&on_press);
                button = button
                    .on_click(move |_, window, cx| click(window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if is_activation_key(event) {
                            cx.stop_propagation();
                            on_press(window, cx);
                        }
                    });
            }
        }

        let button = button
            .when(self.loading, |button| {
                button.child(spinner(spinner_id, icon_size, foreground))
            })
            .when_some(icon, |button, icon| {
                button.child(
                    svg()
                        .path(icon)
                        .size(rems(icon_size / 16.))
                        .text_color(foreground),
                )
            })
            .child(self.label);
        if let Some(content) = self.tooltip {
            let tooltip = Tooltip::new(tooltip_id, content, button);
            if let Some(focus_handle) = tooltip_focus {
                tooltip.focus_handle(focus_handle).into_any_element()
            } else {
                tooltip.into_any_element()
            }
        } else {
            button.into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_height_includes_the_border_box() {
        assert_eq!(ButtonSize::Sm.height(false), 24.);
        assert_eq!(ButtonSize::Sm.height(true), 26.);
        assert_eq!(ButtonSize::Md.height(false), 32.);
        assert_eq!(ButtonSize::Md.height(true), 34.);
    }
}

pub fn spinner(id: impl Into<ElementId>, size: f32, color: gpui::Hsla) -> AnyElement {
    svg()
        .path("loader.svg")
        .size(rems(size / 16.))
        .text_color(color)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(900)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IconButtonSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: SharedString,
    variant: ButtonVariant,
    size: IconButtonSize,
    disabled: bool,
    loading: bool,
    keyboard_activation: bool,
    stop_click_propagation: bool,
    reveal_on_group_hover: Option<SharedString>,
    focus_handle: Option<FocusHandle>,
    tooltip: Option<SharedString>,
    on_press: Option<PressHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            variant: ButtonVariant::Ghost,
            size: IconButtonSize::Md,
            disabled: false,
            loading: false,
            keyboard_activation: true,
            stop_click_propagation: false,
            reveal_on_group_hover: None,
            focus_handle: None,
            tooltip: None,
            on_press: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: IconButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn keyboard_activation(mut self, keyboard_activation: bool) -> Self {
        self.keyboard_activation = keyboard_activation;
        self
    }

    pub fn stop_click_propagation(mut self, stop: bool) -> Self {
        self.stop_click_propagation = stop;
        self
    }

    pub fn reveal_on_group_hover(mut self, group: impl Into<SharedString>) -> Self {
        self.reveal_on_group_hover = Some(group.into());
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_press(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (button_size, icon_size) = match self.size {
            IconButtonSize::Xs => (16., 12.),
            IconButtonSize::Sm => (24., 14.),
            IconButtonSize::Md => (28., 14.),
            IconButtonSize::Lg => (32., 16.),
        };
        let tooltip_focus = self.focus_handle.clone();
        let mouse_focus = self.focus_handle.clone();
        let tooltip_id = (self.id.clone(), "tooltip");
        let inactive = self.disabled || self.loading;
        let keyboard_activation = self.keyboard_activation;
        let stop_click_propagation = self.stop_click_propagation;
        let reveal_on_group_hover = self.reveal_on_group_hover;
        let spinner_id = (self.id.clone(), "loading");
        let foreground = match self.variant {
            ButtonVariant::Danger => theme::danger(),
            ButtonVariant::Primary => theme::accent_ink(),
            ButtonVariant::Warning => theme::bg(),
            ButtonVariant::Secondary => theme::text(),
            ButtonVariant::Ghost => theme::faint(),
        };
        let background = match self.variant {
            ButtonVariant::Primary => theme::accent(),
            ButtonVariant::Warning => theme::warning(),
            ButtonVariant::Secondary => theme::raised(),
            ButtonVariant::Ghost | ButtonVariant::Danger => gpui::transparent_black(),
        };
        let mut button = div()
            .id(self.id)
            .when_some(self.focus_handle, |button, handle| {
                button.track_focus(&handle)
            })
            .tab_index(0)
            .tab_stop(!inactive)
            .flex()
            .items_center()
            .justify_center()
            .size(rems(button_size / 16.))
            .rounded(rems(4. / 16.))
            .bg(background)
            .text_color(foreground)
            .opacity(if inactive {
                0.5
            } else if reveal_on_group_hover.is_some() {
                0.
            } else {
                1.
            })
            .when_some(reveal_on_group_hover, |button, group| {
                button.group_hover(group, |style| style.opacity(1.))
            })
            .cursor(if inactive {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::PointingHand
            })
            .when(!inactive, |button| {
                button.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if let Some(focus) = &mouse_focus {
                        focus.focus(window);
                    }
                    cx.stop_propagation();
                })
            })
            .focus_visible(|style| {
                style.opacity(1.).shadow(focus_ring(match self.variant {
                    ButtonVariant::Primary => theme::accent(),
                    ButtonVariant::Warning => theme::warning(),
                    ButtonVariant::Secondary | ButtonVariant::Ghost => theme::border_strong(),
                    ButtonVariant::Danger => theme::danger(),
                }))
            });
        if !inactive {
            button = button.hover(move |style| match self.variant {
                ButtonVariant::Danger => style
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .text_color(theme::danger()),
                ButtonVariant::Warning => style.opacity(0.9),
                _ => style.bg(theme::raised()).text_color(theme::text()),
            });
            if let Some(on_press) = self.on_press {
                let click = Rc::clone(&on_press);
                button = button.on_click(move |_, window, cx| {
                    if stop_click_propagation {
                        cx.stop_propagation();
                    }
                    click(window, cx);
                });
                if keyboard_activation {
                    button = button.on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if is_activation_key(event) {
                            cx.stop_propagation();
                            on_press(window, cx);
                        }
                    });
                }
            }
        }
        let button = if self.loading {
            button.child(spinner(spinner_id, icon_size, foreground))
        } else {
            button.child(
                svg()
                    .path(self.icon)
                    .size(rems(icon_size / 16.))
                    .text_color(foreground),
            )
        };
        if let Some(content) = self.tooltip {
            let tooltip = Tooltip::new(tooltip_id, content, button);
            if let Some(focus_handle) = tooltip_focus {
                tooltip.focus_handle(focus_handle).into_any_element()
            } else {
                tooltip.into_any_element()
            }
        } else {
            button.into_any_element()
        }
    }
}

pub fn is_activation_key(event: &KeyDownEvent) -> bool {
    matches!(event.keystroke.key.as_str(), "enter" | "space")
}

fn focus_ring(color: gpui::Hsla) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: theme::bg(),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(1.),
        },
        BoxShadow {
            color,
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(3.),
        },
    ]
}

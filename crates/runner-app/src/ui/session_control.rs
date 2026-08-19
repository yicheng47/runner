use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, App, BoxShadow, CursorStyle, ElementId, FocusHandle, FontWeight,
    KeyDownEvent, RenderOnce, SharedString, Window,
};

use crate::theme;
use crate::ui::button::{spinner, PressHandler};
use crate::ui::tooltip::Tooltip;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionControlVariant {
    #[default]
    Pill,
    Header,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionControlKind {
    Resume,
    Resuming,
    Stop,
    Back,
}

#[derive(IntoElement)]
pub struct SessionControl {
    id: ElementId,
    kind: SessionControlKind,
    variant: SessionControlVariant,
    label: Option<SharedString>,
    title: Option<SharedString>,
    back_icon: Option<SharedString>,
    stop_icon_danger: bool,
    lifecycle_disabled: bool,
    focus_handle: Option<FocusHandle>,
    on_press: Option<PressHandler>,
}

impl SessionControl {
    pub fn new(id: impl Into<ElementId>, kind: SessionControlKind) -> Self {
        Self {
            id: id.into(),
            kind,
            variant: SessionControlVariant::Pill,
            label: None,
            title: None,
            back_icon: None,
            stop_icon_danger: true,
            lifecycle_disabled: false,
            focus_handle: None,
            on_press: None,
        }
    }

    pub fn variant(mut self, variant: SessionControlVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn back_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.back_icon = Some(icon.into());
        self
    }

    pub fn stop_icon_danger(mut self, danger: bool) -> Self {
        self.stop_icon_danger = danger;
        self
    }

    pub fn lifecycle_disabled(mut self, disabled: bool) -> Self {
        self.lifecycle_disabled = disabled;
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn on_press(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(handler));
        self
    }

    fn default_label(&self) -> &'static str {
        match self.kind {
            SessionControlKind::Resume => "Resume",
            SessionControlKind::Resuming => "Resuming…",
            SessionControlKind::Stop => "Stop",
            SessionControlKind::Back => "Back to runner",
        }
    }
}

impl RenderOnce for SessionControl {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let label = self
            .label
            .clone()
            .unwrap_or_else(|| SharedString::from(self.default_label()));
        let header =
            self.variant == SessionControlVariant::Header && self.kind != SessionControlKind::Back;
        let tooltip = self.title.clone().or_else(|| header.then(|| label.clone()));
        let tooltip_focus = self.focus_handle.clone();
        let tooltip_id = (self.id.clone(), "tooltip");
        let automatically_disabled = self.kind == SessionControlKind::Resuming;
        let disabled = self.lifecycle_disabled || automatically_disabled;
        let icon: Option<SharedString> = match self.kind {
            SessionControlKind::Resume => Some("play.svg".into()),
            SessionControlKind::Resuming => Some("loader.svg".into()),
            SessionControlKind::Stop => Some("square.svg".into()),
            SessionControlKind::Back => self.back_icon.clone(),
        };
        let spinner_id = (self.id.clone(), "loading");
        let (border, background, foreground, icon_color) = match self.kind {
            SessionControlKind::Resume => (
                theme::with_alpha(theme::accent(), 0.4),
                theme::with_alpha(theme::accent(), 0.1),
                theme::accent(),
                theme::accent(),
            ),
            SessionControlKind::Resuming => (
                theme::with_alpha(theme::info(), 0.4),
                theme::with_alpha(theme::info(), 0.1),
                theme::info(),
                theme::info(),
            ),
            SessionControlKind::Stop => (
                theme::border(),
                theme::raised(),
                theme::text(),
                if self.stop_icon_danger {
                    theme::danger()
                } else {
                    theme::text()
                },
            ),
            SessionControlKind::Back => (
                theme::border(),
                theme::raised(),
                theme::text(),
                theme::muted(),
            ),
        };
        let icon_color = if header {
            match self.kind {
                SessionControlKind::Resume => theme::with_alpha(theme::accent(), 0.8),
                SessionControlKind::Stop => theme::with_alpha(theme::danger(), 0.8),
                _ => icon_color,
            }
        } else {
            icon_color
        };
        let icon_size = if header { 13. } else { 12. };
        let icon_element = match self.kind {
            SessionControlKind::Resuming => Some(spinner(spinner_id, icon_size, icon_color)),
            _ => icon.map(|icon| {
                svg()
                    .path(icon)
                    .size(rems(icon_size / 16.))
                    .text_color(icon_color)
                    .into_any_element()
            }),
        };
        let mut control = div()
            .id(self.id)
            .when_some(self.focus_handle, |control, focus| {
                control.track_focus(&focus)
            })
            .tab_index(0)
            .tab_stop(!disabled)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(6. / 16.))
            .when(header, |control| {
                control.size(rems(28. / 16.)).rounded(rems(4. / 16.))
            })
            .when(!header, |control| {
                control
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(border)
                    .bg(background)
            })
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(rems(11. / 16.))
            .text_color(if header {
                match self.kind {
                    SessionControlKind::Resume => theme::with_alpha(theme::accent(), 0.8),
                    SessionControlKind::Resuming => theme::info(),
                    SessionControlKind::Stop => theme::with_alpha(theme::danger(), 0.8),
                    SessionControlKind::Back => theme::muted(),
                }
            } else {
                foreground
            })
            .opacity(if self.lifecycle_disabled { 0.5 } else { 1. })
            .cursor(if disabled {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::PointingHand
            })
            .focus_visible(|control| {
                control.shadow(vec![BoxShadow {
                    color: theme::border_strong(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                }])
            })
            .children(icon_element)
            .when(!header, |control| control.child(label));
        if !disabled {
            control = control.hover(move |control| match self.kind {
                SessionControlKind::Resume if header => control
                    .bg(theme::with_alpha(theme::accent(), 0.1))
                    .text_color(theme::accent()),
                SessionControlKind::Resume => control.border_color(theme::accent()),
                SessionControlKind::Stop if header => control
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .text_color(theme::danger()),
                _ => control.border_color(theme::border_strong()),
            });
            if let Some(handler) = self.on_press {
                let click = Rc::clone(&handler);
                control = control
                    .on_click(move |_, window, cx| click(window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            handler(window, cx);
                        }
                    });
            }
        }
        if let Some(tooltip) = tooltip {
            let tooltip = Tooltip::new(tooltip_id, tooltip, control);
            if let Some(focus) = tooltip_focus {
                tooltip.focus_handle(focus).into_any_element()
            } else {
                tooltip.into_any_element()
            }
        } else {
            control.into_any_element()
        }
    }
}

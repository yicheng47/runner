use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, App, BoxShadow, CursorStyle, ElementId, KeyDownEvent, RenderOnce, Window,
};

use crate::theme;

pub type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    on: bool,
    disabled: bool,
    on_change: Option<ToggleHandler>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            disabled: false,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let handler = self.on_change;
        let disabled = self.disabled;
        let on = self.on;
        let mut toggle = div()
            .id(self.id)
            .tab_index(0)
            .tab_stop(!disabled)
            .flex()
            .items_center()
            .justify_start()
            .when(on, |toggle| toggle.justify_end())
            .size_full()
            .w(rems(32. / 16.))
            .h(rems(18. / 16.))
            .p(rems(2. / 16.))
            .rounded_full()
            .bg(if on {
                theme::with_alpha(theme::accent(), 0.15)
            } else {
                theme::raised()
            })
            .opacity(if disabled { 0.5 } else { 1. })
            .cursor(if disabled {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::PointingHand
            })
            .focus_visible(|style| {
                style.shadow(vec![BoxShadow {
                    color: theme::border_strong(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                }])
            })
            .child(div().size(rems(14. / 16.)).rounded_full().bg(if on {
                theme::accent()
            } else {
                theme::faint()
            }));
        if !disabled {
            if let Some(handler) = handler {
                let click = Rc::clone(&handler);
                toggle = toggle
                    .on_click(move |_, window, cx| click(!on, window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            handler(!on, window, cx);
                        }
                    });
            }
        }
        toggle
    }
}

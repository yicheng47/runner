use gpui::prelude::*;
use gpui::{
    anchored, deferred, div, point, px, rems, AnchoredPositionMode, AnyElement, AnyView, App,
    Corner, ElementId, FocusHandle, FontWeight, IntoElement, Render, RenderOnce, SharedString,
    Window,
};

use crate::theme;
use crate::ui::app_zoom;

struct TooltipView {
    content: SharedString,
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        tooltip_content(self.content.clone())
    }
}

/// A standalone tooltip view for elements that drive `Window::set_tooltip`
/// themselves instead of going through `Tooltip`.
pub fn tooltip_view(content: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let content = content.into();
    cx.new(|_| TooltipView { content }).into()
}

#[derive(IntoElement)]
pub struct Tooltip {
    id: ElementId,
    content: SharedString,
    child: AnyElement,
    focus_handle: Option<FocusHandle>,
    expand: bool,
}

impl Tooltip {
    pub fn new(
        id: impl Into<ElementId>,
        content: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            child: child.into_any_element(),
            focus_handle: None,
            expand: false,
        }
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn expand(mut self) -> Self {
        self.expand = true;
        self
    }
}

impl RenderOnce for Tooltip {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let zoom = app_zoom(window);
        let hover_content = self.content.clone();
        let expand = self.expand;
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        div()
            .flex()
            .id(self.id)
            .relative()
            .when(expand, |trigger| trigger.min_w(px(0.)).flex_1())
            .when(!focused, |trigger| {
                trigger.tooltip(move |_, cx| {
                    cx.new(|_| TooltipView {
                        content: hover_content.clone(),
                    })
                    .into()
                })
            })
            .child(self.child)
            .when(focused, |trigger| {
                trigger.child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .position_mode(AnchoredPositionMode::Local)
                            .offset(point(px(0.), px(-6. * zoom)))
                            .child(tooltip_content(self.content)),
                    )
                    .with_priority(60),
                )
            })
    }
}

fn tooltip_content(content: SharedString) -> impl IntoElement {
    div()
        .max_w(rems(320. / 16.))
        .px_2()
        .py_1()
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::raised())
        .shadow_lg()
        .font_weight(FontWeight::NORMAL)
        .text_size(rems(11. / 16.))
        .line_height(rems(15. / 16.))
        .text_color(theme::muted())
        .child(content)
}

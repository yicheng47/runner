use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, App, Bounds, Context, CursorStyle, DragMoveEvent, EntityId, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollHandle, Window,
};

use crate::theme;
use runner_terminal::terminal::TerminalSession;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollbarKind {
    App,
    Terminal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollbarMetrics {
    pub viewport: f32,
    pub content: f32,
    pub position: f32,
}

impl ScrollbarMetrics {
    fn thumb(self, track_height: f32) -> Option<(f32, f32)> {
        if self.viewport <= 0. || self.content <= self.viewport {
            return None;
        }
        let height = (track_height * self.viewport / self.content)
            .max(20.)
            .min(track_height);
        let max_position = (self.content - self.viewport).max(0.);
        let ratio = if max_position == 0. {
            0.
        } else {
            (self.position / max_position).clamp(0., 1.)
        };
        Some(((track_height - height) * ratio, height))
    }
}

type MetricsReader = Rc<dyn Fn() -> ScrollbarMetrics>;
type ScrollWriter = Rc<dyn Fn(f32, &mut App)>;

#[derive(Clone)]
struct ScrollbarDrag;

impl Render for ScrollbarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.))
    }
}

pub struct Scrollbar {
    kind: ScrollbarKind,
    metrics: MetricsReader,
    scroll_to: ScrollWriter,
    track_bounds: Bounds<Pixels>,
    last_metrics: ScrollbarMetrics,
    drag_grab: Option<Pixels>,
}

impl Scrollbar {
    pub fn app(handle: ScrollHandle, owner: EntityId) -> Self {
        let read_handle = handle.clone();
        let write_handle = handle;
        Self::new(
            ScrollbarKind::App,
            Rc::new(move || {
                let viewport = f32::from(read_handle.bounds().size.height);
                let maximum = f32::from(read_handle.max_offset().height).max(0.);
                ScrollbarMetrics {
                    viewport,
                    content: viewport + maximum,
                    position: (-f32::from(read_handle.offset().y)).clamp(0., maximum),
                }
            }),
            Rc::new(move |position, cx| {
                let offset = write_handle.offset();
                let maximum = f32::from(write_handle.max_offset().height).max(0.);
                write_handle.set_offset(Point::new(offset.x, px(-position.clamp(0., maximum))));
                cx.notify(owner);
            }),
        )
    }

    pub fn external(kind: ScrollbarKind, metrics: MetricsReader, scroll_to: ScrollWriter) -> Self {
        Self::new(kind, metrics, scroll_to)
    }

    pub fn terminal(terminal: Arc<TerminalSession>) -> Self {
        let read_terminal = Arc::clone(&terminal);
        Self::new(
            ScrollbarKind::Terminal,
            Rc::new(move || {
                let state = read_terminal.scroll_state();
                ScrollbarMetrics {
                    viewport: state.screen_lines as f32,
                    content: (state.screen_lines + state.history_lines) as f32,
                    position: state.history_lines.saturating_sub(state.display_offset) as f32,
                }
            }),
            Rc::new(move |position, _| {
                let state = terminal.scroll_state();
                let from_top = position.round().clamp(0., state.history_lines as f32) as usize;
                terminal.scroll_to_display_offset(state.history_lines.saturating_sub(from_top));
            }),
        )
    }

    fn new(kind: ScrollbarKind, metrics: MetricsReader, scroll_to: ScrollWriter) -> Self {
        Self {
            kind,
            metrics,
            scroll_to,
            track_bounds: Bounds::default(),
            last_metrics: ScrollbarMetrics::default(),
            drag_grab: None,
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let track_height = f32::from(self.track_bounds.size.height);
        let Some((thumb_top, thumb_height)) = (self.metrics)().thumb(track_height) else {
            return;
        };
        let local = event.position.y - self.track_bounds.origin.y;
        let local_value = f32::from(local);
        self.drag_grab = Some(
            if local_value >= thumb_top && local_value <= thumb_top + thumb_height {
                px(local_value - thumb_top)
            } else {
                px(thumb_height / 2.)
            },
        );
        self.scroll_for_pointer(event.position.y, cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_drag_move(
        &mut self,
        event: &DragMoveEvent<ScrollbarDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag_grab.is_some() {
            self.scroll_for_pointer(event.event.position.y, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn scroll_for_pointer(&self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let metrics = (self.metrics)();
        let track_height = f32::from(self.track_bounds.size.height);
        let Some((_, thumb_height)) = metrics.thumb(track_height) else {
            return;
        };
        let grab = f32::from(self.drag_grab.unwrap_or(px(thumb_height / 2.)));
        let top = f32::from(pointer_y - self.track_bounds.origin.y) - grab;
        let available = (track_height - thumb_height).max(1.);
        let ratio = (top / available).clamp(0., 1.);
        (self.scroll_to)(ratio * (metrics.content - metrics.viewport).max(0.), cx);
    }
}

impl Render for Scrollbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = (self.metrics)();
        let track_height = f32::from(self.track_bounds.size.height);
        let thumb = metrics.thumb(track_height);
        let gutter = match self.kind {
            ScrollbarKind::App => 10.,
            ScrollbarKind::Terminal => 8.,
        };
        let inset = match self.kind {
            ScrollbarKind::App => 2.,
            ScrollbarKind::Terminal => 1.,
        };
        let entity = cx.entity();
        div()
            .id("theme-scrollbar")
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(px(gutter))
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_drag(ScrollbarDrag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_drag_move::<ScrollbarDrag>(cx.listener(Self::on_drag_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_grab = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_grab = None;
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        entity.update(cx, |scrollbar, cx| {
                            let metrics = (scrollbar.metrics)();
                            if scrollbar.track_bounds != bounds || scrollbar.last_metrics != metrics
                            {
                                scrollbar.track_bounds = bounds;
                                scrollbar.last_metrics = metrics;
                                cx.notify();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(thumb.map(|(top, height)| {
                div()
                    .absolute()
                    .top(px(top))
                    .left(px(inset))
                    .right(px(inset))
                    .h(px(height))
                    .rounded_full()
                    .bg(theme::border_strong())
                    .hover(|thumb| thumb.bg(theme::faint()))
                    .into_any_element()
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollbar_thumb_tracks_viewport_and_position() {
        let metrics = ScrollbarMetrics {
            viewport: 100.,
            content: 400.,
            position: 150.,
        };
        assert_eq!(metrics.thumb(100.), Some((37.5, 25.)));
        assert_eq!(
            ScrollbarMetrics {
                content: 100.,
                ..metrics
            }
            .thumb(100.),
            None
        );
    }
}

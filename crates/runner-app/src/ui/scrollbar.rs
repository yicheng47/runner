use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, App, Bounds, Context, CursorStyle, DragMoveEvent, EntityId, Hsla,
    MouseButton, MouseDownEvent, Pixels, Point, Rems, Render, ScrollHandle, Window,
};

use crate::theme;
use crate::ui::app_zoom;
use runner_terminal::terminal::TerminalSession;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollbarKind {
    App,
    Terminal,
}

impl ScrollbarKind {
    /// Track width in logical pixels at 100% zoom; laid out in rems so it
    /// follows the app zoom.
    fn gutter(self) -> f32 {
        match self {
            ScrollbarKind::App => 10.,
            ScrollbarKind::Terminal => 8.,
        }
    }

    /// Thumb colors as (rest, active); active covers hover, drag, and the
    /// scroll-activity linger. App panels need a readable resting thumb (#470)
    /// without going as bright as secondary text; the terminal keeps its
    /// darker palette so the thumb never competes with the grid (#520).
    fn thumb_colors(self) -> (Hsla, Hsla) {
        match self {
            ScrollbarKind::App => (
                theme::with_alpha(theme::faint(), 0.7),
                theme::with_alpha(theme::muted(), 0.6),
            ),
            ScrollbarKind::Terminal => (theme::border_strong(), theme::faint()),
        }
    }
}

/// Width the terminal wrappers reserve at their right edge so the grid is
/// measured beside the scrollbar, not under it.
pub fn terminal_scrollbar_gutter() -> Rems {
    rems(ScrollbarKind::Terminal.gutter() / 16.)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollbarMetrics {
    pub viewport: f32,
    pub content: f32,
    pub position: f32,
}

impl ScrollbarMetrics {
    fn thumb(self, track_height: f32, min_height: f32) -> Option<(f32, f32)> {
        // Fractional rems leave containers a sub-pixel short of their padded
        // content; that is rounding, not something to scroll.
        if self.viewport <= 0. || self.content - self.viewport < 1. {
            return None;
        }
        let height = (track_height * self.viewport / self.content)
            .max(min_height)
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

/// How long the thumb stays in its bright state after the list stops moving,
/// the way macOS overlay scrollbars linger before fading.
const SCROLL_ACTIVITY_LINGER: Duration = Duration::from_millis(900);

pub struct Scrollbar {
    kind: ScrollbarKind,
    metrics: MetricsReader,
    scroll_to: ScrollWriter,
    track_bounds: Bounds<Pixels>,
    last_metrics: ScrollbarMetrics,
    drag_grab: Option<Pixels>,
    min_thumb: f32,
    scrolling: bool,
    activity_generation: u64,
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
            min_thumb: 20.,
            scrolling: false,
            activity_generation: 0,
        }
    }

    fn mark_scrolling(&mut self, cx: &mut Context<Self>) {
        self.scrolling = true;
        self.activity_generation = self.activity_generation.wrapping_add(1);
        let generation = self.activity_generation;
        cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(SCROLL_ACTIVITY_LINGER).await;
            let _ = weak.update(cx, |scrollbar, cx| {
                if scrollbar.activity_generation == generation {
                    scrollbar.scrolling = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let track_height = f32::from(self.track_bounds.size.height);
        let Some((thumb_top, thumb_height)) = (self.metrics)().thumb(track_height, self.min_thumb)
        else {
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
        let Some((_, thumb_height)) = metrics.thumb(track_height, self.min_thumb) else {
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = app_zoom(window);
        self.min_thumb = 20. * zoom;
        let metrics = (self.metrics)();
        let track_height = f32::from(self.track_bounds.size.height);
        let thumb = metrics.thumb(track_height, self.min_thumb);
        let gutter = self.kind.gutter();
        let inset = match self.kind {
            ScrollbarKind::App => 2.,
            ScrollbarKind::Terminal => 1.,
        };
        let entity = cx.entity();
        let (rest_color, active_color) = self.kind.thumb_colors();
        let thumb_color = if self.scrolling || self.drag_grab.is_some() {
            active_color
        } else {
            rest_color
        };
        div()
            .id("theme-scrollbar")
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .w(rems(gutter / 16.))
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
                                if metrics.position != scrollbar.last_metrics.position {
                                    scrollbar.mark_scrolling(cx);
                                }
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
                    .left(rems(inset / 16.))
                    .right(rems(inset / 16.))
                    .h(px(height))
                    .rounded_full()
                    .bg(thumb_color)
                    .hover(move |thumb| thumb.bg(active_color))
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
        assert_eq!(metrics.thumb(100., 20.), Some((37.5, 25.)));
        assert_eq!(
            ScrollbarMetrics {
                content: 100.,
                ..metrics
            }
            .thumb(100., 20.),
            None
        );
    }

    #[test]
    fn sub_pixel_overflow_from_rounding_shows_no_thumb() {
        let metrics = ScrollbarMetrics {
            viewport: 238.,
            content: 238.4,
            position: 0.,
        };
        assert_eq!(metrics.thumb(238., 20.), None);
        assert!(ScrollbarMetrics {
            content: 239.,
            ..metrics
        }
        .thumb(238., 20.)
        .is_some());
    }
}

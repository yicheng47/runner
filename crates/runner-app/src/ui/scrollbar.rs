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
use runner_terminal::terminal::TerminalMirror;

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
    /// scroll-activity linger. App thumbs use the active color whenever shown;
    /// terminal thumbs retain their darker resting color.
    fn thumb_colors(self) -> (Hsla, Hsla) {
        match self {
            ScrollbarKind::App => (
                theme::with_alpha(theme::muted(), 0.6),
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

/// Width an app list reserves at its right edge so its rows sit beside the
/// scrollbar, not under it.
pub fn app_scrollbar_gutter() -> Rems {
    rems(ScrollbarKind::App.gutter() / 16.)
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

/// How long the thumb stays active after scrolling or leaving an app list.
const SCROLL_ACTIVITY_LINGER: Duration = Duration::from_millis(900);

pub struct Scrollbar {
    kind: ScrollbarKind,
    horizontal: bool,
    metrics: MetricsReader,
    scroll_to: ScrollWriter,
    track_bounds: Bounds<Pixels>,
    last_metrics: ScrollbarMetrics,
    drag_grab: Option<Pixels>,
    min_thumb: f32,
    scrolling: bool,
    list_hovered: bool,
    strip_hovered: bool,
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
                let maximum = f32::from(read_handle.max_offset().y).max(0.);
                ScrollbarMetrics {
                    viewport,
                    content: viewport + maximum,
                    position: (-f32::from(read_handle.offset().y)).clamp(0., maximum),
                }
            }),
            Rc::new(move |position, cx| {
                let offset = write_handle.offset();
                let maximum = f32::from(write_handle.max_offset().y).max(0.);
                write_handle.set_offset(Point::new(offset.x, px(-position.clamp(0., maximum))));
                cx.notify(owner);
            }),
        )
    }

    pub fn external(kind: ScrollbarKind, metrics: MetricsReader, scroll_to: ScrollWriter) -> Self {
        Self::new(kind, metrics, scroll_to)
    }

    pub fn terminal(terminal: Arc<TerminalMirror>) -> Self {
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
            horizontal: false,
            metrics,
            scroll_to,
            track_bounds: Bounds::default(),
            last_metrics: ScrollbarMetrics::default(),
            drag_grab: None,
            min_thumb: 20.,
            scrolling: false,
            list_hovered: false,
            strip_hovered: false,
            activity_generation: 0,
        }
    }

    /// Lies along the bottom edge and scrolls sideways: its metrics measure
    /// the width.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    fn track_length(&self) -> f32 {
        f32::from(if self.horizontal {
            self.track_bounds.size.width
        } else {
            self.track_bounds.size.height
        })
    }

    fn along_track(&self, position: Point<Pixels>) -> f32 {
        f32::from(if self.horizontal {
            position.x - self.track_bounds.origin.x
        } else {
            position.y - self.track_bounds.origin.y
        })
    }

    fn mark_activity(&mut self, cx: &mut Context<Self>) {
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

    pub fn is_visible(&self) -> bool {
        self.kind == ScrollbarKind::Terminal
            || self.list_hovered
            || self.scrolling
            || self.drag_grab.is_some()
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_visible() {
            return;
        }
        let Some((thumb_start, thumb_length)) =
            (self.metrics)().thumb(self.track_length(), self.min_thumb)
        else {
            return;
        };
        let local = self.along_track(event.position);
        self.drag_grab = Some(
            if local >= thumb_start && local <= thumb_start + thumb_length {
                px(local - thumb_start)
            } else {
                px(thumb_length / 2.)
            },
        );
        self.scroll_for_pointer(event.position, cx);
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
            self.scroll_for_pointer(event.event.position, cx);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn scroll_for_pointer(&self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let metrics = (self.metrics)();
        let track_length = self.track_length();
        let Some((_, thumb_length)) = metrics.thumb(track_length, self.min_thumb) else {
            return;
        };
        let grab = f32::from(self.drag_grab.unwrap_or(px(thumb_length / 2.)));
        let start = self.along_track(pointer) - grab;
        let available = (track_length - thumb_length).max(1.);
        let ratio = (start / available).clamp(0., 1.);
        (self.scroll_to)(ratio * (metrics.content - metrics.viewport).max(0.), cx);
    }
}

impl Render for Scrollbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = app_zoom(window);
        self.min_thumb = 20. * zoom;
        let metrics = (self.metrics)();
        let thumb = metrics.thumb(self.track_length(), self.min_thumb);
        let gutter = self.kind.gutter();
        let horizontal = self.horizontal;
        let (track_selector, thumb_selector) = if horizontal {
            ("SCROLLBAR_TRACK_X", "SCROLLBAR_THUMB_X")
        } else {
            ("SCROLLBAR_TRACK", "SCROLLBAR_THUMB")
        };
        let (inner, outer) = match self.kind {
            ScrollbarKind::App if self.strip_hovered || self.drag_grab.is_some() => (1., 3.),
            ScrollbarKind::App => (3., 3.),
            ScrollbarKind::Terminal => (1., 1.),
        };
        let visible = self.is_visible() && thumb.is_some();
        let entity = cx.entity();
        let (rest_color, active_color) = self.kind.thumb_colors();
        let thumb_color = if self.scrolling || self.drag_grab.is_some() {
            active_color
        } else {
            rest_color
        };
        let track = div()
            .id("theme-scrollbar")
            .debug_selector(move || track_selector.into())
            .absolute()
            .bottom_0()
            .right_0()
            .when(horizontal, |track| track.left_0().h(rems(gutter / 16.)))
            .when(!horizontal, |track| track.top_0().w(rems(gutter / 16.)))
            .when(self.kind == ScrollbarKind::Terminal || visible, |track| {
                track
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
            })
            .when(self.kind == ScrollbarKind::App && visible, |track| {
                track
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.strip_hovered != *hovered {
                            this.strip_hovered = *hovered;
                            cx.notify();
                        }
                    }))
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        entity.update(cx, |scrollbar, cx| {
                            let metrics = (scrollbar.metrics)();
                            if scrollbar.track_bounds != bounds || scrollbar.last_metrics != metrics
                            {
                                if metrics.position != scrollbar.last_metrics.position {
                                    scrollbar.mark_activity(cx);
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
            .children(thumb.filter(|_| visible).map(|(start, length)| {
                div()
                    .absolute()
                    .when(horizontal, |thumb| {
                        thumb
                            .left(px(start))
                            .w(px(length))
                            .top(rems(inner / 16.))
                            .bottom(rems(outer / 16.))
                    })
                    .when(!horizontal, |thumb| {
                        thumb
                            .top(px(start))
                            .h(px(length))
                            .left(rems(inner / 16.))
                            .right(rems(outer / 16.))
                    })
                    .rounded_full()
                    .debug_selector(move || thumb_selector.into())
                    .bg(thumb_color)
                    .map(|thumb| {
                        #[cfg(test)]
                        let thumb = crate::theme_snapshot::record_fill(thumb_selector, thumb);
                        thumb
                    })
                    .hover(move |thumb| thumb.bg(active_color))
                    .into_any_element()
            }));
        div()
            .id("scrollbar-list-hover")
            .absolute()
            .bottom_0()
            .right_0()
            .when(horizontal, |list| list.left_0().h(rems(gutter / 16.)))
            .when(!horizontal, |list| list.top_0().w(rems(gutter / 16.)))
            .when(self.kind == ScrollbarKind::App, |list| {
                list.top_0()
                    .left_0()
                    .size_full()
                    .hover_listener_mode(gpui::HoverListenerMode::InputModalityIndependent)
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.list_hovered != *hovered {
                            this.list_hovered = *hovered;
                            if !hovered {
                                this.mark_activity(cx);
                            }
                            cx.notify();
                        }
                    }))
            })
            .child(track)
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

    struct ScrollbarHost {
        scroll: ScrollHandle,
        bar: gpui::Entity<Scrollbar>,
        presses: Rc<std::cell::Cell<usize>>,
        zoom: f32,
    }

    impl Render for ScrollbarHost {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            window.set_rem_size(px(16. * self.zoom));
            let presses = self.presses.clone();
            div().size_full().p(px(20.)).child(
                div()
                    .debug_selector(|| "SCROLLBAR_LIST".into())
                    .relative()
                    .w(rems(300. / 16.))
                    .h(rems(200. / 16.))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        presses.set(presses.get() + 1)
                    })
                    .child(
                        div()
                            .id("scroll-content")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(div().h(rems(600. / 16.))),
                    )
                    .child(self.bar.clone()),
            )
        }
    }

    fn open(
        kind: ScrollbarKind,
        zoom: f32,
    ) -> (
        gpui::VisualTestContext,
        gpui::Entity<Scrollbar>,
        Rc<std::cell::Cell<usize>>,
    ) {
        let mut cx = gpui::TestAppContext::single();
        let presses = Rc::new(std::cell::Cell::new(0));
        let host = cx.add_window(|_, cx| {
            let scroll = ScrollHandle::new();
            let owner = cx.entity_id();
            let bar = cx.new(|_| match kind {
                ScrollbarKind::App => Scrollbar::app(scroll.clone(), owner),
                ScrollbarKind::Terminal => Scrollbar::external(
                    kind,
                    Rc::new(|| ScrollbarMetrics {
                        viewport: 100.,
                        content: 400.,
                        position: 0.,
                    }),
                    Rc::new(|_, _| {}),
                ),
            });
            ScrollbarHost {
                scroll,
                bar,
                presses: presses.clone(),
                zoom,
            }
        });
        let bar = host.read_with(&cx, |host, _| host.bar.clone()).unwrap();
        let visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        (visual, bar, presses)
    }

    fn assert_thumb(
        visual: &mut gpui::VisualTestContext,
        width: f32,
        right: f32,
        zoom: f32,
        color: Hsla,
    ) {
        let track = visual.debug_bounds("SCROLLBAR_TRACK").unwrap();
        let thumb = visual.debug_bounds("SCROLLBAR_THUMB").unwrap();
        assert!((thumb.size.width - px(width * zoom)).abs() < px(0.5));
        assert!((track.right() - thumb.right() - px(right * zoom)).abs() < px(0.5));
        crate::theme_snapshot::assert_fill_color(visual, "SCROLLBAR_THUMB", Some(color));
    }

    #[test]
    fn app_scrollbar_hover_uses_the_list_then_the_strip_and_lingers_on_leave() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::RunnerLight,
        ] {
            theme::set_active_variant(variant);
            for zoom in [1., 1.3] {
                let (mut visual, bar, _) = open(ScrollbarKind::App, zoom);
                let list = visual.debug_bounds("SCROLLBAR_LIST").unwrap();
                let track = visual.debug_bounds("SCROLLBAR_TRACK").unwrap();
                assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
                assert!((track.size.width - px(10. * zoom)).abs() < px(0.5));
                visual.simulate_mouse_move(list.center(), None, Default::default());
                visual.run_until_parked();
                assert_thumb(
                    &mut visual,
                    4.,
                    3.,
                    zoom,
                    theme::with_alpha(theme::muted(), 0.6),
                );
                visual.simulate_mouse_move(track.center(), None, Default::default());
                visual.run_until_parked();
                assert_thumb(
                    &mut visual,
                    6.,
                    3.,
                    zoom,
                    theme::with_alpha(theme::muted(), 0.6),
                );
                visual.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, Default::default());
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_millis(899));
                visual.run_until_parked();
                assert!(bar.read_with(&visual, |bar, _| bar.is_visible()));
                assert_thumb(
                    &mut visual,
                    4.,
                    3.,
                    zoom,
                    theme::with_alpha(theme::muted(), 0.6),
                );
                visual.executor().advance_clock(Duration::from_millis(1));
                visual.run_until_parked();
                assert!(!bar.read_with(&visual, |bar, _| bar.is_visible()));
                assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
                assert_eq!(visual.debug_bounds("SCROLLBAR_LIST"), Some(list));
            }
        }
    }

    #[test]
    fn app_scrollbar_hidden_track_passes_clicks_and_scroll_refreshes_linger() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (mut visual, bar, presses) = open(ScrollbarKind::App, 1.);
        let track = visual.debug_bounds("SCROLLBAR_TRACK").unwrap();
        visual.simulate_mouse_down(track.center(), MouseButton::Left, Default::default());
        assert_eq!(presses.get(), 1);
        assert!(bar.read_with(&visual, |bar, _| bar.drag_grab.is_none()));
        visual.simulate_mouse_up(track.center(), MouseButton::Left, Default::default());
        visual.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, Default::default());
        visual.run_until_parked();
        visual.executor().advance_clock(SCROLL_ACTIVITY_LINGER);
        visual.run_until_parked();
        assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
        for position in [50., 100.] {
            bar.update(&mut visual, |bar, cx| {
                (bar.scroll_to)(position, cx);
                cx.notify();
            });
            visual.run_until_parked();
            visual.update(|window, _| window.refresh());
            visual.run_until_parked();
            bar.read_with(&visual, |bar, _| {
                assert!(
                    bar.scrolling,
                    "scroll position={position}, metrics={:?}, last={:?}",
                    (bar.metrics)(),
                    bar.last_metrics
                );
            });
            assert_thumb(
                &mut visual,
                4.,
                3.,
                1.,
                theme::with_alpha(theme::muted(), 0.6),
            );
            visual.executor().advance_clock(Duration::from_millis(500));
            visual.run_until_parked();
        }
        assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_some());
        visual.executor().advance_clock(Duration::from_millis(399));
        visual.run_until_parked();
        assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_some());
        visual.executor().advance_clock(Duration::from_millis(1));
        visual.run_until_parked();
        assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
    }

    #[test]
    fn app_scrollbar_drag_keeps_the_wide_thumb_visible_after_linger() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (mut visual, bar, _) = open(ScrollbarKind::App, 1.3);
        let list = visual.debug_bounds("SCROLLBAR_LIST").unwrap();
        visual.simulate_mouse_move(list.center(), None, Default::default());
        let thumb = visual.debug_bounds("SCROLLBAR_THUMB").unwrap();
        visual.simulate_mouse_down(thumb.center(), MouseButton::Left, Default::default());
        visual.simulate_mouse_move(
            gpui::point(px(0.), px(0.)),
            Some(MouseButton::Left),
            Default::default(),
        );
        visual.executor().advance_clock(Duration::from_secs(1));
        visual.run_until_parked();
        assert!(bar.read_with(&visual, |bar, _| bar.drag_grab.is_some()));
        assert_thumb(
            &mut visual,
            6.,
            3.,
            1.3,
            theme::with_alpha(theme::muted(), 0.6),
        );
        visual.simulate_mouse_up(
            gpui::point(px(0.), px(0.)),
            MouseButton::Left,
            Default::default(),
        );
        assert!(bar.read_with(&visual, |bar, _| bar.drag_grab.is_none()));
    }

    #[test]
    fn app_scrollbar_hover_survives_keyboard_until_the_pointer_leaves() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (mut visual, _, _) = open(ScrollbarKind::App, 1.);
        let list = visual.debug_bounds("SCROLLBAR_LIST").unwrap();
        let track = visual.debug_bounds("SCROLLBAR_TRACK").unwrap();
        for (at, width) in [(list.center(), 4.), (track.center(), 6.)] {
            visual.simulate_mouse_move(at, None, Default::default());
            visual.simulate_keystrokes("a");
            visual.executor().advance_clock(Duration::from_secs(1));
            visual.run_until_parked();
            assert_thumb(
                &mut visual,
                width,
                3.,
                1.,
                theme::with_alpha(theme::muted(), 0.6),
            );
        }
        visual.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, Default::default());
        visual.executor().advance_clock(Duration::from_millis(900));
        visual.run_until_parked();
        assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
    }

    struct HorizontalHost {
        bar: gpui::Entity<Scrollbar>,
    }

    impl Render for HorizontalHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().p(px(20.)).child(
                div()
                    .debug_selector(|| "SCROLLBAR_LIST".into())
                    .relative()
                    .w(px(300.))
                    .h(px(200.))
                    .child(self.bar.clone()),
            )
        }
    }

    #[test]
    fn a_horizontal_scrollbar_lies_along_the_bottom_and_drags_sideways() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut cx = gpui::TestAppContext::single();
        let position = Rc::new(std::cell::Cell::new(0.));
        let host = cx.add_window(|_, cx| {
            let read = position.clone();
            let write = position.clone();
            let bar = cx.new(|_| {
                Scrollbar::external(
                    ScrollbarKind::App,
                    Rc::new(move || ScrollbarMetrics {
                        viewport: 300.,
                        content: 1200.,
                        position: read.get(),
                    }),
                    Rc::new(move |value, _| write.set(value)),
                )
                .horizontal()
            });
            HorizontalHost { bar }
        });
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();

        let list = visual.debug_bounds("SCROLLBAR_LIST").unwrap();
        let track = visual.debug_bounds("SCROLLBAR_TRACK_X").unwrap();
        assert_eq!(track.size.width, list.size.width);
        assert_eq!(track.bottom(), list.bottom());
        assert!((track.size.height - px(10.)).abs() < px(0.5));
        assert!(visual.debug_bounds("SCROLLBAR_THUMB_X").is_none());

        visual.simulate_mouse_move(list.center(), None, Default::default());
        visual.run_until_parked();
        let thumb = visual.debug_bounds("SCROLLBAR_THUMB_X").unwrap();
        assert!((thumb.size.width - px(75.)).abs() < px(0.5), "a quarter");
        assert_eq!(thumb.left(), track.left());

        let target = thumb.center() + gpui::point(px(112.5), px(0.));
        visual.simulate_mouse_down(thumb.center(), MouseButton::Left, Default::default());
        // The first move starts the drag; the next one moves it.
        for _ in 0..2 {
            visual.simulate_mouse_move(target, Some(MouseButton::Left), Default::default());
        }
        visual.simulate_mouse_up(target, MouseButton::Left, Default::default());
        assert!((position.get() - 450_f32).abs() < 1., "{}", position.get());
    }

    #[test]
    fn terminal_scrollbar_keeps_its_resting_thumb_and_geometry() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for zoom in [1., 1.3] {
            let (mut visual, _, _) = open(ScrollbarKind::Terminal, zoom);
            assert_thumb(&mut visual, 6., 1., zoom, theme::border_strong());
            let track = visual.debug_bounds("SCROLLBAR_TRACK").unwrap();
            assert!((track.size.width - px(8. * zoom)).abs() < px(0.5));
            visual.executor().advance_clock(Duration::from_secs(2));
            visual.run_until_parked();
            assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_some());
        }
    }
}

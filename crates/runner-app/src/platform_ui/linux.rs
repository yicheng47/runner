#[cfg(target_os = "linux")]
pub(crate) use super::desktop::*;

use gpui::{
    div, point, prelude::*, px, size, App, Bounds, CursorStyle, Decorations, Div, Edges,
    MouseButton, Pixels, Point, ResizeEdge, Size, Window,
};
use std::rc::Rc;

#[cfg(target_os = "linux")]
pub(crate) fn update_hint_tooltip(
    version: &str,
    state: &runner_app::updater::UpdateState,
) -> String {
    if matches!(state, runner_app::updater::UpdateState::Ready { .. }) {
        crate::i18n::tf(
            "format.runner_is_ready_to_install",
            &[("version", version.to_string())],
        )
    } else {
        crate::i18n::tf(
            "format.runner_is_available",
            &[("version", version.to_string())],
        )
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn activate_update_hint(
    _updater: &gpui::Entity<runner_app::updater::Updater>,
    window: &mut Window,
    cx: &mut App,
) {
    window.dispatch_action(
        Box::new(crate::surfaces::update_dialog::OpenUpdateDialog),
        cx,
    );
}

const RESIZE_BORDER: f32 = 6.;
pub(crate) const DESKTOP_APP_ID: &str = "runner";

#[cfg(target_os = "linux")]
pub(crate) fn initial_window_bounds(
    requested: Size<Pixels>,
    origin: Option<Point<Pixels>>,
    cx: &App,
) -> Bounds<Pixels> {
    let mut displays = cx.displays();
    if let Some(primary) = cx.primary_display() {
        displays.sort_by_key(|display| display.id() != primary.id());
    }
    let visible = displays
        .iter()
        .map(|display| display.visible_bounds())
        .collect::<Vec<_>>();
    constrain_initial_bounds(requested, origin, &visible)
}

fn constrain_initial_bounds(
    requested: Size<Pixels>,
    origin: Option<Point<Pixels>>,
    displays: &[Bounds<Pixels>],
) -> Bounds<Pixels> {
    let display = origin
        .and_then(|origin| displays.iter().find(|display| display.contains(&origin)))
        .or_else(|| displays.first());
    let Some(display) = display else {
        return Bounds::new(origin.unwrap_or_default(), requested);
    };
    let size = requested.min(&display.size);
    let origin =
        origin.unwrap_or_else(|| display.center() - point(size.width / 2., size.height / 2.));
    Bounds::new(
        point(
            origin
                .x
                .max(display.left())
                .min(display.right() - size.width),
            origin
                .y
                .max(display.top())
                .min(display.bottom() - size.height),
        ),
        size,
    )
}

fn frame_insets(decorations: Decorations, maximized: bool, fullscreen: bool) -> Edges<Pixels> {
    let Decorations::Client { tiling } = decorations else {
        return Edges::default();
    };
    if maximized || fullscreen {
        return Edges::default();
    }
    Edges {
        top: px(if tiling.top { 0. } else { RESIZE_BORDER }),
        right: px(if tiling.right { 0. } else { RESIZE_BORDER }),
        bottom: px(if tiling.bottom { 0. } else { RESIZE_BORDER }),
        left: px(if tiling.left { 0. } else { RESIZE_BORDER }),
    }
}

fn resize_regions(outer: Size<Pixels>, inset: Edges<Pixels>) -> Vec<(ResizeEdge, Bounds<Pixels>)> {
    let right = outer.width - inset.right;
    let bottom = outer.height - inset.bottom;
    [
        (ResizeEdge::TopLeft, 0., 0., inset.left, inset.top),
        (
            ResizeEdge::Top,
            f32::from(inset.left),
            0.,
            right - inset.left,
            inset.top,
        ),
        (
            ResizeEdge::TopRight,
            f32::from(right),
            0.,
            inset.right,
            inset.top,
        ),
        (
            ResizeEdge::Right,
            f32::from(right),
            f32::from(inset.top),
            inset.right,
            bottom - inset.top,
        ),
        (
            ResizeEdge::BottomRight,
            f32::from(right),
            f32::from(bottom),
            inset.right,
            inset.bottom,
        ),
        (
            ResizeEdge::Bottom,
            f32::from(inset.left),
            f32::from(bottom),
            right - inset.left,
            inset.bottom,
        ),
        (
            ResizeEdge::BottomLeft,
            0.,
            f32::from(bottom),
            inset.left,
            inset.bottom,
        ),
        (
            ResizeEdge::Left,
            0.,
            f32::from(inset.top),
            inset.left,
            bottom - inset.top,
        ),
    ]
    .into_iter()
    .filter(|(_, _, _, width, height)| *width > px(0.) && *height > px(0.))
    .map(|(edge, x, y, width, height)| {
        (edge, Bounds::new(point(px(x), px(y)), size(width, height)))
    })
    .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn resize_frame(content: Div, window: &mut Window) -> Div {
    let decorations = window.window_decorations();
    // GPUI subtracts these CSD margins from Wayland geometry, respecting tiled edges.
    window.set_client_inset(if matches!(decorations, Decorations::Client { .. }) {
        px(RESIZE_BORDER)
    } else {
        px(0.)
    });
    let inset = frame_insets(decorations, window.is_maximized(), window.is_fullscreen());
    render_resize_frame(content, window.viewport_size(), inset, |edge, window, _| {
        window.start_window_resize(edge)
    })
}

fn render_resize_frame(
    content: Div,
    outer: Size<Pixels>,
    inset: Edges<Pixels>,
    request: impl Fn(ResizeEdge, &mut Window, &mut App) + 'static,
) -> Div {
    let request = Rc::new(request);
    div()
        .debug_selector(|| "LINUX_WINDOW_FRAME".into())
        .relative()
        .size_full()
        .pt(inset.top)
        .pr(inset.right)
        .pb(inset.bottom)
        .pl(inset.left)
        .bg(crate::theme::sidebar())
        .child(content)
        .children(
            resize_regions(outer, inset)
                .into_iter()
                .map(|(edge, bounds)| {
                    let request = request.clone();
                    div()
                        .debug_selector(move || format!("LINUX_RESIZE_{edge:?}"))
                        .absolute()
                        .left(bounds.origin.x)
                        .top(bounds.origin.y)
                        .w(bounds.size.width)
                        .h(bounds.size.height)
                        .cursor(match edge {
                            ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                            ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                            ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                CursorStyle::ResizeUpLeftDownRight
                            }
                            ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                CursorStyle::ResizeUpRightDownLeft
                            }
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            request(edge, window, cx);
                        })
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, Tiling};
    use std::cell::{Cell, RefCell};

    #[test]
    fn desktop_entry_matches_window_identity() {
        let path = "packaging/linux/runner.desktop";
        assert_eq!(
            std::path::Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str()),
            Some(DESKTOP_APP_ID)
        );
        let desktop = include_str!("../../../../packaging/linux/runner.desktop");
        assert_eq!(
            desktop
                .lines()
                .find_map(|line| line.strip_prefix("StartupWMClass=")),
            Some(DESKTOP_APP_ID)
        );
    }

    #[test]
    fn initial_bounds_use_enumerated_display_without_primary_and_fit_small_displays() {
        let display = Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(768.)));
        let wanted = size(px(1440.), px(900.));
        assert_eq!(constrain_initial_bounds(wanted, None, &[display]), display);
        assert_eq!(
            constrain_initial_bounds(wanted, Some(point(px(32.), px(32.))), &[display]),
            display
        );
        let second = Bounds::new(point(px(1280.), px(0.)), size(px(800.), px(600.)));
        assert_eq!(
            constrain_initial_bounds(wanted, Some(point(px(1300.), px(20.))), &[display, second]),
            second
        );
        let small = Bounds::new(point(px(0.), px(0.)), size(px(500.), px(300.)));
        assert_eq!(constrain_initial_bounds(wanted, None, &[small]), small);
        assert_eq!(constrain_initial_bounds(wanted, None, &[]).size, wanted);
    }

    #[test]
    fn normal_edges_cover_eight_directions_without_content_or_tiled_edges() {
        let outer = size(px(800.), px(600.));
        let decorations = Decorations::Client {
            tiling: Tiling::default(),
        };
        let inset = frame_insets(decorations, false, false);
        let regions = resize_regions(outer, inset);
        assert_eq!(regions.len(), 8);
        for (position, expected) in [
            ((0., 0.), ResizeEdge::TopLeft),
            ((6., 1.), ResizeEdge::Top),
            ((794., 1.), ResizeEdge::TopRight),
            ((1., 1.), ResizeEdge::TopLeft),
            ((400., 1.), ResizeEdge::Top),
            ((799., 1.), ResizeEdge::TopRight),
            ((799., 300.), ResizeEdge::Right),
            ((799., 599.), ResizeEdge::BottomRight),
            ((400., 599.), ResizeEdge::Bottom),
            ((1., 599.), ResizeEdge::BottomLeft),
            ((1., 300.), ResizeEdge::Left),
        ] {
            let found = regions
                .iter()
                .filter(|(_, bounds)| bounds.contains(&point(px(position.0), px(position.1))))
                .map(|(edge, _)| *edge)
                .collect::<Vec<_>>();
            assert_eq!(found, vec![expected]);
        }
        for point in [
            point(px(6.), px(6.)),
            point(px(400.), px(300.)),
            point(px(780.), px(22.)),
            point(px(-1.), px(0.)),
            point(px(800.), px(300.)),
            point(px(400.), px(600.)),
        ] {
            assert!(regions.iter().all(|(_, bounds)| !bounds.contains(&point)));
        }
        for (maximized, fullscreen) in [(true, false), (false, true), (true, true)] {
            assert!(
                resize_regions(outer, frame_insets(decorations, maximized, fullscreen)).is_empty()
            );
        }
        assert!(resize_regions(outer, frame_insets(Decorations::Server, false, false)).is_empty());
        let tiled = Decorations::Client {
            tiling: Tiling {
                top: true,
                left: true,
                ..Tiling::default()
            },
        };
        assert_eq!(
            resize_regions(outer, frame_insets(tiled, false, false))
                .iter()
                .map(|(edge, _)| *edge)
                .collect::<Vec<_>>(),
            vec![
                ResizeEdge::Right,
                ResizeEdge::BottomRight,
                ResizeEdge::Bottom
            ]
        );
    }

    #[test]
    fn border_mouse_down_requests_resize_but_content_and_caption_keep_their_input() {
        struct FrameView {
            requested: Rc<RefCell<Vec<ResizeEdge>>>,
            content_clicks: Rc<Cell<usize>>,
            caption_clicks: Rc<Cell<usize>>,
            maximized: bool,
            fullscreen: bool,
        }
        impl Render for FrameView {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let requested = self.requested.clone();
                let content_clicks = self.content_clicks.clone();
                let caption_clicks = self.caption_clicks.clone();
                render_resize_frame(
                    div()
                        .debug_selector(|| "FRAME_CONTENT".into())
                        .relative()
                        .size_full()
                        .on_mouse_down(MouseButton::Left, move |_, _, _| {
                            content_clicks.set(content_clicks.get() + 1)
                        })
                        .child(
                            div()
                                .debug_selector(|| "FRAME_CAPTION_BUTTON".into())
                                .absolute()
                                .right_0()
                                .top_0()
                                .w(px(46.))
                                .h(px(32.))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    cx.stop_propagation();
                                    caption_clicks.set(caption_clicks.get() + 1);
                                }),
                        ),
                    window.viewport_size(),
                    frame_insets(
                        Decorations::Client {
                            tiling: Tiling::default(),
                        },
                        self.maximized,
                        self.fullscreen,
                    ),
                    move |edge, _, _| requested.borrow_mut().push(edge),
                )
            }
        }
        let requested = Rc::new(RefCell::new(Vec::new()));
        let content_clicks = Rc::new(Cell::new(0));
        let caption_clicks = Rc::new(Cell::new(0));
        let mut cx = gpui::TestAppContext::single();
        let host = cx.add_window(|_, _| FrameView {
            requested: requested.clone(),
            content_clicks: content_clicks.clone(),
            caption_clicks: caption_clicks.clone(),
            maximized: false,
            fullscreen: false,
        });
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(800.), px(600.)));
        visual.run_until_parked();
        let content = visual.debug_bounds("FRAME_CONTENT").unwrap();
        assert_eq!(content.origin, point(px(6.), px(6.)));
        assert_eq!(content.size, size(px(788.), px(588.)));
        visual.simulate_click(point(px(1.), px(1.)), gpui::Modifiers::default());
        assert_eq!(*requested.borrow(), vec![ResizeEdge::TopLeft]);
        for position in [point(px(400.), px(300.)), point(px(780.), px(22.))] {
            visual.simulate_click(position, gpui::Modifiers::default());
        }
        assert_eq!(*requested.borrow(), vec![ResizeEdge::TopLeft]);
        assert_eq!(content_clicks.get(), 1);
        assert_eq!(caption_clicks.get(), 1);
        for (maximized, fullscreen) in [(true, false), (false, true)] {
            requested.borrow_mut().clear();
            host.update(&mut visual, |view, _, cx| {
                view.maximized = maximized;
                view.fullscreen = fullscreen;
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("LINUX_RESIZE_TopLeft").is_none());
            assert_eq!(
                visual.debug_bounds("FRAME_CONTENT").unwrap().size,
                size(px(800.), px(600.))
            );
            visual.simulate_click(point(px(1.), px(1.)), gpui::Modifiers::default());
            assert!(requested.borrow().is_empty());
        }
    }
}

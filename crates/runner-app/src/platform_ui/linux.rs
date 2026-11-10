#[cfg(target_os = "linux")]
pub(crate) use super::desktop::*;

use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Corners, CursorStyle, Decorations, Div, Edges,
    MouseButton, Pixels, Point, ResizeEdge, Size, Window, WindowBackgroundAppearance,
};
use std::rc::Rc;

const RESIZE_BORDER: f32 = 6.;
const CORNER_RADIUS: f32 = 8.;
pub(crate) const DESKTOP_APP_ID: &str = "runner";

#[cfg(test)]
thread_local! {
    // GPUI's test windows always report server decorations.
    static TEST_FRAME_STATE: std::cell::Cell<Option<(Decorations, bool, bool)>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn window_background(decorations: Decorations) -> WindowBackgroundAppearance {
    match decorations {
        Decorations::Client { .. } => WindowBackgroundAppearance::Transparent,
        Decorations::Server => WindowBackgroundAppearance::Opaque,
    }
}

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

fn frame_corners(decorations: Decorations, maximized: bool, fullscreen: bool) -> Corners<Pixels> {
    let inset = frame_insets(decorations, maximized, fullscreen);
    let radius = |a, b| {
        if a > px(0.) && b > px(0.) {
            px(CORNER_RADIUS)
        } else {
            px(0.)
        }
    };
    Corners {
        top_left: radius(inset.top, inset.left),
        top_right: radius(inset.top, inset.right),
        bottom_right: radius(inset.bottom, inset.right),
        bottom_left: radius(inset.bottom, inset.left),
    }
}

fn round_corners<T: Styled>(surface: T, corners: Corners<Pixels>) -> T {
    surface
        .rounded_tl(corners.top_left)
        .rounded_tr(corners.top_right)
        .rounded_br(corners.bottom_right)
        .rounded_bl(corners.bottom_left)
}

pub(crate) fn round_window_corners<T: Styled>(surface: T, window: &Window) -> T {
    // GPUI masks are rectangular, so edge-reaching panels must round their own paint.
    let state = (
        window.window_decorations(),
        window.is_maximized(),
        window.is_fullscreen(),
    );
    #[cfg(test)]
    let state = TEST_FRAME_STATE
        .with(|state_override| state_override.get())
        .unwrap_or(state);
    round_corners(surface, frame_corners(state.0, state.1, state.2))
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
    render_resize_frame(
        content,
        window.viewport_size(),
        decorations,
        window.is_maximized(),
        window.is_fullscreen(),
        |edge, window, _| window.start_window_resize(edge),
    )
}

fn render_resize_frame(
    content: Div,
    outer: Size<Pixels>,
    decorations: Decorations,
    maximized: bool,
    fullscreen: bool,
    request: impl Fn(ResizeEdge, &mut Window, &mut App) + 'static,
) -> Div {
    let request = Rc::new(request);
    let inset = frame_insets(decorations, maximized, fullscreen);
    let corners = frame_corners(decorations, maximized, fullscreen);
    let content = content.when(
        matches!(decorations, Decorations::Client { .. }),
        |content| {
            round_corners(content, corners)
                .overflow_hidden()
                .bg(crate::theme::sidebar())
                .border_1()
                .border_color(crate::theme::border())
                .when(inset != Edges::default(), |content| {
                    content.shadow(vec![gpui::BoxShadow {
                        color: crate::theme::with_alpha(gpui::black(), 0.3),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(6.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                })
        },
    );
    div()
        .debug_selector(|| "LINUX_WINDOW_FRAME".into())
        .relative()
        .size_full()
        .pt(inset.top)
        .pr(inset.right)
        .pb(inset.bottom)
        .pl(inset.left)
        .overflow_hidden()
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
    fn background_is_transparent_only_for_client_decorations() {
        assert_eq!(
            window_background(Decorations::Client {
                tiling: Tiling::default(),
            }),
            WindowBackgroundAppearance::Transparent
        );
        assert_eq!(
            window_background(Decorations::Server),
            WindowBackgroundAppearance::Opaque
        );
    }

    #[test]
    fn corners_round_only_where_both_edges_are_floating() {
        for (tiling, expected) in [
            (Tiling::default(), [8., 8., 8., 8.]),
            (
                Tiling {
                    top: true,
                    ..Tiling::default()
                },
                [0., 0., 8., 8.],
            ),
            (
                Tiling {
                    right: true,
                    ..Tiling::default()
                },
                [8., 0., 0., 8.],
            ),
            (
                Tiling {
                    bottom: true,
                    ..Tiling::default()
                },
                [8., 8., 0., 0.],
            ),
            (
                Tiling {
                    left: true,
                    ..Tiling::default()
                },
                [0., 8., 8., 0.],
            ),
            (
                Tiling {
                    top: true,
                    left: true,
                    ..Tiling::default()
                },
                [0., 0., 8., 0.],
            ),
            (
                Tiling {
                    top: true,
                    right: true,
                    bottom: true,
                    left: true,
                },
                [0.; 4],
            ),
        ] {
            let decorations = Decorations::Client { tiling };
            assert_eq!(
                frame_corners(decorations, false, false),
                Corners {
                    top_left: px(expected[0]),
                    top_right: px(expected[1]),
                    bottom_right: px(expected[2]),
                    bottom_left: px(expected[3]),
                },
                "{tiling:?}"
            );
            for (maximized, fullscreen) in [(true, false), (false, true), (true, true)] {
                assert_eq!(
                    frame_corners(decorations, maximized, fullscreen),
                    Corners::default()
                );
            }
        }
        assert_eq!(
            frame_corners(Decorations::Server, false, false),
            Corners::default()
        );
    }

    #[test]
    fn frame_paints_opaque_rounded_content_without_filling_the_resize_margin() {
        struct FrameView {
            decorations: Decorations,
            maximized: bool,
            fullscreen: bool,
        }
        impl Render for FrameView {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                render_resize_frame(
                    div()
                        .debug_selector(|| "FRAME_CONTENT".into())
                        .size_full()
                        .bg(crate::theme::sidebar()),
                    window.viewport_size(),
                    self.decorations,
                    self.maximized,
                    self.fullscreen,
                    |_, _, _| {},
                )
            }
        }
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let floating = Decorations::Client {
            tiling: Tiling::default(),
        };
        let tiled = Decorations::Client {
            tiling: Tiling {
                top: true,
                left: true,
                ..Tiling::default()
            },
        };
        for theme in [
            crate::theme::ThemeVariant::Carbon,
            crate::theme::ThemeVariant::RunnerLight,
        ] {
            crate::theme::set_active_variant(theme);
            for (decorations, maximized, fullscreen) in [
                (floating, false, false),
                (tiled, false, false),
                (floating, true, false),
                (floating, false, true),
                (Decorations::Server, false, false),
            ] {
                let mut cx = gpui::TestAppContext::single();
                let host = cx.add_window(|_, _| FrameView {
                    decorations,
                    maximized,
                    fullscreen,
                });
                let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(800.), px(600.)));
                visual.run_until_parked();
                let content = visual.debug_bounds("FRAME_CONTENT").unwrap();
                let inset = frame_insets(decorations, maximized, fullscreen);
                assert_eq!(content.origin, point(inset.left, inset.top));
                assert_eq!(
                    content.size,
                    size(
                        px(800.) - inset.left - inset.right,
                        px(600.) - inset.top - inset.bottom
                    )
                );
                visual.update(|window, _| {
                    let scale = window.scale_factor();
                    let quads = window.painted_quads();
                    let fills = quads
                        .iter()
                        .filter(|quad| !quad.background.is_transparent())
                        .collect::<Vec<_>>();
                    assert_eq!(fills.len(), 1);
                    let fill = fills[0];
                    assert_eq!(fill.bounds, content.scale(scale));
                    assert_eq!(fill.background.as_solid(), Some(crate::theme::sidebar()));
                    assert_eq!(fill.background.as_solid().unwrap().a, 1.);
                    assert_eq!(
                        fill.corner_radii,
                        frame_corners(decorations, maximized, fullscreen).scale(scale)
                    );
                    let borders = quads
                        .iter()
                        .filter(|quad| quad.border_color.a > 0.)
                        .collect::<Vec<_>>();
                    if matches!(decorations, Decorations::Client { .. }) {
                        assert!(!borders.is_empty());
                        for border in borders {
                            assert_eq!(border.border_color, crate::theme::border());
                            assert_eq!(border.border_widths, Edges::all(px(1.)).scale(scale));
                        }
                    } else {
                        assert!(borders.is_empty());
                    }
                });
            }
        }
    }

    #[test]
    fn real_dialog_scrims_keep_shell_corners_including_when_deferred() {
        struct ResetFrameState;
        impl Drop for ResetFrameState {
            fn drop(&mut self) {
                TEST_FRAME_STATE.with(|state| state.set(None));
            }
        }
        struct DialogFrame {
            decorations: Decorations,
            maximized: bool,
            fullscreen: bool,
            deferred: bool,
            drawer: bool,
        }
        impl Render for DialogFrame {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let dialog = if self.drawer {
                    crate::ui::Drawer::new(
                        "Role details",
                        div().child("Choose a role"),
                        Rc::new(|_, _| {}),
                    )
                    .footer(div().child("Done"))
                    .into_any_element()
                } else {
                    crate::ui::Modal::new(
                        "New chat",
                        div().child("Choose a role"),
                        Rc::new(|_, _| {}),
                    )
                    .into_any_element()
                };
                let dialog = if self.deferred {
                    gpui::deferred(dialog).into_any_element()
                } else {
                    dialog.into_any_element()
                };
                render_resize_frame(
                    div()
                        .debug_selector(|| "FRAME_CONTENT".into())
                        .relative()
                        .size_full()
                        .child(dialog),
                    window.viewport_size(),
                    self.decorations,
                    self.maximized,
                    self.fullscreen,
                    |_, _, _| {},
                )
            }
        }
        let _reset = ResetFrameState;
        let floating = Decorations::Client {
            tiling: Tiling::default(),
        };
        let tiled = Decorations::Client {
            tiling: Tiling {
                left: true,
                ..Tiling::default()
            },
        };
        for (decorations, maximized, fullscreen) in [
            (floating, false, false),
            (tiled, false, false),
            (floating, true, false),
            (floating, false, true),
            (Decorations::Server, false, false),
        ] {
            TEST_FRAME_STATE.with(|state| state.set(Some((decorations, maximized, fullscreen))));
            for (deferred, drawer) in [(false, false), (true, false), (true, true)] {
                let mut cx = gpui::TestAppContext::single();
                let host = cx.add_window(|_, _| DialogFrame {
                    decorations,
                    maximized,
                    fullscreen,
                    deferred,
                    drawer,
                });
                let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
                visual.simulate_resize(size(px(800.), px(600.)));
                visual.run_until_parked();
                let content = visual.debug_bounds("FRAME_CONTENT").unwrap();
                let panel = visual
                    .debug_bounds(if drawer {
                        "DRAWER_PANEL"
                    } else {
                        "MODAL_PANEL"
                    })
                    .unwrap();
                visual.update(|window, _| {
                    let quads = window.painted_quads();
                    let scrims = quads
                        .iter()
                        .filter(|quad| {
                            quad.background.as_solid() == Some(gpui::rgba(0x00000099).into())
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(scrims.len(), 1);
                    let scrim = scrims[0];
                    assert_eq!(
                        scrim.corner_radii,
                        frame_corners(decorations, maximized, fullscreen)
                            .scale(window.scale_factor())
                    );
                    let bounds = content.scale(window.scale_factor());
                    assert!(
                        scrim.bounds.left() >= bounds.left()
                            && scrim.bounds.right() <= bounds.right()
                    );
                    assert!(
                        scrim.bounds.top() >= bounds.top()
                            && scrim.bounds.bottom() <= bounds.bottom()
                    );
                    if drawer {
                        let panel = quads
                            .iter()
                            .find(|quad| {
                                quad.bounds == panel.scale(window.scale_factor())
                                    && quad.background.as_solid() == Some(crate::theme::panel())
                            })
                            .unwrap();
                        assert_eq!(panel.corner_radii.top_right, scrim.corner_radii.top_right);
                        assert_eq!(
                            panel.corner_radii.bottom_right,
                            scrim.corner_radii.bottom_right
                        );
                        let footer = quads
                            .iter()
                            .find(|quad| {
                                quad.background.as_solid()
                                    == Some(crate::theme::with_alpha(crate::theme::bg(), 0.4))
                            })
                            .unwrap();
                        assert_eq!(
                            footer.corner_radii.bottom_right,
                            scrim.corner_radii.bottom_right
                        );
                    }
                });
            }
        }
    }

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
                    Decorations::Client {
                        tiling: Tiling::default(),
                    },
                    self.maximized,
                    self.fullscreen,
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

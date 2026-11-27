use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use runner_core::protocol::changes::DiffOp;

use super::*;

const WIDTH: f32 = 500.;
const HEIGHT: f32 = 300.;

fn open(
    mode: DiffViewMode,
    lines: u32,
    columns: usize,
) -> (WindowHandle<UnifiedDiffView>, VisualTestContext) {
    let text = format!("{}\n", "x".repeat(columns)).repeat(lines as usize);
    open_sides(
        mode,
        lines,
        &text,
        &text,
        vec![DiffOp::Context { count: lines }],
    )
}

fn open_sides(
    mode: DiffViewMode,
    lines: u32,
    old: &str,
    new: &str,
    ops: Vec<DiffOp>,
) -> (WindowHandle<UnifiedDiffView>, VisualTestContext) {
    let old = Arc::new(Document::decode(old.as_bytes()));
    let new = Arc::new(Document::decode(new.as_bytes()));
    let hunk = DiffHunk {
        header: "@@".into(),
        old_start: 1,
        old_lines: lines,
        new_start: 1,
        new_lines: lines,
        ops,
    };
    let cx = &mut TestAppContext::single();
    let window = cx.add_window(|_, cx| {
        let mut view =
            UnifiedDiffView::new(old, new, &[hunk], None, "file.rs".into(), false, false, cx);
        view.set_mode(mode, cx);
        view.set_show_viewed_footer(true);
        view
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(WIDTH), px(HEIGHT)));
    // The pointer starts over the view, which shows the bars.
    visual.simulate_mouse_move(point(px(-10.), px(-10.)), None, Modifiers::default());
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(1));
    visual.run_until_parked();
    (window, visual)
}

fn metrics(window: &WindowHandle<UnifiedDiffView>, visual: &VisualTestContext) -> Metrics {
    window
        .read_with(visual, |view, _| view.metrics.get().unwrap())
        .unwrap()
}

#[test]
fn the_scrollbars_take_a_lane_beside_the_diff_on_the_axes_that_scroll() {
    for mode in [DiffViewMode::Unified, DiffViewMode::Split] {
        for (lines, columns, right, bottom) in [
            (3, 5, 0., 0.),
            (200, 5, 10., 0.),
            (3, 500, 0., 10.),
            (200, 500, 10., 10.),
        ] {
            let (window, mut visual) = open(mode, lines, columns);
            let hidden = metrics(&window, &visual);
            let viewport = hidden.viewport();
            assert!(visual.debug_bounds("SCROLLBAR_THUMB").is_none());
            assert!(visual.debug_bounds("SCROLLBAR_THUMB_X").is_none());
            assert_eq!(viewport.right(), px(WIDTH - right), "{lines}x{columns}");
            assert_eq!(viewport.bottom(), px(HEIGHT - bottom), "{lines}x{columns}");
            let divider = ((px(WIDTH) - px(1.)) * 0.5).floor();
            assert_eq!(hidden.left_col_width, divider, "the divider stays centred");
            let text_right = hidden.side_text_left_abs(DiffSide::Right)
                + match mode {
                    DiffViewMode::Unified => hidden.scroll_viewport_width(),
                    DiffViewMode::Split => hidden.right_text_width(),
                };
            assert_eq!(text_right, viewport.right());
            assert_eq!(hidden.side_right_abs(DiffSide::Right), viewport.right());

            visual.simulate_mouse_move(viewport.center(), None, Modifiers::default());
            visual.run_until_parked();
            assert_eq!(metrics(&window, &visual).lanes, hidden.lanes);
            let vertical = visual.debug_bounds("SCROLLBAR_THUMB");
            let horizontal = visual.debug_bounds("SCROLLBAR_THUMB_X");
            assert_eq!(vertical.is_some(), right > 0., "{lines}x{columns}");
            assert_eq!(horizontal.is_some(), bottom > 0., "{lines}x{columns}");
            assert!(vertical.is_none_or(|thumb| thumb.left() >= viewport.right()));
            assert!(horizontal.is_none_or(|thumb| thumb.top() >= viewport.bottom()));
        }
    }
}

#[test]
fn a_line_that_fits_only_the_left_of_a_split_takes_no_bottom_lane() {
    let short = "x\n".repeat(200);
    let context = vec![DiffOp::Context { count: 200 }];
    let (window, visual) = open_sides(DiffViewMode::Split, 200, &short, &short, context);
    let empty = metrics(&window, &visual);
    assert_eq!(empty.lanes.right, px(10.));
    let columns =
        ((empty.left_text_width() - px(TEXT_PAD_RIGHT)) / empty.char_width).floor() as usize;
    let width = empty.char_width * columns as f32 + px(TEXT_PAD_RIGHT);
    assert!(
        width > empty.right_text_width(),
        "the line fits only the left"
    );
    let diff_ops = || {
        vec![
            DiffOp::Removed { count: 1 },
            DiffOp::Added { count: 1 },
            DiffOp::Context { count: 199 },
        ]
    };
    let old = format!("{}\n{}", "x".repeat(columns), "x\n".repeat(199));
    let (window, mut visual) = open_sides(DiffViewMode::Split, 200, &old, &short, diff_ops());
    let lanes = metrics(&window, &visual).lanes;
    assert_eq!((lanes.right, lanes.bottom), (px(10.), px(0.)));
    visual.simulate_mouse_move(point(px(100.), px(100.)), None, Modifiers::default());
    window
        .update(&mut visual, |view, _, cx| {
            view.scroll_by(point(px(-1e6), px(0.)), cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SCROLLBAR_THUMB_X").is_none());
    let scroll = window.read_with(&visual, |view, _| view.scroll).unwrap();
    assert_eq!(scroll.x, px(0.));

    // A removed line that does overflow the left side scrolls only by its
    // own column's overflow, not the narrower right column's.
    let overflow_cols = columns + 10;
    let old_wide = format!("{}\n{}", "x".repeat(overflow_cols), "x\n".repeat(199));
    let (window, mut visual) = open_sides(DiffViewMode::Split, 200, &old_wide, &short, diff_ops());
    let wide_metrics = metrics(&window, &visual);
    assert_eq!(
        (wide_metrics.lanes.right, wide_metrics.lanes.bottom),
        (px(10.), px(10.))
    );
    visual.simulate_mouse_move(point(px(100.), px(100.)), None, Modifiers::default());
    window
        .update(&mut visual, |view, _, cx| {
            view.scroll_by(point(px(-1e6), px(0.)), cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SCROLLBAR_THUMB_X").is_some());
    let expected_max = wide_metrics.char_width * overflow_cols as f32 + px(TEXT_PAD_RIGHT)
        - wide_metrics.left_text_width();
    let scroll = window.read_with(&visual, |view, _| view.scroll).unwrap();
    assert!((scroll.x - expected_max).abs() < px(0.5));
}

#[test]
fn the_end_of_the_diff_and_its_footer_scroll_clear_of_the_lanes() {
    for mode in [DiffViewMode::Unified, DiffViewMode::Split] {
        let (window, mut visual) = open(mode, 200, 500);
        window
            .update(&mut visual, |view, _, cx| {
                view.scroll_by(point(px(-1e6), px(-1e6)), cx)
            })
            .unwrap();
        visual.run_until_parked();
        let metrics = metrics(&window, &visual);
        let viewport = metrics.viewport();
        let (scroll, columns) = window
            .read_with(&visual, |view, _| (view.scroll, view.new_doc.columns()))
            .unwrap();
        let end = metrics.side_text_left_abs(DiffSide::Right) + metrics.char_width * columns as f32
            - scroll.x;
        assert!(end <= viewport.right());
        let footer = visual.debug_bounds("CHANGES_DIFF_VIEWED").unwrap();
        assert!(footer.bottom() <= viewport.bottom());
        assert_eq!(footer.right(), viewport.right());
    }
}

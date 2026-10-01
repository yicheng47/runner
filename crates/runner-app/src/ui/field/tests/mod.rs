use super::*;

mod editing;
mod path;
mod rendered;
mod undo;

/// The test platform's text system advances every character by 0.6 em;
/// fields default to 14 px text, and textarea rows are 20 px.
const ADVANCE: f32 = 14. * 0.6;

const ROW: f32 = 20.;

/// Wraps a 110 px wide textarea's text after ten characters.
const WRAPPED: &str = "alpha beta gamma\n\nend";

struct FieldHost {
    input: Entity<TextField>,
    width: Pixels,
}

impl Render for FieldHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Room around the field, so a drag can leave it inside the window.
        div().p(px(100.)).child(
            div()
                .w(self.width)
                .debug_selector(|| "FIELD_HOST".into())
                .child(self.input.clone()),
        )
    }
}

fn open_field(
    width: f32,
    build: impl FnOnce(FocusHandle) -> TextField + 'static,
) -> (gpui::VisualTestContext, Entity<TextField>) {
    let mut cx = gpui::TestAppContext::single();
    let window = cx.add_window(|_, cx| FieldHost {
        input: cx.new(|cx| build(cx.focus_handle())),
        width: px(width),
    });
    cx.run_until_parked();
    let visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.run_until_parked();
    let input = window.read_with(&cx, |host, _| host.input.clone()).unwrap();
    (visual, input)
}

fn text_origin(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> Point<Pixels> {
    input.read_with(visual, |field, _| {
        field
            .text_state
            .borrow()
            .origin
            .expect("the text was painted")
    })
}

/// A point `chars` advances along the text's row `row`, a little below
/// the row's top.
fn at(origin: Point<Pixels>, chars: f32, row: f32) -> Point<Pixels> {
    origin + point(px(chars * ADVANCE), px(row * ROW + 5.))
}

#[track_caller]
fn assert_near(actual: Point<Pixels>, expected: Point<Pixels>) {
    assert!(
        (actual.x - expected.x).abs() < px(0.01) && (actual.y - expected.y).abs() < px(0.01),
        "{actual:?} is not {expected:?}"
    );
}

#[track_caller]
fn assert_bounds_near(actual: Option<Bounds<Pixels>>, expected: Bounds<Pixels>) {
    let actual = actual.expect("bounds");
    assert_near(actual.origin, expected.origin);
    assert_near(actual.bottom_right(), expected.bottom_right());
}

fn click(visual: &mut gpui::VisualTestContext, position: Point<Pixels>, count: usize) {
    click_with(visual, position, count, gpui::Modifiers::none());
}

fn click_with(
    visual: &mut gpui::VisualTestContext,
    position: Point<Pixels>,
    click_count: usize,
    modifiers: gpui::Modifiers,
) {
    visual.simulate_event(MouseDownEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count,
        first_mouse: false,
    });
    visual.simulate_event(MouseUpEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count,
    });
}

fn selection(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> (usize, usize) {
    input.read_with(visual, |field, _| {
        (field.buffer.selection.anchor, field.buffer.selection.caret)
    })
}

fn caret(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> usize {
    selection(visual, input).1
}

/// The caret's top left, relative to the text's origin, as painted.
fn caret_position(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> Point<Pixels> {
    input.read_with(visual, |field, _| {
        let caret = field.buffer.selection.caret;
        let state = field.text_state.borrow();
        state
            .shaped
            .as_ref()
            .unwrap()
            .caret_position(caret, field.caret_row_end == Some(caret))
    })
}

/// Runs the auto-scroll timer `ticks` times; the test clock fires one
/// timer per advance.
fn tick(visual: &mut gpui::VisualTestContext, ticks: usize) {
    for _ in 0..ticks {
        visual.executor().advance_clock(AUTO_SCROLL_TICK);
        visual.run_until_parked();
    }
}

fn scroll_y(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> Pixels {
    input.read_with(visual, |field, _| field.scroll_handle.offset().y)
}

fn wrapped_field() -> (gpui::VisualTestContext, Entity<TextField>) {
    let (visual, input) = open_field(110., |focus| {
        TextField::textarea(focus, WRAPPED, "", 6, false)
    });
    input.read_with(&visual, |field, _| {
        let state = field.text_state.borrow();
        let lines = &state.shaped.as_ref().unwrap().lines;
        let rows = lines
            .iter()
            .map(|line| line.geometry.rows())
            .collect::<Vec<_>>();
        assert_eq!(rows, [2, 1, 1]);
        assert_eq!(
            lines[0].geometry.spot(6, false).row,
            1,
            "alpha / beta gamma"
        );
    });
    (visual, input)
}

use super::*;

use super::super::layout::LineGeometry;

#[test]
fn a_click_lands_where_the_text_is_painted_after_wraps_at_line_ends_and_below() {
    let (mut visual, input) = wrapped_field();
    let origin = text_origin(&visual, &input);

    click(&mut visual, at(origin, 2.3, 1.), 1);
    assert_eq!(caret(&visual, &input), 8, "after the soft wrap");
    assert_near(
        caret_position(&visual, &input),
        point(px(2. * ADVANCE), px(ROW)),
    );

    click(&mut visual, at(origin, 9.6, 1.), 1);
    assert_eq!(caret(&visual, &input), 16, "the end of the wrapped line");

    click(&mut visual, at(origin, 9., 0.), 1);
    assert_eq!(caret(&visual, &input), 6, "past the end of the first row");
    // The caret stays at the end of the row that was clicked.
    assert_near(
        caret_position(&visual, &input),
        point(px(6. * ADVANCE), px(0.)),
    );

    click(&mut visual, at(origin, 0.4, 1.), 1);
    assert_eq!(caret(&visual, &input), 6, "the start of the second row");
    assert_near(caret_position(&visual, &input), point(px(0.), px(ROW)));

    click(&mut visual, at(origin, 3., 2.), 1);
    assert_eq!(caret(&visual, &input), 17, "the empty line");
    assert_near(caret_position(&visual, &input), point(px(0.), px(2. * ROW)));

    click(&mut visual, at(origin, 1.6, 3.), 1);
    assert_eq!(
        caret(&visual, &input),
        20,
        "the closest boundary on the last line"
    );

    click(&mut visual, at(origin, 1., 4.5), 1);
    assert_eq!(caret(&visual, &input), WRAPPED.len(), "below the text");
    assert_near(
        caret_position(&visual, &input),
        point(px(3. * ADVANCE), px(3. * ROW)),
    );

    click(&mut visual, at(origin, 3., 2.), 1);
    click_with(
        &mut visual,
        at(origin, 2.3, 1.),
        1,
        gpui::Modifiers::shift(),
    );
    assert_eq!(selection(&visual, &input), (17, 8), "shift-click extends");
}

#[test]
fn a_long_line_hit_tests_the_painted_glyphs_to_its_end() {
    let text = "The quick brown fox jumps over the lazy dog, then naps.";
    let (mut visual, input) = open_field(600., move |focus| TextField::new(focus, text, "", false));
    let origin = text_origin(&visual, &input);
    for index in [0, 17, 40, text.len() - 1, text.len()] {
        click(
            &mut visual,
            origin + point(px((index as f32 + 0.3) * ADVANCE), px(5.)),
            1,
        );
        assert_eq!(caret(&visual, &input), index);
        assert_near(
            caret_position(&visual, &input),
            point(px(index as f32 * ADVANCE), px(0.)),
        );
    }
    click(&mut visual, origin + point(px(-4.), px(5.)), 1);
    assert_eq!(caret(&visual, &input), 0);
}

#[test]
fn double_and_triple_clicks_select_the_word_and_line_under_the_pointer() {
    let (mut visual, input) = wrapped_field();
    let origin = text_origin(&visual, &input);
    let selected = |visual: &gpui::VisualTestContext| {
        input.read_with(visual, |field, _| {
            field.buffer.selected_text().unwrap_or_default().to_owned()
        })
    };

    click(&mut visual, at(origin, 4.8, 0.), 2);
    assert_eq!(
        selected(&visual),
        "alpha",
        "the right half of its last letter"
    );
    click(&mut visual, at(origin, 3.8, 1.), 2);
    assert_eq!(selected(&visual), "beta", "a word after the soft wrap");
    click(&mut visual, at(origin, 3., 2.), 2);
    assert_eq!(selected(&visual), "\n", "an empty line");
    click(&mut visual, at(origin, 1., 4.5), 2);
    assert_eq!(selected(&visual), "end", "below the text, the last word");

    click(&mut visual, at(origin, 1., 1.), 3);
    assert_eq!(
        selected(&visual),
        "alpha beta gamma\n",
        "the whole wrapped line"
    );
    click(&mut visual, at(origin, 3., 2.), 3);
    assert_eq!(selected(&visual), "\n");
}

#[test]
fn a_drag_past_the_field_keeps_selecting_and_scrolls_until_the_button_is_up() {
    let text = (0..12)
        .map(|line| format!("line {line:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut visual, input) = open_field(300., move |focus| {
        TextField::textarea(focus, text, "", 3, false)
    });
    let origin = text_origin(&visual, &input);
    let field = visual.debug_bounds("FIELD_HOST").unwrap();
    let left = Some(MouseButton::Left);
    let none = gpui::Modifiers::none();

    visual.simulate_mouse_down(at(origin, 2., 0.), MouseButton::Left, none);
    visual.simulate_mouse_move(
        point(field.right() + px(50.), at(origin, 0., 1.).y),
        left,
        none,
    );
    assert_eq!(
        selection(&visual, &input),
        (2, 15),
        "right of the field, to the row's end"
    );

    visual.simulate_mouse_move(
        point(field.left() + px(20.), field.bottom() + px(30.)),
        left,
        none,
    );
    let below = caret(&visual, &input);
    assert!(
        below > 15,
        "below the field, into the rows under the pointer: {below}"
    );
    assert_eq!(
        scroll_y(&visual, &input),
        px(0.),
        "scrolling waits for the timer"
    );

    tick(&mut visual, 5);
    let scrolled = scroll_y(&visual, &input);
    assert!(scrolled < px(0.), "the field scrolls toward the pointer");
    assert!(caret(&visual, &input) > below, "and the selection follows");

    visual.simulate_mouse_up(
        point(field.left(), field.bottom() + px(30.)),
        MouseButton::Left,
        none,
    );
    let stopped = selection(&visual, &input);
    let stopped_at = scroll_y(&visual, &input);
    tick(&mut visual, 10);
    assert_eq!(
        scroll_y(&visual, &input),
        stopped_at,
        "mouse up stops scrolling"
    );

    visual.simulate_mouse_move(at(origin, 0., 0.), left, none);
    assert_eq!(
        selection(&visual, &input),
        stopped,
        "the listener is inert after mouse up"
    );

    let origin = text_origin(&visual, &input);
    assert_eq!(
        origin.y - field.top(),
        px(7. - 92.5),
        "five ticks scrolled 92.5 px"
    );
    visual.simulate_mouse_down(at(origin, 2., 5.), MouseButton::Left, none);
    visual.simulate_mouse_move(
        point(field.left() + px(20.), field.top() - px(40.)),
        left,
        none,
    );
    assert!(
        caret(&visual, &input) > 0,
        "above the field, a row scrolled out of view"
    );
    tick(&mut visual, 20);
    assert_eq!(
        scroll_y(&visual, &input),
        px(0.),
        "scrolled back to the top"
    );
    assert_eq!(caret(&visual, &input), 0, "above the text, its start");
    visual.simulate_mouse_up(at(origin, 0., 0.), MouseButton::Left, none);
}

#[test]
fn up_and_down_move_by_visual_row_keeping_x() {
    let (mut visual, input) = wrapped_field();
    let origin = text_origin(&visual, &input);
    click(&mut visual, at(origin, 2., 1.), 1);
    assert_eq!(caret(&visual, &input), 8);

    visual.simulate_keystrokes("up");
    assert_eq!(caret(&visual, &input), 2, "across the soft wrap");
    visual.simulate_keystrokes("up");
    assert_eq!(caret(&visual, &input), 0, "from the first row, the start");
    visual.simulate_keystrokes("down");
    assert_eq!(caret(&visual, &input), 8, "back down, keeping x");
    visual.simulate_keystrokes("down");
    assert_eq!(caret(&visual, &input), 17, "onto the empty line");
    visual.simulate_keystrokes("down");
    assert_eq!(caret(&visual, &input), 20, "x kept through the empty line");
    visual.simulate_keystrokes("down");
    assert_eq!(
        caret(&visual, &input),
        WRAPPED.len(),
        "from the last row, the end"
    );

    visual.simulate_keystrokes("shift-up shift-up");
    assert_eq!(
        selection(&visual, &input),
        (WRAPPED.len(), 8),
        "shift extends"
    );
    visual.simulate_keystrokes("up");
    assert_eq!(
        selection(&visual, &input),
        (2, 2),
        "up from the selection's start"
    );

    click(&mut visual, at(origin, 9., 0.), 1);
    visual.simulate_keystrokes("down");
    assert_eq!(caret(&visual, &input), 12, "from the end of a wrapped row");

    click(&mut visual, at(origin, 2., 1.), 1);
    visual.simulate_keystrokes("down");
    assert_eq!(caret(&visual, &input), 17);
    visual.simulate_keystrokes("left right down");
    assert_eq!(
        caret(&visual, &input),
        18,
        "a sideways move drops the kept x, so Down starts from the empty line's own x"
    );
}

#[test]
fn glyphs_out_of_source_order_map_only_to_grapheme_boundaries() {
    let cases = [
        // Shaped right to left, wrapped after two glyphs.
        (
            "abcd",
            vec![(3, px(0.)), (2, px(10.)), (1, px(20.)), (0, px(30.))],
            vec![2],
        ),
        // Wrapped inside a grapheme, before its combining accent.
        (
            "e\u{301}x",
            vec![(0, px(0.)), (1, px(10.)), (3, px(10.))],
            vec![1],
        ),
    ];
    for (source, glyphs, wraps) in cases {
        let geometry = LineGeometry::new(source, &glyphs, &wraps, px(40.));
        let boundaries = source
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([source.len()])
            .collect::<Vec<_>>();
        assert_eq!(geometry.rows(), 2);
        for row in 0..geometry.rows() {
            for x in [-5., 0., 5., 12., 25., 35., 60.] {
                for under in [false, true] {
                    let (index, _) = geometry.index_at(row, px(x), under);
                    assert!(
                        boundaries.contains(&index),
                        "{source:?} row {row} at {x}: {index}"
                    );
                }
            }
        }
        for index in 0..=source.len() + 1 {
            for upstream in [false, true] {
                assert!(geometry.spot(index, upstream).row < geometry.rows());
            }
        }
        for (_, left, right) in geometry.spans(0..source.len(), Some(px(4.))) {
            assert!(left <= right);
        }
    }
}

#[test]
fn a_textarea_scrolls_to_keep_the_caret_in_view() {
    let text = (0..12)
        .map(|line| format!("line {line:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let len = text.len();
    let (mut visual, input) = open_field(300., move |focus| {
        TextField::textarea(focus, text, "", 3, false)
    });
    let origin = text_origin(&visual, &input);
    let bottom = px(-(12. - 3.) * ROW);
    click(&mut visual, at(origin, 1., 0.), 1);

    visual.simulate_keystrokes("end");
    assert_eq!(scroll_y(&visual, &input), bottom, "moving to the end");
    visual.simulate_keystrokes("home");
    assert_eq!(scroll_y(&visual, &input), px(0.), "moving to the start");
    visual.simulate_keystrokes("down down down down");
    assert_eq!(
        scroll_y(&visual, &input),
        px(-2. * ROW),
        "moving down a row at a time"
    );

    input.update(&mut visual, |field, cx| {
        field.buffer.move_to(len, false);
        cx.notify();
    });
    visual.run_until_parked();
    assert_eq!(
        scroll_y(&visual, &input),
        px(-2. * ROW),
        "only edits and moves scroll"
    );
    visual.simulate_input("x");
    assert_eq!(
        input.read_with(&visual, |field, _| field.text().len()),
        len + 1
    );
    assert_eq!(scroll_y(&visual, &input), bottom, "typing at the end");
}

#[test]
fn auto_grow_follows_the_wrapped_rows_up_to_its_maximum() {
    let (mut visual, input) = open_field(110., |focus| {
        TextField::textarea(focus, "", "", 1, false).auto_grow(6)
    });
    let mut height_for = |text: &str| {
        input.update(&mut visual, |field, cx| field.reset(text.to_owned(), cx));
        visual.run_until_parked();
        visual.debug_bounds("FIELD_HOST").unwrap().size.height
    };
    let chrome = 14.;
    assert_eq!(height_for(""), px(ROW + chrome), "one row");
    assert_eq!(height_for("one"), px(ROW + chrome));
    assert_eq!(height_for("a\nb\nc"), px(3. * ROW + chrome), "three lines");
    assert_eq!(
        height_for("alpha beta gamma delta"),
        px(4. * ROW + chrome),
        "soft wraps grow it too: alpha / beta / gamma / delta"
    );
    assert_eq!(
        height_for(&"line\n".repeat(20)),
        px(6. * ROW + chrome),
        "capped at its maximum"
    );
}

#[test]
fn ime_bounds_and_character_index_come_from_the_painted_layout() {
    let (mut visual, input) = wrapped_field();
    let origin = text_origin(&visual, &input);
    let host = visual.debug_bounds("FIELD_HOST").unwrap();
    let (beta, empty, across, index) = visual.update(|window, cx| {
        input.update(cx, |field, cx| {
            (
                field.bounds_for_range(8..10, host, window, cx),
                field.bounds_for_range(0..0, host, window, cx),
                field.bounds_for_range(3..8, host, window, cx),
                field.character_index_for_point(at(origin, 2.2, 1.), window, cx),
            )
        })
    });
    assert_bounds_near(
        beta,
        Bounds::from_corners(
            origin + point(px(2. * ADVANCE), px(ROW)),
            origin + point(px(4. * ADVANCE), px(2. * ROW)),
        ),
    );
    assert_eq!(beta.map(|beta| host.contains(&beta.center())), Some(true));
    assert_bounds_near(empty, Bounds::new(origin, size(px(0.), px(ROW))));
    // A range across a wrap reports its first row.
    assert_bounds_near(
        across,
        Bounds::from_corners(
            origin + point(px(3. * ADVANCE), px(0.)),
            origin + point(px(6. * ADVANCE), px(ROW)),
        ),
    );
    assert_eq!(index, Some(8));

    click(&mut visual, at(origin, 9., 0.), 1);
    assert_eq!(caret(&visual, &input), 6);
    let wrap_end = visual.update(|window, cx| {
        input.update(cx, |field, cx| {
            field.bounds_for_range(6..6, host, window, cx)
        })
    });
    assert_bounds_near(
        wrap_end,
        Bounds::new(
            origin + point(px(6. * ADVANCE), px(0.)),
            size(px(0.), px(ROW)),
        ),
    );
}

#[test]
fn an_empty_field_places_the_caret_before_its_placeholder_and_takes_input() {
    let (mut visual, input) = open_field(300., |focus| {
        TextField::new(focus, "", "Search roles", false)
    });
    let origin = text_origin(&visual, &input);
    click(&mut visual, origin + point(px(40.), px(5.)), 1);
    assert_near(caret_position(&visual, &input), Point::default());
    visual.simulate_input("ab");
    visual.simulate_keystrokes("left");
    assert_eq!(
        input.read_with(&visual, |field, _| field.text().to_owned()),
        "ab"
    );
    assert_eq!(caret(&visual, &input), 1);
    visual.simulate_keystrokes("up");
    assert_eq!(caret(&visual, &input), 1, "a single-line field ignores Up");
}

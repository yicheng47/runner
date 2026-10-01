use super::*;

use super::super::buffer::Boundary;
use super::super::input::handle_key_down_for_platform;

fn type_text(input: &mut TextBuffer, text: &str) {
    for character in text.chars() {
        input.replace_text_in_range(None, &character.to_string());
    }
}

fn caret_at(offset: usize) -> Selection {
    Selection {
        anchor: offset,
        caret: offset,
    }
}

#[test]
fn typing_merges_into_one_step() {
    let mut input = TextBuffer::default();
    input.reset("say ");
    type_text(&mut input, "hello");
    assert_eq!(input.text, "say hello");
    assert_eq!(input.history.undo.len(), 1);

    input.edited = false;
    assert!(input.undo());
    assert_eq!(input.text, "say ");
    assert_eq!(input.selection, caret_at(4));
    assert!(input.edited, "an undo is an edit");
    assert!(!input.undo());
    assert!(input.redo());
    assert_eq!(input.text, "say hello");
    assert_eq!(input.selection, caret_at(9));
    assert!(!input.redo());
}

#[test]
fn a_newline_splits_a_run_and_typing_after_it_is_a_new_step() {
    let mut input = TextBuffer {
        multiline: true,
        ..TextBuffer::default()
    };
    input.reset("");
    type_text(&mut input, "one");
    assert!(input.replace_selection("\n"), "Enter");
    type_text(&mut input, "two");
    type_text(&mut input, "\n");
    type_text(&mut input, "three");
    assert_eq!(input.text, "one\ntwo\nthree");

    for text in ["one\ntwo\n", "one\ntwo", "one\n", "one", ""] {
        assert!(input.undo());
        assert_eq!(input.text, text);
    }
    assert!(!input.undo());
}

#[test]
fn paste_cut_and_delete_selection_are_one_step_each() {
    let mut input = TextBuffer::default();
    input.reset("");
    type_text(&mut input, "ab");
    assert!(input.replace_selection("cd"), "paste");
    type_text(&mut input, "ef");
    assert!(input.replace_selection("gh"), "paste");
    assert!(input.replace_selection("ij"), "paste");
    assert_eq!(input.text, "abcdefghij");
    assert_eq!(input.history.undo.len(), 5);

    input.select_all();
    assert!(input.delete_left(Boundary::Grapheme), "cut");
    assert_eq!(input.text, "");
    assert!(input.undo());
    assert_eq!(input.text, "abcdefghij");
    assert_eq!(
        input.selection,
        Selection {
            anchor: 0,
            caret: 10
        },
        "the cut text is selected again"
    );

    input.move_to(2, false);
    input.move_to(4, true);
    assert!(input.delete_left(Boundary::Grapheme), "Backspace over cd");
    input.move_to(0, false);
    input.move_to(2, true);
    assert!(input.delete_right(Boundary::Grapheme), "Delete over ab");
    assert_eq!(input.text, "efghij");

    for text in [
        "abefghij",
        "abcdefghij",
        "abcdefgh",
        "abcdef",
        "abcd",
        "ab",
        "",
    ] {
        assert!(input.undo());
        assert_eq!(input.text, text);
    }
}

#[test]
fn backspace_and_forward_delete_runs_merge() {
    let mut input = TextBuffer::default();
    input.reset("a王菲bcdef");
    input.move_to("a王菲".len(), false);
    assert!(input.delete_left(Boundary::Grapheme));
    assert!(input.delete_left(Boundary::Grapheme));
    assert_eq!(input.text, "abcdef");
    input.move_to(2, false);
    assert!(input.delete_right(Boundary::Grapheme));
    assert!(input.delete_right(Boundary::Grapheme));
    assert_eq!(input.text, "abef");
    assert!(
        input.delete_left(Boundary::Grapheme),
        "a Backspace after Delete"
    );
    assert_eq!(input.history.undo.len(), 3);

    assert!(input.undo());
    assert_eq!(input.text, "abef");
    assert_eq!(input.selection, caret_at(2));
    assert!(input.undo());
    assert_eq!(input.text, "abcdef");
    assert_eq!(input.selection, caret_at(2));
    assert!(input.undo());
    assert_eq!(input.text, "a王菲bcdef");
    assert_eq!(input.selection, caret_at("a王菲".len()));

    input.reset("one two three");
    assert!(input.delete_left(Boundary::Word));
    assert!(input.delete_left(Boundary::Word));
    assert_eq!(input.history.undo.len(), 2, "word deletes are steps");
    assert!(input.delete_left(Boundary::Line));
    assert_eq!(input.history.undo.len(), 3, "so is a line delete");
}

#[test]
fn a_caret_move_splits_typing() {
    let mut input = TextBuffer::default();
    input.reset("");
    type_text(&mut input, "ab");
    input.move_left(Boundary::Grapheme, false);
    input.move_right(Boundary::Grapheme, false);
    type_text(&mut input, "cd");
    input.select_all();
    input.move_to(4, false);
    type_text(&mut input, "e");
    input.select_word_at(0);
    input.move_to(5, false);
    type_text(&mut input, "f");
    assert_eq!(input.text, "abcdef");
    assert_eq!(input.history.undo.len(), 4);

    assert!(input.undo());
    assert!(input.undo());
    assert!(input.undo());
    assert_eq!(input.text, "ab");
    assert_eq!(input.selection, caret_at(2));
}

#[test]
fn a_new_edit_clears_redo() {
    let mut input = TextBuffer::default();
    input.reset("");
    type_text(&mut input, "ab");
    assert!(input.replace_selection("cd"));
    assert!(input.undo());
    type_text(&mut input, "x");
    assert!(!input.redo());
    assert_eq!(input.text, "abx");
    assert!(input.undo());
    assert_eq!(input.text, "ab", "typing after an undo is a new step");
}

#[test]
fn undo_and_redo_restore_the_selection() {
    let mut input = TextBuffer::default();
    input.reset("alpha beta");
    let beta = Selection {
        anchor: 10,
        caret: 6,
    };
    input.selection = beta;
    type_text(&mut input, "gamma");
    assert_eq!(input.text, "alpha gamma");
    assert_eq!(input.history.undo.len(), 2, "the replacement, then typing");

    assert!(input.undo());
    assert_eq!(input.text, "alpha g");
    assert_eq!(input.selection, caret_at(7));
    assert!(input.undo());
    assert_eq!(input.text, "alpha beta");
    assert_eq!(
        input.selection, beta,
        "the replaced text, as it was selected"
    );
    assert!(input.redo());
    assert_eq!(input.text, "alpha g");
    assert_eq!(input.selection, caret_at(7));
    assert!(input.redo());
    assert_eq!(input.text, "alpha gamma");
    assert_eq!(input.selection, caret_at(11), "the end of the merged run");
}

#[test]
fn an_ime_composition_is_one_step_with_utf8_offsets() {
    let mut input = TextBuffer::default();
    input.reset("王菲");
    input.move_to("王".len(), false);
    for (marked, caret) in [("n", 1), ("ni", 2), ("ni h", 4), ("ni hao", 6)] {
        input.replace_and_mark_text_in_range(None, marked, Some(caret..caret));
        assert_eq!(input.text, format!("王{marked}菲"));
        assert!(input.history.undo.is_empty(), "{marked} is not a step");
        assert!(!input.undo(), "undo waits for the composition");
        assert!(!input.redo());
        assert_eq!(input.text, format!("王{marked}菲"));
    }
    assert!(input.replace_text_in_range(None, "你好"));
    assert_eq!(input.text, "王你好菲");
    assert!(input.marked.is_none());
    assert_eq!(input.history.undo.len(), 1);
    assert_eq!(
        input.history.undo[0].changes,
        [Change {
            offset: "王".len(),
            old_text: String::new(),
            new_text: "你好".into(),
        }]
    );

    assert!(input.undo());
    assert_eq!(input.text, "王菲");
    assert_eq!(input.selection, caret_at("王".len()));
    assert!(input.redo());
    assert_eq!(input.text, "王你好菲");
    assert_eq!(input.selection, caret_at("王你好".len()));

    type_text(&mut input, "!");
    assert_eq!(input.history.undo.len(), 2, "the next keystroke is a step");

    // A composition over a selection undoes to that selection.
    input.move_to("王你好!".len(), false);
    input.move_to("王你好!菲".len(), true);
    input.replace_and_mark_text_in_range(None, "fei", Some(3..3));
    input.replace_text_in_range(None, "飞");
    assert_eq!(input.text, "王你好!飞");
    assert!(input.undo());
    assert_eq!(input.text, "王你好!菲");
    assert_eq!(
        input.selection,
        Selection {
            anchor: "王你好!".len(),
            caret: "王你好!菲".len(),
        }
    );
}

#[test]
fn a_cancelled_composition_records_nothing_and_an_unmarked_one_is_a_step() {
    let mut input = TextBuffer::default();
    input.reset("ab");
    input.replace_and_mark_text_in_range(None, "n", Some(1..1));
    input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
    input.replace_and_mark_text_in_range(None, "", None);
    assert_eq!(input.text, "ab");
    assert!(input.marked.is_none());
    assert!(input.history.undo.is_empty());

    input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
    input.replace_text_in_range(None, "");
    assert_eq!(input.text, "ab");
    assert!(input.history.undo.is_empty());

    input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
    input.unmark_text();
    assert_eq!(input.text, "abni");
    assert_eq!(input.history.undo.len(), 1);

    input.replace_and_mark_text_in_range(None, "hao", Some(3..3));
    input.move_to(0, false);
    assert_eq!(input.history.undo.len(), 2, "a click keeps the composition");
    assert!(input.undo());
    assert_eq!(input.text, "abni");
    assert_eq!(input.selection, caret_at(4));
    assert!(input.undo());
    assert_eq!(input.text, "ab");
}

#[test]
fn reset_clears_the_history() {
    let mut input = TextBuffer::default();
    input.reset("");
    type_text(&mut input, "ab");
    assert!(input.replace_selection("cd"));
    assert!(input.undo());
    input.replace_and_mark_text_in_range(None, "ni", Some(2..2));

    input.reset("fresh");
    assert!(!input.undo());
    assert!(!input.redo());
    assert_eq!(input.text, "fresh");
    type_text(&mut input, "!");
    assert!(input.undo());
    assert_eq!(input.text, "fresh");
    assert!(!input.undo());
}

#[test]
fn set_text_is_one_step_and_corrects_the_edit_it_follows() {
    let mut input = TextBuffer::default();
    input.reset("draft");
    assert!(input.set_text("final copy"));
    assert!(!input.set_text("final copy"));
    assert_eq!(input.history.undo.len(), 1);
    assert!(input.undo());
    assert_eq!(input.text, "draft");
    assert_eq!(input.selection, caret_at(5));
    assert!(input.redo());
    assert_eq!(input.text, "final copy");
    assert_eq!(input.selection, caret_at(10));

    // The handle fields lowercase what was typed through `set_text`.
    type_text(&mut input, "A");
    assert!(input.set_text("final copya"));
    assert!(input.replace_selection("BC"));
    assert!(input.set_text("final copyabc"));
    assert_eq!(input.history.undo.len(), 3);
    assert!(input.undo());
    assert_eq!(input.text, "final copya");
    assert!(input.undo());
    assert_eq!(input.text, "final copy");
    assert_eq!(input.selection, caret_at(10));
    assert!(input.redo());
    assert_eq!(input.text, "final copya");
    assert_eq!(input.selection, caret_at(11));

    type_text(&mut input, "D");
    input.move_to(0, false);
    assert!(input.set_text("final copyad"));
    assert!(input.undo());
    assert_eq!(input.text, "final copyaD", "after a move it is a step");

    type_text(&mut input, "e");
    assert!(input.set_text("FINAL COPYAD"));
    assert!(input.undo());
    assert_eq!(input.text, "efinal copyaD", "so is uppercasing");
    assert!(input.undo());
    assert_eq!(input.text, "final copyaD");

    type_text(&mut input, "x");
    assert!(input.set_text("canonical"));
    assert!(input.undo());
    assert_eq!(
        input.text, "xfinal copyaD",
        "a new value after an edit is a step"
    );
    assert!(input.undo());
    assert_eq!(input.text, "final copyaD");
}

#[test]
fn the_history_keeps_the_last_1000_steps() {
    let mut input = TextBuffer::default();
    input.reset("");
    for _ in 0..1100 {
        input.replace_selection("x");
    }
    assert_eq!(input.history.undo.len(), MAX_UNDO_STEPS);
    type_text(&mut input, "A");
    assert!(input.set_text(&format!("{}a", "x".repeat(1100))));
    assert_eq!(
        input.history.undo.len(),
        MAX_UNDO_STEPS,
        "a correction drops no step"
    );
    assert!(input.undo());
    assert_eq!(input.text, "x".repeat(1100));
    for _ in 1..MAX_UNDO_STEPS {
        assert!(input.undo());
    }
    assert!(!input.undo());
    assert_eq!(input.text, "x".repeat(101));
}

#[test]
fn undo_and_redo_keys_follow_the_platform() {
    let cx = gpui::TestAppContext::single();
    for windows in [false, true] {
        cx.update(|cx| {
            let input = cx.new(|_| TextBuffer::default());
            input.update(cx, |input, cx| {
                let press = |input: &mut TextBuffer, key: &str, cx: &mut Context<TextBuffer>| {
                    handle_key_down_for_platform(
                        input,
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(key).unwrap(),
                            is_held: false,
                            prefer_character_input: false,
                        },
                        cx,
                        windows,
                    )
                };
                let command = if windows { "ctrl" } else { "cmd" };
                input.reset("");
                type_text(input, "one");
                cx.write_to_clipboard(ClipboardItem::new_string(" two".into()));
                assert!(press(input, &format!("{command}-v"), cx));
                assert_eq!(input.text, "one two");
                assert!(press(input, &format!("{command}-z"), cx));
                assert_eq!(input.text, "one");
                assert!(press(input, &format!("{command}-shift-z"), cx));
                assert_eq!(input.text, "one two");
                assert!(press(input, &format!("{command}-a"), cx));
                assert!(press(input, &format!("{command}-x"), cx));
                assert_eq!(input.text, "");
                assert!(press(input, &format!("{command}-z"), cx));
                assert_eq!(input.text, "one two");
                assert_eq!(input.selected_text(), Some("one two"));
                assert!(press(input, &format!("{command}-z"), cx));
                assert!(press(input, &format!("{command}-z"), cx));
                assert_eq!(input.text, "");
                assert!(press(input, &format!("{command}-z"), cx), "nothing left");
                if windows {
                    assert!(press(input, "ctrl-y", cx));
                    assert_eq!(input.text, "one");
                    assert!(!press(input, "cmd-z", cx));
                } else {
                    assert!(!press(input, "cmd-y", cx));
                    assert!(!press(input, "ctrl-z", cx));
                    assert!(press(input, "cmd-shift-z", cx));
                    assert_eq!(input.text, "one");
                }
            });
        });
    }
}

#[test]
fn undo_and_redo_reach_an_enabled_field_by_key_and_action() {
    let (mut visual, input) = open_field(300., |focus| TextField::new(focus, "", "", false));
    let origin = text_origin(&visual, &input);
    let text = |visual: &gpui::VisualTestContext| {
        input.read_with(visual, |field, _| field.text().to_owned())
    };
    let undo = if cfg!(windows) { "ctrl-z" } else { "cmd-z" };
    click(&mut visual, origin + point(px(40.), px(5.)), 1);
    visual.simulate_input("one");
    visual.simulate_keystrokes("left");
    visual.simulate_input("x");
    assert_eq!(text(&visual), "onxe");

    input.update(&mut visual, |field, _| field.mark_clean());
    visual.simulate_keystrokes(undo);
    assert_eq!(text(&visual), "one");
    assert_eq!(caret(&visual, &input), 2);
    assert!(input.read_with(&visual, |field, _| field.edited()));
    visual.dispatch_action(Undo);
    assert_eq!(text(&visual), "");
    visual.dispatch_action(Redo);
    assert_eq!(text(&visual), "one");

    input.update(&mut visual, |field, cx| field.set_disabled(true, cx));
    visual.dispatch_action(Undo);
    visual.simulate_keystrokes(undo);
    assert_eq!(text(&visual), "one", "a disabled field keeps its text");
}

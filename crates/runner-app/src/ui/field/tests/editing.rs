use super::*;

use super::super::buffer::Boundary;
use super::super::input::enter_behavior;
#[cfg(test)]
use super::super::input::enter_should_submit;
use super::super::input::handle_key_down_for_platform;
use super::super::input::EnterBehavior;

#[test]
fn editing_shortcuts_use_command_on_macos_and_control_on_windows() {
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
                input.reset("one two");
                assert!(press(input, &format!("{command}-a"), cx));
                assert_eq!(input.selected_text(), Some("one two"));
                assert!(press(input, &format!("{command}-c"), cx));
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().as_deref(),
                    Some("one two")
                );
                assert!(press(input, &format!("{command}-x"), cx));
                assert_eq!(input.text, "");
                assert!(press(input, &format!("{command}-v"), cx));
                assert_eq!(input.text, "one two");
                let word = if windows { "ctrl" } else { "alt" };
                assert!(press(input, &format!("{word}-left"), cx));
                assert_eq!(input.selection.caret, 4);
                assert!(press(input, &format!("{word}-shift-right"), cx));
                assert_eq!(input.selected_text(), Some("two"));
                assert!(press(input, "right", cx));
                assert_eq!(input.selection.caret, 7);
                assert!(press(input, "home", cx));
                assert_eq!(input.selection.caret, 0);
                assert!(press(input, "end", cx));
                assert_eq!(input.selection.caret, 7);
                if !windows {
                    assert!(!press(input, "ctrl-a", cx));
                    assert!(press(input, "cmd-left", cx));
                    assert_eq!(input.selection.caret, 0);
                    assert!(press(input, "cmd-right", cx));
                    assert_eq!(input.selection.caret, 7);
                }
            });
        });
    }
}

#[test]
fn marked_text_uses_utf16_offsets_and_blocks_enter_until_committed() {
    let mut input = TextBuffer::default();
    input.reset("王菲");
    input.selection = Selection {
        anchor: "王".len(),
        caret: "王".len(),
    };

    assert!(input.replace_and_mark_text_in_range(None, "bianji", Some(6..6)));
    assert_eq!(input.text, "王bianji菲");
    assert_eq!(input.marked_text_range(), Some(1..7));
    assert!(!enter_should_submit(input.marked.is_some()));

    assert!(input.replace_and_mark_text_in_range(None, "编辑", Some(2..2)));
    assert!(!input.replace_text_in_range(None, "编辑"));
    assert_eq!(input.text, "王编辑菲");
    assert!(enter_should_submit(input.marked.is_some()));
}

#[test]
fn deletion_respects_grapheme_clusters() {
    let mut input = TextBuffer::default();
    input.reset("A👨‍👩‍👧‍👦B");
    input.move_left(Boundary::Grapheme, false);

    assert!(input.delete_left(Boundary::Grapheme));
    assert_eq!(input.text, "AB");
    assert!(input.edited);
}

#[test]
fn marked_text_updates_replace_the_existing_composition() {
    let mut input = TextBuffer::default();
    input.reset("王菲");
    input.selection = Selection {
        anchor: "王".len(),
        caret: "王".len(),
    };

    assert!(input.replace_and_mark_text_in_range(None, "bian", Some(4..4)));
    assert!(input.replace_and_mark_text_in_range(None, "编辑", Some(2..2)));

    assert_eq!(input.text, "王编辑菲");
    assert_eq!(input.marked_text_range(), Some(1..3));
    assert_eq!(input.selected_text_range().range, 3..3);
    assert_eq!(input.character_index_utf16(), 3);
}

#[test]
fn selection_round_trips_through_utf16_for_clipboard_text() {
    let mut input = TextBuffer::default();
    input.reset("A😀王B");
    input.selection = Selection {
        anchor: "A😀王".len(),
        caret: "A".len(),
    };

    assert_eq!(input.selected_text(), Some("😀王"));
    assert_eq!(input.selected_text_range().range, 1..4);
    assert!(input.selected_text_range().reversed);

    let mut adjusted = None;
    assert_eq!(input.text_for_range(1..4, &mut adjusted), "😀王");
    assert_eq!(adjusted, Some(1..4));
}

#[test]
fn pointer_selection_uses_byte_safe_word_and_line_ranges() {
    let mut input = TextBuffer::default();
    input.reset("alpha 王菲\nbeta");

    input.move_to("alpha ".len(), false);
    input.move_to("alpha 王菲".len(), true);
    assert_eq!(input.selected_text(), Some("王菲"));

    input.select_word_at("alpha ".len());
    assert_eq!(input.selected_text(), Some("王"));

    input.select_line_at("alpha 王".len());
    assert_eq!(input.selected_text(), Some("alpha 王菲\n"));
}

#[test]
fn single_line_input_normalizes_commits_and_paste() {
    let mut input = TextBuffer::default();
    input.reset("");

    assert!(input.replace_text_in_range(None, "alpha\r\nbeta\ngamma"));
    assert_eq!(input.text, "alpha beta gamma");
}

#[test]
fn enter_submits_only_after_composition_is_committed() {
    assert!(enter_should_submit(false));
    assert!(!enter_should_submit(true));
}

#[test]
fn multiline_commits_preserve_lines_and_enter_inserts_a_line() {
    let mut input = TextBuffer {
        multiline: true,
        ..TextBuffer::default()
    };
    input.reset("alpha");

    assert!(input.replace_text_in_range(None, "beta\r\ngamma"));
    assert_eq!(input.text, "alphabeta\ngamma");
    assert_eq!(
        enter_behavior(TextFieldKind::Textarea { rows: 3 }, false),
        EnterBehavior::InsertNewline
    );
    assert_eq!(
        enter_behavior(TextFieldKind::Textarea { rows: 3 }, true),
        EnterBehavior::Block
    );
}

#[test]
fn field_validation_exposes_only_error_messages() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    assert!(!FieldValidation::Valid.is_error());
    assert_eq!(FieldValidation::Valid.message(), None);

    let error = FieldValidation::error("Required");
    assert!(error.is_error());
    assert_eq!(error.message().map(SharedString::as_ref), Some("Required"));
    assert_eq!(input_border_color(&error, false), theme::danger());
    assert_eq!(input_border_color(&error, true), theme::danger());
    assert_eq!(
        input_border_color(&FieldValidation::Valid, true),
        theme::faint()
    );
}

#[test]
fn range_replacement_preserves_unicode_text_and_is_one_undo_step() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        let input =
            cx.new(|cx| TextField::textarea(cx.focus_handle(), "请 👩‍💻 @rev 后续", "", 1, false));
        input.update(cx, |input, cx| {
            let start = "请 👩‍💻 ".len();
            let caret = "请 👩‍💻 @rev".len();
            input.buffer.move_to(caret, false);
            assert!(input.replace_range(start..caret, "@reviewer ", cx));
            assert_eq!(input.text(), "请 👩‍💻 @reviewer  后续");
            assert_eq!(input.caret_offset(), "请 👩‍💻 @reviewer ".len());
            assert!(input.edited());
            assert!(input.buffer.undo());
            assert_eq!(input.text(), "请 👩‍💻 @rev 后续");
            assert_eq!(input.caret_offset(), caret);
            assert!(!input.buffer.undo());
            assert!(input.buffer.redo());
            assert_eq!(input.text(), "请 👩‍💻 @reviewer  后续");
            assert_eq!(input.caret_offset(), "请 👩‍💻 @reviewer ".len());
        });
    });
}

#[test]
fn range_replacement_refuses_active_composition_invalid_ranges_and_disabled_input() {
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        let input = cx.new(|cx| TextField::new(cx.focus_handle(), "请 @rev", "", false));
        input.update(cx, |input, cx| {
            input
                .buffer
                .replace_and_mark_text_in_range(None, "ni", Some(2..2));
            let before = input.buffer.clone();
            assert!(!input.replace_range("请 ".len().."请 @rev".len(), "@reviewer ", cx));
            assert_eq!(input.buffer, before);
            input.buffer.unmark_text();
            let before = input.buffer.clone();
            assert!(!input.replace_range(1..2, "", cx));
            assert!(!input.replace_range(0..input.text().len() + 1, "", cx));
            assert_eq!(input.buffer, before);
            input.set_disabled(true, cx);
            assert!(!input.replace_range(0..input.text().len(), "", cx));
            assert_eq!(input.buffer, before);
        });
    });
}

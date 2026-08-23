use runner_app::terminal_ime::{terminal_key_route, TerminalComposition, TerminalKeyRoute};

#[test]
fn composition_replaces_and_clears_marked_text_with_utf16_selection_math() {
    let mut composition = TerminalComposition::default();

    composition.replace_and_mark(None, "ni", Some(2..2));
    assert_eq!(composition.marked_text(), Some("ni"));
    assert_eq!(composition.marked_range_utf16(), Some(0..2));
    assert_eq!(composition.selected_range(), &(2..2));

    composition.replace_and_mark(Some(1..2), "ǐ", Some(1..1));
    assert_eq!(composition.marked_text(), Some("nǐ"));
    assert_eq!(composition.marked_range_utf16(), Some(0..2));
    assert_eq!(composition.selected_range(), &(3..3));

    composition.replace_and_mark(None, "拼音", Some(1..1));
    assert_eq!(composition.marked_text(), Some("拼音"));
    assert_eq!(composition.marked_range_utf16(), Some(0..2));
    assert_eq!(composition.selected_range(), &(3..3));

    composition.clear();
    assert_eq!(composition.marked_text(), None);
    assert_eq!(composition.marked_range_utf16(), None);
    assert_eq!(composition.selected_range(), &(0..0));
}

#[test]
fn routing_sends_printable_text_only_through_the_input_handler() {
    assert_eq!(
        terminal_key_route(false, false, false, false, false, "n"),
        TerminalKeyRoute::Ime
    );
    assert_eq!(
        terminal_key_route(false, false, false, false, false, "中"),
        TerminalKeyRoute::Ime
    );
    assert_eq!(
        terminal_key_route(false, false, true, false, false, "c"),
        TerminalKeyRoute::Raw
    );
    assert_eq!(
        terminal_key_route(false, false, false, true, false, "b"),
        TerminalKeyRoute::Raw
    );
    assert_eq!(
        terminal_key_route(false, false, false, false, true, "f1"),
        TerminalKeyRoute::Raw
    );
    assert_eq!(
        terminal_key_route(false, true, false, false, false, "t"),
        TerminalKeyRoute::AppShortcut
    );
    assert_eq!(
        terminal_key_route(false, false, false, false, false, "enter"),
        TerminalKeyRoute::Raw
    );
    assert_eq!(
        terminal_key_route(false, false, false, false, false, "left"),
        TerminalKeyRoute::Raw
    );
}

#[test]
fn composing_routes_special_keys_to_the_ime() {
    for key in ["enter", "backspace", "left"] {
        assert_eq!(
            terminal_key_route(true, false, false, false, false, key),
            TerminalKeyRoute::Ime
        );
    }
}

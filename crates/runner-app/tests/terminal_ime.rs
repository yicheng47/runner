use gpui::Keystroke;
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
    for (key, expected) in [
        ("n", TerminalKeyRoute::Ime),
        ("中", TerminalKeyRoute::Ime),
        ("ctrl-c", TerminalKeyRoute::Raw),
        ("alt-b", TerminalKeyRoute::Raw),
        ("fn-f1", TerminalKeyRoute::Raw),
        ("cmd-t", TerminalKeyRoute::AppShortcut),
        ("cmd-k", TerminalKeyRoute::AppShortcut),
        ("cmd-c", TerminalKeyRoute::AppShortcut),
        ("cmd-v", TerminalKeyRoute::AppShortcut),
        ("shift-cmd-left", TerminalKeyRoute::AppShortcut),
        ("alt-cmd-left", TerminalKeyRoute::AppShortcut),
        ("ctrl-cmd-left", TerminalKeyRoute::AppShortcut),
        ("fn-cmd-left", TerminalKeyRoute::AppShortcut),
        ("shift-cmd-backspace", TerminalKeyRoute::AppShortcut),
        ("cmd-delete", TerminalKeyRoute::AppShortcut),
        ("enter", TerminalKeyRoute::Raw),
        ("left", TerminalKeyRoute::Raw),
    ] {
        assert_eq!(
            terminal_key_route(false, &Keystroke::parse(key).unwrap(), true),
            expected,
            "{key}",
        );
    }
}

#[test]
fn macos_editing_chords_route_raw_only_on_macos_outside_composition() {
    let expected = if cfg!(target_os = "macos") {
        TerminalKeyRoute::Raw
    } else {
        TerminalKeyRoute::AppShortcut
    };
    for key in ["cmd-left", "cmd-right", "cmd-backspace"] {
        let keystroke = Keystroke::parse(key).unwrap();
        for option_is_meta in [false, true] {
            assert_eq!(
                terminal_key_route(false, &keystroke, option_is_meta),
                expected,
                "{key} option_is_meta={option_is_meta}",
            );
        }
        assert_eq!(
            terminal_key_route(true, &keystroke, true),
            TerminalKeyRoute::AppShortcut,
            "{key} while composing",
        );
    }
}

#[test]
fn composing_routes_special_keys_to_the_ime() {
    for key in ["enter", "backspace", "left"] {
        assert_eq!(
            terminal_key_route(true, &Keystroke::parse(key).unwrap(), true),
            TerminalKeyRoute::Ime
        );
    }
}

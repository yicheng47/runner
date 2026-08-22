//! Keystroke and paste encoding — the model-side mapping from UI input
//! events to the byte sequences a PTY expects (the `mappings/` analog of
//! Zed's terminal crate). Pure functions: terminal mode state is passed
//! in, bytes come out.

use alacritty_terminal::term::TermMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputKind {
    Content { text: String },
    Edit,
    Submit,
    Cancel,
    Navigate,
}

pub fn classify_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key_char: Option<&str>,
) -> InputKind {
    if key == "enter" && !ctrl && !alt && !shift {
        return InputKind::Submit;
    }
    if key == "enter" && shift && !ctrl && !alt {
        return InputKind::Content { text: "\n".into() };
    }
    if matches!(key, "backspace" | "delete")
        || (ctrl && !alt && matches!(key, "h" | "u" | "w" | "k"))
    {
        return InputKind::Edit;
    }
    if ctrl && !alt && key == "c" {
        return InputKind::Cancel;
    }
    if key == "tab" && !ctrl && !alt {
        return InputKind::Content { text: "\t".into() };
    }
    if !ctrl && !alt {
        if let Some(text) = key_char.filter(|text| text.chars().any(|c| !c.is_control())) {
            return InputKind::Content {
                text: text.to_owned(),
            };
        }
    }
    InputKind::Navigate
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MouseAction {
    Press,
    Release,
    Motion,
}

/// Encode a non-text keystroke (or ctrl-chord) into PTY bytes.
/// `app_cursor` selects the DECCKM application-cursor sequences for the
/// arrow keys. Returns `None` when the key is not ours to handle (the
/// caller lets the event propagate, e.g. to a global binding).
pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key_char: Option<&str>,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    if key == "enter" && shift && !ctrl && !alt {
        return Some(b"\x1b\r".to_vec());
    }

    let mut bytes: Vec<u8> = Vec::new();
    if ctrl {
        let encoded = match key {
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic() => {
                Some(k.as_bytes()[0].to_ascii_uppercase() - b'@')
            }
            "space" | "@" => Some(0),
            "[" => Some(0x1b),
            "\\" => Some(0x1c),
            "]" => Some(0x1d),
            _ => None,
        };
        bytes.push(encoded?);
    } else {
        let mut alt_letter = [0u8; 1];
        let sequence: &[u8] = match key {
            "enter" => b"\r",
            "backspace" => b"\x7f",
            "tab" => b"\t",
            "escape" => b"\x1b",
            "up" if app_cursor => b"\x1bOA",
            "down" if app_cursor => b"\x1bOB",
            "right" if app_cursor => b"\x1bOC",
            "left" if app_cursor => b"\x1bOD",
            "up" => b"\x1b[A",
            "down" => b"\x1b[B",
            "right" => b"\x1b[C",
            "left" => b"\x1b[D",
            "home" => b"\x1b[H",
            "end" => b"\x1b[F",
            "pageup" => b"\x1b[5~",
            "pagedown" => b"\x1b[6~",
            "insert" => b"\x1b[2~",
            "delete" => b"\x1b[3~",
            "f1" => b"\x1bOP",
            "f2" => b"\x1bOQ",
            "f3" => b"\x1bOR",
            "f4" => b"\x1bOS",
            "f5" => b"\x1b[15~",
            "f6" => b"\x1b[17~",
            "f7" => b"\x1b[18~",
            "f8" => b"\x1b[19~",
            "f9" => b"\x1b[20~",
            "f10" => b"\x1b[21~",
            "f11" => b"\x1b[23~",
            "f12" => b"\x1b[24~",
            "f13" => b"\x1b[25~",
            "f14" => b"\x1b[26~",
            "f15" => b"\x1b[28~",
            "f16" => b"\x1b[29~",
            "f17" => b"\x1b[31~",
            "f18" => b"\x1b[32~",
            "f19" => b"\x1b[33~",
            "f20" => b"\x1b[34~",
            "space" => b" ",
            _ if alt && key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
                alt_letter[0] = if shift {
                    key.as_bytes()[0].to_ascii_uppercase()
                } else {
                    key.as_bytes()[0]
                };
                &alt_letter
            }
            _ => match key_char {
                Some(text) if !text.is_empty() => text.as_bytes(),
                _ => return None,
            },
        };
        bytes.extend_from_slice(sequence);
    }
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// Route a wheel gesture the way a real terminal would: apps that
/// enabled mouse reporting (claude, codex — the reason the resume seam
/// disables 1000/1002/1003/1006) receive wheel-button reports and
/// scroll their own transcript; alt-screen apps with DECSET 1007 get
/// arrow keys; everything else returns `None` and the caller scrolls
/// the local viewport. `bypass_reporting` (shift held, xterm
/// convention) forces the viewport path. Reports carry cell (1;1) —
/// the single-transcript agent TUIs we host ignore wheel coordinates.
pub fn encode_scroll(mode: TermMode, delta_lines: i32, bypass_reporting: bool) -> Option<Vec<u8>> {
    if delta_lines == 0 || bypass_reporting {
        return None;
    }
    let up = delta_lines > 0;
    let count = delta_lines.unsigned_abs() as usize;
    if mode.intersects(TermMode::MOUSE_MODE) {
        let button: u8 = if up { 64 } else { 65 };
        return Some(if mode.contains(TermMode::SGR_MOUSE) {
            format!("\x1b[<{button};1;1M").into_bytes().repeat(count)
        } else {
            [0x1b, b'[', b'M', 32 + button, 33, 33].repeat(count)
        });
    }
    if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
        let key = if up { "up" } else { "down" };
        let arrow = encode_key(
            key,
            false,
            false,
            false,
            None,
            mode.contains(TermMode::APP_CURSOR),
        )?;
        return Some(arrow.repeat(count));
    }
    None
}

pub fn encode_mouse_press(
    mode: TermMode,
    button: MouseButton,
    column: usize,
    row: usize,
    modifiers: MouseModifiers,
    bypass_reporting: bool,
) -> Option<Vec<u8>> {
    encode_mouse(
        mode,
        MouseAction::Press,
        button,
        column,
        row,
        modifiers,
        bypass_reporting,
    )
}

pub fn encode_mouse_release(
    mode: TermMode,
    button: MouseButton,
    column: usize,
    row: usize,
    modifiers: MouseModifiers,
    bypass_reporting: bool,
) -> Option<Vec<u8>> {
    encode_mouse(
        mode,
        MouseAction::Release,
        button,
        column,
        row,
        modifiers,
        bypass_reporting,
    )
}

pub fn encode_mouse_motion(
    mode: TermMode,
    button: MouseButton,
    column: usize,
    row: usize,
    modifiers: MouseModifiers,
    bypass_reporting: bool,
) -> Option<Vec<u8>> {
    encode_mouse(
        mode,
        MouseAction::Motion,
        button,
        column,
        row,
        modifiers,
        bypass_reporting,
    )
}

fn encode_mouse(
    mode: TermMode,
    action: MouseAction,
    button: MouseButton,
    column: usize,
    row: usize,
    modifiers: MouseModifiers,
    bypass_reporting: bool,
) -> Option<Vec<u8>> {
    if bypass_reporting || !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    if action == MouseAction::Motion
        && !mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    {
        return None;
    }

    let button_code = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let modifier_code = (u8::from(modifiers.shift) * 4)
        | (u8::from(modifiers.alt) * 8)
        | (u8::from(modifiers.control) * 16);
    let code = button_code | modifier_code | if action == MouseAction::Motion { 32 } else { 0 };
    let column = column + 1;
    let row = row + 1;

    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        Some(format!("\x1b[<{code};{column};{row}{suffix}").into_bytes())
    } else {
        let code = if action == MouseAction::Release {
            modifier_code | 3
        } else {
            code
        };
        let code = code.checked_add(32)?;
        let column = u8::try_from(column).ok()?.checked_add(32)?;
        let row = u8::try_from(row).ok()?.checked_add(32)?;
        Some(vec![0x1b, b'[', b'M', code, column, row])
    }
}

/// Encode pasted text, stripping raw escapes and wrapping in bracketed
/// paste markers when the terminal has that mode enabled.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let sanitized = text.replace('\x1b', "");
    if bracketed {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(sanitized.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        sanitized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_key, encode_key, encode_mouse_motion, encode_mouse_press, encode_mouse_release,
        encode_scroll, InputKind, MouseButton, MouseModifiers, TermMode,
    };

    #[test]
    fn key_classification_separates_submit_content_edit_and_navigation() {
        assert_eq!(
            classify_key("enter", false, false, false, None),
            InputKind::Submit
        );
        assert_eq!(
            classify_key("enter", false, false, true, None),
            InputKind::Content { text: "\n".into() }
        );
        assert_eq!(
            classify_key("enter", false, true, false, None),
            InputKind::Navigate
        );
        assert_eq!(
            classify_key("x", false, false, false, Some("x")),
            InputKind::Content { text: "x".into() }
        );
        for key in ["backspace", "delete"] {
            assert_eq!(
                classify_key(key, false, false, false, None),
                InputKind::Edit
            );
        }
        for key in ["h", "u", "w", "k"] {
            assert_eq!(classify_key(key, true, false, false, None), InputKind::Edit);
        }
        assert_eq!(
            classify_key("c", true, false, false, None),
            InputKind::Cancel
        );
        for key in ["left", "escape", "f1"] {
            assert_eq!(
                classify_key(key, false, false, false, None),
                InputKind::Navigate
            );
        }
    }

    #[test]
    fn option_letter_chords_use_the_plain_key() {
        assert_eq!(
            encode_key("b", false, true, false, Some("∫"), false),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            encode_key("f", false, true, false, Some("ƒ"), false),
            Some(b"\x1bf".to_vec())
        );
        assert_eq!(
            encode_key("d", false, true, false, Some("∂"), false),
            Some(b"\x1bd".to_vec())
        );
        assert_eq!(
            encode_key("d", false, true, true, Some("Î"), false),
            Some(b"\x1bD".to_vec())
        );
    }

    #[test]
    fn option_letter_fix_does_not_change_other_key_paths() {
        assert_eq!(
            encode_key("b", false, false, false, Some("∫"), false),
            Some("∫".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key("b", true, true, false, Some("∫"), false),
            Some(b"\x1b\x02".to_vec())
        );
        assert_eq!(
            encode_key("1", false, true, false, Some("¡"), false),
            Some("\x1b¡".as_bytes().to_vec())
        );
    }

    #[test]
    fn wheel_reports_go_to_mouse_mode_apps() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_scroll(mode, 2, false),
            Some(b"\x1b[<64;1;1M\x1b[<64;1;1M".to_vec())
        );
        assert_eq!(
            encode_scroll(mode, -1, false),
            Some(b"\x1b[<65;1;1M".to_vec())
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, 1, false),
            Some(vec![0x1b, b'[', b'M', 96, 33, 33])
        );
    }

    #[test]
    fn alternate_scroll_sends_arrows_only_on_the_alt_screen() {
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(
            encode_scroll(mode, 2, false),
            Some(b"\x1b[A\x1b[A".to_vec())
        );
        assert_eq!(
            encode_scroll(mode | TermMode::APP_CURSOR, -1, false),
            Some(b"\x1bOB".to_vec())
        );
        assert_eq!(encode_scroll(TermMode::ALTERNATE_SCROLL, 1, false), None);
    }

    #[test]
    fn viewport_scroll_wins_for_plain_apps_and_shift_bypass() {
        assert_eq!(encode_scroll(TermMode::NONE, 3, false), None);
        let mouse = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(encode_scroll(mouse, 3, true), None);
        assert_eq!(encode_scroll(mouse, 0, false), None);
    }

    #[test]
    fn mouse_reports_cover_legacy_press_release_and_drag_modes() {
        let click = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            encode_mouse_press(
                TermMode::NONE,
                MouseButton::Left,
                4,
                2,
                MouseModifiers::default(),
                false,
            ),
            None
        );
        assert_eq!(
            encode_mouse_press(
                click,
                MouseButton::Left,
                4,
                2,
                MouseModifiers::default(),
                false,
            ),
            Some(vec![0x1b, b'[', b'M', 32, 37, 35])
        );
        assert_eq!(
            encode_mouse_release(
                click,
                MouseButton::Left,
                4,
                2,
                MouseModifiers::default(),
                false,
            ),
            Some(vec![0x1b, b'[', b'M', 35, 37, 35])
        );
        assert_eq!(
            encode_mouse_motion(
                click,
                MouseButton::Left,
                4,
                2,
                MouseModifiers::default(),
                false,
            ),
            None
        );
        assert_eq!(
            encode_mouse_motion(
                TermMode::MOUSE_DRAG,
                MouseButton::Right,
                4,
                2,
                MouseModifiers {
                    alt: true,
                    ..Default::default()
                },
                false,
            ),
            Some(vec![0x1b, b'[', b'M', 74, 37, 35])
        );
    }

    #[test]
    fn sgr_mouse_reports_preserve_button_modifiers_and_large_columns() {
        let mode = TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE;
        let modifiers = MouseModifiers {
            shift: true,
            alt: true,
            control: true,
        };
        assert_eq!(
            encode_mouse_press(mode, MouseButton::Middle, 299, 4, modifiers, false),
            Some(b"\x1b[<29;300;5M".to_vec())
        );
        assert_eq!(
            encode_mouse_motion(mode, MouseButton::Middle, 299, 4, modifiers, false),
            Some(b"\x1b[<61;300;5M".to_vec())
        );
        assert_eq!(
            encode_mouse_release(mode, MouseButton::Middle, 299, 4, modifiers, false),
            Some(b"\x1b[<29;300;5m".to_vec())
        );
        assert_eq!(
            encode_mouse_press(
                mode,
                MouseButton::Left,
                0,
                0,
                MouseModifiers::default(),
                true,
            ),
            None
        );
    }

    #[test]
    fn legacy_mouse_suppresses_unaddressable_coordinates() {
        assert_eq!(
            encode_mouse_press(
                TermMode::MOUSE_REPORT_CLICK,
                MouseButton::Left,
                223,
                0,
                MouseModifiers::default(),
                false,
            ),
            None
        );
    }

    #[test]
    fn function_keys_keep_their_terminal_sequences() {
        assert_eq!(
            encode_key("f1", false, false, false, None, false),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key("f12", false, false, false, None, false),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            encode_key("f20", false, false, false, None, false),
            Some(b"\x1b[34~".to_vec())
        );
    }

    #[test]
    fn shift_enter_uses_runner_multiline_sequence() {
        assert_eq!(
            encode_key("enter", false, false, true, None, false),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            encode_key("enter", false, false, false, None, false),
            Some(b"\r".to_vec())
        );
    }
}

//! Keystroke and paste encoding — the model-side mapping from UI input
//! events to the byte sequences a PTY expects (the `mappings/` analog of
//! Zed's terminal crate). Pure functions: terminal mode state is passed
//! in, bytes come out.

use alacritty_terminal::term::TermMode;
pub use runner_core::protocol::terminal::InputKind;

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
    if key == "tab" && !ctrl && !alt && !shift {
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
/// `mode` selects the DECCKM application-cursor sequences for the arrow
/// keys and the kitty key forms an app has opted into. Returns `None` when
/// the key is not ours to handle (the caller lets the event propagate,
/// e.g. to a global binding).
pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key_char: Option<&str>,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        if let Some(bytes) = encode_kitty_key(key, ctrl, alt, shift) {
            return Some(bytes);
        }
    }
    if key == "enter" && shift && !ctrl && !alt {
        return Some(b"\x1b\r".to_vec());
    }
    if key == "tab" && shift && !ctrl && !alt {
        return Some(b"\x1b[Z".to_vec());
    }
    if let Some(bytes) = encode_modified_special_key(key, ctrl, alt, shift) {
        return Some(bytes);
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
            "up" if mode.contains(TermMode::APP_CURSOR) => b"\x1bOA",
            "down" if mode.contains(TermMode::APP_CURSOR) => b"\x1bOB",
            "right" if mode.contains(TermMode::APP_CURSOR) => b"\x1bOC",
            "left" if mode.contains(TermMode::APP_CURSOR) => b"\x1bOD",
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

/// The kitty keyboard protocol's "disambiguate escape codes" level, for an
/// app that pushed it. Esc and every chord whose legacy bytes are
/// ambiguous become `CSI code ; mods u`, with the unshifted codepoint and
/// `mods = 1 + shift + 2*alt + 4*ctrl`. A strict decoder such as pi stops
/// reading `ESC d` as Alt+D once the protocol is on, and reads `ESC CR` as
/// Shift+Enter, so these chords cannot keep their legacy bytes. Plain and
/// shifted text, unmodified Enter/Tab/Backspace and the functional keys keep
/// their existing encodings. A chord the legacy path leaves to the app stays
/// unhandled here too.
fn encode_kitty_key(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
    let modified_named = (alt || shift) && !ctrl;
    let (code, csi_u) = match key {
        "escape" => (27, !ctrl),
        "enter" => (13, modified_named),
        "tab" => (9, modified_named),
        "backspace" => (127, modified_named),
        "space" => (32, ctrl || alt),
        _ => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let ctrl_has_legacy_form =
                c.is_ascii_alphabetic() || matches!(c, '@' | '[' | '\\' | ']');
            (
                c.to_ascii_lowercase() as u32,
                alt || (ctrl && ctrl_has_legacy_form),
            )
        }
    };
    if !csi_u {
        return None;
    }
    let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    Some(if mods > 1 {
        format!("\x1b[{code};{mods}u").into_bytes()
    } else {
        format!("\x1b[{code}u").into_bytes()
    })
}

/// xterm's modified form for the keys whose plain sequence already
/// starts with ESC. Prefixing those with another ESC (the Option-as-Meta
/// convention that is right for letters) hands the TUI `ESC ESC [ C`,
/// which crossterm reads as a bare Esc followed by the characters `[C`
/// — Option+Right typed `[C` into codex's composer. Real terminals send
/// `CSI 1;m C` / `CSI n;m ~` with `m = 1 + shift + 2*alt + 4*ctrl`;
/// F1–F4 leave their SS3 form when modified, and DECCKM does not apply.
fn encode_modified_special_key(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
    if !(ctrl || alt || shift) {
        return None;
    }
    let (number, final_byte): (u8, u8) = match key {
        "up" => (1, b'A'),
        "down" => (1, b'B'),
        "right" => (1, b'C'),
        "left" => (1, b'D'),
        "end" => (1, b'F'),
        "home" => (1, b'H'),
        "f1" => (1, b'P'),
        "f2" => (1, b'Q'),
        "f3" => (1, b'R'),
        "f4" => (1, b'S'),
        "insert" => (2, b'~'),
        "delete" => (3, b'~'),
        "pageup" => (5, b'~'),
        "pagedown" => (6, b'~'),
        "f5" => (15, b'~'),
        "f6" => (17, b'~'),
        "f7" => (18, b'~'),
        "f8" => (19, b'~'),
        "f9" => (20, b'~'),
        "f10" => (21, b'~'),
        "f11" => (23, b'~'),
        "f12" => (24, b'~'),
        "f13" => (25, b'~'),
        "f14" => (26, b'~'),
        "f15" => (28, b'~'),
        "f16" => (29, b'~'),
        "f17" => (31, b'~'),
        "f18" => (32, b'~'),
        "f19" => (33, b'~'),
        "f20" => (34, b'~'),
        _ => return None,
    };
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    Some(format!("\x1b[{number};{modifier}{}", final_byte as char).into_bytes())
}

/// Route a wheel gesture the way a real terminal would: apps that
/// enabled mouse reporting (claude, codex — the reason the resume seam
/// disables 1000/1002/1003/1006) receive wheel-button reports and
/// scroll their own transcript; alt-screen apps with DECSET 1007 get
/// arrow keys; everything else returns `None` and the caller scrolls
/// the local viewport. `bypass_reporting` (shift held, xterm
/// convention) forces the viewport path.
pub fn encode_scroll(
    mode: TermMode,
    delta_lines: i32,
    bypass_reporting: bool,
    column: usize,
    row: usize,
) -> Option<Vec<u8>> {
    if delta_lines == 0 || bypass_reporting {
        return None;
    }
    let up = delta_lines > 0;
    let count = delta_lines.unsigned_abs() as usize;
    if mode.intersects(TermMode::MOUSE_MODE) {
        let button: u8 = if up { 64 } else { 65 };
        let Some(column) = column.checked_add(1) else {
            return Some(Vec::new());
        };
        let Some(row) = row.checked_add(1) else {
            return Some(Vec::new());
        };
        return Some(if mode.contains(TermMode::SGR_MOUSE) {
            format!("\x1b[<{button};{column};{row}M")
                .into_bytes()
                .repeat(count)
        } else {
            let Some(column) = u8::try_from(column)
                .ok()
                .and_then(|value| value.checked_add(32))
            else {
                return Some(Vec::new());
            };
            let Some(row) = u8::try_from(row)
                .ok()
                .and_then(|value| value.checked_add(32))
            else {
                return Some(Vec::new());
            };
            [0x1b, b'[', b'M', 32 + button, column, row].repeat(count)
        });
    }
    if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
        let key = if up { "up" } else { "down" };
        let arrow = encode_key(key, false, false, false, None, mode)?;
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
            encode_key("b", false, true, false, Some("∫"), TermMode::empty()),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            encode_key("f", false, true, false, Some("ƒ"), TermMode::empty()),
            Some(b"\x1bf".to_vec())
        );
        assert_eq!(
            encode_key("d", false, true, false, Some("∂"), TermMode::empty()),
            Some(b"\x1bd".to_vec())
        );
        assert_eq!(
            encode_key("d", false, true, true, Some("Î"), TermMode::empty()),
            Some(b"\x1bD".to_vec())
        );
    }

    #[test]
    fn modified_special_keys_use_the_xterm_parameter_form() {
        // Option+Right must never be ESC ESC [ C — crossterm reads that as
        // Esc then the text "[C".
        assert_eq!(
            encode_key("right", false, true, false, None, TermMode::empty()),
            Some(b"\x1b[1;3C".to_vec())
        );
        assert_eq!(
            encode_key("left", false, true, false, None, TermMode::APP_CURSOR),
            Some(b"\x1b[1;3D".to_vec())
        );
        assert_eq!(
            encode_key("right", false, false, true, None, TermMode::empty()),
            Some(b"\x1b[1;2C".to_vec())
        );
        assert_eq!(
            encode_key("right", true, false, false, None, TermMode::empty()),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            encode_key("up", true, true, true, None, TermMode::empty()),
            Some(b"\x1b[1;8A".to_vec())
        );
        assert_eq!(
            encode_key("home", false, true, false, None, TermMode::empty()),
            Some(b"\x1b[1;3H".to_vec())
        );
        assert_eq!(
            encode_key("end", false, false, true, None, TermMode::empty()),
            Some(b"\x1b[1;2F".to_vec())
        );
        assert_eq!(
            encode_key("delete", false, true, false, None, TermMode::empty()),
            Some(b"\x1b[3;3~".to_vec())
        );
        assert_eq!(
            encode_key("pageup", false, false, true, None, TermMode::empty()),
            Some(b"\x1b[5;2~".to_vec())
        );
        assert_eq!(
            encode_key("f1", false, true, false, None, TermMode::empty()),
            Some(b"\x1b[1;3P".to_vec())
        );
        assert_eq!(
            encode_key("f5", true, false, false, None, TermMode::empty()),
            Some(b"\x1b[15;5~".to_vec())
        );
        // Unmodified keys keep their plain and DECCKM forms.
        assert_eq!(
            encode_key("right", false, false, false, None, TermMode::empty()),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            encode_key("right", false, false, false, None, TermMode::APP_CURSOR),
            Some(b"\x1bOC".to_vec())
        );
        // Keys whose sequence does not start with ESC keep the ESC prefix.
        assert_eq!(
            encode_key("backspace", false, true, false, None, TermMode::empty()),
            Some(b"\x1b\x7f".to_vec())
        );
        assert_eq!(
            encode_key("enter", false, true, false, None, TermMode::empty()),
            Some(b"\x1b\r".to_vec())
        );
    }

    #[test]
    fn shift_tab_is_backtab_and_navigation() {
        assert_eq!(
            encode_key("tab", false, false, true, None, TermMode::empty()),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key("tab", false, false, false, None, TermMode::empty()),
            Some(b"\t".to_vec())
        );
        assert_eq!(
            classify_key("tab", false, false, true, None),
            InputKind::Navigate
        );
        assert_eq!(
            classify_key("tab", false, false, false, None),
            InputKind::Content { text: "\t".into() }
        );
    }

    #[test]
    fn option_letter_fix_does_not_change_other_key_paths() {
        assert_eq!(
            encode_key("b", false, false, false, Some("∫"), TermMode::empty()),
            Some("∫".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key("b", true, true, false, Some("∫"), TermMode::empty()),
            Some(b"\x1b\x02".to_vec())
        );
        assert_eq!(
            encode_key("1", false, true, false, Some("¡"), TermMode::empty()),
            Some("\x1b¡".as_bytes().to_vec())
        );
    }

    #[test]
    fn wheel_reports_go_to_mouse_mode_apps() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_scroll(mode, 2, false, 7, 4),
            Some(b"\x1b[<64;8;5M\x1b[<64;8;5M".to_vec())
        );
        assert_eq!(
            encode_scroll(mode, -1, false, 7, 4),
            Some(b"\x1b[<65;8;5M".to_vec())
        );
        assert_eq!(
            encode_scroll(mode, 1, false, 0, 0),
            Some(b"\x1b[<64;1;1M".to_vec())
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, 1, false, 0, 0),
            Some(vec![0x1b, b'[', b'M', 96, 33, 33])
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, -2, false, 7, 4),
            Some([0x1b, b'[', b'M', 97, 40, 37].repeat(2))
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, 1, false, 222, 222),
            Some(vec![0x1b, b'[', b'M', 96, 255, 255])
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, 1, false, 223, 4),
            Some(Vec::new())
        );
        assert_eq!(
            encode_scroll(TermMode::MOUSE_REPORT_CLICK, 1, false, 7, usize::MAX),
            Some(Vec::new())
        );
    }

    #[test]
    fn alternate_scroll_sends_arrows_only_on_the_alt_screen() {
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(
            encode_scroll(mode, 2, false, 7, 4),
            Some(b"\x1b[A\x1b[A".to_vec())
        );
        assert_eq!(
            encode_scroll(mode | TermMode::APP_CURSOR, -1, false, 7, 4),
            Some(b"\x1bOB".to_vec())
        );
        assert_eq!(
            encode_scroll(TermMode::ALTERNATE_SCROLL, 1, false, 7, 4),
            None
        );
    }

    #[test]
    fn viewport_scroll_wins_for_plain_apps_and_shift_bypass() {
        assert_eq!(encode_scroll(TermMode::NONE, 3, false, 7, 4), None);
        let mouse = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(encode_scroll(mouse, 3, true, 7, 4), None);
        assert_eq!(encode_scroll(mouse, 0, false, 7, 4), None);
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
            encode_key("f1", false, false, false, None, TermMode::empty()),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key("f12", false, false, false, None, TermMode::empty()),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            encode_key("f20", false, false, false, None, TermMode::empty()),
            Some(b"\x1b[34~".to_vec())
        );
    }

    #[test]
    fn shift_enter_uses_runner_multiline_sequence() {
        assert_eq!(
            encode_key("enter", false, false, true, None, TermMode::empty()),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            encode_key("enter", false, false, false, None, TermMode::empty()),
            Some(b"\r".to_vec())
        );
    }

    /// `(key, ctrl, alt, shift, key_char, bytes)`
    type KeyCase = (
        &'static str,
        bool,
        bool,
        bool,
        Option<&'static str>,
        &'static str,
    );

    fn assert_kitty_keys(cases: &[KeyCase]) {
        for &(key, ctrl, alt, shift, key_char, expected) in cases {
            assert_eq!(
                encode_key(
                    key,
                    ctrl,
                    alt,
                    shift,
                    key_char,
                    TermMode::DISAMBIGUATE_ESC_CODES
                ),
                Some(expected.as_bytes().to_vec()),
                "{key} ctrl={ctrl} alt={alt} shift={shift}"
            );
        }
    }

    #[test]
    fn kitty_disambiguate_encodes_ambiguous_chords_as_csi_u() {
        assert_kitty_keys(&[
            ("escape", false, false, false, None, "\x1b[27u"),
            ("escape", false, true, false, None, "\x1b[27;3u"),
            ("enter", false, false, true, None, "\x1b[13;2u"),
            ("enter", false, true, false, None, "\x1b[13;3u"),
            ("tab", false, false, true, None, "\x1b[9;2u"),
            ("tab", false, true, false, None, "\x1b[9;3u"),
            ("backspace", false, false, true, None, "\x1b[127;2u"),
            ("backspace", false, true, false, None, "\x1b[127;3u"),
            ("d", false, true, false, Some("∂"), "\x1b[100;3u"),
            ("d", false, true, true, Some("Î"), "\x1b[100;4u"),
            ("1", false, true, false, Some("¡"), "\x1b[49;3u"),
            ("space", false, true, false, None, "\x1b[32;3u"),
            ("c", true, false, false, None, "\x1b[99;5u"),
            ("c", true, false, true, None, "\x1b[99;6u"),
            ("c", true, true, false, None, "\x1b[99;7u"),
            ("[", true, false, false, None, "\x1b[91;5u"),
            ("space", true, false, false, None, "\x1b[32;5u"),
        ]);
    }

    #[test]
    fn kitty_disambiguate_leaves_text_and_functional_keys_alone() {
        assert_kitty_keys(&[
            ("enter", false, false, false, None, "\r"),
            ("tab", false, false, false, None, "\t"),
            ("backspace", false, false, false, None, "\x7f"),
            ("a", false, false, false, Some("a"), "a"),
            ("a", false, false, true, Some("A"), "A"),
            ("1", false, false, true, Some("!"), "!"),
            ("up", false, false, false, None, "\x1b[A"),
            ("right", false, true, false, None, "\x1b[1;3C"),
            ("delete", true, false, false, None, "\x1b[3;5~"),
            ("f5", false, false, false, None, "\x1b[15~"),
        ]);
    }

    #[test]
    fn kitty_disambiguate_does_not_claim_chords_the_legacy_path_declines() {
        let kitty = TermMode::DISAMBIGUATE_ESC_CODES;
        for key in ["enter", "tab", "escape", "1"] {
            assert_eq!(encode_key(key, true, false, false, None, kitty), None);
            assert_eq!(
                encode_key(key, true, false, false, None, TermMode::empty()),
                None
            );
        }
    }
}

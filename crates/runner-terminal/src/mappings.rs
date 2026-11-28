//! Keystroke and paste encoding — the model-side mapping from UI input
//! events to the byte sequences a PTY expects (the `mappings/` analog of
//! Zed's terminal crate). Pure functions: terminal mode state is passed
//! in, bytes come out.

use std::cell::RefCell;

use ghostty_vt_sys as sys;

use crate::types::TerminalModeFlags as TermMode;
use crate::vt::{KeyEncoder, MouseEncoder};
pub use runner_core::protocol::terminal::InputKind;

thread_local! {
    static KEY_ENCODER: RefCell<Option<KeyEncoder>> = const { RefCell::new(None) };
    static MOUSE_ENCODER: RefCell<Option<MouseEncoder>> = const { RefCell::new(None) };
}

fn with_key_encoder<R>(f: impl FnOnce(&mut KeyEncoder) -> R) -> Option<R> {
    KEY_ENCODER.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            *guard = KeyEncoder::new().ok();
        }
        guard.as_mut().map(f)
    })
}

fn with_mouse_encoder<R>(f: impl FnOnce(&mut MouseEncoder) -> R) -> Option<R> {
    MOUSE_ENCODER.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            *guard = MouseEncoder::new().ok();
        }
        guard.as_mut().map(f)
    })
}

pub fn classify_key(
    mode: crate::TerminalModeFlags,
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
    if key == "." && ctrl && !alt && !shift && !mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        return InputKind::Content { text: ".".into() };
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

fn map_physical_key(key: &str) -> Option<sys::GhosttyKey> {
    Some(match key {
        "enter" => sys::GHOSTTY_KEY_ENTER,
        "backspace" => sys::GHOSTTY_KEY_BACKSPACE,
        "tab" => sys::GHOSTTY_KEY_TAB,
        "escape" => sys::GHOSTTY_KEY_ESCAPE,
        "space" => sys::GHOSTTY_KEY_SPACE,
        "up" => sys::GHOSTTY_KEY_ARROW_UP,
        "down" => sys::GHOSTTY_KEY_ARROW_DOWN,
        "right" => sys::GHOSTTY_KEY_ARROW_RIGHT,
        "left" => sys::GHOSTTY_KEY_ARROW_LEFT,
        "home" => sys::GHOSTTY_KEY_HOME,
        "end" => sys::GHOSTTY_KEY_END,
        "pageup" => sys::GHOSTTY_KEY_PAGE_UP,
        "pagedown" => sys::GHOSTTY_KEY_PAGE_DOWN,
        "insert" => sys::GHOSTTY_KEY_INSERT,
        "delete" => sys::GHOSTTY_KEY_DELETE,
        "f1" => sys::GHOSTTY_KEY_F1,
        "f2" => sys::GHOSTTY_KEY_F2,
        "f3" => sys::GHOSTTY_KEY_F3,
        "f4" => sys::GHOSTTY_KEY_F4,
        "f5" => sys::GHOSTTY_KEY_F5,
        "f6" => sys::GHOSTTY_KEY_F6,
        "f7" => sys::GHOSTTY_KEY_F7,
        "f8" => sys::GHOSTTY_KEY_F8,
        "f9" => sys::GHOSTTY_KEY_F9,
        "f10" => sys::GHOSTTY_KEY_F10,
        "f11" => sys::GHOSTTY_KEY_F11,
        "f12" => sys::GHOSTTY_KEY_F12,
        "f13" => sys::GHOSTTY_KEY_F13,
        "f14" => sys::GHOSTTY_KEY_F14,
        "f15" => sys::GHOSTTY_KEY_F15,
        "f16" => sys::GHOSTTY_KEY_F16,
        "f17" => sys::GHOSTTY_KEY_F17,
        "f18" => sys::GHOSTTY_KEY_F18,
        "f19" => sys::GHOSTTY_KEY_F19,
        "f20" => sys::GHOSTTY_KEY_F20,
        "a" | "A" => sys::GHOSTTY_KEY_A,
        "b" | "B" => sys::GHOSTTY_KEY_B,
        "c" | "C" => sys::GHOSTTY_KEY_C,
        "d" | "D" => sys::GHOSTTY_KEY_D,
        "e" | "E" => sys::GHOSTTY_KEY_E,
        "f" | "F" => sys::GHOSTTY_KEY_F,
        "g" | "G" => sys::GHOSTTY_KEY_G,
        "h" | "H" => sys::GHOSTTY_KEY_H,
        "i" | "I" => sys::GHOSTTY_KEY_I,
        "j" | "J" => sys::GHOSTTY_KEY_J,
        "k" | "K" => sys::GHOSTTY_KEY_K,
        "l" | "L" => sys::GHOSTTY_KEY_L,
        "m" | "M" => sys::GHOSTTY_KEY_M,
        "n" | "N" => sys::GHOSTTY_KEY_N,
        "o" | "O" => sys::GHOSTTY_KEY_O,
        "p" | "P" => sys::GHOSTTY_KEY_P,
        "q" | "Q" => sys::GHOSTTY_KEY_Q,
        "r" | "R" => sys::GHOSTTY_KEY_R,
        "s" | "S" => sys::GHOSTTY_KEY_S,
        "t" | "T" => sys::GHOSTTY_KEY_T,
        "u" | "U" => sys::GHOSTTY_KEY_U,
        "v" | "V" => sys::GHOSTTY_KEY_V,
        "w" | "W" => sys::GHOSTTY_KEY_W,
        "x" | "X" => sys::GHOSTTY_KEY_X,
        "y" | "Y" => sys::GHOSTTY_KEY_Y,
        "z" | "Z" => sys::GHOSTTY_KEY_Z,
        "0" => sys::GHOSTTY_KEY_DIGIT_0,
        "1" => sys::GHOSTTY_KEY_DIGIT_1,
        "2" => sys::GHOSTTY_KEY_DIGIT_2,
        "3" => sys::GHOSTTY_KEY_DIGIT_3,
        "4" => sys::GHOSTTY_KEY_DIGIT_4,
        "5" => sys::GHOSTTY_KEY_DIGIT_5,
        "6" => sys::GHOSTTY_KEY_DIGIT_6,
        "7" => sys::GHOSTTY_KEY_DIGIT_7,
        "8" => sys::GHOSTTY_KEY_DIGIT_8,
        "9" => sys::GHOSTTY_KEY_DIGIT_9,
        "`" => sys::GHOSTTY_KEY_BACKQUOTE,
        "-" => sys::GHOSTTY_KEY_MINUS,
        "=" => sys::GHOSTTY_KEY_EQUAL,
        "[" => sys::GHOSTTY_KEY_BRACKET_LEFT,
        "]" => sys::GHOSTTY_KEY_BRACKET_RIGHT,
        "\\" => sys::GHOSTTY_KEY_BACKSLASH,
        ";" => sys::GHOSTTY_KEY_SEMICOLON,
        "'" => sys::GHOSTTY_KEY_QUOTE,
        "," => sys::GHOSTTY_KEY_COMMA,
        "." => sys::GHOSTTY_KEY_PERIOD,
        "/" => sys::GHOSTTY_KEY_SLASH,
        _ => return None,
    })
}

fn is_functional_key(key: &str) -> bool {
    matches!(
        key,
        "up" | "down"
            | "right"
            | "left"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "insert"
            | "delete"
            | "f1"
            | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
            | "f13"
            | "f14"
            | "f15"
            | "f16"
            | "f17"
            | "f18"
            | "f19"
            | "f20"
    )
}

fn is_ctrl_encodable_char(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '@' | '[' | '\\' | ']' | '.')
}

/// Encode a non-text keystroke (or ctrl-chord) into PTY bytes via
/// `libghostty-vt`'s `KeyEncoder`, keeping Runner's legacy Shift+Enter,
/// macOS Option+Left/Right, Kitty disambiguate-only (bit 0) cap, and
/// declined-chord boundaries intact. Returns `None` when the key is not ours
/// to handle (the caller lets the event propagate, e.g. to a global binding).
pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key_char: Option<&str>,
    mode: crate::TerminalModeFlags,
) -> Option<Vec<u8>> {
    // Phase 1 implements Kitty keyboard disambiguate-only (bit 0).
    // Keep unmodified arrows, F1-F4, and F13-F20 on legacy SS3/tilde encoding
    // even when DISAMBIGUATE_ESC_CODES is active.
    let kitty_disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES);
    let legacy_f13_to_f20 = matches!(
        key,
        "f13" | "f14" | "f15" | "f16" | "f17" | "f18" | "f19" | "f20"
    );
    let unmodified_legacy_special = !ctrl
        && !alt
        && !shift
        && matches!(
            key,
            "up" | "down" | "left" | "right" | "f1" | "f2" | "f3" | "f4"
        );
    let kitty_flags = if unmodified_legacy_special || legacy_f13_to_f20 {
        0
    } else {
        mode.kitty_keyboard_bits() & 1
    };

    // Preserve xterm CSI 1;<mod>R for modified F3 across all modes (matching
    // Runner's pre-migration mapper rather than Kitty's CSI 13;<mod>~).
    if key == "f3" && (ctrl || alt || shift) {
        let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
        return Some(format!("\x1b[1;{mods}R").into_bytes());
    }

    if !kitty_disambiguate
        && cfg!(target_os = "macos")
        && !mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
        && alt
        && !ctrl
        && !shift
    {
        // macOS word navigation must also work in shells without CSI arrow bindings.
        match key {
            "left" => return Some(b"\x1bb".to_vec()),
            "right" => return Some(b"\x1bf".to_vec()),
            _ => {}
        }
    }

    // Plain Space and Shift+Space always send ASCII space (0x20), including
    // under Kitty disambiguate bit 0.
    if key == "space" && !ctrl && !alt {
        return Some(b" ".to_vec());
    }

    // Strict Kitty decoders (e.g. pi) expect CSI u for modified Enter/Tab/Backspace
    // once disambiguate mode is active. Ctrl+Enter also encodes as CSI 13;<mod>u,
    // while Ctrl+Tab and Ctrl+Backspace remain declined.
    if kitty_disambiguate && (ctrl || alt || shift) {
        let code = match key {
            "enter" => Some(13u8),
            "tab" if !ctrl => Some(9),
            "backspace" if !ctrl => Some(127),
            _ => None,
        };
        if let Some(code) = code {
            let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
            return Some(format!("\x1b[{code};{mods}u").into_bytes());
        }
    }

    // Legacy Enter/Tab/Escape without modifyOtherKeys: encode Ctrl+Enter as LF
    // (ESC-prefixed with Alt), decline other Ctrl chords, keep Runner's
    // Shift+Enter (`ESC CR`) and Shift+Tab (`CSI Z`), and use plain ESC prefix
    // for Alt rather than `CSI 27;...~`.
    if !kitty_disambiguate && matches!(key, "enter" | "tab" | "escape") {
        if ctrl {
            return (key == "enter").then(|| {
                if alt {
                    b"\x1b\n".to_vec()
                } else {
                    b"\n".to_vec()
                }
            });
        }
        let bytes: &[u8] = match key {
            "enter" if shift && !alt => b"\x1b\r",
            "enter" if alt => b"\x1b\r",
            "enter" => b"\r",
            "tab" if shift && !alt => b"\x1b[Z",
            "tab" if alt => b"\x1b\t",
            "tab" => b"\t",
            "escape" if alt => b"\x1b\x1b",
            "escape" => b"\x1b",
            _ => unreachable!(),
        };
        return Some(bytes.to_vec());
    }

    // Do not claim Ctrl chords the legacy path declines (e.g. Ctrl+Tab,
    // Ctrl+Escape, Ctrl+Backspace, Ctrl+1), except when Alt is also held under
    // Kitty disambiguate mode (e.g. Ctrl+Alt+1 -> CSI 49;7u).
    if ctrl && !(kitty_disambiguate && alt) && !is_functional_key(key) && key != "space" {
        let mut chars = key.chars();
        let allowed = match (chars.next(), chars.next()) {
            (Some(c), None) => is_ctrl_encodable_char(c),
            _ => false,
        };
        if !allowed {
            return None;
        }
    }
    if ctrl && matches!(key, "tab" | "backspace" | "escape") {
        return None;
    }

    // Legacy Ctrl C0 punctuation, Space, and Ctrl+I/M chords (Shift ignored,
    // optional ESC prefix for Alt). Ghostty's legacy KeyEncoder suppresses
    // Ctrl+I/M without modifyOtherKeys, whereas Runner preserves TAB/CR.
    if !kitty_disambiguate && ctrl && !is_functional_key(key) {
        let ctrl_byte = match key {
            "space" | "@" => Some(0x00u8),
            "i" | "I" => Some(0x09u8),
            "m" | "M" => Some(0x0du8),
            "." => Some(b'.'),
            "[" => Some(0x1bu8),
            "\\" => Some(0x1cu8),
            "]" => Some(0x1du8),
            _ => None,
        };
        if let Some(byte) = ctrl_byte {
            return Some(if alt { vec![0x1b, byte] } else { vec![byte] });
        }
    }

    // Legacy Option/Alt + non-ASCII or macOS Option-as-plain-key handling when
    // Kitty disambiguate is off.
    if !kitty_disambiguate && alt && !ctrl && !is_functional_key(key) {
        if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
            let ch = if shift {
                key.as_bytes()[0].to_ascii_uppercase()
            } else {
                key.as_bytes()[0]
            };
            return Some(vec![0x1b, ch]);
        }
        if cfg!(target_os = "macos")
            && !mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
            && key.chars().count() == 1
        {
            let mut out = Vec::with_capacity(1 + key.len());
            out.push(0x1b);
            out.extend_from_slice(key.as_bytes());
            return Some(out);
        }
        if !matches!(key, "enter" | "backspace" | "tab" | "escape" | "space") {
            let text = key_char.filter(|t| !t.is_empty())?;
            let mut out = Vec::with_capacity(1 + text.len());
            out.push(0x1b);
            out.extend_from_slice(text.as_bytes());
            return Some(out);
        }
    }

    // Plain single-character non-ASCII / layout-mapped text when no Ctrl/Alt is held.
    if !ctrl
        && !alt
        && !is_functional_key(key)
        && !matches!(key, "enter" | "backspace" | "tab" | "escape" | "space")
    {
        return key_char
            .filter(|t| !t.is_empty())
            .map(|t| t.as_bytes().to_vec());
    }

    let mut mods: sys::GhosttyMods = 0;
    // Legacy Ctrl+Shift+letter sends the same C0 control byte as Ctrl+letter.
    if shift && (kitty_disambiguate || !ctrl || is_functional_key(key)) {
        mods |= sys::GHOSTTY_MODS_SHIFT as sys::GhosttyMods;
    }
    if ctrl {
        mods |= sys::GHOSTTY_MODS_CTRL as sys::GhosttyMods;
    }
    if alt {
        mods |= sys::GHOSTTY_MODS_ALT as sys::GhosttyMods;
    }

    let physical = map_physical_key(key).unwrap_or(sys::GHOSTTY_KEY_UNIDENTIFIED);
    let unshifted_codepoint = if key == "space" {
        b' ' as u32
    } else {
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => c.to_ascii_lowercase() as u32,
            _ => 0,
        }
    };

    // Pass UTF-8 text only for plain character input (not Ctrl/Alt chords or named keys),
    // so Ghostty's KeyEncoder produces the exact legacy/Kitty sequences.
    let utf8 = if !ctrl
        && !alt
        && !is_functional_key(key)
        && !matches!(key, "enter" | "backspace" | "tab" | "escape" | "space")
    {
        key_char.unwrap_or("")
    } else {
        ""
    };

    let opt_as_alt =
        if cfg!(target_os = "macos") && !mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
            sys::GHOSTTY_OPTION_AS_ALT_TRUE
        } else {
            sys::GHOSTTY_OPTION_AS_ALT_FALSE
        };

    let encoded = with_key_encoder(|enc| {
        // Keep unmodified Home/End on CSI H/F even when DECCKM is active.
        let app_cursor = mode.contains(TermMode::APP_CURSOR) && !matches!(key, "home" | "end");
        let _ = enc.set_cursor_key_application(app_cursor);
        let _ = enc.set_keypad_key_application(false);
        let _ = enc.set_alt_esc_prefix(true);
        let _ = enc.set_kitty_flags(kitty_flags);
        let _ = enc.set_macos_option_as_alt(opt_as_alt);
        enc.encode(
            sys::GHOSTTY_KEY_ACTION_PRESS,
            physical,
            mods,
            0,
            utf8,
            unshifted_codepoint,
        )
    })?
    .ok()?;

    if encoded.is_empty() {
        None
    } else {
        Some(encoded)
    }
}

fn mouse_tracking_mode(mode: TermMode) -> sys::GhosttyMouseTrackingMode {
    if mode.contains(TermMode::MOUSE_MOTION) {
        sys::GHOSTTY_MOUSE_TRACKING_ANY
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        sys::GHOSTTY_MOUSE_TRACKING_BUTTON
    } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        sys::GHOSTTY_MOUSE_TRACKING_NORMAL
    } else {
        sys::GHOSTTY_MOUSE_TRACKING_NONE
    }
}

fn mouse_format(mode: TermMode) -> sys::GhosttyMouseFormat {
    if mode.contains(TermMode::SGR_MOUSE) {
        sys::GHOSTTY_MOUSE_FORMAT_SGR
    } else {
        sys::GHOSTTY_MOUSE_FORMAT_X10
    }
}

fn encode_mouse_via_vt(
    mode: TermMode,
    action: sys::GhosttyMouseAction,
    button: Option<sys::GhosttyMouseButton>,
    column: usize,
    row: usize,
    modifiers: MouseModifiers,
    any_button_pressed: bool,
) -> Option<Vec<u8>> {
    let screen_w = u32::try_from(column.checked_add(1)?).ok()?;
    let screen_h = u32::try_from(row.checked_add(1)?).ok()?;
    let mut mods: sys::GhosttyMods = 0;
    if modifiers.shift {
        mods |= sys::GHOSTTY_MODS_SHIFT as sys::GhosttyMods;
    }
    if modifiers.alt {
        mods |= sys::GHOSTTY_MODS_ALT as sys::GhosttyMods;
    }
    if modifiers.control {
        mods |= sys::GHOSTTY_MODS_CTRL as sys::GhosttyMods;
    }
    with_mouse_encoder(|enc| {
        let _ = enc.set_tracking(mouse_tracking_mode(mode));
        let _ = enc.set_format(mouse_format(mode));
        let _ = enc.set_size(screen_w, screen_h, 1, 1);
        let _ = enc.set_any_button_pressed(any_button_pressed);
        enc.encode(action, button, mods, column as f32, row as f32)
    })?
    .ok()
}

/// Route a wheel gesture the way a real terminal would: apps that
/// enabled mouse reporting (claude, codex — the reason the resume seam
/// disables 1000/1002/1003/1006) receive wheel-button reports and
/// scroll their own transcript; alt-screen apps with DECSET 1007 get
/// arrow keys; everything else returns `None` and the caller scrolls
/// the local viewport. `bypass_reporting` (shift held, xterm
/// convention) forces the viewport path.
pub fn encode_scroll(
    mode: crate::TerminalModeFlags,
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
        let button = if up {
            sys::GHOSTTY_MOUSE_BUTTON_FOUR
        } else {
            sys::GHOSTTY_MOUSE_BUTTON_FIVE
        };
        let single = encode_mouse_via_vt(
            mode,
            sys::GHOSTTY_MOUSE_ACTION_PRESS,
            Some(button),
            column,
            row,
            MouseModifiers::default(),
            false,
        )
        .unwrap_or_default();
        return Some(single.repeat(count));
    }
    if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
        let key = if up { "up" } else { "down" };
        let arrow = encode_key(key, false, false, false, None, mode)?;
        return Some(arrow.repeat(count));
    }
    None
}

pub fn encode_mouse_press(
    mode: crate::TerminalModeFlags,
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
    mode: crate::TerminalModeFlags,
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
    mode: crate::TerminalModeFlags,
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
    mode: crate::TerminalModeFlags,
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
    let raw_action = match action {
        MouseAction::Press => sys::GHOSTTY_MOUSE_ACTION_PRESS,
        MouseAction::Release => sys::GHOSTTY_MOUSE_ACTION_RELEASE,
        MouseAction::Motion => sys::GHOSTTY_MOUSE_ACTION_MOTION,
    };
    let raw_button = match button {
        MouseButton::Left => sys::GHOSTTY_MOUSE_BUTTON_LEFT,
        MouseButton::Middle => sys::GHOSTTY_MOUSE_BUTTON_MIDDLE,
        MouseButton::Right => sys::GHOSTTY_MOUSE_BUTTON_RIGHT,
    };
    let bytes = encode_mouse_via_vt(
        mode,
        raw_action,
        Some(raw_button),
        column,
        row,
        modifiers,
        action == MouseAction::Motion,
    )?;
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

/// Encode pasted text, stripping raw escapes and wrapping in bracketed paste
/// markers when the terminal has that mode enabled.
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
        encode_paste, encode_scroll, InputKind, MouseButton, MouseModifiers, TermMode,
    };

    #[test]
    fn key_classification_separates_submit_content_edit_and_navigation() {
        assert_eq!(
            classify_key(TermMode::empty(), "enter", false, false, false, None),
            InputKind::Submit
        );
        assert_eq!(
            classify_key(TermMode::empty(), "enter", false, false, true, None),
            InputKind::Content { text: "\n".into() }
        );
        assert_eq!(
            classify_key(TermMode::empty(), "enter", false, true, false, None),
            InputKind::Navigate
        );
        assert_eq!(
            classify_key(TermMode::empty(), "x", false, false, false, Some("x")),
            InputKind::Content { text: "x".into() }
        );
        for key in ["backspace", "delete"] {
            assert_eq!(
                classify_key(TermMode::empty(), key, false, false, false, None),
                InputKind::Edit
            );
        }
        for key in ["h", "u", "w", "k"] {
            assert_eq!(
                classify_key(TermMode::empty(), key, true, false, false, None),
                InputKind::Edit
            );
        }
        assert_eq!(
            classify_key(TermMode::empty(), "c", true, false, false, None),
            InputKind::Cancel
        );
        for key in ["left", "escape", "f1"] {
            assert_eq!(
                classify_key(TermMode::empty(), key, false, false, false, None),
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
        assert_eq!(
            encode_key("up", false, true, false, None, TermMode::empty()),
            Some(b"\x1b[1;3A".to_vec())
        );
        assert_eq!(
            encode_key("down", false, true, false, None, TermMode::APP_CURSOR),
            Some(b"\x1b[1;3B".to_vec())
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
    fn option_arrows_preserve_platform_protocol_and_modifier_boundaries() {
        for mode in [
            TermMode::empty(),
            TermMode::APP_CURSOR,
            TermMode::DISAMBIGUATE_ESC_CODES,
            TermMode::DISAMBIGUATE_ESC_CODES | TermMode::APP_CURSOR,
            TermMode::REPORT_EVENT_TYPES,
            TermMode::REPORT_ALTERNATE_KEYS,
            TermMode::REPORT_ALL_KEYS_AS_ESC,
            TermMode::REPORT_ASSOCIATED_TEXT,
            TermMode::KITTY_KEYBOARD_PROTOCOL,
            TermMode::KITTY_KEYBOARD_PROTOCOL | TermMode::APP_CURSOR,
        ] {
            for (key, suffix, meta) in [("left", 'D', b'b'), ("right", 'C', b'f')] {
                for (ctrl, alt, shift) in [
                    (false, false, false),
                    (false, true, false),
                    (false, false, true),
                    (true, false, false),
                    (false, true, true),
                    (true, true, false),
                    (true, false, true),
                    (true, true, true),
                ] {
                    let expected = if cfg!(target_os = "macos")
                        && !mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
                        && alt
                        && !ctrl
                        && !shift
                    {
                        vec![0x1b, meta]
                    } else if ctrl || alt || shift {
                        let mods = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
                        format!("\x1b[1;{mods}{suffix}").into_bytes()
                    } else if mode.contains(TermMode::APP_CURSOR) {
                        format!("\x1bO{suffix}").into_bytes()
                    } else {
                        format!("\x1b[{suffix}").into_bytes()
                    };
                    assert_eq!(
                        encode_key(key, ctrl, alt, shift, None, mode),
                        Some(expected),
                        "{key} ctrl={ctrl} alt={alt} shift={shift} mode={mode:?}"
                    );
                    assert_eq!(
                        classify_key(TermMode::empty(), key, ctrl, alt, shift, None),
                        InputKind::Navigate
                    );
                }
            }
        }
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
            classify_key(TermMode::empty(), "tab", false, false, true, None),
            InputKind::Navigate
        );
        assert_eq!(
            classify_key(TermMode::empty(), "tab", false, false, false, None),
            InputKind::Content { text: "\t".into() }
        );
    }

    #[test]
    fn option_text_and_control_paths_keep_their_encoding() {
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
            Some(if cfg!(target_os = "macos") {
                b"\x1b1".to_vec()
            } else {
                "\x1b¡".as_bytes().to_vec()
            })
        );
    }

    #[test]
    fn meta_uses_the_key_without_option_and_preserves_kitty_encoding() {
        for (key, shift, text, plain) in [
            ("l", false, "@", "l"),
            ("5", false, "[", "5"),
            ("/", false, "\\", "/"),
            ("d", true, "Î", "D"),
            ("ö", false, "œ", "ö"),
        ] {
            let expected = if cfg!(target_os = "macos") || key == "l" || key == "d" {
                plain
            } else {
                text
            };
            assert_eq!(
                encode_key(key, false, true, shift, Some(text), TermMode::empty()),
                Some(format!("\x1b{expected}").into_bytes()),
            );
        }
        assert_eq!(
            encode_key(
                "5",
                false,
                true,
                false,
                Some("["),
                TermMode::DISAMBIGUATE_ESC_CODES
            ),
            Some(b"\x1b[53;3u".to_vec()),
        );
        for mode in [
            TermMode::REPORT_EVENT_TYPES,
            TermMode::REPORT_ALTERNATE_KEYS,
            TermMode::REPORT_ALL_KEYS_AS_ESC,
            TermMode::REPORT_ASSOCIATED_TEXT,
        ] {
            assert_eq!(
                encode_key("5", false, true, false, Some("["), mode),
                Some(b"\x1b[".to_vec()),
            );
        }
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
    fn ctrl_period_classification_tracks_only_printable_legacy_input() {
        for mode in [TermMode::empty(), TermMode::DISAMBIGUATE_ESC_CODES] {
            assert_eq!(
                classify_key(mode, ".", true, false, false, Some(".")),
                if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
                    InputKind::Navigate
                } else {
                    InputKind::Content { text: ".".into() }
                }
            );
            for (alt, shift) in [(true, false), (false, true), (true, true)] {
                assert_eq!(
                    classify_key(mode, ".", true, alt, shift, Some(".")),
                    InputKind::Navigate
                );
            }
        }
    }

    #[test]
    fn ctrl_period_preserves_its_modifier_only_after_kitty_negotiation() {
        assert_eq!(
            encode_key(".", true, false, false, Some("."), TermMode::empty()),
            Some(b".".to_vec())
        );
        assert_eq!(
            encode_key(
                ".",
                true,
                false,
                false,
                Some("."),
                TermMode::DISAMBIGUATE_ESC_CODES
            ),
            Some(b"\x1b[46;5u".to_vec())
        );
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
            ("left", false, true, false, None, "\x1b[1;3D"),
            ("delete", true, false, false, None, "\x1b[3;5~"),
            ("f5", false, false, false, None, "\x1b[15~"),
        ]);
    }

    #[test]
    fn ctrl_enter_encodes_csi_u_under_kitty_and_lf_in_legacy_mode() {
        for mode in [
            TermMode::empty(),
            TermMode::APP_CURSOR,
            TermMode::REPORT_EVENT_TYPES,
            TermMode::REPORT_ALTERNATE_KEYS,
            TermMode::REPORT_ALL_KEYS_AS_ESC,
            TermMode::REPORT_ASSOCIATED_TEXT,
        ] {
            for (alt, shift, expected) in [
                (false, false, b"\n".as_slice()),
                (false, true, b"\n".as_slice()),
                (true, false, b"\x1b\n".as_slice()),
                (true, true, b"\x1b\n".as_slice()),
            ] {
                assert_eq!(
                    encode_key("enter", true, alt, shift, None, mode),
                    Some(expected.to_vec()),
                    "legacy ctrl+enter alt={alt} shift={shift} mode={mode:?}"
                );
                assert_eq!(
                    classify_key(mode, "enter", true, alt, shift, None),
                    InputKind::Navigate
                );
            }
        }
        for mode in [
            TermMode::DISAMBIGUATE_ESC_CODES,
            TermMode::DISAMBIGUATE_ESC_CODES | TermMode::APP_CURSOR,
            TermMode::KITTY_KEYBOARD_PROTOCOL,
        ] {
            for (alt, shift, expected) in [
                (false, false, b"\x1b[13;5u".as_slice()),
                (false, true, b"\x1b[13;6u".as_slice()),
                (true, false, b"\x1b[13;7u".as_slice()),
                (true, true, b"\x1b[13;8u".as_slice()),
            ] {
                assert_eq!(
                    encode_key("enter", true, alt, shift, None, mode),
                    Some(expected.to_vec()),
                    "kitty ctrl+enter alt={alt} shift={shift} mode={mode:?}"
                );
                assert_eq!(
                    classify_key(mode, "enter", true, alt, shift, None),
                    InputKind::Navigate
                );
            }
        }
    }

    #[test]
    fn kitty_disambiguate_does_not_claim_chords_the_legacy_path_declines() {
        let kitty = TermMode::DISAMBIGUATE_ESC_CODES;
        for key in ["tab", "escape", "backspace", "1"] {
            assert_eq!(encode_key(key, true, false, false, None, kitty), None);
            assert_eq!(
                encode_key(key, true, false, false, None, TermMode::empty()),
                None
            );
        }
    }

    #[test]
    fn paste_encoding_brackets_and_sanitizes() {
        assert_eq!(encode_paste("hello", true), b"\x1b[200~hello\x1b[201~");
        assert_eq!(encode_paste("a\nb", false), b"a\nb");
        assert_eq!(encode_paste("a\r\nb", false), b"a\r\nb");
        assert_eq!(encode_paste("a\x1bb", false), b"ab");
        assert_eq!(encode_paste("a\x03b\0c\td", false), b"a\x03b\0c\td");
        assert_eq!(
            encode_paste("a\x1b[201~b", true),
            b"\x1b[200~a[201~b\x1b[201~"
        );
    }

    #[test]
    fn legacy_and_kitty_chords_preserve_runner_encoding_contract() {
        let legacy = TermMode::empty();
        let kitty = TermMode::DISAMBIGUATE_ESC_CODES;

        assert_eq!(
            encode_key("space", false, false, false, None, legacy),
            Some(b" ".to_vec())
        );
        assert_eq!(
            encode_key("space", false, false, true, None, legacy),
            Some(b" ".to_vec())
        );
        assert_eq!(
            encode_key("space", false, false, false, None, kitty),
            Some(b" ".to_vec())
        );
        assert_eq!(
            encode_key("space", false, false, true, None, kitty),
            Some(b" ".to_vec())
        );
        assert_eq!(
            encode_key("space", true, false, false, None, legacy),
            Some(vec![0x00])
        );
        assert_eq!(
            encode_key("space", true, false, false, None, kitty),
            Some(b"\x1b[32;5u".to_vec())
        );
        assert_eq!(
            encode_key("space", false, true, false, None, legacy),
            Some(b"\x1b ".to_vec())
        );
        assert_eq!(
            encode_key("space", false, true, false, None, kitty),
            Some(b"\x1b[32;3u".to_vec())
        );

        // Legacy Enter/Tab/Escape without modifyOtherKeys keep classic encodings.
        assert_eq!(
            encode_key("enter", false, false, true, None, legacy),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            encode_key("enter", false, true, false, None, legacy),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(
            encode_key("tab", false, false, true, None, legacy),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key("tab", false, true, false, None, legacy),
            Some(b"\x1b\t".to_vec())
        );
        assert_eq!(
            encode_key("escape", false, true, false, None, legacy),
            Some(b"\x1b\x1b".to_vec())
        );

        // Legacy Ctrl+Shift+letter sends C0 control; Kitty disambiguates with mod 6.
        assert_eq!(
            encode_key("a", true, false, true, Some("a"), legacy),
            Some(vec![0x01])
        );
        assert_eq!(
            encode_key("a", true, true, true, Some("a"), legacy),
            Some(vec![0x1b, 0x01])
        );
        assert_eq!(
            encode_key("a", true, false, true, Some("a"), kitty),
            Some(b"\x1b[97;6u".to_vec())
        );
        assert_eq!(
            encode_key("a", true, true, false, Some("a"), kitty),
            Some(b"\x1b[97;7u".to_vec())
        );
        assert_eq!(
            encode_key("a", true, true, true, Some("a"), kitty),
            Some(b"\x1b[97;8u".to_vec())
        );

        // Legacy C0 symbol chords (matching Runner's existing Ctrl allowlist).
        assert_eq!(
            encode_key("[", true, false, false, Some("["), legacy),
            Some(vec![0x1b])
        );
        assert_eq!(
            encode_key("\\", true, false, false, Some("\\"), legacy),
            Some(vec![0x1c])
        );
        assert_eq!(
            encode_key("]", true, false, false, Some("]"), legacy),
            Some(vec![0x1d])
        );
        assert_eq!(
            encode_key("@", true, false, false, Some("@"), legacy),
            Some(vec![0x00])
        );
        assert_eq!(
            encode_key(".", true, false, false, Some("."), legacy),
            Some(vec![b'.'])
        );
        assert_eq!(encode_key("^", true, false, false, Some("^"), legacy), None);
        assert_eq!(encode_key("_", true, false, false, Some("_"), legacy), None);

        // Legacy Ctrl+I / Ctrl+M keep TAB / CR (and ESC-prefixed with Alt).
        for (key, byte) in [("i", 0x09u8), ("I", 0x09), ("m", 0x0d), ("M", 0x0d)] {
            assert_eq!(
                encode_key(key, true, false, false, Some(key), legacy),
                Some(vec![byte])
            );
            assert_eq!(
                encode_key(key, true, false, true, Some(key), legacy),
                Some(vec![byte])
            );
            assert_eq!(
                encode_key(key, true, true, false, Some(key), legacy),
                Some(vec![0x1b, byte])
            );
            assert_eq!(
                encode_key(key, true, true, true, Some(key), legacy),
                Some(vec![0x1b, byte])
            );
        }

        // Unmodified Home/End stay on CSI H/F even with APP_CURSOR enabled.
        let app_cursor = TermMode::APP_CURSOR;
        assert_eq!(
            encode_key("home", false, false, false, None, app_cursor),
            Some(b"\x1b[H".to_vec())
        );
        assert_eq!(
            encode_key("end", false, false, false, None, app_cursor),
            Some(b"\x1b[F".to_vec())
        );

        // Unmodified F1..F4 keep SS3; modified F3 keeps CSI 1;<mod>R; F13..F20 keep tilde codes.
        assert_eq!(
            encode_key("f1", false, false, false, None, legacy),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key("f1", false, false, false, None, kitty),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key("f1", true, false, false, None, kitty),
            Some(b"\x1b[1;5P".to_vec())
        );
        assert_eq!(
            encode_key("f3", false, false, false, None, kitty),
            Some(b"\x1bOR".to_vec())
        );
        assert_eq!(
            encode_key("f3", true, false, false, None, legacy),
            Some(b"\x1b[1;5R".to_vec())
        );
        assert_eq!(
            encode_key("f3", true, false, false, None, kitty),
            Some(b"\x1b[1;5R".to_vec())
        );
        assert_eq!(
            encode_key("f13", false, false, false, None, kitty),
            Some(b"\x1b[25~".to_vec())
        );
        assert_eq!(
            encode_key("f20", true, false, false, None, kitty),
            Some(b"\x1b[34;5~".to_vec())
        );
    }
}

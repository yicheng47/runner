//! Keystroke and paste encoding — the model-side mapping from UI input
//! events to the byte sequences a PTY expects (the `mappings/` analog of
//! Zed's terminal crate). Pure functions: terminal mode state is passed
//! in, bytes come out.

/// Encode a non-text keystroke (or ctrl-chord) into PTY bytes.
/// `app_cursor` selects the DECCKM application-cursor sequences for the
/// arrow keys. Returns `None` when the key is not ours to handle (the
/// caller lets the event propagate, e.g. to a global binding).
pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    key_char: Option<&str>,
    app_cursor: bool,
) -> Option<Vec<u8>> {
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
            "delete" => b"\x1b[3~",
            "space" => b" ",
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

//! Streaming scanner splitting a PTY byte stream into plain text and complete
//! Kitty graphics APC commands (`ESC _ G ... ESC \`).

/// Maximum bytes accepted inside one Kitty graphics command after `ESC _ G`
/// and before `ESC \`. Larger commands are discarded through their terminator
/// without a reply.
pub const MAX_COMMAND_BYTES: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanItem {
    Text(Vec<u8>),
    /// Raw Kitty command bytes after `ESC _ G` and before `ESC \`.
    Command(Vec<u8>),
    /// A command exceeded `max_bytes` and was discarded through `ESC \`.
    Discarded,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScanOutput<'a> {
    /// Entire input slice is plain text (no graphics commands, no buffered escape).
    Plain(&'a [u8]),
    /// Scanned items produced from this chunk. May be empty while buffering an
    /// incomplete escape introducer or graphics command across chunks.
    Items(Vec<ScanItem>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Ground,
    /// Saw `ESC`.
    Esc,
    /// Saw `ESC _`.
    ApcStart,
    /// Collecting bytes inside `ESC _ G ...`.
    Graphics,
    /// Saw `ESC` inside `Graphics`.
    GraphicsEsc,
    /// Discarding an over-limit `ESC _ G ...` command through its terminator.
    Discard,
    /// Saw `ESC` inside `Discard`.
    DiscardEsc,
}

pub struct Scanner {
    state: State,
    max_bytes: usize,
    command: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Self::with_max_bytes(MAX_COMMAND_BYTES)
    }

    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            state: State::Ground,
            max_bytes,
            command: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.command.clear();
        self.command.shrink_to_fit();
    }

    /// Returns true if the fast path can pass `bytes` straight through without
    /// copying or entering the byte-by-byte state machine.
    fn is_plain_ground(bytes: &[u8]) -> bool {
        let mut i = 0;
        while let Some(offset) = bytes[i..].iter().position(|&b| b == 0x1b) {
            let esc = i + offset;
            match bytes.get(esc + 1) {
                None => return false,
                Some(&0x1b) => {
                    i = esc + 1;
                }
                Some(&b'_') => match bytes.get(esc + 2) {
                    None | Some(&b'G') | Some(&0x1b) => return false,
                    Some(_) => {
                        i = esc + 3;
                    }
                },
                Some(_) => {
                    i = esc + 2;
                }
            }
        }
        true
    }

    pub fn scan<'a>(&mut self, bytes: &'a [u8]) -> ScanOutput<'a> {
        if bytes.is_empty() {
            return ScanOutput::Plain(bytes);
        }
        if self.state == State::Ground && Self::is_plain_ground(bytes) {
            return ScanOutput::Plain(bytes);
        }

        let mut items: Vec<ScanItem> = Vec::new();
        let mut text: Vec<u8> = Vec::new();
        let flush_text = |items: &mut Vec<ScanItem>, text: &mut Vec<u8>| {
            if !text.is_empty() {
                items.push(ScanItem::Text(std::mem::take(text)));
            }
        };

        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                State::Ground => {
                    if let Some(offset) = bytes[i..].iter().position(|&b| b == 0x1b) {
                        text.extend_from_slice(&bytes[i..i + offset]);
                        i += offset + 1;
                        self.state = State::Esc;
                    } else {
                        text.extend_from_slice(&bytes[i..]);
                        break;
                    }
                }
                State::Esc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'_' => self.state = State::ApcStart,
                        0x1b => {
                            // `ESC ESC`: the first ESC passes to vte as text and the second
                            // stays pending so `ESC ESC _ G` starts a graphics command.
                            text.push(0x1b);
                        }
                        _ => {
                            text.push(0x1b);
                            text.push(b);
                            self.state = State::Ground;
                        }
                    }
                }
                State::ApcStart => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'G' => {
                            flush_text(&mut items, &mut text);
                            self.command.clear();
                            self.state = State::Graphics;
                        }
                        0x1b => {
                            text.extend_from_slice(b"\x1b_");
                            self.state = State::Esc;
                        }
                        _ => {
                            text.extend_from_slice(b"\x1b_");
                            text.push(b);
                            self.state = State::Ground;
                        }
                    }
                }
                State::Graphics => {
                    let rest = &bytes[i..];
                    let special = rest.iter().position(|&b| matches!(b, 0x18 | 0x1a | 0x1b));
                    let run_len = special.unwrap_or(rest.len());
                    if run_len > 0 {
                        if self.command.len().saturating_add(run_len) > self.max_bytes {
                            self.command.clear();
                            self.command.shrink_to_fit();
                            self.state = State::Discard;
                        } else {
                            self.command.extend_from_slice(&rest[..run_len]);
                        }
                        i += run_len;
                    }
                    if let Some(pos) = special {
                        let b = rest[pos];
                        i += 1;
                        if self.state == State::Discard {
                            match b {
                                0x18 | 0x1a => {
                                    text.push(b);
                                    self.state = State::Ground;
                                }
                                0x1b => self.state = State::DiscardEsc,
                                _ => unreachable!(),
                            }
                        } else {
                            match b {
                                0x18 | 0x1a => {
                                    self.command.clear();
                                    self.command.shrink_to_fit();
                                    text.push(b);
                                    self.state = State::Ground;
                                }
                                0x1b => self.state = State::GraphicsEsc,
                                _ => unreachable!(),
                            }
                        }
                    }
                }
                State::GraphicsEsc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'\\' => {
                            flush_text(&mut items, &mut text);
                            let cmd = std::mem::take(&mut self.command);
                            items.push(ScanItem::Command(cmd));
                            self.state = State::Ground;
                        }
                        0x18 | 0x1a => {
                            self.command.clear();
                            self.command.shrink_to_fit();
                            text.push(b);
                            self.state = State::Ground;
                        }
                        0x1b => {
                            self.command.clear();
                            self.command.shrink_to_fit();
                            self.state = State::Esc;
                        }
                        b'_' => {
                            self.command.clear();
                            self.command.shrink_to_fit();
                            self.state = State::ApcStart;
                        }
                        _ => {
                            self.command.clear();
                            self.command.shrink_to_fit();
                            text.push(0x1b);
                            text.push(b);
                            self.state = State::Ground;
                        }
                    }
                }
                State::Discard => {
                    let rest = &bytes[i..];
                    if let Some(pos) = rest.iter().position(|&b| matches!(b, 0x18 | 0x1a | 0x1b)) {
                        let b = rest[pos];
                        i += pos + 1;
                        match b {
                            0x18 | 0x1a => {
                                text.push(b);
                                self.state = State::Ground;
                            }
                            0x1b => self.state = State::DiscardEsc,
                            _ => unreachable!(),
                        }
                    } else {
                        break;
                    }
                }
                State::DiscardEsc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        b'\\' => {
                            flush_text(&mut items, &mut text);
                            items.push(ScanItem::Discarded);
                            self.state = State::Ground;
                        }
                        0x18 | 0x1a => {
                            text.push(b);
                            self.state = State::Ground;
                        }
                        0x1b => {
                            self.state = State::Esc;
                        }
                        b'_' => {
                            self.state = State::ApcStart;
                        }
                        _ => {
                            text.push(0x1b);
                            text.push(b);
                            self.state = State::Ground;
                        }
                    }
                }
            }
        }

        flush_text(&mut items, &mut text);
        ScanOutput::Items(items)
    }
}

/// Remove every complete Kitty graphics command from `bytes`, leaving every
/// other byte unchanged. Returns `None` when `bytes` contains no graphics
/// commands at all so callers can forward the original slice unchanged.
pub fn strip_graphics_commands(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut scanner = Scanner::new();
    match scanner.scan(bytes) {
        ScanOutput::Plain(_) => None,
        ScanOutput::Items(items) => {
            let mut has_graphics = false;
            let mut out = Vec::new();
            for item in items {
                match item {
                    ScanItem::Text(text) => out.extend_from_slice(&text),
                    ScanItem::Command(_) | ScanItem::Discarded => has_graphics = true,
                }
            }
            // Flush any trailing non-graphics escape introducer left in the scanner.
            match scanner.state {
                State::Esc => out.push(0x1b),
                State::ApcStart => out.extend_from_slice(b"\x1b_"),
                _ => {}
            }
            if has_graphics {
                Some(out)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_all(scanner: &mut Scanner, chunks: &[&[u8]]) -> Vec<ScanItem> {
        let mut out: Vec<ScanItem> = Vec::new();
        for chunk in chunks {
            match scanner.scan(chunk) {
                ScanOutput::Plain(bytes) => {
                    if !bytes.is_empty() {
                        push_merged(&mut out, ScanItem::Text(bytes.to_vec()));
                    }
                }
                ScanOutput::Items(items) => {
                    for item in items {
                        push_merged(&mut out, item);
                    }
                }
            }
        }
        out
    }

    fn push_merged(out: &mut Vec<ScanItem>, item: ScanItem) {
        if let (Some(ScanItem::Text(prev)), ScanItem::Text(next)) = (out.last_mut(), &item) {
            prev.extend_from_slice(next);
        } else {
            out.push(item);
        }
    }

    #[test]
    fn plain_and_ansi_text_take_zero_copy_fast_path() {
        let mut scanner = Scanner::new();
        for sample in [
            b"hello world".as_slice(),
            b"\x1b[31mred\x1b[0m",
            b"before\x1b_Xother apc\x1b\\after",
            b"\x1b\x1b[32mgreen",
        ] {
            assert_eq!(scanner.scan(sample), ScanOutput::Plain(sample));
        }
    }

    #[test]
    fn stream_with_text_chunked_graphics_and_other_apcs_matches_at_every_split() {
        let stream = b"before\x1b_Xother;apc\x1b\\mid\x1b_Ga=t,f=24,s=1,v=2,i=1,m=1;AAAA\x1b\\between\x1b_Gm=0;AAAA\x1b\\after";
        let expected = vec![
            ScanItem::Text(b"before\x1b_Xother;apc\x1b\\mid".to_vec()),
            ScanItem::Command(b"a=t,f=24,s=1,v=2,i=1,m=1;AAAA".to_vec()),
            ScanItem::Text(b"between".to_vec()),
            ScanItem::Command(b"m=0;AAAA".to_vec()),
            ScanItem::Text(b"after".to_vec()),
        ];

        assert_eq!(collect_all(&mut Scanner::new(), &[stream]), expected);

        for offset in 0..=stream.len() {
            let mut scanner = Scanner::new();
            assert_eq!(
                collect_all(&mut scanner, &[&stream[..offset], &stream[offset..]]),
                expected,
                "split at {offset}"
            );
        }

        let single_bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(
            collect_all(&mut Scanner::new(), &single_bytes),
            expected,
            "byte-by-byte"
        );
    }

    #[test]
    fn can_sub_stray_esc_and_double_esc_follow_vte() {
        // CAN cancels a graphics command and passes through as text.
        let can = b"a\x1b_Ga=t;partial\x18b";
        assert_eq!(
            collect_all(&mut Scanner::new(), &[can]),
            vec![ScanItem::Text(b"a\x18b".to_vec())]
        );

        // SUB cancels a graphics command and passes through as text.
        let sub = b"a\x1b_Ga=t;partial\x1ab";
        assert_eq!(
            collect_all(&mut Scanner::new(), &[sub]),
            vec![ScanItem::Text(b"a\x1ab".to_vec())]
        );

        // Stray ESC cancels the command and starts a new sequence for vte.
        let stray = b"a\x1b_Ga=t;partial\x1b[31mred";
        assert_eq!(
            collect_all(&mut Scanner::new(), &[stray]),
            vec![ScanItem::Text(b"a\x1b[31mred".to_vec())]
        );

        // ESC ESC _ G: first ESC goes to vte as text, second starts the command.
        let double_esc = b"\x1b\x1b_Ga=q,i=1,s=1,v=1,f=24;AAAA\x1b\\after";
        let expected = vec![
            ScanItem::Text(b"\x1b".to_vec()),
            ScanItem::Command(b"a=q,i=1,s=1,v=1,f=24;AAAA".to_vec()),
            ScanItem::Text(b"after".to_vec()),
        ];
        for offset in 0..=double_esc.len() {
            assert_eq!(
                collect_all(
                    &mut Scanner::new(),
                    &[&double_esc[..offset], &double_esc[offset..]]
                ),
                expected,
                "double esc split at {offset}"
            );
        }

        // ESC _ ESC _ G: first ESC _ passes to vte, second ESC _ G starts the command.
        let nested = b"\x1b_\x1b_Ga=d\x1b\\ok";
        assert_eq!(
            collect_all(&mut Scanner::new(), &[nested]),
            vec![
                ScanItem::Text(b"\x1b_".to_vec()),
                ScanItem::Command(b"a=d".to_vec()),
                ScanItem::Text(b"ok".to_vec()),
            ]
        );

        // Cancelled command immediately followed by another command.
        let back_to_back = b"\x1b_Ga=t;partial\x1b_Ga=d\x1b\\done";
        assert_eq!(
            collect_all(&mut Scanner::new(), &[back_to_back]),
            vec![
                ScanItem::Command(b"a=d".to_vec()),
                ScanItem::Text(b"done".to_vec()),
            ]
        );
    }

    #[test]
    fn overlong_command_is_discarded_and_ended_by_st_can_sub_or_esc() {
        assert_eq!(MAX_COMMAND_BYTES, 48 * 1024 * 1024);

        // Normal ST terminates discard with Discarded marker.
        let mut scanner = Scanner::with_max_bytes(8);
        let stream = b"hi\x1b_G0123456789\x1b\\bye";
        assert_eq!(
            collect_all(&mut scanner, &[stream]),
            vec![
                ScanItem::Text(b"hi".to_vec()),
                ScanItem::Discarded,
                ScanItem::Text(b"bye".to_vec()),
            ]
        );

        // CAN ends discard.
        let mut scanner = Scanner::with_max_bytes(8);
        assert_eq!(
            collect_all(&mut scanner, &[b"hi\x1b_G0123456789\x18bye"]),
            vec![ScanItem::Text(b"hi\x18bye".to_vec())]
        );

        // SUB ends discard.
        let mut scanner = Scanner::with_max_bytes(8);
        assert_eq!(
            collect_all(&mut scanner, &[b"hi\x1b_G0123456789\x1abye"]),
            vec![ScanItem::Text(b"hi\x1abye".to_vec())]
        );

        // Stray ESC ends discard and forwards the new sequence.
        let mut scanner = Scanner::with_max_bytes(8);
        assert_eq!(
            collect_all(&mut scanner, &[b"hi\x1b_G0123456789\x1b[32mok"]),
            vec![ScanItem::Text(b"hi\x1b[32mok".to_vec())]
        );

        // Boundary: exactly max_bytes succeeds.
        let mut scanner = Scanner::with_max_bytes(8);
        assert_eq!(
            collect_all(&mut scanner, &[b"\x1b_G01234567\x1b\\"]),
            vec![ScanItem::Command(b"01234567".to_vec())]
        );
    }

    #[test]
    fn strip_graphics_removes_commands_and_preserves_text() {
        assert_eq!(strip_graphics_commands(b"plain text\x1b[31mred"), None);
        assert_eq!(
            strip_graphics_commands(b"a\x1b_Ga=t,i=1,f=24,s=1,v=1,q=2;AAAA\x1b\\b"),
            Some(b"ab".to_vec())
        );
        assert_eq!(
            strip_graphics_commands(b"\x1b_Ga=d,d=A,q=2\x1b\\"),
            Some(Vec::new())
        );
    }
}

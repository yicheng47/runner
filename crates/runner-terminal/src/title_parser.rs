//! OSC 0/2 and RIS (`ESC c`) title stream parser.
//! Keeps VT sequence parsing inside `runner-terminal` so callers such as
//! runtime title classifiers never depend on `vte` directly.

use alacritty_terminal::vte;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TitleEvent {
    Set(String),
    Reset,
}

#[derive(Default)]
pub struct TitleParser {
    parser: vte::Parser,
}

#[derive(Default)]
struct Collector(Vec<TitleEvent>);

impl vte::Perform for Collector {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if !matches!(params.first(), Some(selector) if *selector == b"0" || *selector == b"2") {
            return;
        }
        let mut raw = Vec::new();
        for (index, part) in params.iter().skip(1).enumerate() {
            if index > 0 {
                raw.push(b';');
            }
            raw.extend_from_slice(part);
        }
        match String::from_utf8(raw) {
            Ok(title) if !title.is_empty() => self.0.push(TitleEvent::Set(title)),
            _ => self.0.push(TitleEvent::Reset),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if !ignore && intermediates.is_empty() && byte == b'c' {
            self.0.push(TitleEvent::Reset);
        }
    }
}

impl TitleParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<TitleEvent> {
        let mut events = Collector::default();
        self.parser.advance(&mut events, bytes);
        events.0
    }
}

#[cfg(test)]
mod tests {
    use super::{TitleEvent, TitleParser};

    #[test]
    fn parses_osc0_osc2_bel_st_and_preserves_semicolons() {
        let mut parser = TitleParser::new();
        assert_eq!(
            parser.advance(b"\x1b]0;project; branch\x07"),
            vec![TitleEvent::Set("project; branch".into())]
        );
        assert_eq!(
            parser.advance(b"\x1b]2;another;title\x1b\\"),
            vec![TitleEvent::Set("another;title".into())]
        );
        assert_eq!(parser.advance(b"\x1b]7;file:///tmp\x07"), Vec::new());
    }

    #[test]
    fn parses_split_osc_across_every_byte_boundary() {
        let sequence = b"\x1b]0;\xe2\xa0\x99 project; branch\x07";
        for split in 1..sequence.len() {
            let mut parser = TitleParser::new();
            assert_eq!(
                parser.advance(&sequence[..split]),
                Vec::new(),
                "split {split}"
            );
            assert_eq!(
                parser.advance(&sequence[split..]),
                vec![TitleEvent::Set("⠙ project; branch".into())],
                "split {split}"
            );
        }
    }

    #[test]
    fn empty_invalid_utf8_and_ris_emit_reset() {
        let mut parser = TitleParser::new();
        assert_eq!(parser.advance(b"\x1b]0;\x07"), vec![TitleEvent::Reset]);
        assert_eq!(
            parser.advance(b"\x1b]2;\xff\xfe\x07"),
            vec![TitleEvent::Reset]
        );
        assert_eq!(parser.advance(b"\x1bc"), vec![TitleEvent::Reset]);
    }
}

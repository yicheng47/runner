use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window};
use runner_terminal::terminal::TerminalSession;

use crate::text_util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKeyRoute {
    AppShortcut,
    Ime,
    Raw,
}

pub fn terminal_key_route(
    composing: bool,
    platform: bool,
    control: bool,
    alt: bool,
    function: bool,
    key: &str,
) -> TerminalKeyRoute {
    if platform {
        TerminalKeyRoute::AppShortcut
    } else if composing {
        TerminalKeyRoute::Ime
    } else if control || alt || function || is_raw_terminal_key(key) {
        TerminalKeyRoute::Raw
    } else {
        TerminalKeyRoute::Ime
    }
}

pub fn swallows_option_copy(
    platform: bool,
    control: bool,
    alt: bool,
    key: &str,
    key_char: Option<&str>,
) -> bool {
    alt && !platform
        && !control
        && (key.eq_ignore_ascii_case("c") || key == "ç" || key_char == Some("ç"))
}

fn is_raw_terminal_key(key: &str) -> bool {
    matches!(
        key,
        "enter"
            | "backspace"
            | "tab"
            | "escape"
            | "up"
            | "down"
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TerminalComposition {
    marked_text: String,
    selected_range: Range<usize>,
}

impl TerminalComposition {
    pub fn marked_text(&self) -> Option<&str> {
        (!self.marked_text.is_empty()).then_some(self.marked_text.as_str())
    }

    pub fn selected_range(&self) -> &Range<usize> {
        &self.selected_range
    }

    pub fn marked_range_utf16(&self) -> Option<Range<usize>> {
        self.marked_text()
            .map(|text| 0..text.encode_utf16().count())
    }

    pub fn replace_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| text_util::range_from_utf16(&self.marked_text, range))
            .unwrap_or(0..self.marked_text.len());
        self.marked_text.replace_range(range.clone(), new_text);
        if self.marked_text.is_empty() {
            self.clear();
            return;
        }
        self.selected_range =
            text_util::marked_selection(range.start, new_text, new_selected_range_utf16.as_ref());
    }

    pub fn clear(&mut self) {
        self.marked_text.clear();
        self.selected_range = 0..0;
    }

    fn selected_range_utf16(&self) -> Range<usize> {
        text_util::range_to_utf16(&self.marked_text, &self.selected_range)
    }

    fn text_for_range_utf16(&self, range_utf16: &Range<usize>) -> Option<(String, Range<usize>)> {
        self.marked_text().map(|text| {
            let range = text_util::range_from_utf16(text, range_utf16);
            let adjusted = text_util::range_to_utf16(text, &range);
            (text[range].to_owned(), adjusted)
        })
    }
}

pub struct TerminalInput {
    session: Arc<TerminalSession>,
    composition: TerminalComposition,
    write_result: Option<Result<(), String>>,
}

impl TerminalInput {
    pub fn new(session: Arc<TerminalSession>) -> Self {
        Self {
            session,
            composition: TerminalComposition::default(),
            write_result: None,
        }
    }

    pub fn marked_text(&self) -> Option<&str> {
        self.composition.marked_text()
    }

    pub fn is_composing(&self) -> bool {
        self.marked_text().is_some()
    }

    pub fn cancel_composition(&mut self) -> bool {
        if !self.is_composing() {
            return false;
        }
        self.composition.clear();
        true
    }

    pub fn replace_and_mark_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        self.composition
            .replace_and_mark(range_utf16, new_text, new_selected_range_utf16);
        self.session.scroll_to_bottom();
    }

    pub fn take_write_result(&mut self) -> Option<Result<(), String>> {
        self.write_result.take()
    }

    pub fn commit_text(&mut self, text: &str) -> runner_backend::error::Result<()> {
        self.composition.clear();
        if text.is_empty() {
            return Ok(());
        }
        self.session.write_user_bytes(text.as_bytes())?;
        self.session.scroll_to_bottom();
        Ok(())
    }

    fn commit_marked_text(&mut self) -> runner_backend::error::Result<()> {
        let text = self.marked_text().unwrap_or_default().to_owned();
        self.commit_text(&text)
    }

    fn record_write_result(&mut self, result: runner_backend::error::Result<()>) {
        self.write_result = Some(result.map_err(|error| error.to_string()));
    }
}

impl EntityInputHandler for TerminalInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, adjusted) = self.composition.text_for_range_utf16(&range_utf16)?;
        adjusted_range.replace(adjusted);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition.selected_range_utf16(),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composition.marked_range_utf16()
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.commit_marked_text();
        self.record_write_result(result);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.commit_text(new_text);
        self.record_write_result(result);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text(range_utf16, new_text, new_selected_range_utf16);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        cursor_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(cursor_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _position: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::swallows_option_copy;

    #[test]
    fn option_copy_never_reaches_the_terminal() {
        assert!(swallows_option_copy(false, false, true, "c", Some("ç")));
        assert!(swallows_option_copy(false, false, true, "ç", Some("ç")));
        assert!(!swallows_option_copy(true, false, true, "c", Some("ç")));
        assert!(!swallows_option_copy(false, true, true, "c", Some("ç")));
        assert!(!swallows_option_copy(false, false, true, "v", Some("√")));
    }
}

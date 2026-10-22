use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Bounds, Context, EntityInputHandler, Keystroke, Pixels, UTF16Selection, Window};
use runner_terminal::terminal::TerminalMirror;
use serde::{Deserialize, Serialize};

use crate::text_util;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsMeta {
    Both,
    Left,
    Right,
    Off,
    #[default]
    #[serde(other)]
    Automatic,
}

impl OptionAsMeta {
    pub const ALL: [Self; 5] = [
        Self::Automatic,
        Self::Both,
        Self::Left,
        Self::Right,
        Self::Off,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Both => "both",
            Self::Left => "left",
            Self::Right => "right",
            Self::Off => "off",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::Automatic => "terminal.option_meta_automatic",
            Self::Both => "terminal.option_meta_both",
            Self::Left => "terminal.option_meta_left",
            Self::Right => "terminal.option_meta_right",
            Self::Off => "terminal.option_meta_off",
        }
    }

    pub fn description_key(self) -> &'static str {
        match self {
            Self::Automatic => "terminal.option_meta_automatic_description",
            Self::Both => "terminal.option_meta_both_description",
            Self::Left => "terminal.option_meta_left_description",
            Self::Right => "terminal.option_meta_right_description",
            Self::Off => "terminal.option_meta_off_description",
        }
    }

    pub fn is_meta(self, cx: &App) -> bool {
        #[cfg(target_os = "macos")]
        {
            let flags = if matches!(self, Self::Left | Self::Right) {
                current_option_flags()
            } else {
                0
            };
            self.for_layout(cx.keyboard_layout().id(), flags)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cx;
            true
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn for_layout(self, layout_id: &str, flags: usize) -> bool {
        match self {
            Self::Automatic => matches!(
                layout_id,
                "com.apple.keylayout.US"
                    | "com.apple.keylayout.USInternational"
                    | "com.apple.keylayout.USInternational-PC"
                    | "com.apple.keylayout.ABC"
            ),
            Self::Both => true,
            Self::Left => flags & NX_DEVICELALTKEYMASK != 0,
            Self::Right => flags & NX_DEVICERALTKEYMASK != 0,
            Self::Off => false,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
const NX_DEVICELALTKEYMASK: usize = 0x20;
#[cfg(any(target_os = "macos", test))]
const NX_DEVICERALTKEYMASK: usize = 0x40;

#[cfg(target_os = "macos")]
fn current_option_flags() -> usize {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    // GPUI dispatches key-down synchronously, while AppKit still owns this event.
    let main_thread = MainThreadMarker::new().expect("terminal keys run on the main thread");
    NSApplication::sharedApplication(main_thread)
        .currentEvent()
        .map_or(0, |event| event.modifierFlags().bits())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKeyRoute {
    AppShortcut,
    Ime,
    Raw,
}

pub fn terminal_key_route(
    composing: bool,
    keystroke: &Keystroke,
    option_is_meta: bool,
) -> TerminalKeyRoute {
    let modifiers = keystroke.modifiers;
    let printable = keystroke
        .key_char
        .as_deref()
        .is_some_and(|text| !text.is_empty() && text.chars().all(|c| !c.is_control()));
    if modifiers.platform {
        TerminalKeyRoute::AppShortcut
    } else if composing {
        TerminalKeyRoute::Ime
    } else if modifiers.control
        || modifiers.function
        || is_raw_terminal_key(&keystroke.key)
        || (modifiers.alt && (option_is_meta || !printable))
    {
        TerminalKeyRoute::Raw
    } else {
        TerminalKeyRoute::Ime
    }
}

pub fn swallows_option_copy(keystroke: &Keystroke, option_is_meta: bool) -> bool {
    let modifiers = keystroke.modifiers;
    option_is_meta
        && modifiers.alt
        && !modifiers.platform
        && !modifiers.control
        && (keystroke.key.eq_ignore_ascii_case("c")
            || keystroke.key == "ç"
            || keystroke.key_char.as_deref() == Some("ç"))
}

// macOS layouts type U+00A0 for Option+Space, which a shell reads as part of a word.
fn typed_text(text: &str) -> &str {
    if text == "\u{a0}" {
        " "
    } else {
        text
    }
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
    session: Arc<TerminalMirror>,
    composition: TerminalComposition,
    write_result: Option<Result<(), String>>,
}

impl TerminalInput {
    pub fn new(session: Arc<TerminalMirror>) -> Self {
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
        self.session.set_composing(false);
        true
    }

    pub fn replace_and_mark_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) {
        if !self.session.control().desktop() {
            self.cancel_composition();
            return;
        }
        self.composition
            .replace_and_mark(range_utf16, new_text, new_selected_range_utf16);
        self.session.set_composing(self.is_composing());
        self.session.scroll_to_bottom();
    }

    pub fn take_write_result(&mut self) -> Option<Result<(), String>> {
        self.write_result.take()
    }

    pub fn commit_text(&mut self, text: &str) -> runner_core::protocol::ClientResult<()> {
        self.composition.clear();
        self.session.set_composing(false);
        if !self.session.control().desktop() {
            return Ok(());
        }
        if text.is_empty() {
            return Ok(());
        }
        self.session.send_text(text)?;
        self.session.scroll_to_bottom();
        Ok(())
    }

    fn commit_marked_text(&mut self) -> runner_core::protocol::ClientResult<()> {
        let text = self.marked_text().unwrap_or_default().to_owned();
        self.commit_text(&text)
    }

    fn record_write_result(&mut self, result: runner_core::protocol::ClientResult<()>) {
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
        let result = self.commit_text(typed_text(new_text));
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
    use super::{
        swallows_option_copy, terminal_key_route, typed_text, OptionAsMeta, TerminalKeyRoute,
    };
    use gpui::Keystroke;

    #[test]
    fn automatic_uses_meta_only_for_us_and_abc_layouts() {
        for layout in ["US", "USInternational", "USInternational-PC", "ABC"] {
            let mut key = Keystroke::parse("alt-b").unwrap();
            key.key_char = Some("∫".into());
            assert_eq!(
                terminal_key_route(
                    false,
                    &key,
                    OptionAsMeta::Automatic
                        .for_layout(&format!("com.apple.keylayout.{layout}"), 0x20),
                ),
                TerminalKeyRoute::Raw,
                "{layout}",
            );
        }
        for layout in ["German", "Spanish", "SwissGerman", "ABC-QWERTZ", "unknown"] {
            assert!(
                !OptionAsMeta::Automatic.for_layout(&format!("com.apple.keylayout.{layout}"), 0x20)
            );
        }
    }

    #[test]
    fn german_option_characters_follow_the_mode_and_option_side() {
        for (mode, flags, expected) in [
            (OptionAsMeta::Automatic, 0x20, TerminalKeyRoute::Ime),
            (OptionAsMeta::Off, 0x20, TerminalKeyRoute::Ime),
            (OptionAsMeta::Both, 0x20, TerminalKeyRoute::Raw),
            (OptionAsMeta::Left, 0x20, TerminalKeyRoute::Raw),
            (OptionAsMeta::Left, 0x40, TerminalKeyRoute::Ime),
            (OptionAsMeta::Right, 0x20, TerminalKeyRoute::Ime),
            (OptionAsMeta::Right, 0x40, TerminalKeyRoute::Raw),
            (OptionAsMeta::Left, 0x60, TerminalKeyRoute::Raw),
            (OptionAsMeta::Right, 0x60, TerminalKeyRoute::Raw),
        ] {
            for (chord, text) in [
                ("alt-l", "@"),
                ("alt-5", "["),
                ("alt-6", "]"),
                ("alt-7", "|"),
                ("alt-/", "\\"),
                ("alt-n", "˜"),
            ] {
                let mut key = Keystroke::parse(chord).unwrap();
                key.key_char = Some(text.into());
                assert_eq!(
                    terminal_key_route(
                        false,
                        &key,
                        mode.for_layout("com.apple.keylayout.German", flags)
                    ),
                    expected,
                    "{mode:?} flags={flags:#x} {chord}",
                );
            }
        }
    }

    #[test]
    fn option_non_text_chords_remain_raw_in_every_mode() {
        for mode in OptionAsMeta::ALL {
            for (chord, text) in [
                ("alt-enter", Some("\n")),
                ("alt-backspace", None),
                ("alt-left", None),
                ("alt-right", None),
                ("alt-f1", None),
                ("ctrl-alt-l", Some("@")),
                ("alt-l", Some("\u{7f}")),
                ("alt-l", Some("@\n")),
                ("alt-l", Some("")),
            ] {
                let mut key = Keystroke::parse(chord).unwrap();
                key.key_char = text.map(str::to_owned);
                assert_eq!(
                    terminal_key_route(
                        false,
                        &key,
                        mode.for_layout("com.apple.keylayout.German", 0x40)
                    ),
                    TerminalKeyRoute::Raw,
                    "{mode:?} {chord} {text:?}",
                );
            }
        }
    }

    #[test]
    fn option_copy_never_reaches_the_terminal_while_option_is_meta() {
        for (chord, text, meta, swallowed) in [
            ("alt-c", "ç", true, true),
            ("alt-ç", "ç", true, true),
            ("cmd-alt-c", "ç", true, false),
            ("ctrl-alt-c", "ç", true, false),
            ("alt-v", "√", true, false),
            ("alt-c", "ç", false, false),
        ] {
            let mut key = Keystroke::parse(chord).unwrap();
            key.key_char = Some(text.into());
            assert_eq!(
                swallows_option_copy(&key, meta),
                swallowed,
                "{chord} meta={meta}"
            );
        }
    }

    #[test]
    fn option_space_types_a_plain_space() {
        assert_eq!(typed_text("\u{a0}"), " ");
        assert_eq!(typed_text("@"), "@");
        assert_eq!(typed_text("a\u{a0}b"), "a\u{a0}b");
    }
}

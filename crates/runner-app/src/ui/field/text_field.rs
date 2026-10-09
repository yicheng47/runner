use super::*;

use super::working_dir::home_dir;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextFieldKind {
    #[default]
    Input,
    Textarea {
        rows: u8,
    },
}

impl TextFieldKind {
    pub(super) fn multiline(self) -> bool {
        matches!(self, Self::Textarea { .. })
    }

    pub(super) fn rows(self) -> u8 {
        match self {
            Self::Input => 1,
            Self::Textarea { rows } => rows.max(1),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FieldValidation {
    #[default]
    Valid,
    Error(SharedString),
}

impl FieldValidation {
    pub fn error(message: impl Into<SharedString>) -> Self {
        Self::Error(message.into())
    }

    pub fn message(&self) -> Option<&SharedString> {
        match self {
            Self::Valid => None,
            Self::Error(message) => Some(message),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

impl TextField {
    pub fn new(
        focus_handle: FocusHandle,
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        monospace: bool,
    ) -> Self {
        let mut buffer = TextBuffer::default();
        buffer.reset(text);
        Self {
            focus_handle,
            buffer,
            placeholder: placeholder.into(),
            placeholder_as_value: false,
            monospace,
            fill_height: false,
            kind: TextFieldKind::Input,
            disabled: false,
            validation: FieldValidation::Valid,
            bare: false,
            text_size: theme::text_title(),
            right_padding: 10.,
            hover_border: false,
            disabled_cursor_not_allowed: false,
            scroll_handle: ScrollHandle::new(),
            scrollbar: None,
            selecting: false,
            drag_position: None,
            auto_scroll: None,
            text_state: Rc::default(),
            caret_row_end: None,
            vertical_goal: None,
            auto_grow_rows: None,
            key_interceptor: None,
            truncate_unfocused: false,
            compact_path: false,
            compact_shown: Rc::new(RefCell::new(None)),
        }
    }

    pub fn textarea(
        focus_handle: FocusHandle,
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        rows: u8,
        monospace: bool,
    ) -> Self {
        let mut field = Self::new(focus_handle, text, placeholder, monospace);
        field.kind = TextFieldKind::Textarea { rows: rows.max(1) };
        field.buffer.multiline = true;
        field
    }

    /// Let a textarea grow with its wrapped content, from its base `rows` up
    /// to `max_rows`; longer content scrolls as before.
    pub fn auto_grow(mut self, max_rows: u8) -> Self {
        self.auto_grow_rows = Some(max_rows);
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.fill_height = true;
        self
    }

    pub fn with_scrollbar(mut self, cx: &mut Context<Self>) -> Self {
        let owner = cx.entity_id();
        let handle = self.scroll_handle.clone();
        self.scrollbar = Some(cx.new(|_| Scrollbar::app(handle, owner)));
        self
    }

    pub fn key_interceptor(mut self, interceptor: KeyDownInterceptor) -> Self {
        self.key_interceptor = Some(interceptor);
        self
    }

    pub fn truncate_unfocused(mut self) -> Self {
        self.truncate_unfocused = true;
        self
    }

    /// While unfocused, show the text as a path compacted to fit: home as
    /// `~`, then middle folders folded into `…`, always keeping the last
    /// folder. Focused, the field edits the full path.
    pub fn compact_path(mut self) -> Self {
        self.compact_path = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.buffer.text
    }

    /// The caret's UTF-8 byte offset in `text()`.
    pub fn caret_offset(&self) -> usize {
        self.buffer.selection.caret
    }

    pub fn edited(&self) -> bool {
        self.buffer.edited
    }

    pub fn mark_clean(&mut self) {
        self.buffer.edited = false;
    }

    pub fn is_composing(&self) -> bool {
        self.buffer.marked.is_some()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn placeholder_as_value(mut self, placeholder_as_value: bool) -> Self {
        self.placeholder_as_value = placeholder_as_value;
        self
    }

    pub fn set_placeholder_as_value(&mut self, placeholder_as_value: bool, cx: &mut Context<Self>) {
        if self.placeholder_as_value != placeholder_as_value {
            self.placeholder_as_value = placeholder_as_value;
            cx.notify();
        }
    }

    pub fn text_size(mut self, text_size: Rems) -> Self {
        self.text_size = text_size;
        self
    }

    pub fn right_padding(mut self, right_padding: f32) -> Self {
        self.right_padding = right_padding;
        self
    }

    pub fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.reset(text);
        self.vertical_goal = None;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.set_text(&text.into());
        self.vertical_goal = None;
        self.buffer.edited = true;
        cx.notify();
    }

    /// Replaces a UTF-8 byte range as one undo step, leaving the caret after
    /// the inserted text. Refuses disabled input, active IME composition,
    /// and ranges that are outside the text or split a character.
    pub fn replace_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.disabled || !self.buffer.replace_range(range, text) {
            return false;
        }
        self.vertical_goal = None;
        self.caret_row_end = None;
        self.reveal_caret();
        cx.notify();
        true
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.vertical_goal = None;
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }

    pub fn set_validation(&mut self, validation: FieldValidation, cx: &mut Context<Self>) {
        if self.validation != validation {
            self.validation = validation;
            cx.notify();
        }
    }

    pub fn set_bare(&mut self, bare: bool, cx: &mut Context<Self>) {
        if self.bare != bare {
            self.bare = bare;
            cx.notify();
        }
    }

    pub fn placeholder_uses_value_style(&self) -> bool {
        self.placeholder_as_value
    }

    #[cfg(test)]
    pub(crate) fn text_right_padding(&self) -> f32 {
        self.right_padding
    }

    pub fn set_right_padding(&mut self, right_padding: f32, cx: &mut Context<Self>) {
        if self.right_padding != right_padding {
            self.right_padding = right_padding;
            cx.notify();
        }
    }

    pub fn set_hover_border(&mut self, hover_border: bool, cx: &mut Context<Self>) {
        if self.hover_border != hover_border {
            self.hover_border = hover_border;
            cx.notify();
        }
    }

    pub fn set_disabled_cursor_not_allowed(
        &mut self,
        disabled_cursor_not_allowed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.disabled_cursor_not_allowed != disabled_cursor_not_allowed {
            self.disabled_cursor_not_allowed = disabled_cursor_not_allowed;
            cx.notify();
        }
    }

    pub(super) fn reveal_caret(&self) {
        self.text_state.borrow_mut().reveal_caret = true;
    }

    /// The shaped text for the current value, reshaped at the last painted
    /// width and style when an edit landed since, and where it was painted
    /// at the current scroll offset.
    pub(super) fn with_text<R>(
        &self,
        window: &Window,
        f: impl FnOnce(&FieldText, Option<Point<Pixels>>) -> R,
    ) -> Option<R> {
        let mut state = self.text_state.borrow_mut();
        let shaped = state.shaped.as_ref()?;
        let current = if self.buffer.text.is_empty() {
            shaped.key.placeholder && shaped.key.text == self.placeholder
        } else {
            !shaped.key.placeholder && shaped.key.text.as_ref() == self.buffer.text
        };
        if !current {
            let (text, placeholder) = self.display_text();
            let key = FieldTextKey {
                text,
                placeholder,
                ..shaped.key.clone()
            };
            let wrap_width = shaped.wrap_width;
            state.shaped = Some(FieldText::new(key, wrap_width, window));
        }
        let origin = state
            .origin
            .map(|origin| origin + (self.scroll_offset() - state.scroll_offset));
        state.shaped.as_ref().map(|shaped| f(shaped, origin))
    }

    fn scroll_offset(&self) -> Point<Pixels> {
        if self.kind.multiline() {
            self.scroll_handle.offset()
        } else {
            Point::default()
        }
    }

    fn display_text(&self) -> (SharedString, bool) {
        if self.buffer.text.is_empty() {
            (self.placeholder.clone(), true)
        } else {
            (SharedString::from(self.buffer.text.clone()), false)
        }
    }

    pub(super) fn text_key(&self, window: &Window) -> FieldTextKey {
        let style = window.text_style();
        let rem_size = window.rem_size();
        let (text, placeholder) = self.display_text();
        FieldTextKey {
            text,
            placeholder,
            multiline: self.kind.multiline(),
            font: style.font(),
            color: if placeholder && !self.placeholder_as_value {
                theme::faint()
            } else {
                style.color
            },
            font_size: style.font_size.to_pixels(rem_size),
            line_height: style.line_height_in_pixels(rem_size),
        }
    }

    /// The compacted path to show, or `None` while the field edits it.
    pub(super) fn compact_path_shown(&self, focused: bool) -> Option<String> {
        if !self.compact_path || focused || self.kind != TextFieldKind::Input {
            return None;
        }
        let text = &self.buffer.text;
        if text.is_empty() {
            return None;
        }
        let chosen = self
            .compact_shown
            .borrow()
            .as_ref()
            .filter(|(source, _)| source == text)
            .map(|(_, shown)| shown.clone());
        chosen.or_else(|| {
            compact_path_candidates(text, home_dir().as_deref())
                .into_iter()
                .next()
        })
    }

    pub(super) fn render_text(&self, focused: bool, field: Entity<Self>) -> AnyElement {
        let read_only = if let Some(shown) = self.compact_path_shown(focused) {
            let rendered = shown.clone();
            Some(
                div()
                    .debug_selector(move || format!("TEXT_FIELD_COMPACT {rendered}"))
                    .min_w(px(0.))
                    .w_full()
                    .truncate()
                    .child(shown),
            )
        } else if self.kind == TextFieldKind::Input
            && self.truncate_unfocused
            && !focused
            && !self.buffer.text.is_empty()
        {
            Some(
                div()
                    .min_w(px(0.))
                    .w_full()
                    .truncate()
                    .child(self.buffer.text.clone()),
            )
        } else {
            None
        };
        match read_only {
            Some(display) => {
                self.text_state.borrow_mut().origin = None;
                display.into_any_element()
            }
            None => TextFieldElement { field }.into_any_element(),
        }
    }
}

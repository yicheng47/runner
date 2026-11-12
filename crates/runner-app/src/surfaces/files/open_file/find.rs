//! In-file find, its asynchronous results and its bar over the code view.

use super::*;

impl FilesView {
    // ── Find ───────────────────────────────────────────────────────────

    /// ⌘F: find over the active file; a rendered page switches to its
    /// source, which find searches.
    pub(in super::super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_rendered().is_some() {
            self.toggle_source(window, cx);
        }
        if self.active_view().is_none() {
            return;
        }
        if self.viewer.find.is_none() {
            let seed = self
                .active_view()
                .and_then(|view| view.read(cx).selected_text())
                .and_then(|text| text.lines().next().map(str::to_owned))
                .filter(|line| !line.trim().is_empty())
                .unwrap_or_default();
            let root = cx.weak_entity();
            let field = cx.new(move |field_cx| {
                let mut field = TextField::new(
                    field_cx.focus_handle(),
                    "",
                    crate::i18n::message("terminal.find_placeholder"),
                    false,
                )
                .text_size(theme::text_body())
                .key_interceptor(Rc::new(move |event, window, cx| {
                    root.update(cx, |this, cx| this.on_find_key(event, window, cx))
                        .unwrap_or(false)
                }));
                field.set_bare(true, field_cx);
                field.set_right_padding(0., field_cx);
                field.set_text(seed, field_cx);
                field
            });
            let observe = cx.observe(&field, |this, field, cx| {
                let text = field.read(cx).text().to_owned();
                if let Some(find) = &mut this.viewer.find {
                    if find.query.text != text {
                        find.query.text = text;
                        find.current = None;
                        this.release_cited(cx);
                        this.search(cx);
                    }
                }
            });
            let text = field.read(cx).text().to_owned();
            self.viewer.find = Some(Find {
                field,
                query: FileQuery {
                    text,
                    ..FileQuery::default()
                },
                found: Found::Empty,
                searched: None,
                current: None,
                search: 0,
                _observe: observe,
            });
            self.search(cx);
        }
        if let Some(find) = &self.viewer.find {
            find.field.update(cx, |field, field_cx| {
                field.select_all(field_cx);
                field.focus_handle().focus(window, field_cx);
            });
        }
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer.find = None;
        if let Some(view) = self.active_view().cloned() {
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            view.read(cx).focus_handle().clone().focus(window, cx);
        }
        cx.notify();
    }

    fn on_find_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.platform && !modifiers.control && !modifiers.alt;
        let step_key = if cfg!(target_os = "macos") {
            keystroke.key.eq_ignore_ascii_case("g") && modifiers.platform && !modifiers.control
        } else {
            keystroke.key == "f3" && plain
        };
        if keystroke.key == "escape" && plain && !modifiers.shift {
            self.close_find(window, cx);
            true
        } else if (keystroke.key == "enter" && plain) || step_key {
            self.step_find(!modifiers.shift, cx);
            true
        } else {
            false
        }
    }

    fn release_cited(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.active_view().cloned() {
            view.update(cx, |view, _| view.release_cited());
        }
    }

    /// Searches the active file for find's query off the UI thread.
    pub(super) fn search(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view().cloned() else {
            return;
        };
        let document = view.read(cx).document().clone();
        let Some(find) = &mut self.viewer.find else {
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            return;
        };
        // Matches from another text name byte ranges that are not this
        // one's: they go before anything can draw or step through them.
        if !find
            .searched
            .as_ref()
            .is_some_and(|searched| Arc::ptr_eq(searched, &document))
        {
            find.found = Found::Empty;
            find.searched = None;
            find.current = None;
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
        }
        self.viewer.next_search += 1;
        let search = self.viewer.next_search;
        find.search = search;
        let line_in_view = view.read(cx).line_in_view() as usize - 1;
        #[cfg(test)]
        if self.viewer.hold_search {
            return;
        }
        let Some(find) = &mut self.viewer.find else {
            return;
        };
        let query = find.query.clone();
        let searched = document.clone();
        let task = cx.background_spawn(async move { document::find(&document, &query) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_search(search, searched, found, line_in_view, cx)
            });
        })
        .detach();
    }

    /// A search's reply: kept only for the newest search of this very text.
    pub(super) fn apply_search(
        &mut self,
        search: u64,
        searched: Arc<Document>,
        found: Found,
        line_in_view: usize,
        cx: &mut Context<Self>,
    ) {
        {
            let shown = self
                .active_view()
                .map(|view| view.read(cx).document().clone());
            let Some(find) = &mut self.viewer.find else {
                return;
            };
            let same_text = shown.is_some_and(|shown| Arc::ptr_eq(&shown, &searched));
            if find.search != search || !same_text {
                return;
            }
            if !find
                .searched
                .as_ref()
                .is_some_and(|before| Arc::ptr_eq(before, &searched))
            {
                find.current = None;
            }
            find.searched = Some(searched);
            if let Found::Matches(matches, _) = &found {
                find.current = find
                    .current
                    .filter(|current| *current < matches.len())
                    .or_else(|| {
                        (!matches.is_empty()).then(|| {
                            matches
                                .iter()
                                .position(|found| found.line >= line_in_view)
                                .unwrap_or(0)
                        })
                    });
            }
            find.found = found;
            self.show_matches(cx);
        }
    }

    /// The active text's matches: none unless find searched this very text.
    fn shown_matches(&self, cx: &App) -> Option<&Find> {
        let document = self.active_view()?.read(cx).document();
        self.viewer.find.as_ref().filter(|find| {
            find.searched
                .as_ref()
                .is_some_and(|searched| Arc::ptr_eq(searched, document))
        })
    }

    fn show_matches(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view().cloned() else {
            return;
        };
        let (matches, current) = match self.shown_matches(cx) {
            Some(Find {
                found: Found::Matches(matches, _),
                current,
                ..
            }) => (Arc::new(matches.clone()), *current),
            _ => (Arc::default(), None),
        };
        view.update(cx, |view, cx| view.set_find(matches, current, cx));
        cx.notify();
    }

    pub(super) fn step_find(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.shown_matches(cx).is_none() {
            return;
        }
        self.release_cited(cx);
        let Some(find) = &mut self.viewer.find else {
            return;
        };
        let Found::Matches(matches, _) = &find.found else {
            return;
        };
        if matches.is_empty() {
            return;
        }
        let count = matches.len();
        find.current = Some(match find.current {
            Some(current) if next => (current + 1) % count,
            Some(current) => (current + count - 1) % count,
            None => 0,
        });
        self.show_matches(cx);
    }

    fn toggle_find(&mut self, regex: bool, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.viewer.find {
            if regex {
                find.query.regex = !find.query.regex;
            } else {
                find.query.match_case = !find.query.match_case;
            }
            find.current = None;
        }
        self.release_cited(cx);
        self.search(cx);
        cx.notify();
    }

    pub(super) fn find_count(find: &Find) -> Option<(SharedString, bool)> {
        match &find.found {
            Found::Empty => None,
            Found::Invalid => Some((crate::i18n::t("terminal.find_invalid_regex"), true)),
            Found::Matches(matches, _) if matches.is_empty() => {
                Some((crate::i18n::t("terminal.find_no_results"), true))
            }
            Found::Matches(matches, more) => {
                let total = if *more {
                    format!("{MATCH_LIMIT}+")
                } else {
                    matches.len().to_string()
                };
                let current = find.current.map_or(0, |current| current + 1).to_string();
                Some((
                    crate::i18n::tf(
                        "terminal.find_count",
                        &[("current", current), ("total", total)],
                    )
                    .into(),
                    false,
                ))
            }
        }
    }

    pub(super) fn render_code(
        &self,
        view: Entity<CodeView>,
        banner: Option<AnyElement>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let find = self.viewer.find.as_ref().map(|find| {
            let root = cx.weak_entity();
            let action = |act: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                let root = root.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let _ = root.update(cx, |this, cx| act(this, window, cx));
                }) as crate::terminal::find::BarAction
            };
            let step_root = root.clone();
            div()
                .absolute()
                .top(px(8.))
                .left(px(14.))
                .right(px(14.))
                .flex()
                .justify_end()
                .child(
                    find_bar_panel(FindBarParts {
                        prefix: "file-find",
                        field: find.field.clone(),
                        match_case: find.query.match_case,
                        regex: find.query.regex,
                        count: Self::find_count(find),
                        can_step: matches!(&find.found, Found::Matches(matches, _) if !matches.is_empty()),
                        footer: None,
                        toggle_case: action(|this, _, cx| this.toggle_find(false, cx)),
                        toggle_regex: action(|this, _, cx| this.toggle_find(true, cx)),
                        step: Rc::new(move |next, _, cx| {
                            let _ = step_root.update(cx, |this, cx| this.step_find(next, cx));
                        }),
                        close: action(|this, window, cx| this.close_find(window, cx)),
                    })
                    .id("file-find")
                    .debug_selector(|| "FILE_FIND_BAR".into()),
                )
        });
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .children(banner)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(view)
                    .children(find),
            )
            .into_any_element()
    }
}

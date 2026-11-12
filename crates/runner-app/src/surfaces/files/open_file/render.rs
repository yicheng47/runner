//! The file column, tab strip, overflow list and file content states.

use super::*;

const STRIP_HEIGHT: f32 = 44.;

const LIST_WIDTH: f32 = 300.;

/// A 28px control in the strip's right-hand group.
fn strip_button(id: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .when(cfg!(test), |element| {
            element.debug_selector(move || id.to_uppercase().replace('-', "_"))
        })
        .flex_none()
        .h(px(26.))
        .min_w(px(26.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .text_size(theme::text_meta())
        .text_color(theme::muted())
        .when(active, |element| element.bg(theme::sidebar_selected()))
        .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl FilesView {
    // ── Rendering ──────────────────────────────────────────────────────

    fn tab_letter(&self, tab: &FileTab) -> Option<(&'static str, gpui::Hsla)> {
        if matches!(
            self.viewer.files.get(&tab.path).map(|file| &file.content),
            Some(Content::Deleted(_))
        ) {
            return Some(("D", theme::danger()));
        }
        let letter: Letter = self.tree_ref()?.letter(&tab.path)?;
        Some((letter.text(), letter_color(letter)))
    }

    fn render_tab(&self, index: usize, tab: &FileTab, cx: &mut Context<Self>) -> AnyElement {
        let active = self.viewer.tabs.active_index() == Some(index);
        let group: SharedString = format!("file-tab-{index}").into();
        let path = tab.path.clone();
        let letter = self.tab_letter(tab);
        div()
            .id(("file-tab", index))
            .when(cfg!(test), |element| {
                let path = path.clone();
                element.debug_selector(move || format!("FILE_TAB {path}"))
            })
            .group(group.clone())
            .flex_none()
            .h(px(28.))
            .pl(px(10.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .border_1()
            .border_color(if active {
                theme::border()
            } else {
                gpui::transparent_black()
            })
            .cursor_pointer()
            .text_size(theme::text_ui())
            .text_color(if active {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active, |element| element.bg(theme::raised()))
            .hover(|element| element.text_color(theme::text()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate_tab(index, cx);
                match this.active_view() {
                    Some(view) => view.read(cx).focus_handle().clone().focus(window, cx),
                    None => this.focus.focus(window, cx),
                }
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab(index, window, cx)),
            )
            .child(div().whitespace_nowrap().child(tab.name().to_owned()))
            .children(letter.map(|(text, color)| {
                div()
                    .when(cfg!(test), |element| {
                        let path = path.clone();
                        element.debug_selector(move || format!("FILE_TAB_LETTER {path} {text}"))
                    })
                    .flex_none()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_meta())
                    .text_color(color)
                    .child(text)
            }))
            .child(
                div()
                    .id(("file-tab-close", index))
                    .when(cfg!(test), |element| {
                        let path = path.clone();
                        element.debug_selector(move || format!("FILE_TAB_CLOSE {path}"))
                    })
                    .flex_none()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .when(!active, |element| {
                        element
                            .invisible()
                            .group_hover(group, |element| element.visible())
                    })
                    .hover(|element| element.bg(theme::sidebar_selected()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(index, window, cx);
                    }))
                    .child(
                        svg()
                            .size(px(11.))
                            .path("close.svg")
                            .text_color(theme::muted()),
                    ),
            )
            .into_any_element()
    }

    fn render_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self
            .viewer
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| self.render_tab(index, tab, cx))
            .collect::<Vec<_>>();
        let offset = self.viewer.strip.offset().x;
        let max = self.viewer.strip.max_offset().x;
        let overflowing = max > px(0.5);
        let fade = |left: bool| {
            let bg = theme::bg();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(28.))
                .when(left, |element| element.left_0())
                .when(!left, |element| element.right_0())
                .bg(linear_gradient(
                    if left { 90. } else { 270. },
                    linear_color_stop(bg, 0.),
                    linear_color_stop(theme::with_alpha(bg, 0.), 1.),
                ))
        };
        let count = self.viewer.tabs.tabs().len();
        let list_anchor = self.viewer.list_anchor.clone();
        let open_in_anchor = self.viewer.open_in_anchor.clone();
        let (_, effective) = found_editors(cx);
        let deleted = self.active_deleted();
        let open_in_label = crate::i18n::tf(
            "format.open_in_editor",
            &[("editor", effective.label().to_owned())],
        );
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_TAB_STRIP".into())
            })
            .flex_none()
            .h(px(STRIP_HEIGHT))
            .pl(px(8.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .child(
                        div()
                            .id("file-tabs")
                            .size_full()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .overflow_x_scroll()
                            .track_scroll(&self.viewer.strip)
                            .children(tabs),
                    )
                    .when(offset < px(-0.5), |element| element.child(fade(true)))
                    .when(overflowing && -offset < max - px(0.5), |element| {
                        element.child(fade(false))
                    }),
            )
            .when(overflowing || self.viewer.list_open, |element| {
                element.child(
                    strip_button("file-tab-list", self.viewer.list_open)
                        .child(
                            canvas(move |bounds, _, _| list_anchor.set(bounds), |_, _, _, _| {})
                                .absolute()
                                .inset_0(),
                        )
                        .relative()
                        .child(
                            svg()
                                .size(px(13.))
                                .path("files.svg")
                                .text_color(theme::muted()),
                        )
                        .child(count.to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.viewer.list_open = !this.viewer.list_open;
                            this.viewer.open_in_open = false;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.active_markdown(), |element, source| {
                element.child(
                    strip_button("file-markdown-source", source)
                        .when(deleted, |element| element.opacity(0.4).cursor_default())
                        .when(!source, |element| {
                            element.border_color(gpui::transparent_black())
                        })
                        .tooltip(move |_, cx| {
                            runner_app::ui::tooltip::tooltip_view(
                                crate::i18n::t(if source {
                                    "ui.show_markdown_rendered"
                                } else {
                                    "ui.show_markdown_source"
                                }),
                                cx,
                            )
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_source(window, cx)))
                        .child(svg().size(px(14.)).path("code.svg").text_color(if source {
                            theme::text()
                        } else {
                            theme::muted()
                        })),
                )
            })
            .when(self.has_tabs(), |strip| {
                strip.child(
                    div()
                        .relative()
                        .flex_none()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        // A deleted file has nothing to open: closing is its only action.
                        .when(deleted, |element| element.opacity(0.4))
                        .child(
                            canvas(
                                move |bounds, _, _| open_in_anchor.set(bounds),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .child(
                            div()
                                .id("file-open-in")
                                .when(cfg!(test), |element| {
                                    element.debug_selector(|| "FILE_OPEN_IN".into())
                                })
                                .h(px(24.))
                                .w(px(26.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_l_md()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .when(!deleted, |element| {
                                    element
                                        .cursor_pointer()
                                        .hover(|element| {
                                            element.bg(theme::with_alpha(
                                                theme::sidebar_selected(),
                                                0.6,
                                            ))
                                        })
                                        .tooltip(move |_, cx| {
                                            runner_app::ui::tooltip::tooltip_view(
                                                open_in_label.clone(),
                                                cx,
                                            )
                                        })
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.open_in(None, cx)),
                                        )
                                })
                                .child(editor_icon(effective, 14.)),
                        )
                        .child(div().w(px(1.)).h(px(14.)).bg(theme::border()))
                        .child(
                            div()
                                .id("file-open-in-menu")
                                .when(cfg!(test), |element| {
                                    element.debug_selector(|| "FILE_OPEN_IN_MENU".into())
                                })
                                .h(px(24.))
                                .w(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_r_md()
                                .when(self.viewer.open_in_open, |element| {
                                    element.bg(theme::sidebar_selected())
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .when(!deleted, |element| {
                                    element
                                        .cursor_pointer()
                                        .hover(|element| {
                                            element.bg(theme::with_alpha(
                                                theme::sidebar_selected(),
                                                0.6,
                                            ))
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.viewer.open_in_open = !this.viewer.open_in_open;
                                            this.viewer.list_open = false;
                                            cx.notify();
                                        }))
                                })
                                .child(
                                    svg()
                                        .size(px(11.))
                                        .path("chevron-down.svg")
                                        .text_color(theme::muted()),
                                ),
                        ),
                )
            })
            .child(
                strip_button("file-tree-toggle", !self.viewer.tree_hidden)
                    .tooltip(|_, cx| {
                        runner_app::ui::tooltip::tooltip_view(
                            crate::i18n::t("ui.toggle_file_tree"),
                            cx,
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.viewer.tree_hidden = !this.viewer.tree_hidden;
                        cx.notify();
                    }))
                    .child(svg().size(px(14.)).path("list-tree.svg").text_color(
                        if self.viewer.tree_hidden {
                            theme::muted()
                        } else {
                            theme::text()
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_placeholder(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: String,
        body: String,
        action: AnyElement,
    ) -> AnyElement {
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(move || selector.into())
            })
            .flex_1()
            .min_h(px(0.))
            .px(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .text_center()
            .child(svg().size(px(28.)).path(icon).text_color(theme::faint()))
            .child(
                div()
                    .pt(px(8.))
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .map(|element| {
                        #[cfg(test)]
                        let element =
                            crate::theme_snapshot::record_text("FILE_STATE_TITLE", element);
                        element
                    })
                    .child(title),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(body),
            )
            .child(div().pt(px(12.)).child(action))
            .into_any_element()
    }

    fn search_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("file-empty-search")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_EMPTY_SEARCH".into())
            })
            .flex_none()
            .px(px(10.))
            .py(px(5.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_md()
            .border_1()
            .border_color(theme::border())
            .cursor_pointer()
            .text_size(theme::text_ui())
            .text_color(theme::text())
            .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_finder(window, cx)))
            .child(
                svg()
                    .size(px(13.))
                    .path("search.svg")
                    .text_color(theme::muted()),
            )
            .child(crate::i18n::text("ui.search_files"))
            .children(super::super::finder::search_key_hint(cx))
            .into_any_element()
    }

    /// Where the file goes before one is chosen: the checkout's changes, one
    /// click from opening, or a prompt when there are none to list.
    fn render_no_file(&self, cx: &mut Context<Self>) -> AnyElement {
        let reading =
            self.root_is_git() && self.tree_ref().is_none_or(|tree| tree.status().is_none());
        if reading {
            return div().flex_1().into_any_element();
        }
        let changed = self
            .tree_ref()
            .map(|tree| tree.changed_files())
            .unwrap_or_default();
        // More than listed when the status ran past its limits.
        let total = self
            .tree_ref()
            .and_then(|tree| tree.status())
            .map_or(changed.len(), |status| status.changed.max(changed.len()));
        if changed.is_empty() {
            return self.render_placeholder(
                "FILE_NO_FILE",
                "file-text.svg",
                crate::i18n::t("ui.no_file_open").to_string(),
                crate::i18n::t("ui.choose_a_file_or_search").to_string(),
                self.search_button(cx),
            );
        }
        let count = changed.len();
        let rows = changed;
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_CHANGED_LIST".into())
            })
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .px(px(16.))
            .pt(px(12.))
            .child(
                div()
                    .flex_none()
                    .pb(px(8.))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(crate::i18n::tn("count.changed", total)),
                    )
                    .child(self.search_button(cx)),
            )
            .child(
                gpui::uniform_list(
                    "file-changed-list",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        rows[range]
                            .iter()
                            .map(|(path, letter)| this.render_changed_row(path, *letter, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h(px(0.)),
            )
            .into_any_element()
    }

    fn render_changed_row(&self, path: &str, letter: Letter, cx: &mut Context<Self>) -> AnyElement {
        let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
        let open = path.to_owned();
        div()
            .id(gpui::SharedString::from(format!("file-changed-{path}")))
            .when(cfg!(test), |element| {
                let selector = format!("FILE_CHANGED {path}");
                element.debug_selector(move || selector)
            })
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_md()
            .cursor_pointer()
            .text_size(theme::text_ui())
            .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus.focus(window, cx);
                this.open_file(open.clone(), cx);
            }))
            .child(self.file_icon(name))
            .child(
                div()
                    .flex_none()
                    .max_w(gpui::relative(0.6))
                    .truncate()
                    .text_color(theme::text())
                    .child(name.to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(folder.to_owned()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_meta())
                    .text_color(letter_color(letter))
                    .child(letter.text()),
            )
            .into_any_element()
    }

    fn render_file(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.viewer.tabs.active() else {
            return self.render_no_file(cx);
        };
        let name = tab.name().to_owned();
        let Some(file) = self.viewer.files.get(&tab.path) else {
            return div().flex_1().into_any_element();
        };
        let root = cx.weak_entity();
        match &file.content {
            Content::Loading => div().flex_1().into_any_element(),
            Content::Failed(error) => div()
                .p_4()
                .text_size(theme::text_ui())
                .text_color(theme::danger())
                .child(error.clone())
                .into_any_element(),
            Content::Binary { size } => self.render_placeholder(
                "FILE_BINARY",
                "file.svg",
                name,
                crate::i18n::tf("format.binary_file_size", &[("size", format_size(*size))]),
                Button::new(
                    "file-open-default",
                    crate::i18n::text("ui.open_in_default_app"),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("FILE_OPEN_DEFAULT")
                .on_press(move |_, cx| {
                    let _ = root.update(cx, |this, cx| {
                        if let Some(path) = this.active_path() {
                            this.launch(Some(FileLinkEditor::DefaultApp), path, None, cx);
                        }
                    });
                })
                .into_any_element(),
            ),
            Content::TooLarge { size } => {
                let (_, effective) = found_editors(cx);
                self.render_placeholder(
                    "FILE_TOO_LARGE",
                    "triangle-alert.svg",
                    format!("{name} · {}", format_size(*size)),
                    crate::i18n::tf(
                        "format.files_up_to_open_in_the_viewer",
                        &[(
                            "size",
                            format_size(runner_core::protocol::files::MAX_READ_BYTES),
                        )],
                    ),
                    Button::new(
                        "file-open-large",
                        crate::i18n::tf(
                            "format.open_in_editor",
                            &[("editor", effective.label().to_owned())],
                        ),
                    )
                    .variant(ButtonVariant::Secondary)
                    .debug_selector("FILE_OPEN_LARGE")
                    .on_press(move |_, cx| {
                        let _ = root.update(cx, |this, cx| this.open_in(None, cx));
                    })
                    .into_any_element(),
                )
            }
            Content::Text(_) if self.shows_rendered(file) => {
                self.render_rendered(file.rendered.clone(), None)
            }
            Content::Text(view) => self.render_code(view.clone(), None, window, cx),
            Content::Deleted(view) => {
                let close_root = cx.weak_entity();
                let banner = div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILE_DELETED".into())
                    })
                    .flex_none()
                    .h(px(34.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .text_size(theme::text_ui())
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path("file-x.svg")
                            .text_color(theme::danger()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(theme::text())
                            .map(|element| {
                                #[cfg(test)]
                                let element = crate::theme_snapshot::record_text(
                                    "FILE_DELETED_TEXT",
                                    element,
                                );
                                element
                            })
                            .child(crate::i18n::tf(
                                "format.file_was_deleted",
                                &[("name", name.clone())],
                            )),
                    )
                    .child(
                        div()
                            .id("file-deleted-close")
                            .when(cfg!(test), |element| {
                                element.debug_selector(|| "FILE_DELETED_CLOSE".into())
                            })
                            .flex_none()
                            .px(px(6.))
                            .rounded_sm()
                            .cursor_pointer()
                            .text_color(theme::muted())
                            .hover(|element| element.text_color(theme::text()))
                            .on_click(move |_, window, cx| {
                                let _ = close_root.update(cx, |this, cx| {
                                    if let Some(index) = this.viewer.tabs.active_index() {
                                        this.close_tab(index, window, cx);
                                    }
                                });
                            })
                            .child(crate::i18n::text("ui.close_file_tab")),
                    )
                    .into_any_element();
                match view {
                    Some(_) if self.shows_rendered(file) => {
                        self.render_rendered(file.rendered.clone(), Some(banner))
                    }
                    Some(view) => self.render_code(view.clone(), Some(banner), window, cx),
                    None => div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(banner)
                        .into_any_element(),
                }
            }
        }
    }

    fn render_rendered(
        &self,
        page: Option<Entity<MarkdownView>>,
        banner: Option<AnyElement>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .children(banner)
            .child(div().flex_1().min_h(px(0.)).children(page))
            .into_any_element()
    }

    fn render_tab_list(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.viewer.list_open {
            return None;
        }
        let active = self.viewer.tabs.active_index();
        let rows = self
            .viewer
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let group: SharedString = format!("file-list-{index}").into();
                let letter = self.tab_letter(tab);
                div()
                    .id(("file-list-item", index))
                    .when(cfg!(test), |element| {
                        let path = tab.path.clone();
                        element.debug_selector(move || format!("FILE_LIST_ITEM {path}"))
                    })
                    .group(group.clone())
                    .px(px(8.))
                    .py(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(active == Some(index), |element| {
                        element.bg(theme::sidebar_selected())
                    })
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.viewer.list_open = false;
                        this.activate_tab(index, cx);
                        this.keep_focus(window, cx);
                    }))
                    .child(self.file_icon(tab.name()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .child(tab.name().to_owned()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(tab.folder().to_owned()),
                    )
                    .children(letter.map(|(text, color)| {
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(color)
                            .child(text)
                    }))
                    .child(
                        div()
                            .id(("file-list-close", index))
                            .flex_none()
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .invisible()
                            .group_hover(group, |element| element.visible())
                            .hover(|element| element.bg(theme::sidebar_selected()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(index, window, cx);
                            }))
                            .child(
                                svg()
                                    .size(px(11.))
                                    .path("close.svg")
                                    .text_color(theme::muted()),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_TAB_LIST_POPUP".into())
            })
            .id("file-tab-list-popup")
            .max_h(px(400.))
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(8.))
                    .pt(px(2.))
                    .pb(px(4.))
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::tn(
                        "count.open_files",
                        self.viewer.tabs.tabs().len(),
                    )),
            )
            .children(rows)
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_sized(
            self.viewer.list_anchor.get(),
            window,
            PopupWidth::Fixed(px(LIST_WIDTH)),
            content,
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    this.viewer.list_open = false;
                    cx.notify();
                })
            }),
        ))
    }

    /// The viewer's column: the strip, then the active file.
    pub(in super::super) fn render_viewer(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_VIEWER".into())
            })
            .track_focus(&self.viewer_focus)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_strip(cx))
            .child(self.render_file(window, cx))
            .into_any_element()
    }

    pub(in super::super) fn render_viewer_popups(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.render_tab_list(window, cx)
            .into_iter()
            .chain(self.render_open_in_menu(window, cx))
            .collect()
    }
}

//! Unified viewer rendering (#1421): aligned 40 px viewer header (unified tab
//! strip + `Code | Diff` + `Pin` + rail toggle + `Maximize` + fallback
//! `Collapse`), 36 px context subbar (Diff mode & Code/Markdown mode), and
//! document body.

use super::*;
use crate::file_icons::{self, IconSet};
use crate::surfaces::changes::{
    render_diff_viewed_footer, render_shared_diff_header, render_shared_diff_placeholder,
    DiffHeaderProps, DiffModeCallback, PlaceholderViewedFooter,
};

const LIST_WIDTH: f32 = 300.;
const TAB_HEIGHT: f32 = 28.;

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
        .border_color(if active {
            theme::border()
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .text_size(theme::text_meta())
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .when(active, |element| element.bg(theme::raised()))
        .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl UnifiedViewer {
    fn file_icon(&self, name: &str) -> AnyElement {
        let dark = !theme::active_variant().is_light();
        let set = self
            .look
            .as_ref()
            .map_or_else(IconSet::default, |look| look.icons);
        match file_icons::file_icon(set, dark, name) {
            Some(icon) if icon.monochrome => svg()
                .flex_none()
                .size(px(13.))
                .path(icon.path)
                .text_color(theme::muted())
                .into_any_element(),
            Some(icon) => img(ImageSource::Resource(Resource::Embedded(icon.path)))
                .flex_none()
                .size(px(13.))
                .into_any_element(),
            None => svg()
                .flex_none()
                .size(px(13.))
                .path("file-text.svg")
                .text_color(theme::muted())
                .into_any_element(),
        }
    }

    fn tab_letter(&self, tab: &FileTab) -> Option<(&'static str, gpui::Hsla)> {
        if let Some(doc) = self.docs.get(&tab.key) {
            if matches!(doc.content, Content::Deleted(_)) {
                return Some(("D", theme::danger()));
            }
            if let FileTabDiff::Loaded(loaded) = &doc.diff {
                return Some(status_letter_and_color(&loaded.status));
            }
            if let Some(status) = &doc.hint_status {
                return Some(status_letter_and_color(status));
            }
        }
        if tab.key.target != SelectedTarget::Uncommitted {
            return None;
        }
        let status = self.status.as_ref()?;
        let letter = status.letter(tab.path())?;
        Some((letter.text(), letter_color(letter)))
    }

    fn tab_target_tag(&self, tab: &FileTab) -> Option<String> {
        if !self.tabs.has_duplicate_path(tab.path()) {
            return None;
        }
        match &tab.key.target {
            SelectedTarget::Uncommitted => None,
            SelectedTarget::Branch => Some(crate::i18n::text("viewer.tag_branch").to_owned()),
            SelectedTarget::Commit(id) => Some(
                self.docs
                    .get(&tab.key)
                    .and_then(|d| d.commit_short_id.clone())
                    .unwrap_or_else(|| id[..id.len().min(7)].to_owned()),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn active_diff_scope_label(&self) -> Option<String> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        Some(self.diff_scope_label(&tab.key, doc))
    }

    fn diff_scope_label(&self, key: &TabKey, doc: &OpenDocument) -> String {
        let has_head = match &doc.diff {
            FileTabDiff::Loaded(loaded) => loaded.has_head,
            _ => doc.has_head || self.root_has_head(),
        };
        match &key.target {
            SelectedTarget::Uncommitted => crate::i18n::text(if has_head {
                "changes.working_tree_vs_head"
            } else {
                "changes.working_tree_unborn"
            })
            .to_owned(),
            SelectedTarget::Branch => {
                let base_raw = doc.diff_base.as_deref().or(doc.resolved_base.as_deref());
                let branch = if doc.branch_known {
                    doc.branch_name.as_deref()
                } else {
                    doc.branch_name.as_deref().or_else(|| {
                        self.root
                            .as_ref()
                            .and_then(|r| r.git.as_ref())
                            .and_then(|g| g.branch.as_deref())
                    })
                };
                match (branch, base_raw) {
                    (Some(branch), Some(base)) => crate::i18n::tf(
                        "viewer.all_changes_vs_base",
                        &[
                            ("branch", branch.to_owned()),
                            ("base", display_base_ref(base).to_owned()),
                        ],
                    ),
                    _ => crate::i18n::text("changes.all_changes").to_owned(),
                }
            }
            SelectedTarget::Commit(id) => {
                let short = doc
                    .commit_short_id
                    .clone()
                    .unwrap_or_else(|| id[..id.len().min(7)].to_owned());
                crate::i18n::tf("viewer.commit_vs_parent", &[("sha", short)])
            }
        }
    }

    fn render_tab(&self, index: usize, tab: &FileTab, cx: &mut Context<Self>) -> AnyElement {
        let active = self.tabs.active_index() == Some(index);
        let group: SharedString = format!("file-tab-{index}").into();
        let path = tab.path().to_owned();
        let letter = self.tab_letter(tab);
        let preview = tab.preview;
        let is_diff = self
            .docs
            .get(&tab.key)
            .is_some_and(|doc| doc.mode == DocMode::Diff);
        let target_tag = self.tab_target_tag(tab);
        let leading_icon = if is_diff {
            let color = letter.map_or_else(theme::warning, |(_, c)| c);
            svg()
                .flex_none()
                .size(px(12.))
                .path("git-compare.svg")
                .text_color(color)
                .into_any_element()
        } else {
            self.file_icon(tab.name())
        };
        let label = div()
            .whitespace_nowrap()
            .when(preview, |el| el.italic())
            .when(cfg!(test) && preview, {
                let path = path.clone();
                move |el| el.debug_selector(move || format!("FILE_TAB_PREVIEW {path}"))
            })
            .when(cfg!(test) && !preview, {
                let path = path.clone();
                move |el| el.debug_selector(move || format!("FILE_TAB_PINNED {path}"))
            })
            .child(tab.name().to_owned());
        div()
            .id(("file-tab", index))
            .when(cfg!(test), {
                let path = path.clone();
                move |element| element.debug_selector(move || format!("FILE_TAB {path}"))
            })
            .group(group.clone())
            .flex_none()
            .h(px(TAB_HEIGHT))
            .pl(px(8.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(5.))
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
            .when(preview, |el| {
                el.tooltip(|_, cx| {
                    runner_app::ui::tooltip::tooltip_view(
                        crate::i18n::t("viewer.preview_tab_hint"),
                        cx,
                    )
                })
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.activate_tab(index, cx);
                if event.click_count() >= 2 {
                    this.pin_tab(index, cx);
                }
                this.focus_active(window, cx);
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tab(index, window, cx)),
            )
            .child(leading_icon)
            .child(label)
            .children(target_tag.map(|tag| {
                div()
                    .flex_none()
                    .px(px(4.))
                    .rounded(px(3.))
                    .bg(theme::panel())
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_micro())
                    .text_color(theme::faint())
                    .child(tag)
            }))
            .children(letter.map(|(text, color)| {
                let path = path.clone();
                div()
                    .when(cfg!(test), |element| {
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
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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

    fn render_code_diff_switch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.active_can_diff() {
            return None;
        }
        let showing_diff = self.active_showing_diff();
        let deleted = self.active_deleted();
        let is_image = self.active_is_image();
        let code_active = !showing_diff && !deleted;
        let diff_active = showing_diff;
        let code_label_key = if is_image {
            "viewer.image"
        } else {
            "viewer.code"
        };
        let deleted_tip_key = if is_image {
            "viewer.file_deleted_image_unavailable"
        } else {
            "viewer.file_deleted_code_unavailable"
        };

        let code_btn = div()
            .id("viewer-mode-code")
            .when(cfg!(test), move |el| {
                el.debug_selector(move || {
                    if deleted {
                        "VIEWER_MODE_CODE disabled".into()
                    } else if code_active {
                        "VIEWER_MODE_CODE active".into()
                    } else {
                        "VIEWER_MODE_CODE".into()
                    }
                })
            })
            .h_full()
            .px(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .text_size(theme::text_meta())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(deleted, move |el| {
                el.opacity(0.4)
                    .text_color(theme::faint())
                    .cursor_default()
                    .tooltip(move |_, cx| {
                        runner_app::ui::tooltip::tooltip_view(crate::i18n::t(deleted_tip_key), cx)
                    })
            })
            .when(!deleted && code_active, |el| {
                el.bg(theme::sidebar_selected())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .cursor_pointer()
            })
            .when(!deleted && !code_active, |el| {
                el.text_color(theme::muted())
                    .cursor_pointer()
                    .hover(|h| h.text_color(theme::text()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_active_mode(DocMode::Code, window, cx);
                    }))
            })
            .child(crate::i18n::text(code_label_key));

        let diff_btn = div()
            .id("viewer-mode-diff")
            .when(cfg!(test), move |el| {
                el.debug_selector(move || {
                    if diff_active {
                        "VIEWER_MODE_DIFF active".into()
                    } else {
                        "VIEWER_MODE_DIFF".into()
                    }
                })
            })
            .h_full()
            .px(px(8.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_size(theme::text_meta())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(diff_active, |el| {
                el.bg(theme::sidebar_selected())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
            })
            .when(!diff_active, |el| {
                el.text_color(theme::muted())
                    .hover(|h| h.text_color(theme::text()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_active_mode(DocMode::Diff, window, cx);
                    }))
            })
            .child(crate::i18n::text("viewer.diff"));

        Some(
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "VIEWER_MODE_SWITCH".into())
                })
                .flex_none()
                .h(px(26.))
                .p(px(2.))
                .rounded(px(6.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .flex()
                .items_center()
                .gap(px(2.))
                .child(code_btn)
                .child(diff_btn)
                .into_any_element(),
        )
    }

    fn render_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| self.render_tab(index, tab, cx))
            .collect::<Vec<_>>();
        let offset = self.strip.offset().x;
        let max = self.strip.max_offset().x;
        let overflowing = max > px(0.5);
        // The fade covers only the tabs' band, so it stays clear of the card's rounded corners (#1440).
        let fade = |left: bool| {
            let bg = theme::bg();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(28.))
                .when(left, |element| element.left_0())
                .when(!left, |element| element.right_0())
                .flex()
                .items_center()
                .child(
                    div()
                        .when(cfg!(test), |element| {
                            element.debug_selector(move || {
                                if left {
                                    "FILE_TAB_FADE_LEFT".into()
                                } else {
                                    "FILE_TAB_FADE_RIGHT".into()
                                }
                            })
                        })
                        .w_full()
                        .h(px(TAB_HEIGHT))
                        .bg(linear_gradient(
                            if left { 90. } else { 270. },
                            linear_color_stop(bg, 0.),
                            linear_color_stop(theme::with_alpha(bg, 0.), 1.),
                        )),
                )
        };
        let count = self.tabs.tabs().len();
        let list_anchor = self.list_anchor.clone();
        let active_is_preview = self.tabs.active().is_some_and(|t| t.preview);
        let active_index = self.tabs.active_index();
        let show_rail_column = self.chrome.show_rail_column;
        let rail_toggle_tip = if self.tree_hidden || !show_rail_column {
            "viewer.show_rail"
        } else {
            "viewer.hide_rail"
        };
        let macos_inset = self.chrome.macos_titlebar_inset.take();
        let maximize_el = self.chrome.maximize_control.take().map(|control| {
            let settings = crate::app_store::global_app_store(cx)
                .read(cx)
                .settings
                .clone();
            side_panel::maximize_button(control, &settings)
        });
        let collapse_el = (!show_rail_column)
            .then(|| self.chrome.collapse_when_rail_hidden.take())
            .flatten();
        let code_diff_switch = self.render_code_diff_switch(cx);

        let row = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_TAB_STRIP".into())
            })
            .flex_none()
            .h(rems(40. / 16.))
            .pl(px(8.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(theme::border())
            .when_some(macos_inset, |header, (pad, spacer)| {
                header.pl(px(pad)).children(spacer)
            })
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
                            .track_scroll(&self.strip)
                            .children(tabs),
                    )
                    .when(offset < px(-0.5), |element| element.child(fade(true)))
                    .when(overflowing && -offset < max - px(0.5), |element| {
                        element.child(fade(false))
                    }),
            )
            .when(overflowing || self.list_open, |element| {
                element.child(
                    strip_button("file-tab-list", self.list_open)
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
                            this.list_open = !this.list_open;
                            this.open_in_open = false;
                            cx.notify();
                        })),
                )
            })
            .children(code_diff_switch)
            .when(active_is_preview, |element| {
                element.child(
                    strip_button("viewer-pin-tab", false)
                        .tooltip(|_, cx| {
                            runner_app::ui::tooltip::tooltip_view(
                                crate::i18n::t("viewer.pin_tab"),
                                cx,
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(index) = active_index {
                                this.pin_tab(index, cx);
                            }
                        }))
                        .child(
                            svg()
                                .size(px(13.))
                                .path("pin.svg")
                                .text_color(theme::muted()),
                        ),
                )
            })
            .child(
                strip_button("file-tree-toggle", !self.tree_hidden && show_rail_column)
                    .tooltip(move |_, cx| {
                        runner_app::ui::tooltip::tooltip_view(crate::i18n::t(rail_toggle_tip), cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tree_hidden = !this.tree_hidden;
                        this.notify_owner(cx);
                        cx.notify();
                    }))
                    .child(svg().size(px(14.)).path("list-tree.svg").text_color(
                        if self.tree_hidden || !show_rail_column {
                            theme::muted()
                        } else {
                            theme::text()
                        },
                    )),
            )
            .children(maximize_el)
            .children(collapse_el.map(|collapse| {
                div()
                    .flex_none()
                    .debug_selector(|| "PANEL_COLLAPSE".into())
                    .child(collapse)
            }));

        if let Some(wrap) = self.chrome.wrap_titlebar.clone() {
            wrap("unified-viewer-titlebar-drag", row, cx)
        } else {
            row.into_any_element()
        }
    }

    fn render_code_subbar(
        &self,
        tab: &FileTab,
        doc: &OpenDocument,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let deleted = doc.is_working_tree_deleted(&tab.key);
        let name = tab.name().to_owned();
        let image_meta = match &doc.content {
            Content::Image {
                width,
                height,
                size,
                ..
            }
            | Content::Deleted(Some(DeletedContent::Image {
                width,
                height,
                size,
                ..
            })) => Some(image::format_dimensions_and_size(*width, *height, *size)),
            Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_))) => doc
                .svg_meta
                .map(|(w, h, s)| image::format_dimensions_and_size(w, h, s)),
            _ => None,
        };
        let folder = if image_meta.is_some() || (self.active_is_image() && !doc.svg_invalid) {
            None
        } else {
            abbreviate_folder(tab.path())
        };
        let hunks = self.active_hunk_lines();
        let hunk_count = hunks.len();
        let show_hunk_badge = hunk_count > 0 && !deleted && self.active_image_canvas().is_none();
        let markdown_source = self.active_markdown();
        let can_find = self.active_view().is_some()
            || self.active_rendered().is_some()
            || self.shows_svg_rendered(doc);
        let find_shortcut = if cfg!(target_os = "macos") {
            "⌘F"
        } else {
            "Ctrl+F"
        };

        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "FILE_CODE_SUBBAR".into())
            })
            .flex_none()
            .h(px(34.))
            .pl(px(10.))
            .pr(px(8.))
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .overflow_x_hidden()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .min_w(px(36.))
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .min_w(px(20.))
                            .flex_shrink(1.)
                            .truncate()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(name),
                    )
                    .children(image_meta.map(|meta| {
                        let meta_for_selector = meta.clone();
                        div()
                            .when(cfg!(test), move |el| {
                                let meta = meta_for_selector.clone();
                                el.debug_selector(move || format!("FILE_IMAGE_META {meta}"))
                            })
                            .min_w(px(0.))
                            .flex_shrink(2.)
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(meta)
                    }))
                    .children(folder.map(|f| {
                        div()
                            .min_w(px(0.))
                            .flex_shrink(3.)
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(f)
                    }))
                    .when(show_hunk_badge, |el| {
                        el.child(
                            div()
                                .id("file-hunks-badge")
                                .when(cfg!(test), |d| {
                                    d.debug_selector(|| "FILE_HUNKS_BADGE".into())
                                })
                                .flex_none()
                                .h(px(20.))
                                .px(px(6.))
                                .rounded(px(4.))
                                .bg(theme::with_alpha(theme::warning(), 0.14))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .cursor_pointer()
                                .text_size(theme::text_caption())
                                .text_color(theme::warning())
                                .tooltip(|_, cx| {
                                    runner_app::ui::tooltip::tooltip_view(
                                        crate::i18n::t("viewer.jump_next_hunk"),
                                        cx,
                                    )
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.jump_hunk(true, cx);
                                }))
                                .child(crate::i18n::tn("viewer.changed_hunks", hunk_count))
                                .child(
                                    div()
                                        .font_family(theme::UI_MONOSPACE_FONT)
                                        .text_color(theme::with_alpha(theme::warning(), 0.8))
                                        .child("[ ]"),
                                ),
                        )
                    }),
            )
            .when_some(markdown_source, |el, source| {
                let pill = |id: &'static str,
                            selector: &'static str,
                            label_key: &'static str,
                            active: bool,
                            target_source: bool| {
                    div()
                        .id(id)
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || {
                                if active {
                                    format!("{selector} active")
                                } else {
                                    selector.into()
                                }
                            })
                        })
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .text_size(theme::text_caption())
                        .when(!deleted, |el| el.cursor_pointer())
                        .when(active, |el| {
                            el.bg(theme::raised())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                        })
                        .when(!active, |el| {
                            el.text_color(theme::muted())
                                .when(!deleted, |h| h.hover(|h| h.text_color(theme::text())))
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(!deleted && !active, |el| {
                            el.on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if this.active_markdown() != Some(target_source) {
                                    this.toggle_source(window, cx);
                                }
                            }))
                        })
                        .child(crate::i18n::text(label_key))
                };
                el.child(
                    div()
                        .id("file-markdown-source")
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "FILE_MARKDOWN_SOURCE".into())
                        })
                        .flex_none()
                        .p(px(2.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .when(deleted, |el| el.opacity(0.4))
                        .child(pill(
                            "file-md-rendered",
                            "FILE_MD_RENDERED",
                            "side_panel.rendered",
                            !source,
                            false,
                        ))
                        .child(pill(
                            "file-md-source",
                            "FILE_MD_SOURCE",
                            "side_panel.source",
                            source,
                            true,
                        )),
                )
            })
            .when(can_find, |el| {
                el.child(
                    div()
                        .id("file-find-button")
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "FILE_FIND_BUTTON".into())
                        })
                        .flex_none()
                        .h(px(24.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .text_size(theme::text_caption())
                        .text_color(theme::muted())
                        .hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                        .tooltip(|_, cx| {
                            runner_app::ui::tooltip::tooltip_view(
                                crate::i18n::t("viewer.find_in_file"),
                                cx,
                            )
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| this.open_find(window, cx)))
                        .child(
                            svg()
                                .size(px(11.))
                                .path("search.svg")
                                .text_color(theme::muted()),
                        )
                        .child(find_shortcut),
                )
            })
            .child(self.render_open_in_control(deleted, cx))
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

    fn render_file(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.tabs.active() else {
            return div().flex_1().into_any_element();
        };
        let key = tab.key.clone();
        let name = tab.name().to_owned();
        let Some(doc) = self.docs.get(&key) else {
            return div().flex_1().into_any_element();
        };
        if doc.mode == DocMode::Diff && self.active_can_diff() {
            return self.render_tab_diff(&key, doc, cx);
        }
        let subbar = self.render_code_subbar(tab, doc, cx);
        let root = cx.weak_entity();
        let body = match &doc.content {
            Content::Idle | Content::Loading => div().flex_1().into_any_element(),
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
            Content::Image { canvas, .. } => canvas.clone().into_any_element(),
            Content::ImageTooLarge {
                width,
                height,
                size,
            } => {
                let (_, effective) = found_editors(cx);
                self.render_placeholder(
                    "FILE_IMAGE_TOO_LARGE",
                    "triangle-alert.svg",
                    format!(
                        "{name} · {}",
                        image::format_dimensions_and_size(*width, *height, *size)
                    ),
                    crate::i18n::text("viewer.image_too_many_pixels").to_owned(),
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
            Content::ImageFailed => self.render_placeholder(
                "FILE_IMAGE_FAILED",
                "image-off.svg",
                crate::i18n::text("viewer.image_decode_failed_title").to_owned(),
                crate::i18n::tf("viewer.image_decode_failed_body", &[("name", name)]),
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
            Content::Text(_) if self.shows_markdown_rendered(doc) => {
                self.render_rendered(doc.rendered.clone(), None)
            }
            Content::Text(_) if self.shows_svg_rendered(doc) => {
                if let Some((width, height, size)) = doc.svg_too_large {
                    let (_, effective) = found_editors(cx);
                    self.render_placeholder(
                        "FILE_IMAGE_TOO_LARGE",
                        "triangle-alert.svg",
                        format!(
                            "{name} · {}",
                            image::format_dimensions_and_size(width, height, size)
                        ),
                        crate::i18n::text("viewer.image_too_many_pixels").to_owned(),
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
                } else {
                    self.render_svg_rendered(doc.svg_canvas.clone(), None)
                }
            }
            Content::Text(view) => self.render_code(view.clone(), None, window, cx),
            Content::Deleted(kept) => {
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
                                    if let Some(index) = this.tabs.active_index() {
                                        this.close_tab(index, window, cx);
                                    }
                                });
                            })
                            .child(crate::i18n::text("ui.close_file_tab")),
                    )
                    .into_any_element();
                match kept {
                    Some(DeletedContent::Text(_)) if self.shows_markdown_rendered(doc) => {
                        self.render_rendered(doc.rendered.clone(), Some(banner))
                    }
                    Some(DeletedContent::Text(_)) if self.shows_svg_rendered(doc) => {
                        if let Some((width, height, size)) = doc.svg_too_large {
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .min_h(px(0.))
                                .flex()
                                .flex_col()
                                .child(banner)
                                .child(self.render_placeholder(
                                    "FILE_IMAGE_TOO_LARGE",
                                    "triangle-alert.svg",
                                    format!(
                                        "{name} · {}",
                                        image::format_dimensions_and_size(width, height, size)
                                    ),
                                    crate::i18n::text("viewer.image_too_many_pixels").to_owned(),
                                    div().into_any_element(),
                                ))
                                .into_any_element()
                        } else {
                            self.render_svg_rendered(doc.svg_canvas.clone(), Some(banner))
                        }
                    }
                    Some(DeletedContent::Text(view)) => {
                        self.render_code(view.clone(), Some(banner), window, cx)
                    }
                    Some(DeletedContent::Image { canvas, .. }) => {
                        self.render_svg_rendered(Some(canvas.clone()), Some(banner))
                    }
                    None => div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(banner)
                        .into_any_element(),
                }
            }
        };
        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(subbar)
            .child(body)
            .into_any_element()
    }

    fn render_tab_diff(
        &self,
        key: &TabKey,
        doc: &OpenDocument,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let effective_mode = self.effective_diff_mode(cx);
        let (status_opt, mut counts_opt, mode_change, conflicted, viewed_tuple) = match &doc.diff {
            FileTabDiff::Loaded(loaded) => (
                Some(&loaded.status),
                Some(loaded.counts),
                loaded.mode_change.as_ref(),
                loaded.conflicted,
                Some((loaded.viewed, loaded.changed_since_viewed)),
            ),
            FileTabDiff::NoChanges => (None, None, None, false, None),
            _ => (
                doc.hint_status.as_ref(),
                doc.hint_counts,
                None,
                false,
                doc.hint_viewed,
            ),
        };
        let hide_bin_count = match &doc.diff {
            FileTabDiff::Loaded(loaded) => matches!(loaded.kind, LoadedDiffKind::Image { .. }),
            FileTabDiff::Idle | FileTabDiff::Loading => raster_image_format(&key.path).is_some(),
            _ => false,
        };
        if hide_bin_count && matches!(counts_opt, Some(DiffCounts::Binary)) {
            counts_opt = None;
        }
        let one_sided_image_diff = match &doc.diff {
            FileTabDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Image { one_sided, .. } => *one_sided,
                LoadedDiffKind::Svg {
                    source: false,
                    one_sided,
                    ..
                } => *one_sided,
                _ => false,
            },
            FileTabDiff::Idle | FileTabDiff::Loading => {
                markdown::image_format(&key.path).is_some()
                    && matches!(
                        status_opt,
                        Some(FileStatus::Added | FileStatus::Untracked | FileStatus::Deleted)
                    )
            }
            _ => false,
        };
        let svg_diff_source = match &doc.diff {
            FileTabDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Svg { source, .. } => Some(*source),
                _ => None,
            },
            _ => None,
        };
        let extra_control = svg_diff_source.map(|source| {
            let toggle_key = key.clone();
            let pill = |id: &'static str,
                        selector: &'static str,
                        label_key: &'static str,
                        active: bool,
                        target_source: bool| {
                let toggle_key = toggle_key.clone();
                div()
                    .id(id)
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || {
                            if active {
                                format!("{selector} active")
                            } else {
                                selector.into()
                            }
                        })
                    })
                    .h(px(22.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(theme::text_meta())
                    .when(active, |el| {
                        el.bg(theme::raised())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                    })
                    .when(!active, |el| {
                        el.text_color(theme::muted())
                            .hover(|h| h.text_color(theme::text()))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(!active, |el| {
                        el.on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.set_svg_diff_source(&toggle_key, target_source, window, cx);
                        }))
                    })
                    .child(crate::i18n::text(label_key))
            };
            div()
                .id("diff-svg-source")
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "DIFF_SVG_SOURCE".into())
                })
                .flex_none()
                .p(px(2.))
                .rounded(px(6.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .flex()
                .items_center()
                .gap(px(2.))
                .child(pill(
                    "diff-svg-rendered",
                    "DIFF_SVG_RENDERED",
                    "side_panel.rendered",
                    !source,
                    false,
                ))
                .child(pill(
                    "diff-svg-source-btn",
                    "DIFF_SVG_SOURCE_BTN",
                    "side_panel.source",
                    source,
                    true,
                ))
                .into_any_element()
        });
        let deleted = doc.is_working_tree_deleted(key);
        let weak = cx.weak_entity();
        let mode_handler: DiffModeCallback = Rc::new(move |mode, window, cx| {
            let _ = weak.update(cx, |this, cx| this.select_diff_mode(mode, window, cx));
        });
        let viewed = viewed_tuple.map(|(is_viewed, changed_since)| {
            let toggle_weak = cx.weak_entity();
            let toggle_key = key.clone();
            let on_toggle: crate::surfaces::changes::diff::DiffCallback = Rc::new(move |_, cx| {
                let toggle_key = toggle_key.clone();
                let _ =
                    toggle_weak.update(cx, |this, cx| this.toggle_viewed_for_tab(&toggle_key, cx));
            });
            (is_viewed, changed_since, on_toggle)
        });
        let compare_label = Some(self.diff_scope_label(key, doc));
        let open_in_control = Some(self.render_open_in_control(deleted, cx));

        let header = render_shared_diff_header(DiffHeaderProps {
            path: Some(key.path.as_str()),
            status: status_opt,
            counts: counts_opt,
            mode_change,
            conflicted,
            compare_label,
            extra_control,
            mode_switch: (!one_sided_image_diff).then_some((effective_mode, mode_handler)),
            viewed,
            open_external: None,
            open_in_control,
            on_close: None,
        });

        let body = match &doc.diff {
            FileTabDiff::Idle | FileTabDiff::Loading => div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "FILE_DIFF_LOADING".into())
                })
                .flex_1()
                .min_h(px(0.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("changes.loading"))
                .into_any_element(),
            FileTabDiff::NoChanges => {
                let message = match &key.target {
                    SelectedTarget::Branch => {
                        let base = doc
                            .diff_base
                            .as_deref()
                            .or(doc.resolved_base.as_deref())
                            .map(display_base_ref)
                            .unwrap_or("base");
                        crate::i18n::tf(
                            "changes.no_changes_since_base",
                            &[("base", base.to_owned())],
                        )
                    }
                    SelectedTarget::Commit(id) => {
                        let sha = doc
                            .commit_short_id
                            .clone()
                            .unwrap_or_else(|| id[..id.len().min(7)].to_owned());
                        crate::i18n::tf("changes.no_changes_in_commit", &[("sha", sha)])
                    }
                    SelectedTarget::Uncommitted => crate::i18n::text("ui.empty").to_owned(),
                };
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "FILE_DIFF_NO_CHANGES".into())
                    })
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(message)
                    .into_any_element()
            }
            FileTabDiff::Failed(error) => div()
                .flex_1()
                .p_4()
                .text_size(theme::text_ui())
                .text_color(theme::danger())
                .child(error.clone())
                .into_any_element(),
            FileTabDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Unified(entity) => {
                    entity.update(cx, |v, cx| v.set_mode(effective_mode, cx));
                    entity.clone().into_any_element()
                }
                LoadedDiffKind::Image { canvas, .. } => {
                    canvas.update(cx, |c, cx| c.set_diff_mode(effective_mode, cx));
                    let toggle_entity = cx.entity();
                    let toggle_key = key.clone();
                    let footer = render_diff_viewed_footer(
                        &loaded.path,
                        PlaceholderViewedFooter {
                            viewed: loaded.viewed,
                            changed_since_viewed: loaded.changed_since_viewed,
                            on_toggle: Rc::new(move |_, cx| {
                                let toggle_key = toggle_key.clone();
                                toggle_entity.update(cx, |this, cx| {
                                    this.toggle_viewed_for_tab(&toggle_key, cx);
                                });
                            }),
                        },
                    );
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .child(canvas.clone())
                        .child(footer)
                        .into_any_element()
                }
                LoadedDiffKind::Svg {
                    canvas,
                    unified,
                    source,
                    ..
                } => {
                    if *source {
                        unified.update(cx, |v, cx| v.set_mode(effective_mode, cx));
                        unified.clone().into_any_element()
                    } else {
                        canvas.update(cx, |c, cx| c.set_diff_mode(effective_mode, cx));
                        let toggle_entity = cx.entity();
                        let toggle_key = key.clone();
                        let footer = render_diff_viewed_footer(
                            &loaded.path,
                            PlaceholderViewedFooter {
                                viewed: loaded.viewed,
                                changed_since_viewed: loaded.changed_since_viewed,
                                on_toggle: Rc::new(move |_, cx| {
                                    let toggle_key = toggle_key.clone();
                                    toggle_entity.update(cx, |this, cx| {
                                        this.toggle_viewed_for_tab(&toggle_key, cx);
                                    });
                                }),
                            },
                        );
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .child(canvas.clone())
                            .child(footer)
                            .into_any_element()
                    }
                }
                _ => {
                    let Some(root_path) = self.root_path() else {
                        return div().flex_1().into_any_element();
                    };
                    let open_entity = cx.entity();
                    let toggle_entity = cx.entity();
                    let toggle_key = key.clone();
                    render_shared_diff_placeholder(
                        root_path,
                        &loaded.path,
                        &loaded.status,
                        &loaded.kind,
                        Rc::new(move |editor, path, cx| {
                            open_entity.update(cx, |this, cx| {
                                this.launch(editor, path, None, cx);
                            });
                        }),
                        Some(PlaceholderViewedFooter {
                            viewed: loaded.viewed,
                            changed_since_viewed: loaded.changed_since_viewed,
                            on_toggle: Rc::new(move |_, cx| {
                                let toggle_key = toggle_key.clone();
                                toggle_entity.update(cx, |this, cx| {
                                    this.toggle_viewed_for_tab(&toggle_key, cx);
                                });
                            }),
                        }),
                        cx,
                    )
                }
            },
        };

        div()
            .id("changes-diff-column")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_DIFF_COLUMN".into())
            })
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
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

    fn render_svg_rendered(
        &self,
        canvas: Option<Entity<ImageCanvas>>,
        banner: Option<AnyElement>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .children(banner)
            .children(canvas)
            .into_any_element()
    }

    fn render_tab_list(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.list_open {
            return None;
        }
        let active = self.tabs.active_index();
        let rows = self
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let group: SharedString = format!("file-list-{index}").into();
                let letter = self.tab_letter(tab);
                let preview = tab.preview;
                let path = tab.path().to_owned();
                div()
                    .id(("file-list-item", index))
                    .when(cfg!(test), |element| {
                        let path = path.clone();
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
                        this.list_open = false;
                        this.activate_tab(index, cx);
                        this.focus_active(window, cx);
                    }))
                    .child(self.file_icon(tab.name()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .when(preview, |el| el.italic())
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
                    .child(crate::i18n::tn("count.open_files", self.tabs.tabs().len())),
            )
            .children(rows)
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_sized(
            self.list_anchor.get(),
            window,
            PopupWidth::Fixed(px(LIST_WIDTH)),
            content,
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    this.list_open = false;
                    cx.notify();
                })
            }),
        ))
    }

    fn render_viewer_popups(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let hidden_rail_finder = (!self.chrome.show_rail_column)
            .then(|| {
                self.chrome
                    .finder_host
                    .as_ref()
                    .and_then(gpui::WeakEntity::upgrade)
                    .and_then(|files| files.update(cx, |f, cx| f.render_finder(window, cx)))
            })
            .flatten();
        self.render_tab_list(window, cx)
            .into_iter()
            .chain(self.render_open_in_menu(window, cx))
            .chain(hidden_rail_finder)
            .collect()
    }
}

impl Render for UnifiedViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reported = (self.widening(), self.has_tabs(), self.has_dirty_changes());
        if reported != self.reported {
            self.reported = reported;
            self.notify_owner(cx);
        }
        if self._focus_lost.is_none() {
            self._focus_lost = Some(cx.on_focus_lost(window, Self::on_focus_lost));
        }
        if !self.chrome.show_rail_column
            && self.has_tabs()
            && self
                .chrome
                .hidden_rail_focus
                .as_ref()
                .is_some_and(|rail| rail.contains_focused(window, cx))
        {
            self.focus_active(window, cx);
        }
        // Promote placeholder focus once an async Diff/Code/Markdown load completes,
        // strictly while focus is still waiting on the viewer wrapper itself.
        if self.viewer_focus.is_focused(window) {
            if self.active_showing_diff() {
                if let Some(diff) = self.active_diff_view() {
                    diff.read(cx).focus_handle().clone().focus(window, cx);
                } else if let Some(canvas) = self.active_image_canvas() {
                    canvas.read(cx).focus_handle().clone().focus(window, cx);
                }
            } else if let Some(page) = self.active_rendered() {
                page.read(cx).focus_handle().clone().focus(window, cx);
            } else if let Some(code) = self.active_view() {
                code.read(cx).focus_handle().clone().focus(window, cx);
            } else if let Some(canvas) = self.active_image_canvas() {
                canvas.read(cx).focus_handle().clone().focus(window, cx);
            }
        }
        let strip = self.render_strip(cx);
        let file = self.render_file(window, cx);
        let popups = self.render_viewer_popups(window, cx);
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_VIEWER".into())
            })
            .key_context("FileViewer")
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|this, _: &crate::CloseFileTab, window, cx| {
                match this.tabs.active_index() {
                    Some(index) => this.close_tab(index, window, cx),
                    None => cx.propagate(),
                }
            }))
            .on_action(cx.listener(|this, _: &crate::NextFileTab, window, cx| {
                this.cycle_tab(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::PreviousFileTab, window, cx| {
                this.cycle_tab(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::FindInFile, window, cx| this.open_find(window, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::ZoomFileIn, window, cx| {
                this.zoom(1, Box::new(crate::ZoomIn), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomFileOut, window, cx| {
                this.zoom(-1, Box::new(crate::ZoomOut), window, cx)
            }))
            .on_key_down(cx.listener(Self::on_viewer_key))
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(strip)
            .child(
                div()
                    .track_focus(&self.viewer_focus)
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .pb(rems(8. / 16.))
                    .flex()
                    .flex_col()
                    .child(file),
            )
            .children(popups)
    }
}

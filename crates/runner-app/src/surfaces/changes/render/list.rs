use super::*;

impl ChangesView {
    // ── List column (Commits + Files) ──────────────────────────────────

    pub(super) fn render_list_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list_focused = self.list_focus.is_focused(window);
        let commits_focused = list_focused && self.focus_place == FocusPlace::Commits;
        let files_focused = list_focused && self.focus_place == FocusPlace::Files;
        let tree_view = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        let viewing = self.viewer.read(cx).has_tabs();

        let Body::Root(state) = &self.body else {
            return div().into_any_element();
        };

        let branch_header = self.render_branch_header(state, window, cx);
        let following_bar = self.context.following.clone().map(|following| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_FOLLOWING_BADGE".into())
                })
                .flex_none()
                .px(px(10.))
                .py(px(4.))
                .bg(theme::with_alpha(theme::raised(), 0.45))
                .border_b_1()
                .border_color(theme::border())
                .text_size(theme::text_caption())
                .text_color(theme::muted())
                .truncate()
                .child(following)
        });
        let base_row = self.render_base_row(state, window, cx);
        let commits_section = self.render_commits_section(state, commits_focused, cx);
        let summary_row = self.render_files_summary(state, tree_view, cx);
        let row_count = state.list_rows.len();

        let files_list = uniform_list(
            "changes-files-list",
            row_count,
            cx.processor(move |this, range: Range<usize>, window, cx| {
                let list_focused =
                    this.list_focus.is_focused(window) && this.focus_place == FocusPlace::Files;
                let Body::Root(state) = &this.body else {
                    return Vec::new();
                };
                let rows: Vec<ListRow> = state.list_rows[range.clone()].to_vec();
                rows.into_iter()
                    .zip(range)
                    .map(|(row, idx)| this.render_list_row(idx, row, list_focused, cx))
                    .collect()
            }),
        )
        .track_scroll(&self.files_scroll)
        .py(px(4.))
        .size_full();

        let error_banner = self.error.clone().map(|err| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_ERROR".into())
                })
                .px(px(10.))
                .py(px(6.))
                .text_size(theme::text_meta())
                .text_color(theme::danger())
                .child(err)
        });

        div()
            .id("changes-list-column")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_LIST_COLUMN".into())
            })
            .track_focus(&self.list_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(Self::on_list_key))
            .flex_1()
            .min_w(px(0.))
            .bg(if viewing { theme::panel() } else { theme::bg() })
            .h_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(branch_header)
            .children(following_bar)
            .child(base_row)
            .children(error_banner)
            .child(commits_section)
            .child(summary_row)
            .child(
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(move || {
                            if files_focused {
                                "CHANGES_FILES_LIST focused".into()
                            } else {
                                "CHANGES_FILES_LIST".into()
                            }
                        })
                    })
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(files_list)
                    .child(self.files_scrollbar.clone()),
            )
            .into_any_element()
    }

    fn render_commits_section(
        &self,
        state: &RootChanges,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(commits) = &state.commits else {
            return div()
                .flex_none()
                .h(px(34.))
                .border_b_1()
                .border_color(theme::border())
                .into_any_element();
        };
        let rows = build_commit_rows(commits, state.branch_count, state.uncommitted_count);
        let hidden_older = older_commits_hidden(commits);
        let rendered_rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let target = row.target();
                let selected = target == state.selected_target;
                let click_target = target.clone();
                let (selector, leading, title, count, tooltip_text) = match row {
                    CommitRow::AllChanges { count } => (
                        "CHANGES_COMMIT_ALL".to_owned(),
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path("git-pull-request.svg")
                            .text_color(if selected {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .into_any_element(),
                        crate::i18n::text("changes.all_changes").to_owned(),
                        Some(count),
                        None,
                    ),
                    CommitRow::Uncommitted { count } => (
                        "CHANGES_COMMIT_UNCOMMITTED".to_owned(),
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path("circle-dot.svg")
                            .text_color(if count > 0 {
                                theme::warning()
                            } else {
                                theme::faint()
                            })
                            .into_any_element(),
                        crate::i18n::text("changes.uncommitted_changes").to_owned(),
                        Some(count),
                        None,
                    ),
                    CommitRow::Commit(summary) => {
                        let tip = format!(
                            "{} · {}",
                            summary.author,
                            summary.date.format("%Y-%m-%d %H:%M UTC")
                        );
                        let short = summary.short_id.clone();
                        (
                            format!("CHANGES_COMMIT {}", summary.short_id),
                            div()
                                .flex_none()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::faint())
                                .child(short)
                                .into_any_element(),
                            summary.subject,
                            None,
                            Some(tip),
                        )
                    }
                };

                div()
                    .id(("changes-commit-row", index))
                    .when(cfg!(test), move |el| {
                        let s = selector.clone();
                        el.debug_selector(move || s.clone())
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme::sidebar_selected()))
                    .when(selected && focused, |el| el.border_color(theme::faint()))
                    .when(!selected, |el| {
                        el.hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    })
                    .when_some(tooltip_text, |el, tip| {
                        el.tooltip(move |_, cx| tooltip_view(tip.clone(), cx))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_place = FocusPlace::Commits;
                        this.list_focus.focus(window, cx);
                        this.select_target(click_target.clone(), cx);
                    }))
                    .child(leading)
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(theme::text_ui())
                            .when(selected, |t| {
                                t.font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                            })
                            .when(!selected, |t| t.text_color(theme::muted()))
                            .child(title),
                    )
                    .children(count.map(|c| {
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(c.to_string())
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .id("changes-commits-scroll")
            .when(cfg!(test), |el| {
                el.debug_selector(move || {
                    if focused {
                        "CHANGES_COMMITS focused".into()
                    } else {
                        "CHANGES_COMMITS".into()
                    }
                })
            })
            .flex_none()
            .max_h(px(186.))
            .overflow_y_scroll()
            .track_scroll(&self.commits_scroll)
            .py(px(4.))
            .border_b_1()
            .border_color(theme::border())
            .flex()
            .flex_col()
            .children(rendered_rows)
            .when(hidden_older > 0, |el| {
                el.child(
                    div()
                        .when(cfg!(test), |d| {
                            d.debug_selector(|| "CHANGES_OLDER_COMMITS_NOTE".into())
                        })
                        .px(px(10.))
                        .py(px(4.))
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::tn(
                            "changes.older_commits_not_shown",
                            hidden_older,
                        )),
                )
            })
            .into_any_element()
    }

    fn render_files_summary(
        &self,
        state: &RootChanges,
        tree_view: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files_text = crate::i18n::tn("changes.files", state.totals.files);
        let summary_text = if state.totals.viewed > 0 {
            crate::i18n::tf(
                "changes.files_viewed",
                &[
                    ("files", files_text),
                    ("viewed", state.totals.viewed.to_string()),
                ],
            )
        } else {
            files_text
        };
        let show_counts = state.totals.files > 0
            && (state.totals.added > 0
                || state.totals.removed > 0
                || state
                    .files
                    .iter()
                    .any(|f| matches!(f.counts, DiffCounts::Lines { .. })));

        let mode_button = |id: &'static str,
                           selector: &'static str,
                           icon: &'static str,
                           tip_key: &'static str,
                           active: bool,
                           target_tree: bool,
                           cx: &mut Context<Self>| {
            div()
                .id(id)
                .when(cfg!(test), move |el| {
                    el.debug_selector(move || selector.into())
                })
                .flex_none()
                .size(px(22.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(active, |el| el.bg(theme::raised()))
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .tooltip(move |_, cx| tooltip_view(crate::i18n::t(tip_key), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.set_tree_view(target_tree, cx)))
                .child(svg().size(px(13.)).path(icon).text_color(if active {
                    theme::text()
                } else {
                    theme::faint()
                }))
        };

        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_FILES_SUMMARY".into())
            })
            .flex_none()
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(summary_text),
            )
            .when(show_counts, |el| {
                el.child(
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::accent())
                        .child(format!("+{}", state.totals.added)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::danger())
                        .child(format!("−{}", state.totals.removed)),
                )
            })
            .child(div().flex_1())
            .child(mode_button(
                "changes-mode-flat",
                "CHANGES_MODE_FLAT",
                "list.svg",
                "changes.flat_list",
                !tree_view,
                false,
                cx,
            ))
            .child(mode_button(
                "changes-mode-tree",
                "CHANGES_MODE_TREE",
                "list-tree.svg",
                "changes.tree_view",
                tree_view,
                true,
                cx,
            ))
            .into_any_element()
    }

    fn render_list_row(
        &self,
        row_index: usize,
        row: ListRow,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Body::Root(state) = &self.body else {
            return div().into_any_element();
        };
        let highlighted = row_index == state.highlighted_list_row;
        match row {
            ListRow::Folder {
                path,
                name,
                depth,
                expanded,
            } => {
                let folder_path = path.clone();
                let icon_set = files::settings(cx).map_or(IconSet::default(), |s| s.file_icons);
                let dark = !theme::active_variant().is_light();
                let folder_icon = match file_icons::folder_icon(icon_set, dark, &name, expanded) {
                    Some(icon) if icon.monochrome => svg()
                        .flex_none()
                        .size(px(14.))
                        .path(icon.path)
                        .text_color(theme::muted())
                        .into_any_element(),
                    Some(icon) => img(ImageSource::Resource(Resource::Embedded(icon.path)))
                        .flex_none()
                        .size(px(14.))
                        .into_any_element(),
                    None => svg()
                        .flex_none()
                        .size(px(14.))
                        .path("folder.svg")
                        .text_color(theme::muted())
                        .into_any_element(),
                };
                div()
                    .id(("changes-folder-row", row_index))
                    .when(cfg!(test), move |el| {
                        let p = path.clone();
                        el.debug_selector(move || format!("CHANGES_FOLDER {p}"))
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .pl(px(10. + depth as f32 * INDENT))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(highlighted && list_focused, |el| {
                        el.border_color(theme::faint())
                    })
                    .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_place = FocusPlace::Files;
                        this.list_focus.focus(window, cx);
                        if let Body::Root(state) = &mut this.body {
                            state.highlighted_list_row = row_index;
                        }
                        this.toggle_folder(&folder_path, cx);
                    }))
                    .child(
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path(if expanded {
                                "chevron-down.svg"
                            } else {
                                "chevron-right.svg"
                            })
                            .text_color(theme::faint()),
                    )
                    .child(folder_icon)
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .child(name),
                    )
                    .into_any_element()
            }
            ListRow::File {
                file_index,
                path,
                name,
                folder_hint,
                depth,
            } => {
                let Some(file) = state.files.get(file_index) else {
                    return div().into_any_element();
                };
                let selected = state.selected_path.as_deref() == Some(path.as_str());
                let viewed = file.viewed;
                let changed_since = file.changed_since_viewed;
                let (letter, letter_color) = status_letter_and_color(&file.status);
                let counts = file.counts;
                let tooltip_str = match &file.status {
                    FileStatus::Renamed { from } => crate::i18n::tf(
                        "changes.renamed_from",
                        &[("path", path.clone()), ("from", from.clone())],
                    ),
                    _ => path.clone(),
                };
                let hint = if changed_since {
                    Some((
                        crate::i18n::text("changes.changed_since_viewed").to_owned(),
                        theme::warning(),
                    ))
                } else {
                    folder_hint.map(|h| (h, theme::faint()))
                };

                let select_path = path.clone();
                let check_path = path.clone();
                let check_tip = if viewed {
                    "changes.viewed"
                } else {
                    "changes.mark_viewed"
                };

                let counts_el = match counts {
                    DiffCounts::Binary => div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child("bin")
                        .into_any_element(),
                    DiffCounts::Lines { added, removed } => div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .when(viewed, |el| el.opacity(0.55))
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::accent())
                                .child(format!("+{added}")),
                        )
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::danger())
                                .child(format!("−{removed}")),
                        )
                        .into_any_element(),
                };

                div()
                    .id(("changes-file-row", row_index))
                    .when(cfg!(test), {
                        let p = path.clone();
                        move |el| el.debug_selector(move || format!("CHANGES_ROW {p}"))
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .pl(px(10. + depth as f32 * INDENT))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme::sidebar_selected()))
                    .when(highlighted && list_focused, |el| {
                        el.border_color(theme::faint())
                    })
                    .when(!selected, |el| {
                        el.hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    })
                    .tooltip(move |_, cx| tooltip_view(tooltip_str.clone(), cx))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.focus_place = FocusPlace::Files;
                        this.list_focus.focus(window, cx);
                        this.select_file_by_path(select_path.clone(), event.click_count() >= 2, cx);
                    }))
                    .child(
                        div()
                            .id(("changes-row-viewed", row_index))
                            .when(cfg!(test), {
                                let p = path;
                                move |el| {
                                    el.debug_selector(move || format!("CHANGES_VIEWED_BOX {p}"))
                                }
                            })
                            .flex_none()
                            .p(px(2.))
                            .m(px(-2.))
                            .cursor_pointer()
                            .tooltip(move |_, cx| tooltip_view(crate::i18n::t(check_tip), cx))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_viewed_for_path(&check_path, cx);
                            }))
                            .child(render_viewed_checkbox(viewed, changed_since)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(10.))
                            .text_center()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(letter_color)
                            .child(letter),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .text_color(if viewed {
                                theme::faint()
                            } else if selected {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(name),
                    )
                    .children(hint.map(|(text, color)| {
                        div()
                            .min_w(px(0.))
                            .flex_shrink(1.)
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(color)
                            .child(text)
                    }))
                    .child(div().flex_1())
                    .child(counts_el)
                    .into_any_element()
            }
        }
    }
}

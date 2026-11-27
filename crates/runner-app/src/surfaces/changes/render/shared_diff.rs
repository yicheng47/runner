use super::*;

pub(crate) type DiffModeCallback = Rc<dyn Fn(DiffViewMode, &mut Window, &mut App)>;
pub(crate) type OpenExternalCallback = Rc<dyn Fn(Option<FileLinkEditor>, PathBuf, &mut App)>;

pub(crate) struct DiffHeaderProps<'a> {
    pub(crate) path: Option<&'a str>,
    pub(crate) status: Option<&'a FileStatus>,
    pub(crate) counts: Option<DiffCounts>,
    pub(crate) mode_change: Option<&'a ModeChange>,
    pub(crate) conflicted: bool,
    pub(crate) compare_label: Option<String>,
    pub(crate) extra_control: Option<AnyElement>,
    pub(crate) mode_switch: Option<(DiffViewMode, DiffModeCallback)>,
    pub(crate) viewed: Option<(bool, bool, super::diff::DiffCallback)>,
    pub(crate) open_external: Option<crate::surfaces::side_panel::PanelCallback>,
    pub(crate) open_in_control: Option<AnyElement>,
    pub(crate) on_close: Option<crate::surfaces::side_panel::PanelCallback>,
}

pub(crate) struct PlaceholderViewedFooter {
    pub(crate) viewed: bool,
    pub(crate) changed_since_viewed: bool,
    pub(crate) on_toggle: super::diff::DiffCallback,
}

pub(crate) fn render_shared_diff_header(props: DiffHeaderProps<'_>) -> AnyElement {
    let renamed_from = match props.status {
        Some(FileStatus::Renamed { from }) => Some(format!("{from} →")),
        _ => None,
    };
    let name = props.path.map(|p| file_name(p).to_owned());
    let folder = props.path.and_then(abbreviate_folder);
    let mode_badge = props.mode_change.map(|m| {
        crate::i18n::tf(
            "changes.mode_badge",
            &[("old", m.old.clone()), ("new", m.new.clone())],
        )
    });

    let mode_switch = props.mode_switch.map(|(active_mode, on_select)| {
        let pill = |id: &'static str,
                    selector: &'static str,
                    label_key: &'static str,
                    target: DiffViewMode,
                    on_select: DiffModeCallback| {
            let active = active_mode == target;
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
                .on_click(move |_, window, cx| on_select(target, window, cx))
                .child(crate::i18n::text(label_key))
        };

        div()
            .when(cfg!(test), |d| {
                d.debug_selector(|| "CHANGES_DIFF_MODE".into())
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
                "changes-diff-mode-unified",
                "CHANGES_DIFF_MODE_UNIFIED",
                "changes.unified",
                DiffViewMode::Unified,
                on_select.clone(),
            ))
            .child(pill(
                "changes-diff-mode-split",
                "CHANGES_DIFF_MODE_SPLIT",
                "changes.split",
                DiffViewMode::Split,
                on_select,
            ))
    });

    let left_group = div()
        .min_w(px(36.))
        .flex_grow(1.)
        .flex_shrink(1.)
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(6.))
        .children(renamed_from.map(|old| {
            div()
                .min_w(px(0.))
                .flex_shrink(3.)
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(old)
        }))
        .children(name.map(|n| {
            div()
                .min_w(px(20.))
                .flex_shrink(1.)
                .truncate()
                .text_size(theme::text_ui())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(n)
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
        .children(props.compare_label.map(|label| {
            div()
                .when(cfg!(test), |d| {
                    d.debug_selector(|| "CHANGES_DIFF_COMPARE".into())
                })
                .min_w(px(0.))
                .max_w(px(220.))
                .flex_shrink(2.)
                .truncate()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .text_size(theme::text_caption())
                .text_color(theme::muted())
                .child(label)
        }))
        .children(props.counts.map(|counts| {
            match counts {
                DiffCounts::Binary => div()
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_caption())
                    .text_color(theme::faint())
                    .child("bin")
                    .into_any_element(),
                DiffCounts::Lines { added, removed } => div()
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .flex()
                    .items_center()
                    .gap(px(5.))
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
            }
        }))
        .children(mode_badge.map(|badge| {
            div()
                .min_w(px(0.))
                .flex_shrink(2.)
                .truncate()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(theme::raised())
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::muted())
                .child(badge)
        }))
        .when(props.conflicted, |el| {
            el.child(
                div()
                    .when(cfg!(test), |d| {
                        d.debug_selector(|| "CHANGES_CONFLICTED_BADGE".into())
                    })
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .truncate()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(theme::with_alpha(theme::danger(), 0.14))
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::danger())
                    .child(crate::i18n::text("changes.conflicted")),
            )
        });

    div()
        .when(cfg!(test), |el| {
            el.debug_selector(|| "CHANGES_DIFF_HEADER".into())
        })
        .flex_none()
        .h(px(36.))
        .pl(px(10.))
        .pr(px(8.))
        .bg(theme::panel())
        .overflow_x_hidden()
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(theme::border())
        .child(left_group)
        .children(props.extra_control)
        .children(mode_switch)
        .children(props.viewed.map(|(viewed, changed_since, on_toggle)| {
            div()
                .id("changes-header-viewed")
                .when(cfg!(test), move |d| {
                    d.debug_selector(move || {
                        if viewed {
                            "CHANGES_HEADER_VIEWED checked".into()
                        } else {
                            "CHANGES_HEADER_VIEWED".into()
                        }
                    })
                })
                .flex_none()
                .h(px(24.))
                .px(px(7.))
                .rounded(px(5.))
                .border_1()
                .border_color(if viewed {
                    theme::with_alpha(theme::accent(), 0.45)
                } else {
                    theme::border()
                })
                .bg(if viewed {
                    theme::with_alpha(theme::accent(), 0.1)
                } else {
                    theme::bg()
                })
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::raised(), 0.8)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_toggle(window, cx))
                .child(render_viewed_checkbox(viewed, changed_since))
                .child(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(if viewed {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .child(crate::i18n::text("changes.viewed")),
                )
        }))
        .children(props.open_in_control)
        .children(props.open_external.map(|on_open| {
            div()
                .id("changes-header-open-in")
                .when(cfg!(test), |d| {
                    d.debug_selector(|| "CHANGES_HEADER_OPEN_IN".into())
                })
                .flex_none()
                .size(px(24.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .tooltip(|_, cx| tooltip_view(crate::i18n::t("ui.open_in_editor"), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_open(window, cx))
                .child(
                    svg()
                        .size(px(13.))
                        .path("external-link.svg")
                        .text_color(theme::muted()),
                )
        }))
        .children(props.on_close.map(|on_close| {
            div()
                .id("changes-close-diff")
                .when(cfg!(test), |d| {
                    d.debug_selector(|| "CHANGES_CLOSE_DIFF".into())
                })
                .flex_none()
                .size(px(24.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .tooltip(|_, cx| tooltip_view(crate::i18n::t("changes.close_diff"), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_close(window, cx))
                .child(
                    svg()
                        .size(px(13.))
                        .path("close.svg")
                        .text_color(theme::muted()),
                )
        }))
        .into_any_element()
}

pub(crate) fn render_shared_diff_placeholder(
    root_path: &Path,
    rel_path: &str,
    status: &FileStatus,
    kind: &LoadedDiffKind,
    on_open_external: OpenExternalCallback,
    viewed_footer: Option<PlaceholderViewedFooter>,
    cx: &App,
) -> AnyElement {
    let full_path = absolute(root_path, rel_path);
    let exists_on_disk = *status != FileStatus::Deleted && full_path.exists();

    let size_line = |old_size: Option<u64>, new_size: Option<u64>| -> String {
        match (old_size, new_size) {
            (Some(old), Some(new)) => crate::i18n::tf(
                "changes.binary_sizes",
                &[("old", format_size(old)), ("new", format_size(new))],
            ),
            (None, Some(new)) => {
                crate::i18n::tf("changes.binary_added_size", &[("size", format_size(new))])
            }
            (Some(old), None) => {
                crate::i18n::tf("changes.binary_deleted_size", &[("size", format_size(old))])
            }
            (None, None) => String::new(),
        }
    };

    let center = match kind {
        LoadedDiffKind::Unified(_) | LoadedDiffKind::Image { .. } | LoadedDiffKind::Svg { .. } => {
            div().into_any_element()
        }
        LoadedDiffKind::CrlfOnly => render_centered_message(
            "CHANGES_PLACEHOLDER_CRLF",
            "file-text.svg",
            crate::i18n::text("changes.only_line_endings").to_owned(),
            String::new(),
            None,
        ),
        LoadedDiffKind::ModeOnly { old_mode, new_mode } => render_centered_message(
            "CHANGES_PLACEHOLDER_MODE",
            "file-text.svg",
            crate::i18n::tf(
                "changes.mode_changed",
                &[("old", old_mode.clone()), ("new", new_mode.clone())],
            ),
            String::new(),
            None,
        ),
        LoadedDiffKind::Submodule {
            old_commit,
            new_commit,
        } => {
            let short = |s: &str| s[..s.len().min(7)].to_owned();
            let msg = match (old_commit.as_deref(), new_commit.as_deref()) {
                (Some(old), Some(new)) => crate::i18n::tf(
                    "changes.submodule_moved",
                    &[("old", short(old)), ("new", short(new))],
                ),
                (None, Some(new)) => {
                    crate::i18n::tf("changes.submodule_added", &[("new", short(new))])
                }
                (Some(old), None) => {
                    crate::i18n::tf("changes.submodule_removed", &[("old", short(old))])
                }
                (None, None) => String::new(),
            };
            render_centered_message(
                "CHANGES_PLACEHOLDER_SUBMODULE",
                "folder-git-2.svg",
                msg,
                String::new(),
                None,
            )
        }
        LoadedDiffKind::Binary { old_size, new_size } => {
            let target_path = full_path.clone();
            let open = on_open_external;
            let action = exists_on_disk.then(|| {
                Button::new(
                    "changes-open-default-app",
                    crate::i18n::text("changes.open_in_default_app"),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("CHANGES_OPEN_DEFAULT_APP")
                .on_press(move |_, cx| {
                    open(Some(FileLinkEditor::DefaultApp), target_path.clone(), cx);
                })
                .into_any_element()
            });
            render_centered_message(
                "CHANGES_PLACEHOLDER_BINARY",
                "file-text.svg",
                crate::i18n::text("changes.binary_file").to_owned(),
                size_line(*old_size, *new_size),
                action,
            )
        }
        LoadedDiffKind::TooLarge { old_size, new_size } => {
            let (_, effective_editor) = found_editors(cx);
            let target_path = full_path;
            let open = on_open_external;
            let action = exists_on_disk.then(|| {
                let label = crate::i18n::tf(
                    "format.open_in_editor",
                    &[("editor", effective_editor.label().to_owned())],
                );
                div()
                    .id("changes-open-large-in-editor")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "CHANGES_OPEN_IN_EDITOR".into())
                    })
                    .px(px(12.))
                    .h(px(28.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::raised())
                    .hover(|el| el.bg(theme::sidebar_selected()))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .on_click(move |_, _, cx| {
                        open(Some(effective_editor), target_path.clone(), cx);
                    })
                    .child(editor_icon(effective_editor, 14.))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .child(label),
                    )
                    .into_any_element()
            });
            let sizes = size_line(*old_size, *new_size);
            let limit_note = crate::i18n::text("changes.diffs_up_to_limit");
            let body_str = if sizes.is_empty() {
                limit_note.to_owned()
            } else {
                format!("{sizes}\n{limit_note}")
            };
            render_centered_message(
                "CHANGES_PLACEHOLDER_TOO_LARGE",
                "file-text.svg",
                crate::i18n::text("changes.file_too_large").to_owned(),
                body_str,
                action,
            )
        }
    };

    let footer = viewed_footer.map(|footer| render_diff_viewed_footer(rel_path, footer));

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(center)
        .children(footer)
        .into_any_element()
}

pub(crate) fn render_diff_viewed_footer(
    rel_path: &str,
    footer: PlaceholderViewedFooter,
) -> AnyElement {
    let mark_label = crate::i18n::tf(
        "changes.mark_file_viewed",
        &[("name", file_name(rel_path).to_owned())],
    );
    let on_toggle = footer.on_toggle;
    div()
        .id("changes-placeholder-mark-viewed")
        .when(cfg!(test), |el| {
            el.debug_selector(|| "CHANGES_DIFF_VIEWED".into())
        })
        .flex_none()
        .h(px(40.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_t_1()
        .border_color(theme::border())
        .cursor_pointer()
        .hover(|row| row.bg(theme::with_alpha(theme::sidebar_selected(), 0.35)))
        .on_click(move |_, window, cx| on_toggle(window, cx))
        .child(render_viewed_checkbox(
            footer.viewed,
            footer.changed_since_viewed,
        ))
        .child(
            div()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .child(mark_label),
        )
        .into_any_element()
}

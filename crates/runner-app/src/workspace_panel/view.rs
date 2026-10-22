use std::path::Path;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, uniform_list, AnyElement, Context, FontWeight, Render, SharedString, Window,
};
use runner_app::ui::{IconButton, IconButtonSize};
use runner_app::{i18n, theme};

use super::*;

impl WorkspacePanel {
    pub fn render_header(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reveal = (self.tabs.active, self.tabs.tabs.len(), width.to_bits());
        if self.revealed_tab != Some(reveal) {
            if let Some(index) = self
                .tabs
                .tabs
                .iter()
                .position(|tab| tab.id == self.tabs.active)
            {
                self.tabs_scroll.scroll_to_item(index);
                // The first prepaint has not populated the scroll viewport yet.
                let panel = cx.entity().downgrade();
                window.defer(cx, move |_, cx| {
                    let _ = panel.update(cx, |panel, cx| {
                        if panel.revealed_tab == Some(reveal) {
                            panel.tabs_scroll.scroll_to_item(index);
                            cx.notify();
                        }
                    });
                });
            }
            self.revealed_tab = Some(reveal);
        }
        let root = cx.entity();
        let new_root = root.clone();
        let expand_root = root.clone();
        let collapse_root = root.clone();
        let max_width = (f32::from(window.viewport_size().width) * 0.55
            / (f32::from(window.rem_size()) / 16.))
            .clamp(200., 960.);
        // Keep the selected tab's close button reachable beside the header controls.
        let controls_width = 16. + 3. * IconButtonSize::Sm.button_size() + 3. * 4.;
        let tab_width = (width - controls_width).clamp(78., 180.);
        let tabs = self
            .tabs
            .tabs
            .iter()
            .map(|tab| {
                let id = tab.id;
                let active = self.tabs.active == id;
                let title = tab_title(&tab.kind);
                let tooltip = match &tab.kind {
                    TabKind::File(path) => path.display().to_string(),
                    _ => title.clone(),
                };
                let select_root = root.clone();
                let close_root = root.clone();
                div()
                    .id(SharedString::from(format!("workspace-panel-tab-{id}")))
                    .debug_selector(move || format!("WORKSPACE_PANEL_TAB {id}"))
                    .h(rems(28. / 16.))
                    .max_w(rems(tab_width / 16.))
                    .min_w(rems(78. / 16.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(if active {
                        theme::raised()
                    } else {
                        gpui::transparent_black()
                    })
                    .text_size(theme::text_meta())
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::with_alpha(theme::text(), 0.05)))
                    .tooltip(move |_, cx| {
                        runner_app::ui::tooltip::tooltip_view(tooltip.clone(), cx)
                    })
                    .child(
                        svg()
                            .path(tab_icon(&tab.kind))
                            .size(rems(13. / 16.))
                            .text_color(if active {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .flex_none(),
                    )
                    .child(div().min_w(px(0.)).flex_1().truncate().child(title))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("close-panel-tab-{id}")),
                            "close.svg",
                        )
                        .size(IconButtonSize::Xs)
                        .stop_click_propagation(true)
                        .tooltip(i18n::text("panel.close_tab"))
                        .on_press(move |_, cx| {
                            close_root.update(cx, |this, cx| this.close(id, cx))
                        }),
                    )
                    .on_click(move |_, _, cx| {
                        select_root.update(cx, |this, cx| this.select(id, cx))
                    })
            })
            .collect::<Vec<_>>();
        div()
            .h(rems(crate::WORKSPACE_HEADER_HEIGHT / 16.))
            .flex_none()
            .min_w(px(0.))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("workspace-panel-tabs")
                    .debug_selector(|| "WORKSPACE_PANEL_TABS".into())
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll)
                    .children(tabs),
            )
            .child(
                IconButton::new("workspace-panel-new", "plus.svg")
                    .size(IconButtonSize::Sm)
                    .tooltip(i18n::text("panel.new_tab"))
                    .on_press(move |_, cx| {
                        new_root.update(cx, |this, cx| {
                            this.navigation_revision += 1;
                            this.tabs.open(TabKind::New);
                            cx.notify();
                        })
                    }),
            )
            .child(
                IconButton::new(
                    "workspace-panel-expand",
                    if self.normal_width.is_some() {
                        "columns-2.svg"
                    } else {
                        "arrow-up-right.svg"
                    },
                )
                .size(IconButtonSize::Sm)
                .tooltip(i18n::text(if self.normal_width.is_some() {
                    "panel.restore"
                } else {
                    "panel.expand"
                }))
                .on_press(move |_, cx| {
                    expand_root.update(cx, |this, cx| {
                        let next = if let Some(previous) = this.normal_width.take() {
                            previous
                        } else {
                            this.normal_width = Some(width);
                            720_f32.min(max_width).max(width.min(max_width))
                        };
                        cx.emit(PanelEvent::Width(next));
                        cx.notify();
                    })
                }),
            )
            .child(
                IconButton::new("workspace-panel-collapse", "panel-right-open.svg")
                    .size(IconButtonSize::Sm)
                    .tooltip(i18n::text("ui.collapse_side_panel"))
                    .on_press(move |_, cx| {
                        collapse_root.update(cx, |_, cx| cx.emit(PanelEvent::Collapse))
                    }),
            )
            .into_any_element()
    }

    fn render_home(&self, cx: &mut Context<Self>) -> AnyElement {
        let files_root = cx.entity();
        let terminal_root = cx.entity();
        let mut content = div()
            .w_full()
            .max_w(rems(380. / 16.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .mb_3()
                    .child(i18n::text("panel.new_tab")),
            );
        for builtin in &self.builtins {
            let builtin = *builtin;
            let root = cx.entity();
            content = content.child(home_action(
                SharedString::from(format!("panel-home-{builtin:?}")),
                builtin_icon(builtin),
                builtin_title(builtin),
                move |_, cx| root.update(cx, |this, cx| this.activate_builtin(builtin, cx)),
            ));
        }
        content = content
            .child(home_action(
                "panel-home-files",
                "folder.svg",
                i18n::text("panel.files"),
                move |_, cx| files_root.update(cx, |this, cx| this.show_files(cx)),
            ))
            .children(self.terminal_available.then(|| {
                home_action(
                    "panel-home-terminal",
                    "terminal.svg",
                    i18n::text("panel.open_terminal"),
                    move |_, cx| terminal_root.update(cx, |_, cx| cx.emit(PanelEvent::Terminal)),
                )
            }));
        div()
            .size_full()
            .p_5()
            .flex()
            .items_center()
            .justify_center()
            .child(content)
            .into_any_element()
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let refresh = cx.entity();
        let Some(root) = self.root.as_ref() else {
            return status(i18n::text("panel.no_workspace").into());
        };
        let root_tooltip = root.display().to_string();
        let state = self.directories.get(root);
        let rows = self.rows();
        let owner = cx.entity();
        let content = if let Some(error) = state.and_then(|state| state.error.as_ref()) {
            status(error.clone())
        } else if state.is_none_or(|state| !state.loaded) {
            status(i18n::text("panel.loading").into())
        } else if rows.is_empty() {
            status(i18n::text("panel.empty_directory").into())
        } else {
            uniform_list("workspace-file-tree", rows.len(), move |range, _, cx| {
                owner.update(cx, |this, cx| {
                    range
                        .map(|index| this.render_tree_row(&rows[index], cx))
                        .collect::<Vec<_>>()
                })
            })
            .track_scroll(&self.tree_scroll)
            .size_full()
            .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h(px(0.))
            .child(
                div()
                    .h(rems(36. / 16.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path("folder.svg")
                            .size(rems(14. / 16.))
                            .flex_none()
                            .text_color(theme::muted()),
                    )
                    .child(
                        div()
                            .id("workspace-panel-path-label")
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .tooltip(move |_, cx| {
                                runner_app::ui::tooltip::tooltip_view(root_tooltip.clone(), cx)
                            })
                            .child(root.display().to_string()),
                    )
                    .child(
                        IconButton::new("workspace-tree-refresh", "refresh-cw.svg")
                            .size(IconButtonSize::Xs)
                            .tooltip(i18n::text("panel.refresh"))
                            .on_press(move |_, cx| {
                                refresh.update(cx, |this, cx| {
                                    this.refresh(true, cx);
                                    cx.notify();
                                })
                            }),
                    ),
            )
            .children(self.notice.as_ref().map(|notice| {
                div()
                    .px_3()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::warning())
                    .child(notice.clone())
            }))
            .child(
                div()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .into_any_element()
    }

    fn render_tree_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let path = row.entry.path.clone();
        let title = path.display().to_string();
        let open = self.expanded.contains(&path);
        let error = self
            .directories
            .get(&path)
            .and_then(|state| state.error.clone());
        let selected =
            matches!(&self.tabs.selected().kind, TabKind::File(current) if *current == path);
        let loading = self.opening.contains(&path)
            || self
                .directories
                .get(&path)
                .is_some_and(|state| state.loading);
        let tooltip = error.clone().unwrap_or(title);
        let is_dir = row.entry.is_dir;
        let root = cx.entity();
        div()
            .id(SharedString::from(format!(
                "workspace-file-{}",
                path.display()
            )))
            .debug_selector({
                let path = path.clone();
                move || format!("WORKSPACE_FILE {}", path.display())
            })
            .h(rems(30. / 16.))
            .w_full()
            .pr_3()
            .pl(rems((10. + row.depth as f32 * 16.) / 16.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(theme::text_ui())
            .cursor_pointer()
            .text_color(if error.is_some() {
                theme::warning()
            } else if row.entry.ignored {
                theme::faint()
            } else {
                theme::text()
            })
            .when(selected, |row| row.bg(theme::raised()))
            .hover(|row| row.bg(theme::with_alpha(theme::text(), 0.05)))
            .tooltip(move |_, cx| runner_app::ui::tooltip::tooltip_view(tooltip.clone(), cx))
            .child(if is_dir {
                svg()
                    .path(if open {
                        "chevron-down.svg"
                    } else {
                        "chevron-right.svg"
                    })
                    .size(rems(12. / 16.))
                    .text_color(theme::muted())
                    .into_any_element()
            } else {
                div().w(rems(12. / 16.)).flex_none().into_any_element()
            })
            .child(
                svg()
                    .path(if loading {
                        "loader.svg"
                    } else if is_dir {
                        if open {
                            "folder-open.svg"
                        } else {
                            "folder.svg"
                        }
                    } else {
                        file_icon(&path)
                    })
                    .size(rems(15. / 16.))
                    .text_color(if row.entry.ignored {
                        theme::faint()
                    } else {
                        theme::muted()
                    })
                    .flex_none(),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .truncate()
                    .child(row.entry.name.clone()),
            )
            .children(
                row.entry
                    .is_symlink
                    .then(|| div().flex_none().text_color(theme::faint()).child("↗")),
            )
            .on_click(move |_, _, cx| {
                root.update(cx, |this, cx| {
                    if is_dir {
                        this.toggle_directory(path.clone(), cx);
                    } else {
                        this.open_file(path.clone(), cx);
                    }
                })
            })
            .into_any_element()
    }

    fn render_preview(&self, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let state = self.previews.get(&self.tabs.active);
        let refresh = cx.entity();
        let external = path.to_path_buf();
        let content = if let Some(error) = state.and_then(|state| state.error.clone()) {
            status(error)
        } else if let Some(view) = state.and_then(|state| state.view.clone()) {
            view.into_any_element()
        } else {
            status(i18n::text("panel.loading").into())
        };
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let relative = self
            .root
            .as_ref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .display()
            .to_string();
        div()
            .size_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(rems(40. / 16.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("workspace-panel-path-label")
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::SEMIBOLD)
                            .tooltip(move |_, cx| {
                                runner_app::ui::tooltip::tooltip_view(relative.clone(), cx)
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_micro())
                            .text_color(theme::faint())
                            .child(i18n::text("panel.read_only")),
                    )
                    .child(
                        IconButton::new("workspace-file-refresh", "refresh-cw.svg")
                            .size(IconButtonSize::Xs)
                            .tooltip(i18n::text("panel.refresh"))
                            .on_press(move |_, cx| {
                                refresh.update(cx, |this, cx| {
                                    this.refresh(true, cx);
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        IconButton::new("workspace-file-open", "external-link.svg")
                            .size(IconButtonSize::Xs)
                            .tooltip(i18n::text("panel.open_external"))
                            .on_press(move |_, cx| {
                                crate::file_links::open_file_link(
                                    crate::file_links::FileLinkTarget {
                                        path: external.clone(),
                                        line: None,
                                        column: None,
                                    },
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .into_any_element()
    }
}

impl Render for WorkspacePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.tabs.selected().kind.clone() {
            TabKind::Builtin(_) => div().into_any_element(),
            TabKind::New => self.render_home(cx),
            TabKind::Files => self.render_tree(cx),
            TabKind::File(path) => self.render_preview(&path, cx),
        }
    }
}

fn status(message: String) -> AnyElement {
    div()
        .size_full()
        .p_5()
        .text_size(theme::text_ui())
        .text_color(theme::muted())
        .child(message)
        .into_any_element()
}

fn home_action(
    id: impl Into<SharedString>,
    icon: &'static str,
    label: impl Into<SharedString>,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .w_full()
        .px_3()
        .py_3()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(theme::border())
        .text_size(theme::text_ui())
        .cursor_pointer()
        .hover(|style| style.bg(theme::with_alpha(theme::text(), 0.05)))
        .child(svg().path(icon).size(rems(1.)).text_color(theme::muted()))
        .child(label.into())
        .on_click(move |_, window, cx| action(window, cx))
        .into_any_element()
}

fn builtin_title(kind: PanelBuiltin) -> &'static str {
    i18n::text(match kind {
        PanelBuiltin::Info => "panel.info",
        PanelBuiltin::Roles => "panel.roles",
        PanelBuiltin::Meta => "panel.meta",
    })
}
fn builtin_icon(kind: PanelBuiltin) -> &'static str {
    match kind {
        PanelBuiltin::Roles => "users.svg",
        _ => "info.svg",
    }
}
fn file_icon(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "md" | "markdown" => "book-text.svg",
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => "square.svg",
        _ => "file-text.svg",
    }
}
fn tab_icon(kind: &TabKind) -> &'static str {
    match kind {
        TabKind::Builtin(kind) => builtin_icon(*kind),
        TabKind::Files => "folder.svg",
        TabKind::New => "plus.svg",
        TabKind::File(path) => file_icon(path),
    }
}
fn tab_title(kind: &TabKind) -> String {
    match kind {
        TabKind::Builtin(kind) => builtin_title(*kind).into(),
        TabKind::Files => i18n::text("panel.files").into(),
        TabKind::New => i18n::text("panel.new_tab").into(),
        TabKind::File(path) => path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

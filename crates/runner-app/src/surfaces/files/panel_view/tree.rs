//! Tree listings, navigation, row actions and rendering.

use super::*;

const ROW_HEIGHT: f32 = 26.;

const INDENT: f32 = 14.;

const ROW_PAD_LEFT: f32 = 14.;

const CHEVRON_SIZE: f32 = 12.;

type Listings = Vec<(String, Result<Vec<DirEntry>, ClientError>)>;

type MenuAction = Box<dyn Fn(&mut FilesView, &mut Window, &mut Context<FilesView>)>;

impl FilesView {
    pub(super) fn load(&mut self, folders: Vec<String>, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if folders.is_empty() {
            return;
        }
        let root = state.root.path.clone();
        state.next_listing += 1;
        let sequence = state.next_listing;
        self.request(
            cx,
            move |client| {
                Ok(folders
                    .into_iter()
                    .map(|folder| {
                        let listing = client.files_list(&root, &folder);
                        (folder, listing)
                    })
                    .collect::<Vec<_>>())
            },
            move |this, result: Result<Listings, _>, cx| {
                let Ok(listings) = result else { return };
                let mut root_gone = false;
                if let Body::Root(state) = &mut this.body {
                    let mut batch = Vec::with_capacity(listings.len());
                    for (folder, listing) in listings {
                        if state
                            .listed
                            .get(&folder)
                            .is_some_and(|newer| *newer > sequence)
                        {
                            continue;
                        }
                        state.listed.insert(folder.clone(), sequence);
                        match listing {
                            Ok(entries) => batch.push((folder, Some(entries))),
                            Err(_) if folder.is_empty() => root_gone = true,
                            Err(_) => batch.push((folder, None)),
                        }
                    }
                    state.tree.set_listings(batch);
                }
                if root_gone {
                    this.resolve(cx);
                }
                this.after_listing(cx);
                cx.notify();
            },
        );
    }

    // ── The tree ───────────────────────────────────────────────────────

    pub(in super::super) fn tree_mut(&mut self) -> Option<&mut FileTree> {
        match &mut self.body {
            Body::Root(state) => Some(&mut state.tree),
            _ => None,
        }
    }

    pub(in super::super) fn after_move(&mut self, moved: Moved, cx: &mut Context<Self>) {
        if let Some(index) = self
            .tree_mut()
            .and_then(|tree| tree.selected_index())
            .filter(|_| moved.changed)
        {
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
        self.load(moved.load, cx);
        cx.notify();
    }

    /// A click selects the row and opens a folder, or the file in a tab; a
    /// double-click is two clicks, so a folder's second click does nothing.
    fn click_row(&mut self, row: &Row, clicks: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let Some(tree) = self.tree_mut() else { return };
        tree.select(&row.path);
        let moved = if row.folder {
            if clicks >= 2 {
                return;
            }
            tree.toggle(&row.path)
        } else {
            Moved::default()
        };
        self.after_move(moved, cx);
        if !row.folder {
            self.open_file(row.path.clone(), cx);
        }
    }

    fn on_tree_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            return;
        }
        let Body::Root(state) = &mut self.body else {
            return;
        };
        let tree = &mut state.tree;
        let moved = match event.keystroke.key.as_str() {
            "down" => tree.move_down(),
            "up" => tree.move_up(),
            "right" => tree.move_right(),
            "left" => tree.move_left(),
            "space" | "enter" => {
                let selected = tree
                    .selected_index()
                    .map(|index| tree.rows()[index].clone());
                match selected {
                    Some(row) if row.folder => tree.toggle(&row.path),
                    Some(row) => {
                        cx.stop_propagation();
                        self.open_file(row.path, cx);
                        return;
                    }
                    None => Moved::default(),
                }
            }
            _ => match event.keystroke.key_char.as_deref() {
                Some(text) if !text.trim().is_empty() => tree.type_ahead(text, Instant::now()),
                _ => return,
            },
        };
        cx.stop_propagation();
        self.after_move(moved, cx);
    }

    fn open_row_menu(
        &mut self,
        row: Row,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root_path().map(Path::to_path_buf) else {
            return;
        };
        if let Some(tree) = self.tree_mut() {
            tree.select(&row.path);
        }
        let path = absolute(&root, &row.path);
        let mut items = Vec::new();
        let mut actions: Vec<MenuAction> = Vec::new();
        if row.folder {
            items.push(
                MenuItem::new(crate::i18n::message("ui.show_files_from_here"))
                    .icon("folder-tree.svg"),
            );
            let folder = path.clone();
            actions.push(Box::new(move |this, _, cx| {
                this.set_root(Some(folder.clone()), cx)
            }));
        }
        items.push(MenuItem::new(crate::i18n::message("ui.copy_path")).icon("copy.svg"));
        let copied = path.clone();
        actions.push(Box::new(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(
                copied.to_string_lossy().into_owned(),
            ))
        }));
        items.push(MenuItem::new(reveal_label()).icon("app-window.svg"));
        actions.push(Box::new(move |_, _, cx| {
            let path = path.clone();
            let task = cx.background_spawn(async move { reveal(path) });
            cx.spawn(async move |this, cx| {
                if let Err(error) = task.await {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error.into());
                        cx.notify();
                    });
                }
            })
            .detach();
        }));
        let actions = Rc::new(actions);
        let root = cx.weak_entity();
        let dismiss_root = root.clone();
        let return_focus = self.focus.clone();
        let menu = cx.new(move |menu_cx| {
            ContextMenu::new(
                "files-row-menu",
                menu_cx.focus_handle(),
                event.position,
                items,
                Rc::new(move |index, window, cx| {
                    let actions = actions.clone();
                    let _ = root.update(cx, |this, cx| {
                        if let Some(action) = actions.get(index) {
                            action(this, window, cx);
                        }
                    });
                }),
                Rc::new(move |_, cx| {
                    let _ = dismiss_root.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                }),
            )
            .return_focus(Some(return_focus))
        });
        menu.read(cx).focus_handle().focus(window, cx);
        self.menu = Some(menu);
        cx.notify();
    }

    // ── Rendering ──────────────────────────────────────────────────────

    pub(in super::super) fn file_icon(&self, name: &str) -> AnyElement {
        self.icon(&Row {
            path: String::new(),
            name: name.to_owned(),
            depth: 0,
            folder: false,
            expanded: false,
            ignored: false,
            letter: None,
            dot: false,
            cwd: false,
        })
    }

    fn icon(&self, row: &Row) -> AnyElement {
        let dark = !theme::active_variant().is_light();
        let set = self
            .look
            .as_ref()
            .map_or_else(IconSet::default, |look| look.icons);
        let icon = if row.folder {
            file_icons::folder_icon(set, dark, &row.name, row.expanded)
        } else {
            file_icons::file_icon(set, dark, &row.name)
        };
        let opacity = if row.ignored { 0.5 } else { 1. };
        match icon {
            Some(icon) if icon.monochrome => svg()
                .flex_none()
                .size(px(16.))
                .path(icon.path)
                .text_color(if row.ignored {
                    theme::faint()
                } else {
                    theme::muted()
                })
                .into_any_element(),
            Some(icon) => img(ImageSource::Resource(Resource::Embedded(icon.path)))
                .flex_none()
                .size(px(16.))
                .opacity(opacity)
                .into_any_element(),
            None => svg()
                .flex_none()
                .size(px(16.))
                .path(if row.folder {
                    "folder.svg"
                } else {
                    "file-text.svg"
                })
                .text_color(theme::muted())
                .into_any_element(),
        }
    }

    fn render_row(
        &self,
        index: usize,
        row: Row,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self
            .tree_ref()
            .and_then(|tree| tree.selected())
            .is_some_and(|selected| selected == row.path);
        let click_row = row.clone();
        let menu_row = row.clone();
        let chevron = if row.folder {
            svg()
                .flex_none()
                .size(px(CHEVRON_SIZE))
                .path(if row.expanded {
                    "chevron-down.svg"
                } else {
                    "chevron-right.svg"
                })
                .text_color(theme::faint())
                .into_any_element()
        } else {
            div().flex_none().size(px(CHEVRON_SIZE)).into_any_element()
        };
        let name_color = if row.ignored {
            theme::faint()
        } else {
            theme::text()
        };
        let path = row.path.clone();
        div()
            .id(("files-row", index))
            .when(cfg!(test), |element| {
                let path = path.clone();
                element.debug_selector(move || format!("FILES_ROW {path}"))
            })
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .pl(px(ROW_PAD_LEFT + row.depth as f32 * INDENT))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_size(theme::text_ui())
            .when(selected, |element| element.bg(theme::sidebar_selected()))
            .when(selected && focused, |element| {
                element.border_color(theme::faint())
            })
            .when(!selected, |element| {
                element
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            })
            .map(|element| {
                #[cfg(test)]
                let element = if selected {
                    crate::theme_snapshot::record_border(
                        "FILES_ROW_SELECTED",
                        crate::theme_snapshot::record_fill("FILES_ROW_SELECTED", element),
                    )
                } else {
                    element
                };
                element
            })
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    this.click_row(&click_row, event.click_count(), window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_row_menu(menu_row.clone(), event, window, cx)
                }),
            )
            .children((0..row.depth).map(|ancestor| {
                // Runs under each ancestor's chevron, past the row's border so
                // that the rows' segments join into one line.
                div()
                    .when(cfg!(test), |element| {
                        let path = path.clone();
                        element.debug_selector(move || format!("FILES_GUIDE {path} {ancestor}"))
                    })
                    .absolute()
                    .top(px(-1.))
                    .bottom(px(-1.))
                    .left(px(ROW_PAD_LEFT
                        + CHEVRON_SIZE / 2.
                        + ancestor as f32 * INDENT))
                    .w(px(1.))
                    // The theme's line colours can match the panel's background.
                    .bg(theme::with_alpha(theme::faint(), 0.5))
            }))
            .child(chevron)
            .child(self.icon(&row))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(name_color)
                    .map(|element| {
                        #[cfg(test)]
                        let element = if row.ignored {
                            crate::theme_snapshot::record_text("FILES_ROW_IGNORED", element)
                        } else {
                            element
                        };
                        element
                    })
                    .child(row.name.clone()),
            )
            .when(row.cwd, |element| {
                element.child(
                    div()
                        .flex_none()
                        .px(px(5.))
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::border())
                        .text_size(theme::text_micro())
                        .text_color(theme::muted())
                        .when(cfg!(test), |tag| {
                            tag.debug_selector(|| "FILES_CWD_TAG".into())
                        })
                        .child("cwd"),
                )
            })
            .when_some(row.letter, |element, letter| {
                let mark = div()
                    .text_color(letter_color(letter))
                    .map(|element| {
                        #[cfg(test)]
                        let element = {
                            let id = match letter {
                                Letter::Modified => "FILES_LETTER M",
                                Letter::Added => "FILES_LETTER A",
                                Letter::Renamed => "FILES_LETTER R",
                                Letter::Untracked => "FILES_LETTER U",
                                Letter::Unmerged => "FILES_LETTER !",
                            };
                            crate::theme_snapshot::record_text(id, element)
                        };
                        element
                    })
                    .child(letter.text());
                element.child(
                    div()
                        .flex_none()
                        .w(px(12.))
                        .text_center()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .when(cfg!(test), |element| {
                            let path = path.clone();
                            element.debug_selector(move || format!("FILES_LETTER {path}"))
                        })
                        .child(mark),
                )
            })
            .when(row.dot, |element| {
                element.child(
                    div()
                        .flex_none()
                        .w(px(12.))
                        .flex()
                        .justify_center()
                        .child(div().size(px(6.)).rounded_full().bg(theme::warning()))
                        .when(cfg!(test), |element| {
                            let path = path.clone();
                            element.debug_selector(move || format!("FILES_DOT {path}"))
                        }),
                )
            })
            .into_any_element()
    }

    pub(in super::super) fn tree_ref(&self) -> Option<&FileTree> {
        match &self.body {
            Body::Root(state) => Some(&state.tree),
            _ => None,
        }
    }

    pub(super) fn render_tree(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tree) = self.tree_ref() else {
            return div().into_any_element();
        };
        let focused = self.focus.contains_focused(window, cx);
        let count = tree.rows().len();
        let list = gpui::uniform_list(
            "files-tree",
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                let rows = this
                    .tree_ref()
                    .map(|tree| tree.rows()[range.clone()].to_vec())
                    .unwrap_or_default();
                #[cfg(test)]
                crate::render_counts::count_render("files-tree-rows", cx);
                rows.into_iter()
                    .zip(range)
                    .map(|(row, index)| this.render_row(index, row, focused, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .py(px(6.))
        .size_full();
        let truncated = tree.status().is_some_and(|status| status.truncated);
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("files-tree-scroll")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_TREE".into())
                    })
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_tree_key))
                    .child(list)
                    .child(self.scrollbar.clone()),
            )
            .when(truncated, |element| {
                element.child(
                    div()
                        .when(cfg!(test), |element| {
                            element.debug_selector(|| "FILES_STATUS_TRUNCATED".into())
                        })
                        .flex_none()
                        .px(px(12.))
                        .py(px(6.))
                        .border_t_1()
                        .border_color(theme::border())
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("ui.files_status_truncated")),
                )
            })
            .into_any_element()
    }
}

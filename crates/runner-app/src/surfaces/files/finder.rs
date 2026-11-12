//! The file finder (⇧⌘O): a popup centred over the side panel that finds
//! files by name in the root shown, skipping what git ignores.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, point, px, rems, size, svg, AnyElement, Bounds, Context, Entity, FocusHandle, FontWeight,
    KeyDownEvent, ScrollHandle, SharedString, Subscription, Window,
};
use runner_app::ui::menu::{popup_layer_positioned, PopupWidth};
use runner_app::ui::TextField;

use super::panel_view::FilesView;
use crate::theme;

const FINDER_WIDTH: f32 = 520.;
const FINDER_MARGIN: f32 = 8.;
/// The most matches listed: more than anyone reads before typing further.
const MATCH_LIMIT: usize = 100;

pub(super) struct IndexedPath {
    pub(super) path: String,
    lower: String,
    /// Where the file's name starts in `lower`.
    name: usize,
}

impl IndexedPath {
    fn new(path: String) -> Self {
        let lower = path.to_lowercase();
        let name = lower.rfind('/').map_or(0, |slash| slash + 1);
        Self { path, lower, name }
    }

    fn tier(&self, query: &str) -> Option<u8> {
        let name = &self.lower[self.name..];
        if name.starts_with(query) {
            Some(0)
        } else if name.contains(query) {
            Some(1)
        } else if self.lower.contains(query) {
            Some(2)
        } else if is_subsequence(query, name) {
            Some(3)
        } else if is_subsequence(query, &self.lower) {
            Some(4)
        } else {
            None
        }
    }
}

fn is_subsequence(query: &str, text: &str) -> bool {
    let mut text = text.chars();
    query.chars().all(|wanted| text.any(|c| c == wanted))
}

/// The best matches for `query`, best first: a name that starts with it, then
/// one that contains it, then a path that does, then the same letters in
/// order in the name and in the path; within each, the shortest path first.
pub(super) fn rank(paths: &[IndexedPath], query: &str, limit: usize) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut matches = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| Some((path.tier(&query)?, index)))
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| {
        let (left, right) = (&paths[a.1], &paths[b.1]);
        a.0.cmp(&b.0)
            .then(left.path.len().cmp(&right.path.len()))
            .then(left.path.cmp(&right.path))
    });
    matches.truncate(limit);
    matches.into_iter().map(|(_, index)| index).collect()
}

pub(super) struct Finder {
    field: Entity<TextField>,
    query: String,
    /// `None` until the daemon answers.
    paths: Option<Vec<IndexedPath>>,
    truncated: bool,
    error: Option<SharedString>,
    pub(super) matches: Vec<usize>,
    pub(super) highlighted: usize,
    scroll: ScrollHandle,
    reveal: Cell<bool>,
    /// Where focus returns when the finder closes without opening a file.
    return_focus: Option<FocusHandle>,
    _observe: Subscription,
}

impl Finder {
    fn refilter(&mut self) {
        self.matches = self
            .paths
            .as_deref()
            .map(|paths| rank(paths, &self.query, MATCH_LIMIT))
            .unwrap_or_default();
        self.highlighted = 0;
        self.scroll.set_offset(gpui::Point::default());
    }

    pub(super) fn path_at(&self, position: usize) -> Option<&str> {
        let index = *self.matches.get(position)?;
        Some(self.paths.as_ref()?.get(index)?.path.as_str())
    }
}

impl FilesView {
    /// Opens the finder, or closes it when it is open. Without a root to
    /// search, as while a folder is still to be chosen, it stays closed.
    pub(crate) fn toggle_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.finder.is_some() {
            self.close_finder(window, cx);
            return;
        }
        if !self.searchable() {
            return;
        }
        self.picker = None;
        self.menu = None;
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("ui.search_files_by_name"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| this.on_finder_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(finder) = &mut this.finder {
                if finder.query != query {
                    finder.query = query;
                    finder.refilter();
                    cx.notify();
                }
            }
        });
        let return_focus = window.focused(cx);
        field.read(cx).focus_handle().focus(window, cx);
        self.finder = Some(Finder {
            field,
            query: String::new(),
            paths: None,
            truncated: false,
            error: None,
            matches: Vec::new(),
            highlighted: 0,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        self.load_finder_index(cx);
        cx.notify();
    }

    /// Reads the root's files once a root is there to read: at once, or when
    /// the root a finder opened ahead of arrives.
    pub(super) fn load_finder_index(&mut self, cx: &mut Context<Self>) {
        if !self.finder.as_ref().is_some_and(|f| f.paths.is_none()) {
            return;
        }
        if let Some(root) = self.root_path().map(std::path::Path::to_path_buf) {
            self.request(
                cx,
                move |client| client.files_index(&root),
                |this, result, cx| {
                    let Some(finder) = &mut this.finder else {
                        return;
                    };
                    match result {
                        Ok(index) => {
                            finder.truncated = index.truncated;
                            finder.paths =
                                Some(index.paths.into_iter().map(IndexedPath::new).collect());
                            finder.refilter();
                        }
                        Err(error) => finder.error = Some(error.to_string().into()),
                    }
                    cx.notify();
                },
            );
        }
    }

    /// Forgets the old root's files; the new root's are read once it is set.
    pub(super) fn finder_root_changed(&mut self) {
        if let Some(finder) = &mut self.finder {
            finder.paths = None;
            finder.truncated = false;
            finder.error = None;
            finder.refilter();
        }
    }

    pub(super) fn close_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(finder) = self.finder.take() {
            match finder.return_focus {
                Some(focus) => focus.focus(window, cx),
                None => self.focus.focus(window, cx),
            }
            cx.notify();
        }
    }

    fn open_finder_match(&mut self, position: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .finder
            .as_ref()
            .and_then(|finder| finder.path_at(position))
            .map(str::to_owned)
        else {
            return;
        };
        self.finder = None;
        self.focus.focus(window, cx);
        self.open_file(path, cx);
        cx.notify();
    }

    fn on_finder_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(finder) = &mut self.finder else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "down" => {
                finder.highlighted =
                    (finder.highlighted + 1).min(finder.matches.len().saturating_sub(1));
                finder.reveal.set(true);
            }
            "up" => {
                finder.highlighted = finder.highlighted.saturating_sub(1);
                finder.reveal.set(true);
            }
            "escape" => self.close_finder(window, cx),
            "enter" => {
                let position = finder.highlighted;
                self.open_finder_match(position, window, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn render_finder(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let finder = self.finder.as_ref()?;
        let bounds = self.view_bounds.get();
        if bounds.size.width <= px(0.) {
            return None;
        }
        let zoom = crate::app_store::global_app_store(cx)
            .read(cx)
            .settings
            .app_zoom;
        let margin = px(FINDER_MARGIN * zoom);
        let width = px(FINDER_WIDTH * zoom).min(bounds.size.width - margin * 2.);
        let left = bounds.left() + (bounds.size.width - width) / 2.;
        // The popup sits a gap below its anchor, which here has no height.
        let anchor = Bounds::new(
            point(left, bounds.top() + margin - px(4. * zoom)),
            size(width, px(0.)),
        );
        let message = |selector: &'static str, text: SharedString, color| {
            div()
                .when(cfg!(test), move |element| {
                    element.debug_selector(move || selector.into())
                })
                .px(px(10.))
                .py(px(8.))
                .text_size(theme::text_ui())
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        let mut rows = Vec::new();
        if let Some(error) = &finder.error {
            rows.push(message(
                "FILES_FINDER_ERROR",
                error.clone(),
                theme::danger(),
            ));
        } else if finder.paths.is_none() {
            rows.push(message(
                "FILES_FINDER_READING",
                crate::i18n::text("ui.reading_files").into(),
                theme::faint(),
            ));
        } else if finder.matches.is_empty() {
            rows.push(message(
                "FILES_FINDER_EMPTY",
                crate::i18n::text("ui.no_matches").into(),
                theme::faint(),
            ));
        }
        for position in 0..finder.matches.len() {
            let Some(path) = finder.path_at(position) else {
                continue;
            };
            let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
            let highlighted = position == finder.highlighted;
            if highlighted && finder.reveal.replace(false) {
                finder.scroll.scroll_to_item(rows.len());
            }
            rows.push(
                div()
                    .id(("files-finder-item", position))
                    .when(cfg!(test), |element| {
                        element.debug_selector(move || format!("FILES_FINDER_ITEM {position}"))
                    })
                    .px(px(10.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(highlighted, |element| element.bg(theme::sidebar_selected()))
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_finder_match(position, window, cx)
                    }))
                    .child(self.file_icon(name))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::text())
                                    .child(name.to_owned()),
                            )
                            .when(!folder.is_empty(), |element| {
                                element.child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .child(folder.to_owned()),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        if finder.truncated {
            rows.push(message(
                "FILES_FINDER_TRUNCATED",
                crate::i18n::text("ui.finder_truncated").into(),
                theme::faint(),
            ));
        }
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_FINDER".into())
            })
            .flex()
            .flex_col()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path("search.svg")
                            .text_color(theme::faint()),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(finder.field.clone()))
                    .children(search_key_hint(cx)),
            )
            .child(
                div()
                    .id("files-finder-list")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_FINDER_LIST".into())
                    })
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&finder.scroll)
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_positioned(
            anchor,
            window,
            PopupWidth::Fixed(width),
            left,
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_finder(window, cx));
            }),
        ))
    }
}

/// The shortcut that opens the finder, as the user has it bound.
pub(super) fn search_key_hint(cx: &gpui::App) -> Option<AnyElement> {
    let settings = &crate::app_store::global_app_store(cx).read(cx).settings;
    let combo = crate::keymap::effective_binding("search-files", &settings.keymap_overrides)?;
    Some(
        div()
            .flex_none()
            .px(px(5.))
            .py(px(1.))
            .rounded_sm()
            .border_1()
            .border_color(theme::border())
            .text_size(theme::text_micro())
            .font_weight(FontWeight::NORMAL)
            .text_color(theme::faint())
            .child(crate::keymap::format_combo(&combo))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(paths: &[&str]) -> Vec<IndexedPath> {
        paths
            .iter()
            .map(|path| IndexedPath::new((*path).to_owned()))
            .collect()
    }

    fn ranked(paths: &[&str], query: &str) -> Vec<String> {
        let index = index(paths);
        rank(&index, query, 10)
            .into_iter()
            .map(|at| index[at].path.clone())
            .collect()
    }

    #[test]
    fn a_name_beats_its_folders_and_a_start_beats_a_middle() {
        assert_eq!(
            ranked(
                &[
                    "client/mod.rs",
                    "src/terminal_clipboard.rs",
                    "src/deep/client.rs",
                    "src/client.rs",
                ],
                "cli",
            ),
            [
                "src/client.rs",
                "src/deep/client.rs",
                "src/terminal_clipboard.rs",
                "client/mod.rs",
            ]
        );
    }

    #[test]
    fn letters_in_order_still_match_and_case_is_ignored() {
        assert_eq!(
            ranked(&["src/FileViewer.rs", "src/other.rs"], "fvw"),
            ["src/FileViewer.rs"]
        );
        assert_eq!(
            ranked(&["src/FileViewer.rs"], "FILEV"),
            ["src/FileViewer.rs"]
        );
        assert!(ranked(&["src/other.rs"], "zzz").is_empty());
    }

    #[test]
    fn a_path_with_its_folders_matches_and_nothing_typed_lists_the_shortest_first() {
        assert_eq!(ranked(&["a/b/c.rs", "x/y/c.rs"], "x/y"), ["x/y/c.rs"]);
        assert_eq!(ranked(&["a/b/c.rs", "z.rs"], " "), ["z.rs", "a/b/c.rs"]);
    }

    #[test]
    fn the_limit_keeps_the_best() {
        let index = index(&["b.rs", "ab.rs", "c.rs"]);
        assert_eq!(rank(&index, "b", 1), [0]);
    }
}

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, App, Context, FocusHandle, ListAlignment, ListOffset, ListState, Pixels, SharedString,
    Window,
};

use super::draw::{Draw, Look, MARGIN};
use super::layout::selected_range;
use super::links::LinkTarget;
use super::Page;

/// Opens a link's target: a URL in the browser, a file in a new tab.
pub(crate) type OpenLink = Rc<dyn Fn(LinkTarget, &mut Window, &mut App)>;

/// A place in the page's text: a row and a byte offset in its text.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Position {
    pub(crate) row: usize,
    pub(crate) offset: usize,
}

#[cfg(test)]
thread_local! {
    /// The bytes of code the rows drawn on this thread have shaped.
    pub(crate) static DRAWN_CODE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct MarkdownView {
    page: Arc<Page>,
    /// The Markdown file's path from the root, for its links.
    path: String,
    /// Decoded local images, by their path from the root.
    images: HashMap<String, Arc<gpui::Image>>,
    list: ListState,
    focus: FocusHandle,
    open_link: Option<OpenLink>,
    /// The anchor and the head, which moves.
    selection: Option<(Position, Position)>,
    dragging: bool,
    /// Where the left button went down in the page, and whether it has
    /// moved since: a press that moved was a drag, which opens no link.
    press: Option<gpui::Point<Pixels>>,
    pressed_moved: bool,
    /// The file was deleted: the page is dimmed and its links do nothing.
    deleted: bool,
}

impl MarkdownView {
    pub(crate) fn new(
        page: Arc<Page>,
        path: String,
        images: HashMap<String, Arc<gpui::Image>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = ListState::new(page.rows.len(), ListAlignment::Top, px(400.));
        Self {
            page,
            path,
            images,
            list,
            focus: cx.focus_handle(),
            open_link: None,
            selection: None,
            dragging: false,
            press: None,
            pressed_moved: false,
            deleted: false,
        }
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn set_open_link(&mut self, open_link: OpenLink) {
        self.open_link = Some(open_link);
    }

    #[cfg(test)]
    pub(crate) fn images(&self) -> &HashMap<String, Arc<gpui::Image>> {
        &self.images
    }

    /// The file read again: if the full rendered page and decoded images are
    /// identical nothing changes; otherwise the new page renders, scroll stays
    /// at the row it was on, and selection clears only when row text changes.
    pub(crate) fn set_page(
        &mut self,
        page: Arc<Page>,
        images: HashMap<String, Arc<gpui::Image>>,
        cx: &mut Context<Self>,
    ) {
        let same_text = self.page.matches_text(&page);
        if same_text && self.page.same_rendered(&page) && same_images(&self.images, &images) {
            return;
        }
        let old = self.page.rows.len();
        let top = self.list.logical_scroll_top();
        self.page = page;
        self.images = images;
        if !same_text {
            self.selection = None;
        }
        let count = self.page.rows.len();
        self.list.splice(0..old, count);
        if count > 0 {
            self.list.scroll_to(ListOffset {
                item_ix: top.item_ix.min(count - 1),
                offset_in_item: top.offset_in_item,
            });
        }
        cx.notify();
    }

    pub(crate) fn set_deleted(&mut self, deleted: bool, cx: &mut Context<Self>) {
        if self.deleted != deleted {
            self.deleted = deleted;
            cx.notify();
        }
    }

    /// The selected text, rows joined by line breaks; a table row is taken
    /// whole, its cells joined by tabs.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let (anchor, head) = self.selection?;
        let (start, end) = (anchor.min(head), anchor.max(head));
        if start == end {
            return None;
        }
        let mut copied = String::new();
        for row in start.row..=end.row.min(self.page.rows.len().saturating_sub(1)) {
            let line = &self.page.rows[row];
            let text = line.text();
            let range = selected_range(line, row, start, end, text.len());
            if row > start.row && !line.continues() {
                copied.push('\n');
            }
            copied.push_str(&text[range]);
        }
        Some(copied)
    }

    pub(crate) fn select(&mut self, anchor: Position, head: Position, cx: &mut Context<Self>) {
        self.selection = Some((anchor, head));
        cx.notify();
    }

    fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &crate::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.page.rows.len().checked_sub(1) else {
            return;
        };
        let end = Position {
            row: last,
            offset: self.page.rows[last].text().len(),
        };
        self.select(Position::default(), end, cx);
    }

    pub(super) fn begin_selection(
        &mut self,
        at: Position,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let anchor = match self.selection {
            Some((anchor, _)) if extend => anchor,
            _ => at,
        };
        self.selection = Some((anchor, at));
        self.dragging = true;
        cx.notify();
    }

    /// Whether the press now ending moved: a click on a link then opens
    /// nothing, as the press was a drag.
    fn press_moved(&self) -> bool {
        self.pressed_moved
    }

    pub(super) fn drag_to(&mut self, at: Position, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if let Some((anchor, head)) = self.selection {
            if head != at {
                self.selection = Some((anchor, at));
                cx.notify();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn scroll_top(&self) -> ListOffset {
        self.list.logical_scroll_top()
    }

    #[cfg(test)]
    pub(crate) fn scroll_to_row(&mut self, row: usize) {
        self.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
    }

    #[cfg(test)]
    pub(crate) fn page(&self) -> &Arc<Page> {
        &self.page
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
}

fn same_images(
    left: &HashMap<String, Arc<gpui::Image>>,
    right: &HashMap<String, Arc<gpui::Image>>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, left_img)| {
            right.get(key).is_some_and(|right_img| {
                Arc::ptr_eq(left_img, right_img)
                    || (left_img.format == right_img.format && left_img.bytes == right_img.bytes)
            })
        })
}

/// Whether the press that ends in a click moved, as a drag that selects
/// does, so the link under it opens nothing; a selection left from an
/// earlier press does not count.
pub(super) fn dragged(view: &gpui::WeakEntity<MarkdownView>, cx: &App) -> bool {
    view.upgrade()
        .is_some_and(|view| view.read(cx).press_moved())
}

impl Render for MarkdownView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Rc::new(Look::current(window, cx));
        let page = self.page.clone();
        let images = Rc::new(self.images.clone());
        let path: SharedString = self.path.clone().into();
        let open_link = self.open_link.clone().filter(|_| !self.deleted);
        let selection = self.selection;
        let view = cx.weak_entity();
        let count = page.rows.len();
        div()
            .id("file-markdown")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_MARKDOWN".into())
            })
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, _| {
                    view.press = Some(event.position);
                    view.pressed_moved = false;
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, _| {
                let Some(press) = view.press else {
                    return;
                };
                if event.pressed_button != Some(gpui::MouseButton::Left) {
                    view.press = None;
                    return;
                }
                let moved = event.position - press;
                if moved.x.abs() > px(2.) || moved.y.abs() > px(2.) {
                    view.pressed_moved = true;
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging = false),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging = false),
            )
            .when(self.deleted, |element| element.opacity(0.5))
            .child(
                gpui::list(self.list.clone(), move |index, _window, cx| {
                    #[cfg(test)]
                    crate::render_counts::count_render("file-markdown-rows", cx);
                    let _ = cx;
                    let row = &page.rows[index];
                    let selected = match selection {
                        Some((anchor, head)) => {
                            let (start, end) = (anchor.min(head), anchor.max(head));
                            let len = if (start.row..=end.row).contains(&index) {
                                row.text().len()
                            } else {
                                0
                            };
                            selected_range(row, index, start, end, len)
                        }
                        None => 0..0,
                    };
                    let draw = Draw {
                        look: &look,
                        images: &images,
                        from: &path,
                        open_link: open_link.clone(),
                        view: view.clone(),
                        index,
                        selected,
                    };
                    let top = if index == 0 { 0. } else { row.top };
                    div()
                        .when(cfg!(test), move |element| {
                            element.debug_selector(move || format!("FILE_MD_ROW {index}"))
                        })
                        .px(px(MARGIN))
                        .when(index == 0, |element| element.pt(px(MARGIN - 4.)))
                        .when(index + 1 == count, |element| element.pb(px(MARGIN * 2.)))
                        .child(draw.row(row, top))
                        .into_any_element()
                })
                .size_full(),
            )
    }
}

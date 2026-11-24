use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, App, Bounds, Context, DispatchPhase, FocusHandle, ListAlignment, ListOffset,
    ListState, MouseExitEvent, MouseMoveEvent, Pixels, SharedString, Window,
};

use super::draw::{ClickGuard, Draw, Look, Surface, MARGIN};
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
    click_guard: ClickGuard,
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
            click_guard: ClickGuard::default(),
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
        self.page.selected_text(anchor, head)
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

/// Whether the press ending now was a genuine primary click without drag or
/// displacement, so a link under it may open.
pub(super) fn document_allows_click(view: &gpui::WeakEntity<MarkdownView>, cx: &App) -> bool {
    view.upgrade()
        .is_some_and(|view| view.read(cx).click_guard.allows_click())
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
        let bounds_cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let down_bounds = Rc::clone(&bounds_cell);
        let paint_bounds = Rc::clone(&bounds_cell);
        let move_view = view.clone();
        let exit_view = view.clone();
        let tracker = canvas(
            move |bounds, _, _| paint_bounds.set(bounds),
            move |_, _, window, _| {
                let move_view = move_view.clone();
                let exit_view = exit_view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = move_view.update(cx, |view, _| {
                            view.click_guard
                                .on_mouse_move(event.position, event.pressed_button);
                        });
                    }
                });
                window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = exit_view.update(cx, |view, _| view.click_guard.on_mouse_exit());
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        div()
            .id("file-markdown")
            .relative()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_MARKDOWN".into())
            })
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .capture_any_mouse_down(
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, _| {
                    view.click_guard
                        .on_mouse_down(event.button, event.position, down_bounds.get());
                }),
            )
            .capture_any_mouse_up(cx.listener(|view, event: &gpui::MouseUpEvent, _, _| {
                view.click_guard.on_mouse_up(event.button, event.position);
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.dragging = false),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| {
                    view.dragging = false;
                    view.click_guard.reset();
                }),
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
                            if (start.row..=end.row).contains(&index) {
                                let text = row.text();
                                selected_range(row, index, start, end, &text)
                            } else {
                                0..0
                            }
                        }
                        None => 0..0,
                    };
                    let top = if index == 0 { 0. } else { row.top };
                    let draw = Draw {
                        surface: Surface::Document {
                            look: &look,
                            images: &images,
                            from: &path,
                            open_link: open_link.clone(),
                            view: view.clone(),
                            top,
                        },
                        index,
                        selected,
                    };
                    div()
                        .when(cfg!(test), move |element| {
                            element.debug_selector(move || format!("FILE_MD_ROW {index}"))
                        })
                        .px(px(MARGIN))
                        .when(index == 0, |element| element.pt(px(MARGIN - 4.)))
                        .when(index + 1 == count, |element| element.pb(px(MARGIN * 2.)))
                        .child(draw.row(row))
                        .into_any_element()
                })
                .size_full(),
            )
            .child(tracker)
    }
}

//! Open-file state, reads, reloads, tabs, source selection and focus.

mod find;
mod open_in;
mod render;

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, linear_color_stop, linear_gradient, px, rems, svg, AnyElement, App, Bounds,
    ClickEvent, Context, Entity, FontWeight, ImageSource, MouseButton, Pixels, Resource,
    ScrollHandle, SharedString, Subscription, Window,
};
use runner_app::ui::menu::{popup_layer_right_aligned, popup_layer_sized, PopupWidth};
use runner_app::ui::{Button, ButtonVariant, TextField};
use runner_core::protocol::files::{FileContent, FilesChanged};

use super::code::CodeView;
use super::document::{self, Document, FileQuery, Found, MATCH_LIMIT};
use super::markdown::{self, LinkTarget, MarkdownView, Page};
use super::panel_view::{absolute, letter_color, FilesView};
use super::tabs::{FileTab, FileTabs};
use super::tree::Letter;
use crate::app_settings::{FileLinkEditor, MarkdownOpenAs};
#[cfg(not(test))]
use crate::file_links::{self, FileLinkTarget};
use crate::surfaces::side_panel::Widened;
use crate::terminal::find::{find_bar_panel, FindBarParts};
use crate::theme;
use open_in::{editor_icon, found_editors};
pub(super) use open_in::{reveal, reveal_label};

/// What a tab shows below the strip.
pub(super) enum Content {
    Loading,
    Text(Entity<CodeView>),
    Binary {
        size: u64,
    },
    TooLarge {
        size: u64,
    },
    /// Deleted while open: the last version read, if any, dimmed.
    Deleted(Option<Entity<CodeView>>),
    Failed(SharedString),
}

pub(super) struct OpenFile {
    pub(super) content: Content,
    /// The newest read sent; an older reply arriving late is dropped.
    read: u64,
    /// Whether that read's reply is still to come.
    reading: bool,
    /// The size and time of the text last read, to tell whether it changed.
    stamp: Option<Stamp>,
    /// Git has ignored it since it opened, so the watcher says nothing about
    /// it. This stays: git stops listing an ignored file once it is deleted,
    /// which is one of the changes to notice.
    ignored: bool,
    /// A Markdown file's rendered page, drawn instead of its source until
    /// `</>` switches.
    rendered: Option<Entity<MarkdownView>>,
    /// Whether a Markdown file shows its source.
    source: bool,
    /// The line a ⌘-clicked link cites, shown once the file reads.
    line: Option<u32>,
}

/// A Markdown file's page and the local images it shows.
type Rendered = (Arc<Page>, HashMap<String, Arc<gpui::Image>>);

type Stamp = (u64, Option<chrono::DateTime<chrono::Utc>>);

enum Read {
    Text(Arc<Document>, Option<Rendered>, Stamp),
    /// The text still has the stamp the read was told about.
    Unchanged,
    Binary(u64),
    TooLarge(u64),
    Missing,
}

/// How often an open ignored file is read again: the watcher stays quiet about
/// ignored paths, where a build changes thousands.
pub(super) const IGNORED_REFRESH: std::time::Duration = std::time::Duration::from_secs(3);

struct Find {
    field: Entity<TextField>,
    query: FileQuery,
    found: Found,
    /// The text `found` was searched in; its matches are byte ranges there,
    /// so they apply to no other.
    searched: Option<Arc<Document>>,
    current: Option<usize>,
    /// The newest search sent, numbered by the viewer so a reply for a
    /// closed bar never passes for one of the bar reopened.
    search: u64,
    _observe: Subscription,
}

#[derive(Default)]
pub(super) struct Viewer {
    pub(super) tabs: FileTabs,
    pub(super) files: HashMap<String, OpenFile>,
    /// The tree toggle: whether the tree shows beside an open file.
    pub(super) tree_hidden: bool,
    /// How the owner widened the panel for an open file this frame; `None`
    /// leaves the tree its room.
    pub(super) widened: Option<Widened>,
    next_read: u64,
    next_search: u64,
    find: Option<Find>,
    list_open: bool,
    open_in_open: bool,
    strip: ScrollHandle,
    list_anchor: Rc<Cell<Bounds<Pixels>>>,
    open_in_anchor: Rc<Cell<Bounds<Pixels>>>,
    /// The file to scroll into view in the tree once its folder lists.
    pub(super) reveal: Option<String>,
    /// Holds searches back, for a test to look between a reload and its search.
    #[cfg(test)]
    pub(super) hold_search: bool,
    /// What would have opened outside the app, for tests.
    #[cfg(test)]
    pub(super) launched: Vec<(Option<FileLinkEditor>, PathBuf, Option<u32>)>,
    /// The URLs links would have opened in the browser, for tests.
    #[cfg(test)]
    pub(super) opened_urls: Vec<String>,
}

fn is_markdown(path: &str) -> bool {
    super::highlight::grammar_for_path(path).is_some_and(|grammar| grammar.name == "markdown")
}

/// A Markdown file's local image, read through runnerd with its bytes;
/// `None` leaves its alt text showing.
fn read_image(
    client: &runner_core::protocol::DaemonClient,
    root: &std::path::Path,
    path: &str,
) -> Option<Arc<gpui::Image>> {
    let format = markdown::image_format(path)?;
    let bytes = match client.files_read(root, path, true).ok()? {
        FileContent::Text { bytes, .. } => bytes.0,
        FileContent::Binary {
            bytes: Some(bytes), ..
        } => bytes.0,
        _ => return None,
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

/// `bytes` for people: `512 B`, `1.2 MB`, `214 MB`.
pub(super) fn format_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KB", "MB", "GB"] {
        if value < 1024. {
            break;
        }
        value /= 1024.;
        unit = next;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else if value < 10. {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

impl FilesView {
    /// Redraws the tree and every open file when Settings → Side panel →
    /// Files changes what they draw with.
    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let look = super::Look::current(cx);
        if look == self.look {
            return;
        }
        self.look = look;
        for file in self.viewer.files.values() {
            if let Content::Text(view) | Content::Deleted(Some(view)) = &file.content {
                view.update(cx, |_, cx| cx.notify());
            }
            if let Some(rendered) = &file.rendered {
                rendered.update(cx, |_, cx| cx.notify());
            }
        }
        cx.notify();
    }

    pub(super) fn has_tabs(&self) -> bool {
        !self.viewer.tabs.is_empty()
    }

    /// Opens `path` in a tab of its own, or switches to its tab.
    pub(crate) fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        let new = self.viewer.tabs.open(&path);
        self.after_open(path, new, cx);
    }

    /// A ⌘-clicked link in a terminal: `path` in its tab, at `line`. A
    /// Markdown file cited at a line shows its source there.
    pub(super) fn open_cited(&mut self, path: String, line: Option<u32>, cx: &mut Context<Self>) {
        self.open_file(path.clone(), cx);
        if let Some(file) = self.viewer.files.get_mut(&path) {
            file.line = line;
            if line.is_some() && is_markdown(&path) {
                file.source = true;
            }
        }
        self.show_line(&path, cx);
    }

    /// Scrolls to the line a link cited once the file's text is in.
    fn show_line(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(file) = self.viewer.files.get_mut(path) else {
            return;
        };
        let Content::Text(view) = &file.content else {
            return;
        };
        if let Some(line) = file.line.take() {
            view.update(cx, |view, cx| view.go_to_line(line, cx));
        }
        cx.notify();
    }

    fn after_open(&mut self, path: String, new: bool, cx: &mut Context<Self>) {
        if new {
            let source = super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
            self.viewer.files.insert(
                path.clone(),
                OpenFile {
                    content: Content::Loading,
                    read: 0,
                    reading: false,
                    stamp: None,
                    ignored: false,
                    rendered: None,
                    source,
                    line: None,
                },
            );
            self.note_ignored_tabs();
            self.read_file(path.clone(), cx);
        }
        self.tab_shown(cx);
    }

    /// The active tab changed: scroll it into the strip, reveal its file in
    /// the tree and search it when find is open.
    fn tab_shown(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.viewer.tabs.active_index() else {
            self.viewer.find = None;
            cx.notify();
            return;
        };
        self.viewer.strip.scroll_to_item(index);
        let path = self.viewer.tabs.tabs()[index].path.clone();
        if let Some(tree) = self.tree_mut() {
            let moved = tree.reveal(&path);
            self.viewer.reveal = Some(path);
            self.after_move(moved, cx);
        }
        if let Some(path) = self.viewer.tabs.active().map(|tab| tab.path.clone()) {
            self.refresh_ignored(path, cx);
        }
        self.search(cx);
        cx.notify();
    }

    pub(super) fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.viewer.tabs.activate(index);
        self.tab_shown(cx);
    }

    pub(super) fn cycle_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer.tabs.cycle(back);
        self.tab_shown(cx);
        self.keep_focus(window, cx);
    }

    /// Whether the tree is drawn: always without a file open, and beside
    /// one while its toggle is on, which the owner's layout follows
    /// (Jason, 2026-11-05).
    pub(super) fn tree_shown(&self) -> bool {
        !self.shows_file_area()
            || (!self.viewer.tree_hidden && self.viewer.widened.is_none_or(|widened| widened.tree))
    }

    /// Keeps focus on something drawn inside the view, so its keys keep
    /// working: the tree while it shows and has focus, else the active
    /// file's code, else the viewer itself, as while a file loads or shows
    /// a placeholder.
    pub(super) fn keep_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tree_shown = self.tree_shown();
        if tree_shown && self.focus.is_focused(window) {
            return;
        }
        if let Some(view) = self.active_view() {
            view.read(cx).focus_handle().clone().focus(window, cx);
        } else if let Some(page) = self.active_rendered() {
            page.read(cx).focus_handle().clone().focus(window, cx);
        } else if tree_shown {
            self.focus.focus(window, cx);
        } else {
            self.viewer_focus.focus(window, cx);
        }
    }

    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.viewer.tabs.close(index) {
            self.viewer.files.remove(&path);
        }
        if self.viewer.tabs.is_empty() {
            self.reset_viewer();
            self.focus.focus(window, cx);
        }
        self.tab_shown(cx);
        self.keep_focus(window, cx);
    }

    /// Closes every tab, as when the root changes under them.
    pub(super) fn reset_viewer(&mut self) {
        let viewer = std::mem::take(&mut self.viewer);
        self.viewer = Viewer {
            tree_hidden: viewer.tree_hidden,
            widened: viewer.widened,
            // Reads stay numbered across resets, so a reply for a closed
            // tab never passes for one of the same file reopened.
            next_read: viewer.next_read,
            next_search: viewer.next_search,
            #[cfg(test)]
            hold_search: viewer.hold_search,
            #[cfg(test)]
            launched: viewer.launched,
            #[cfg(test)]
            opened_urls: viewer.opened_urls,
            ..Viewer::default()
        };
    }

    /// Marks the open files the latest status says git ignores.
    pub(super) fn note_ignored_tabs(&mut self) {
        let Some(status) = self.tree_ref().and_then(|tree| tree.status()) else {
            return;
        };
        let ignored = self
            .viewer
            .files
            .keys()
            .filter(|path| status.is_ignored(path))
            .cloned()
            .collect::<Vec<_>>();
        for path in ignored {
            if let Some(file) = self.viewer.files.get_mut(&path) {
                file.ignored = true;
            }
        }
    }

    /// Reads `path` again if git has ignored it, unless a read is on its way.
    fn refresh_ignored(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(file) = self.viewer.files.get(&path) else {
            return;
        };
        if file.ignored && !file.reading {
            let stamp = file.stamp;
            self.read_file_since(path, stamp, cx);
        }
    }

    /// Keeps the open ignored files current while the view shows.
    pub(super) fn refresh_ignored_tabs(&self, cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(IGNORED_REFRESH).await;
            let refreshed = this.update(cx, |this, cx| {
                let tabs = this.viewer.tabs.tabs();
                let paths = tabs.iter().map(|tab| tab.path.clone()).collect::<Vec<_>>();
                for path in paths {
                    this.refresh_ignored(path, cx);
                }
            });
            if refreshed.is_err() {
                break;
            }
        })
    }

    /// Resolving the root again drops the replies on their way, so reads
    /// still waiting for one go out again.
    pub(super) fn read_again_after_resolve(&mut self, cx: &mut Context<Self>) {
        let waiting = self
            .viewer
            .files
            .iter()
            .filter(|(_, file)| file.reading)
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in waiting {
            self.read_file(path, cx);
        }
    }

    fn read_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.read_file_since(path, None, cx);
    }

    fn read_file_since(&mut self, path: String, since: Option<Stamp>, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.viewer.next_read += 1;
        let read = self.viewer.next_read;
        if let Some(file) = self.viewer.files.get_mut(&path) {
            file.read = read;
            file.reading = true;
        }
        let relative = path.clone();
        let markdown = is_markdown(&path);
        self.request(
            cx,
            move |client| {
                Ok(match client.files_read(&root, &relative, false)? {
                    FileContent::Text { size, modified, .. } if since == Some((size, modified)) => {
                        Read::Unchanged
                    }
                    FileContent::Text {
                        bytes,
                        size,
                        modified,
                    } => {
                        let document = Arc::new(Document::decode(&bytes.0));
                        let rendered = markdown.then(|| {
                            let page = Page::parse(document.text(), &relative);
                            let images = page
                                .images()
                                .into_iter()
                                .filter_map(|image| {
                                    let decoded = read_image(client, &root, &image)?;
                                    Some((image, decoded))
                                })
                                .collect();
                            (Arc::new(page), images)
                        });
                        Read::Text(document, rendered, (size, modified))
                    }
                    FileContent::Binary { size, .. } => Read::Binary(size),
                    FileContent::TooLarge { size } => Read::TooLarge(size),
                    FileContent::Missing => Read::Missing,
                })
            },
            move |this, result, cx| this.apply_read(path, read, result, cx),
        );
    }

    fn apply_read(
        &mut self,
        path: String,
        read: u64,
        result: Result<Read, runner_core::protocol::ClientError>,
        cx: &mut Context<Self>,
    ) {
        let open_line = self.open_line_handler(path.clone(), cx);
        let open_link = self.open_link_handler(cx);
        let active = self
            .viewer
            .tabs
            .active()
            .is_some_and(|tab| tab.path == path);
        let Some(file) = self.viewer.files.get_mut(&path) else {
            return;
        };
        if file.read != read {
            return;
        }
        file.reading = false;
        let same = match (&result, &file.content) {
            (Ok(Read::Unchanged), _) | (Ok(Read::Missing), Content::Deleted(_)) => true,
            (Ok(Read::Binary(new)), Content::Binary { size })
            | (Ok(Read::TooLarge(new)), Content::TooLarge { size }) => new == size,
            _ => false,
        };
        if same {
            return;
        }
        file.stamp = match &result {
            Ok(Read::Text(_, _, stamp)) => Some(*stamp),
            _ => None,
        };
        let previous = std::mem::replace(&mut file.content, Content::Loading);
        let kept = match previous {
            Content::Text(view) | Content::Deleted(Some(view)) => Some(view),
            _ => None,
        };
        if let Ok(Read::Text(_, Some((page, images)), _)) = &result {
            let (page, images) = (page.clone(), images.clone());
            match &file.rendered {
                Some(view) => view.update(cx, |view, cx| {
                    view.set_page(page, images, cx);
                    view.set_deleted(false, cx);
                }),
                None => {
                    let relative = path.clone();
                    file.rendered = Some(cx.new(|cx| {
                        let mut view = MarkdownView::new(page, relative, images, cx);
                        view.set_open_link(open_link);
                        view
                    }));
                }
            }
        }
        if let (Ok(Read::Missing), Some(view)) = (&result, &file.rendered) {
            view.update(cx, |view, cx| view.set_deleted(true, cx));
        }
        file.content = match result {
            Ok(Read::Text(document, ..)) => match kept {
                Some(view) => {
                    view.update(cx, |view, cx| {
                        view.set_document(document, cx);
                        view.set_deleted(false, cx);
                    });
                    Content::Text(view)
                }
                None => Content::Text(cx.new(|cx| {
                    let mut view =
                        CodeView::new(document, super::highlight::grammar_for_path(&path), cx);
                    view.set_open_line(open_line);
                    view
                })),
            },
            // Handled above, with the tab left as it is.
            Ok(Read::Unchanged) => unreachable!(),
            Ok(Read::Binary(size)) => Content::Binary { size },
            Ok(Read::TooLarge(size)) => Content::TooLarge { size },
            Ok(Read::Missing) => {
                if let Some(view) = &kept {
                    view.update(cx, |view, cx| view.set_deleted(true, cx));
                }
                if active {
                    self.viewer.open_in_open = false;
                }
                Content::Deleted(kept)
            }
            Err(error) => Content::Failed(error.to_string().into()),
        };
        self.show_line(&path, cx);
        if active {
            self.search(cx);
        }
        cx.notify();
    }

    /// Reads again every open file `changed` names, or all of them when the
    /// whole root changed.
    pub(super) fn reload_changed(
        &mut self,
        changed: Option<&FilesChanged>,
        cx: &mut Context<Self>,
    ) {
        let rescan = changed.is_none_or(|changed| changed.paths.is_empty());
        let paths = self
            .viewer
            .tabs
            .tabs()
            .iter()
            .map(|tab| tab.path.clone())
            .filter(|path| {
                rescan
                    || changed.is_some_and(|changed| {
                        changed.paths.iter().any(|changed| {
                            changed == path
                                || path
                                    .strip_prefix(changed.as_str())
                                    .is_some_and(|rest| rest.starts_with('/'))
                        })
                    })
            })
            .collect::<Vec<_>>();
        for path in paths {
            self.read_file(path, cx);
        }
    }

    pub(super) fn after_listing(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.viewer.reveal.clone() else {
            return;
        };
        let Some(index) = self
            .tree_ref()
            .and_then(|tree| tree.rows().iter().position(|row| row.path == path))
        else {
            return;
        };
        self.viewer.reveal = None;
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    /// The active file's code view, unless it shows its rendered page.
    fn active_view(&self) -> Option<&Entity<CodeView>> {
        let tab = self.viewer.tabs.active()?;
        let file = self.viewer.files.get(&tab.path)?;
        if self.shows_rendered(file) {
            return None;
        }
        match &file.content {
            Content::Text(view) | Content::Deleted(Some(view)) => Some(view),
            _ => None,
        }
    }

    fn shows_rendered(&self, file: &OpenFile) -> bool {
        file.rendered.is_some()
            && !file.source
            && matches!(file.content, Content::Text(_) | Content::Deleted(Some(_)))
    }

    /// The active file's rendered page, while it shows.
    fn active_rendered(&self) -> Option<&Entity<MarkdownView>> {
        let tab = self.viewer.tabs.active()?;
        let file = self.viewer.files.get(&tab.path)?;
        self.shows_rendered(file)
            .then_some(file.rendered.as_ref())
            .flatten()
    }

    /// Whether the active file is Markdown with a page, and if so whether
    /// it shows its source.
    fn active_markdown(&self) -> Option<bool> {
        let tab = self.viewer.tabs.active()?;
        let file = self.viewer.files.get(&tab.path)?;
        file.rendered.as_ref().map(|_| file.source)
    }

    /// `</>`: the active Markdown file's source, or its page again. A
    /// deleted file stays as it is: closing is its only action.
    fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.active_markdown() else {
            return;
        };
        if self.active_deleted() {
            return;
        }
        if source {
            // Find searches the source: it closes with it.
            if let Some(view) = self.active_view().cloned() {
                view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            }
            self.viewer.find = None;
        }
        if let Some(file) = self
            .viewer
            .tabs
            .active()
            .and_then(|tab| self.viewer.files.get_mut(&tab.path))
        {
            file.source = !source;
        }
        self.search(cx);
        self.keep_focus(window, cx);
        cx.notify();
    }

    fn open_link_handler(&self, cx: &mut Context<Self>) -> markdown::OpenLink {
        let root = cx.weak_entity();
        Rc::new(move |target, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_link(target, cx));
        })
    }

    /// A link in a rendered page: a URL in the browser, a file in the root
    /// in a tab of its own.
    fn open_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        if self.active_deleted() {
            return;
        }
        match target {
            LinkTarget::Url(url) => {
                #[cfg(test)]
                self.viewer.opened_urls.push(url);
                #[cfg(not(test))]
                cx.open_url(&url);
            }
            LinkTarget::File(path) => self.open_file(path, cx),
            LinkTarget::None => {}
        }
    }

    fn active_deleted(&self) -> bool {
        self.viewer.tabs.active().is_some_and(|tab| {
            matches!(
                self.viewer.files.get(&tab.path).map(|file| &file.content),
                Some(Content::Deleted(_))
            )
        })
    }

    /// Zoom in and out in the Files view: with focus in the viewer, its text a
    /// pixel larger or smaller within the terminal's range; from the tree, its
    /// header or the picker, the app's own zoom.
    pub(super) fn zoom(
        &mut self,
        step: i16,
        app: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewer_focus.contains_focused(window, cx) {
            self.zoom_text(step, cx);
        } else {
            window.dispatch_action(app, cx);
        }
    }

    fn zoom_text(&mut self, step: i16, cx: &mut Context<Self>) {
        use crate::app_settings::{TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN};
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let size = Some(
                        settings
                            .files_font_size()
                            .saturating_add_signed(step)
                            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX),
                    );
                    let changed = settings.files_font_size != size;
                    settings.files_font_size = size;
                    changed
                },
                true,
                cx,
            );
        });
    }
}

#[cfg(test)]
impl FilesView {
    pub(crate) fn open_tabs(&self) -> Vec<String> {
        self.viewer
            .tabs
            .tabs()
            .iter()
            .map(|tab| tab.path.clone())
            .collect()
    }

    pub(crate) fn active_tab(&self) -> Option<String> {
        self.viewer.tabs.active().map(|tab| tab.path.clone())
    }

    pub(crate) fn code_view(&self) -> Option<Entity<CodeView>> {
        self.active_view().cloned()
    }

    pub(crate) fn read_seq(&self, path: &str) -> Option<u64> {
        self.viewer.files.get(path).map(|file| file.read)
    }

    /// A read's reply arriving with `text`, as the daemon would send it.
    pub(crate) fn reply_text(&mut self, path: &str, read: u64, text: &str, cx: &mut Context<Self>) {
        let document = Arc::new(Document::decode(text.as_bytes()));
        self.apply_read(
            path.to_owned(),
            read,
            Ok(Read::Text(document, None, (text.len() as u64, None))),
            cx,
        );
    }

    pub(crate) fn reading(&self, path: &str) -> bool {
        self.viewer.files.get(path).is_some_and(|file| file.reading)
    }

    pub(crate) fn last_search(&self) -> Option<u64> {
        self.viewer.find.as_ref().map(|find| find.search)
    }

    /// A search's reply arriving with `count` matches on the first line.
    pub(crate) fn reply_search(&mut self, search: u64, count: usize, cx: &mut Context<Self>) {
        let Some(searched) = self
            .active_view()
            .map(|view| view.read(cx).document().clone())
        else {
            return;
        };
        let matches = (0..count)
            .map(|index| super::document::FileMatch {
                line: 0,
                range: index..index + 1,
            })
            .collect();
        self.apply_search(search, searched, Found::Matches(matches, false), 0, cx);
    }

    pub(crate) fn reply_binary(
        &mut self,
        path: &str,
        read: u64,
        size: u64,
        cx: &mut Context<Self>,
    ) {
        self.apply_read(path.to_owned(), read, Ok(Read::Binary(size)), cx);
    }

    pub(crate) fn hold_search(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.viewer.hold_search = hold;
        if !hold {
            self.search(cx);
        }
    }

    pub(crate) fn step(&mut self, next: bool, cx: &mut Context<Self>) {
        self.step_find(next, cx);
    }

    pub(crate) fn launched(&self) -> Vec<(Option<FileLinkEditor>, PathBuf, Option<u32>)> {
        self.viewer.launched.clone()
    }

    pub(crate) fn rendered_view(&self) -> Option<Entity<MarkdownView>> {
        self.active_rendered().cloned()
    }

    pub(crate) fn opened_urls(&self) -> Vec<String> {
        self.viewer.opened_urls.clone()
    }

    /// Follows `href` as a click on it in the active rendered page would.
    pub(crate) fn follow_link(&mut self, href: &str, cx: &mut Context<Self>) {
        let Some(from) = self.active_tab() else {
            return;
        };
        self.open_link(markdown::route(href, &from), cx);
    }

    pub(crate) fn find_state(&self) -> Option<(Option<String>, Option<usize>)> {
        let find = self.viewer.find.as_ref()?;
        Some((
            Self::find_count(find).map(|(label, _)| label.to_string()),
            find.current,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn sizes_read_as_people_write_them() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1_258_291), "1.2 MB");
        assert_eq!(format_size(224_395_264), "214 MB");
        assert_eq!(format_size(12 * 1024), "12 KB");
        assert_eq!(format_size(10 * 1024 * 1024), "10 MB");
    }
}

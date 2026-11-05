//! The file viewer inside the Files view: the open files' tabs and their
//! overflow list, Open in, the code view or a file's edge state, and find
//! over the file.

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
use super::tabs::{FileTab, FileTabs};
use super::tree::Letter;
use super::view::{absolute, letter_color, reveal, reveal_label, FilesView};
use crate::app_settings::{FileLinkEditor, MarkdownOpenAs};
#[cfg(not(test))]
use crate::file_links::{self, FileLinkTarget};
use crate::surfaces::side_panel::Widened;
use crate::terminal::find::{find_bar_panel, FindBarParts};
use crate::theme;

const STRIP_HEIGHT: f32 = 44.;

type RowAction = Box<dyn Fn(&mut FilesView, &mut Context<FilesView>)>;
const LIST_WIDTH: f32 = 300.;
const OPEN_IN_WIDTH: f32 = 224.;

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
    /// A Markdown file's rendered page, drawn instead of its source until
    /// `</>` switches.
    rendered: Option<Entity<MarkdownView>>,
    /// Whether a Markdown file shows its source.
    source: bool,
}

/// A Markdown file's page and the local images it shows.
type Rendered = (Arc<Page>, HashMap<String, Arc<gpui::Image>>);

enum Read {
    Text(Arc<Document>, Option<Rendered>),
    Binary(u64),
    TooLarge(u64),
    Missing,
}

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

/// The editors whose command-line launcher is on PATH, then whether the
/// chosen one is among them.
fn found_editors(cx: &App) -> (Vec<FileLinkEditor>, FileLinkEditor) {
    let store = crate::app_store::global_app_store(cx).read(cx);
    let found = FileLinkEditor::ALL
        .into_iter()
        .filter(|editor| {
            editor.cli().is_some_and(|command| {
                store
                    .file_link_environment
                    .1
                    .get(command)
                    .copied()
                    .unwrap_or(false)
            })
        })
        .collect::<Vec<_>>();
    let chosen = store.settings.file_link_editor;
    let effective = if found.contains(&chosen) {
        chosen
    } else {
        FileLinkEditor::DefaultApp
    };
    (found, effective)
}

fn editor_icon(editor: FileLinkEditor, size: f32) -> AnyElement {
    let png = match editor {
        FileLinkEditor::Zed => Some("open-in/zed.png"),
        FileLinkEditor::VsCode => Some("open-in/vscode.png"),
        FileLinkEditor::Cursor => Some("open-in/cursor.png"),
        FileLinkEditor::DefaultApp => None,
    };
    match png {
        Some(path) => img(ImageSource::Resource(Resource::Embedded(path.into())))
            .flex_none()
            .size(px(size))
            .rounded_sm()
            .into_any_element(),
        None => svg()
            .flex_none()
            .size(px(size))
            .path("app-window.svg")
            .text_color(theme::muted())
            .into_any_element(),
    }
}

fn reveal_icon(size: f32) -> AnyElement {
    if cfg!(target_os = "macos") {
        img(ImageSource::Resource(Resource::Embedded(
            "open-in/finder.png".into(),
        )))
        .flex_none()
        .size(px(size))
        .into_any_element()
    } else {
        svg()
            .flex_none()
            .size(px(size))
            .path("folder-open.svg")
            .text_color(theme::muted())
            .into_any_element()
    }
}

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
    /// Redraws the tree and every open file when Settings → Side panel →
    /// Files changes what they draw with.
    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let look = super::Look::current(cx);
        if look == self.look {
            return;
        }
        self.look = look;
        if self.look.as_ref().is_some_and(|look| !look.single_click) {
            // No previews while single clicks keep their tabs: the one
            // open now stays, and turning previews back on leaves it kept.
            self.viewer.tabs.keep_all();
        }
        for file in self.viewer.files.values() {
            if let Content::Text(view) | Content::Deleted(Some(view)) = &file.content {
                view.update(cx, |view, cx| view.settings_changed(cx));
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

    /// Opens `path` in the preview tab, or a kept tab with `keep` or when
    /// Settings → Side panel → Files turns single-click previews off.
    pub(crate) fn open_file(&mut self, path: String, keep: bool, cx: &mut Context<Self>) {
        let keep = keep || super::settings(cx).is_some_and(|settings| !settings.files_single_click);
        let opened = self.viewer.tabs.open(&path, keep);
        self.after_open(path, opened, cx);
    }

    /// Opens `path` in a kept tab beside the active one, keeping every
    /// other tab, as a link in a rendered page does.
    fn open_beside(&mut self, path: String, cx: &mut Context<Self>) {
        let opened = self.viewer.tabs.open_beside(&path);
        self.after_open(path, opened, cx);
    }

    fn after_open(&mut self, path: String, opened: super::tabs::Opened, cx: &mut Context<Self>) {
        if let Some(replaced) = opened.replaced {
            self.viewer.files.remove(&replaced);
        }
        if opened.new {
            let source = super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
            self.viewer.files.insert(
                path.clone(),
                OpenFile {
                    content: Content::Loading,
                    read: 0,
                    rendered: None,
                    source,
                },
            );
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
    /// one while its toggle is on and the panel has room.
    pub(super) fn tree_shown(&self) -> bool {
        !self.has_tabs()
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

    fn read_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.viewer.next_read += 1;
        let read = self.viewer.next_read;
        if let Some(file) = self.viewer.files.get_mut(&path) {
            file.read = read;
        }
        let relative = path.clone();
        let markdown = is_markdown(&path);
        self.request(
            cx,
            move |client| {
                Ok(match client.files_read(&root, &relative, false)? {
                    FileContent::Text { bytes, .. } => {
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
                        Read::Text(document, rendered)
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
        let previous = std::mem::replace(&mut file.content, Content::Loading);
        let kept = match previous {
            Content::Text(view) | Content::Deleted(Some(view)) => Some(view),
            _ => None,
        };
        if let Ok(Read::Text(_, Some((page, images)))) = &result {
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
            Ok(Read::Text(document, _)) => match kept {
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

    fn active_path(&self) -> Option<PathBuf> {
        let root = self.root_path()?;
        let tab = self.viewer.tabs.active()?;
        Some(absolute(root, &tab.path))
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
            LinkTarget::File(path) => self.open_beside(path, cx),
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

    fn open_line_handler(&self, relative: String, cx: &mut Context<Self>) -> super::code::OpenLine {
        let root = cx.weak_entity();
        Rc::new(move |line, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_at_line(&relative, line, cx));
        })
    }

    /// ⌘-click on a line: the file at that line in the chosen editor,
    /// unless it was deleted.
    fn open_at_line(&mut self, relative: &str, line: u32, cx: &mut Context<Self>) {
        let deleted = matches!(
            self.viewer.files.get(relative).map(|file| &file.content),
            None | Some(Content::Deleted(_))
        );
        let Some(root) = self.root_path().filter(|_| !deleted) else {
            return;
        };
        let path = absolute(root, relative);
        self.launch(None, path, Some(line), cx);
    }

    /// Opens `path` outside the app: in `editor`, or the chosen one.
    fn launch(
        &mut self,
        editor: Option<FileLinkEditor>,
        path: PathBuf,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        {
            let _ = cx;
            self.viewer.launched.push((editor, path, line));
        }
        #[cfg(not(test))]
        {
            let target = FileLinkTarget {
                path,
                line,
                column: None,
            };
            match editor {
                Some(editor) => file_links::open_file_with(editor, target, cx),
                None => file_links::open_file_link(target, cx),
            }
        }
    }

    /// Opens the active file in `editor`, at the line in view; picking an
    /// editor from the menu also makes it General → Open file links in.
    fn open_in(&mut self, editor: Option<FileLinkEditor>, cx: &mut Context<Self>) {
        self.viewer.open_in_open = false;
        cx.notify();
        if self.active_deleted() {
            return;
        }
        let Some(path) = self.active_path() else {
            return;
        };
        let line = self.active_view().map(|view| view.read(cx).line_in_view());
        if let Some(editor) = editor {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed = settings.file_link_editor != editor;
                        settings.file_link_editor = editor;
                        changed
                    },
                    true,
                    cx,
                );
            });
        }
        let editor = editor.unwrap_or_else(|| found_editors(cx).1);
        self.launch(Some(editor), path, line, cx);
    }

    fn reveal_active(&mut self, cx: &mut Context<Self>) {
        self.viewer.open_in_open = false;
        if let Some(path) = self.active_path().filter(|_| !self.active_deleted()) {
            cx.background_spawn(async move {
                if let Err(error) = reveal(path) {
                    tracing::warn!("reveal failed: {error}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    // ── Find ───────────────────────────────────────────────────────────

    /// ⌘F: find over the active file; a rendered page switches to its
    /// source, which find searches.
    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_rendered().is_some() {
            self.toggle_source(window, cx);
        }
        if self.active_view().is_none() {
            return;
        }
        if self.viewer.find.is_none() {
            let seed = self
                .active_view()
                .and_then(|view| view.read(cx).selected_text())
                .and_then(|text| text.lines().next().map(str::to_owned))
                .filter(|line| !line.trim().is_empty())
                .unwrap_or_default();
            let root = cx.weak_entity();
            let field = cx.new(move |field_cx| {
                let mut field = TextField::new(
                    field_cx.focus_handle(),
                    "",
                    crate::i18n::message("terminal.find_placeholder"),
                    false,
                )
                .text_size(theme::text_body())
                .key_interceptor(Rc::new(move |event, window, cx| {
                    root.update(cx, |this, cx| this.on_find_key(event, window, cx))
                        .unwrap_or(false)
                }));
                field.set_bare(true, field_cx);
                field.set_right_padding(0., field_cx);
                field.set_text(seed, field_cx);
                field
            });
            let observe = cx.observe(&field, |this, field, cx| {
                let text = field.read(cx).text().to_owned();
                if let Some(find) = &mut this.viewer.find {
                    if find.query.text != text {
                        find.query.text = text;
                        find.current = None;
                        this.search(cx);
                    }
                }
            });
            let text = field.read(cx).text().to_owned();
            self.viewer.find = Some(Find {
                field,
                query: FileQuery {
                    text,
                    ..FileQuery::default()
                },
                found: Found::Empty,
                searched: None,
                current: None,
                search: 0,
                _observe: observe,
            });
            self.search(cx);
        }
        if let Some(find) = &self.viewer.find {
            find.field.update(cx, |field, field_cx| {
                field.select_all(field_cx);
                field.focus_handle().focus(window, field_cx);
            });
        }
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer.find = None;
        if let Some(view) = self.active_view().cloned() {
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            view.read(cx).focus_handle().clone().focus(window, cx);
        }
        cx.notify();
    }

    fn on_find_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.platform && !modifiers.control && !modifiers.alt;
        let step_key = if cfg!(target_os = "macos") {
            keystroke.key.eq_ignore_ascii_case("g") && modifiers.platform && !modifiers.control
        } else {
            keystroke.key == "f3" && plain
        };
        if keystroke.key == "escape" && plain && !modifiers.shift {
            self.close_find(window, cx);
            true
        } else if (keystroke.key == "enter" && plain) || step_key {
            self.step_find(!modifiers.shift, cx);
            true
        } else {
            false
        }
    }

    /// Searches the active file for find's query off the UI thread.
    fn search(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view().cloned() else {
            return;
        };
        let document = view.read(cx).document().clone();
        let Some(find) = &mut self.viewer.find else {
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            return;
        };
        // Matches from another text name byte ranges that are not this
        // one's: they go before anything can draw or step through them.
        if !find
            .searched
            .as_ref()
            .is_some_and(|searched| Arc::ptr_eq(searched, &document))
        {
            find.found = Found::Empty;
            find.searched = None;
            find.current = None;
            view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
        }
        self.viewer.next_search += 1;
        let search = self.viewer.next_search;
        find.search = search;
        let line_in_view = view.read(cx).line_in_view() as usize - 1;
        #[cfg(test)]
        if self.viewer.hold_search {
            return;
        }
        let Some(find) = &mut self.viewer.find else {
            return;
        };
        let query = find.query.clone();
        let searched = document.clone();
        let task = cx.background_spawn(async move { document::find(&document, &query) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_search(search, searched, found, line_in_view, cx)
            });
        })
        .detach();
    }

    /// A search's reply: kept only for the newest search of this very text.
    fn apply_search(
        &mut self,
        search: u64,
        searched: Arc<Document>,
        found: Found,
        line_in_view: usize,
        cx: &mut Context<Self>,
    ) {
        {
            let shown = self
                .active_view()
                .map(|view| view.read(cx).document().clone());
            let Some(find) = &mut self.viewer.find else {
                return;
            };
            let same_text = shown.is_some_and(|shown| Arc::ptr_eq(&shown, &searched));
            if find.search != search || !same_text {
                return;
            }
            if !find
                .searched
                .as_ref()
                .is_some_and(|before| Arc::ptr_eq(before, &searched))
            {
                find.current = None;
            }
            find.searched = Some(searched);
            if let Found::Matches(matches, _) = &found {
                find.current = find
                    .current
                    .filter(|current| *current < matches.len())
                    .or_else(|| {
                        (!matches.is_empty()).then(|| {
                            matches
                                .iter()
                                .position(|found| found.line >= line_in_view)
                                .unwrap_or(0)
                        })
                    });
            }
            find.found = found;
            self.show_matches(cx);
        }
    }

    /// The active text's matches: none unless find searched this very text.
    fn shown_matches(&self, cx: &App) -> Option<&Find> {
        let document = self.active_view()?.read(cx).document();
        self.viewer.find.as_ref().filter(|find| {
            find.searched
                .as_ref()
                .is_some_and(|searched| Arc::ptr_eq(searched, document))
        })
    }

    fn show_matches(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view().cloned() else {
            return;
        };
        let (matches, current) = match self.shown_matches(cx) {
            Some(Find {
                found: Found::Matches(matches, _),
                current,
                ..
            }) => (Arc::new(matches.clone()), *current),
            _ => (Arc::default(), None),
        };
        view.update(cx, |view, cx| view.set_find(matches, current, cx));
        cx.notify();
    }

    fn step_find(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.shown_matches(cx).is_none() {
            return;
        }
        let Some(find) = &mut self.viewer.find else {
            return;
        };
        let Found::Matches(matches, _) = &find.found else {
            return;
        };
        if matches.is_empty() {
            return;
        }
        let count = matches.len();
        find.current = Some(match find.current {
            Some(current) if next => (current + 1) % count,
            Some(current) => (current + count - 1) % count,
            None => 0,
        });
        self.show_matches(cx);
    }

    fn toggle_find(&mut self, regex: bool, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.viewer.find {
            if regex {
                find.query.regex = !find.query.regex;
            } else {
                find.query.match_case = !find.query.match_case;
            }
            find.current = None;
        }
        self.search(cx);
        cx.notify();
    }

    fn find_count(find: &Find) -> Option<(SharedString, bool)> {
        match &find.found {
            Found::Empty => None,
            Found::Invalid => Some((crate::i18n::t("terminal.find_invalid_regex"), true)),
            Found::Matches(matches, _) if matches.is_empty() => {
                Some((crate::i18n::t("terminal.find_no_results"), true))
            }
            Found::Matches(matches, more) => {
                let total = if *more {
                    format!("{MATCH_LIMIT}+")
                } else {
                    matches.len().to_string()
                };
                let current = find.current.map_or(0, |current| current + 1).to_string();
                Some((
                    crate::i18n::tf(
                        "terminal.find_count",
                        &[("current", current), ("total", total)],
                    )
                    .into(),
                    false,
                ))
            }
        }
    }

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
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.viewer.tabs.keep(index);
                }
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
            .child(
                div()
                    .when(tab.preview, |element| element.italic())
                    .when(cfg!(test) && tab.preview, |element| {
                        element.debug_selector(|| "FILE_TAB_PREVIEW".into())
                    })
                    .whitespace_nowrap()
                    .child(tab.name().to_owned()),
            )
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
            .child(
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
                                        element
                                            .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                                    })
                                    .tooltip(move |_, cx| {
                                        runner_app::ui::tooltip::tooltip_view(
                                            open_in_label.clone(),
                                            cx,
                                        )
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.open_in(None, cx)))
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
                                        element
                                            .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
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

    fn render_file(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.viewer.tabs.active() else {
            return div().flex_1().into_any_element();
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

    fn render_code(
        &self,
        view: Entity<CodeView>,
        banner: Option<AnyElement>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let find = self.viewer.find.as_ref().map(|find| {
            let root = cx.weak_entity();
            let action = |act: fn(&mut Self, &mut Window, &mut Context<Self>)| {
                let root = root.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let _ = root.update(cx, |this, cx| act(this, window, cx));
                }) as crate::terminal::find::BarAction
            };
            let step_root = root.clone();
            div()
                .absolute()
                .top(px(8.))
                .left(px(14.))
                .right(px(14.))
                .flex()
                .justify_end()
                .child(
                    find_bar_panel(FindBarParts {
                        prefix: "file-find",
                        field: find.field.clone(),
                        match_case: find.query.match_case,
                        regex: find.query.regex,
                        count: Self::find_count(find),
                        can_step: matches!(&find.found, Found::Matches(matches, _) if !matches.is_empty()),
                        footer: None,
                        toggle_case: action(|this, _, cx| this.toggle_find(false, cx)),
                        toggle_regex: action(|this, _, cx| this.toggle_find(true, cx)),
                        step: Rc::new(move |next, _, cx| {
                            let _ = step_root.update(cx, |this, cx| this.step_find(next, cx));
                        }),
                        close: action(|this, window, cx| this.close_find(window, cx)),
                    })
                    .id("file-find")
                    .debug_selector(|| "FILE_FIND_BAR".into()),
                )
        });
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .children(banner)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(view)
                    .children(find),
            )
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
                            .when(tab.preview, |element| element.italic())
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

    fn render_open_in_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.viewer.open_in_open || self.active_deleted() {
            return None;
        }
        let name = self.viewer.tabs.active()?.name().to_owned();
        let (found, effective) = found_editors(cx);
        let row = |id: (&'static str, usize),
                   icon: AnyElement,
                   label: String,
                   checked: bool,
                   cx: &mut Context<Self>,
                   on_click: RowAction| {
            div()
                .id(id)
                .when(cfg!(test), |element| {
                    element.debug_selector(move || format!("FILE_OPEN_IN_ITEM {} {}", id.0, id.1))
                })
                .px(px(8.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .cursor_pointer()
                .hover(|element| element.bg(theme::sidebar_selected()))
                .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
                .child(icon)
                .child(
                    div()
                        .flex_1()
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(label),
                )
                .when(checked, |element| {
                    element.child(
                        svg()
                            .flex_none()
                            .size(px(13.))
                            .path("check.svg")
                            .text_color(theme::text()),
                    )
                })
                .into_any_element()
        };
        let mut rows = found
            .iter()
            .enumerate()
            .map(|(index, editor)| {
                let editor = *editor;
                row(
                    ("file-open-in-editor", index),
                    editor_icon(editor, 16.),
                    editor.label().to_owned(),
                    editor == effective,
                    cx,
                    Box::new(move |this, cx| this.open_in(Some(editor), cx)),
                )
            })
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            rows.push(
                div()
                    .my(px(4.))
                    .h(px(1.))
                    .bg(theme::border())
                    .into_any_element(),
            );
        }
        rows.push(row(
            ("file-open-in-default", 0),
            editor_icon(FileLinkEditor::DefaultApp, 16.),
            crate::i18n::text("ui.default_app").to_owned(),
            effective == FileLinkEditor::DefaultApp,
            cx,
            Box::new(|this, cx| this.open_in(Some(FileLinkEditor::DefaultApp), cx)),
        ));
        rows.push(row(
            ("file-open-in-reveal", 0),
            reveal_icon(16.),
            reveal_label().to_owned(),
            false,
            cx,
            Box::new(|this, cx| this.reveal_active(cx)),
        ));
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_OPEN_IN_POPUP".into())
            })
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
                    .truncate()
                    .child(crate::i18n::tf("format.open_file_in", &[("name", name)])),
            )
            .children(rows)
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.viewer.open_in_anchor.get(),
            window,
            px(OPEN_IN_WIDTH),
            content,
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    this.viewer.open_in_open = false;
                    cx.notify();
                })
            }),
        ))
    }

    /// The viewer's column: the strip, then the active file.
    pub(super) fn render_viewer(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_viewer_popups(
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

#[cfg(test)]
impl FilesView {
    pub(crate) fn open_tabs(&self) -> Vec<(String, bool)> {
        self.viewer
            .tabs
            .tabs()
            .iter()
            .map(|tab| (tab.path.clone(), tab.preview))
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
        self.apply_read(path.to_owned(), read, Ok(Read::Text(document, None)), cx);
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

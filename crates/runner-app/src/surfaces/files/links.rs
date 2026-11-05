//! ⌘-clicked file links from a terminal (#634 m7): whether a link falls in
//! the Files root, checked off the UI thread, and opening it in the preview.

use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui::{App, AppContext as _, Context};

use super::FilesView;
use crate::file_links::FileLinkTarget;

pub(crate) type ShowPreview = Rc<dyn Fn(&mut App)>;

/// Where the view stands on its root, for a link.
pub(super) enum LinkRoot {
    /// Still resolving.
    Pending,
    Root(PathBuf),
    /// No root to open a link in: a remote session, a folder to choose, one
    /// that is gone, or a failure.
    None,
}

/// One hover over a link, from when it started: its answer, once checked,
/// holds for that hover only, since a symlink can be pointed elsewhere.
struct Hover {
    path: PathBuf,
    since: Instant,
    generation: u64,
    /// The path relative to the root, `None` outside; unset while checking.
    answer: Option<Option<String>>,
}

#[derive(Default)]
pub(super) struct Links {
    /// Numbers clicks, so only the latest one opens anything.
    click: u64,
    /// A click waiting for the root, and what shows the panel once it opens.
    pending: Option<(FileLinkTarget, ShowPreview)>,
    hover: Option<Hover>,
    /// Redraws whoever is waiting to hear where a link opens.
    waiter: Option<ShowPreview>,
}

/// `path` relative to `root`, `/`-separated, when it resolves inside it.
/// Symlinks and `..` resolve first, so a path that leaves the root through
/// either falls outside, as `runnerd` would refuse to read it.
pub(crate) fn relative_in(root: &Path, path: &Path) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    let parts = path
        .strip_prefix(&root)
        .ok()?
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl FilesView {
    /// A ⌘-click on a file link in a terminal: when the file resolves
    /// inside the root, checked afresh, it opens in a tab at the link's
    /// line and `show` opens the panel on it; otherwise the chosen
    /// editor opens it.
    pub(crate) fn open_terminal_link(
        &mut self,
        target: FileLinkTarget,
        show: ShowPreview,
        cx: &mut Context<Self>,
    ) {
        self.links.click += 1;
        let root = match self.link_root() {
            LinkRoot::Pending => {
                self.links.pending = Some((target, show));
                return;
            }
            LinkRoot::None => {
                crate::file_links::open_file_link(target, cx);
                return;
            }
            LinkRoot::Root(root) => root,
        };
        let click = self.links.click;
        let generation = self.generation();
        let path = target.path.clone();
        let task = cx.background_spawn(async move { relative_in(&root, &path) });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.links.click != click {
                    return;
                }
                if this.generation() != generation {
                    // The root changed meanwhile: check against the new one.
                    this.open_terminal_link(target, show, cx);
                    return;
                }
                match answer {
                    Some(relative) => {
                        this.open_cited(relative, target.line, cx);
                        cx.defer(move |cx| show(cx));
                    }
                    None => crate::file_links::open_file_link(target, cx),
                }
            });
        })
        .detach();
    }

    /// Whether a click on `path` opens in the preview, for the hover that
    /// started at `since`; `None` until that is known, and `redraw` runs
    /// when it may be.
    pub(crate) fn link_previews(
        &mut self,
        path: &Path,
        since: Instant,
        redraw: ShowPreview,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let generation = self.generation();
        let root = match self.link_root() {
            LinkRoot::Pending => {
                self.links.waiter = Some(redraw);
                return None;
            }
            LinkRoot::None => return Some(false),
            LinkRoot::Root(root) => root,
        };
        let current = self.links.hover.as_ref().filter(|hover| {
            hover.path == path && hover.since == since && hover.generation == generation
        });
        if let Some(hover) = current {
            if let Some(answer) = &hover.answer {
                return Some(answer.is_some());
            }
            self.links.waiter = Some(redraw);
            return None;
        }
        self.links.hover = Some(Hover {
            path: path.to_path_buf(),
            since,
            generation,
            answer: None,
        });
        self.links.waiter = Some(redraw);
        let checked = path.to_path_buf();
        let task = cx.background_spawn(async move { relative_in(&root, &checked) });
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |this, cx| {
                let hover = this.links.hover.as_mut().filter(|hover| {
                    hover.since == since && hover.generation == generation && hover.answer.is_none()
                });
                if let Some(hover) = hover {
                    hover.answer = Some(answer);
                    this.redraw_link_waiter(cx);
                }
            });
        })
        .detach();
        None
    }

    /// The root resolved: a click waiting on it goes ahead, and whoever
    /// waits on an answer redraws.
    pub(super) fn settle_links(&mut self, cx: &mut Context<Self>) {
        if let Some((target, show)) = self.links.pending.take() {
            self.open_terminal_link(target, show, cx);
        }
        self.redraw_link_waiter(cx);
    }

    fn redraw_link_waiter(&mut self, cx: &mut Context<Self>) {
        if let Some(waiter) = self.links.waiter.take() {
            cx.defer(move |cx| waiter(cx));
        }
    }
}

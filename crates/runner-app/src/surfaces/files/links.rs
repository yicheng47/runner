//! ⌘-clicked file links from a terminal (#634 m7, #1458): root, size and
//! unknown-extension binary checks off the UI thread, routed through the single
//! decision function in [`crate::file_links`].

use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui::{App, AppContext as _, Context};

use super::FilesView;
use crate::app_store::global_app_store;
use crate::file_links::{
    classify_file_link, decide_file_link, open_file_in_editor, open_local_non_preview,
    open_local_terminal_decision, sniff_file_content, FileLinkContent, FileLinkDecision,
    FileLinkKind, FileLinkPanelState, FileLinkTarget,
};

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

/// Filesystem facts gathered off the UI thread for one hovered or clicked link.
#[derive(Clone, Debug)]
struct LinkProbe {
    relative: Option<String>,
    canonical: Option<PathBuf>,
    size: Option<u64>,
    content: FileLinkContent,
}

/// One hover over a link, from when it started: its probe, once checked,
/// holds for that hover only, since a symlink or file size can change.
struct Hover {
    path: PathBuf,
    since: Instant,
    generation: u64,
    /// Unset while checking off the UI thread.
    probe: Option<LinkProbe>,
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

/// Like [`relative_in`], but falls back to comparing normalized paths when an
/// already-canonical outside tab key points at a file that was deleted while
/// open.
pub(crate) fn relative_in_canonical(root: &Path, canonical_path: &Path) -> Option<String> {
    if let Some(relative) = relative_in(root, canonical_path) {
        return Some(relative);
    }
    let root = runner_core::app_paths::normalize_checkout_path(root);
    let path = runner_core::app_paths::normalize_checkout_path(canonical_path);
    let root_parts: Vec<Component<'_>> = root.components().collect();
    let path_parts: Vec<Component<'_>> = path.components().collect();
    if path_parts.len() <= root_parts.len() {
        return None;
    }
    let prefix_matches = root_parts.iter().zip(path_parts.iter()).all(|(a, b)| {
        #[cfg(windows)]
        {
            a.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        }
        #[cfg(not(windows))]
        {
            a == b
        }
    });
    if !prefix_matches {
        return None;
    }
    let parts = path_parts[root_parts.len()..]
        .iter()
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn probe_link(root: Option<&Path>, path: &Path, sniff_unknown: bool) -> LinkProbe {
    let meta = std::fs::metadata(path).ok().filter(|meta| meta.is_file());
    let size = meta.as_ref().map(|meta| meta.len());
    let relative = meta
        .as_ref()
        .and_then(|_| root.and_then(|root| relative_in(root, path)));
    let canonical = if relative.is_none() && meta.is_some() {
        path.canonicalize()
            .ok()
            .filter(|resolved| resolved.is_file())
            .map(|resolved| runner_core::app_paths::normalize_checkout_path(&resolved))
    } else {
        None
    };
    let content = if sniff_unknown
        && meta.is_some()
        && matches!(classify_file_link(path), FileLinkKind::Unknown)
    {
        sniff_file_content(path)
    } else {
        FileLinkContent::Unknown
    };
    LinkProbe {
        relative,
        canonical,
        size,
        content,
    }
}

impl FilesView {
    fn link_decision_inputs(
        &self,
        cx: &App,
    ) -> (
        bool,
        crate::app_settings::FileLinkOpenIn,
        FileLinkPanelState,
    ) {
        let settings = &global_app_store(cx).read(cx).settings;
        (
            !self.is_remote(),
            settings.file_link_open_in,
            FileLinkPanelState::from_settings(settings, true),
        )
    }

    /// A ⌘-click on a file link in a terminal: routed through
    /// [`decide_file_link`] with root, size and unknown-extension binary checks
    /// off the UI thread.
    pub(crate) fn open_terminal_link(
        &mut self,
        target: FileLinkTarget,
        show: ShowPreview,
        cx: &mut Context<Self>,
    ) {
        self.links.click += 1;
        let (local, open_in, panel) = self.link_decision_inputs(cx);
        let preliminary = decide_file_link(
            &target.path,
            None,
            FileLinkContent::Unknown,
            local,
            open_in,
            panel,
        );
        if preliminary != FileLinkDecision::Preview {
            if local {
                open_local_non_preview(target, open_in, panel, cx);
            } else {
                open_file_in_editor(target, cx);
            }
            return;
        }
        let root = match self.link_root() {
            LinkRoot::Pending => {
                self.links.pending = Some((target, show));
                return;
            }
            LinkRoot::Root(root) => Some(root),
            LinkRoot::None => None,
        };
        let click = self.links.click;
        let generation = self.generation();
        let path = target.path.clone();
        let task = cx.background_spawn(async move { probe_link(root.as_deref(), &path, true) });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.links.click != click {
                    return;
                }
                if this.generation() != generation {
                    // The root changed meanwhile: check against the new one.
                    this.open_terminal_link(target, show, cx);
                    return;
                }
                let (local, open_in, panel) = this.link_decision_inputs(cx);
                let decision = decide_file_link(
                    &target.path,
                    probe.size,
                    probe.content,
                    local,
                    open_in,
                    panel,
                );
                if decision != FileLinkDecision::Preview {
                    if local {
                        open_local_terminal_decision(decision, target, probe.content, cx);
                    } else {
                        open_file_in_editor(target, cx);
                    }
                    return;
                }
                if let Some(relative) = probe.relative {
                    this.open_cited(relative, target.line, cx);
                    cx.defer(move |cx| show(cx));
                } else if let Some(canonical) = probe.canonical {
                    this.open_outside(canonical, target.line, cx);
                    cx.defer(move |cx| show(cx));
                } else if local {
                    open_local_terminal_decision(
                        FileLinkDecision::Editor,
                        target,
                        probe.content,
                        cx,
                    );
                } else {
                    open_file_in_editor(target, cx);
                }
            });
        })
        .detach();
    }

    /// Where a click on `path` will open, for the hover that started at
    /// `since`; `None` until root and size checks finish off the UI thread,
    /// calling `redraw` once known.
    pub(crate) fn link_decision(
        &mut self,
        path: &Path,
        since: Instant,
        redraw: ShowPreview,
        cx: &mut Context<Self>,
    ) -> Option<FileLinkDecision> {
        let (local, open_in, panel) = self.link_decision_inputs(cx);
        let preliminary =
            decide_file_link(path, None, FileLinkContent::Unknown, local, open_in, panel);
        if preliminary != FileLinkDecision::Preview {
            return Some(preliminary);
        }
        let generation = self.generation();
        let root = match self.link_root() {
            LinkRoot::Pending => {
                self.links.waiter = Some(redraw);
                return None;
            }
            LinkRoot::Root(root) => Some(root),
            LinkRoot::None => None,
        };
        let current = self.links.hover.as_ref().filter(|hover| {
            hover.path == path && hover.since == since && hover.generation == generation
        });
        if let Some(hover) = current {
            if let Some(probe) = &hover.probe {
                let decision = decide_file_link(
                    path,
                    probe.size,
                    FileLinkContent::Unknown,
                    local,
                    open_in,
                    panel,
                );
                return Some(
                    if decision == FileLinkDecision::Preview
                        && probe.relative.is_none()
                        && probe.canonical.is_none()
                    {
                        FileLinkDecision::Editor
                    } else {
                        decision
                    },
                );
            }
            self.links.waiter = Some(redraw);
            return None;
        }
        self.links.hover = Some(Hover {
            path: path.to_path_buf(),
            since,
            generation,
            probe: None,
        });
        self.links.waiter = Some(redraw);
        let checked = path.to_path_buf();
        let task = cx.background_spawn(async move { probe_link(root.as_deref(), &checked, false) });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |this, cx| {
                let hover = this.links.hover.as_mut().filter(|hover| {
                    hover.since == since && hover.generation == generation && hover.probe.is_none()
                });
                if let Some(hover) = hover {
                    hover.probe = Some(probe);
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

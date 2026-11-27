//! Unified File + Diff viewer (#1421): shared per-subject tab strip (pinned
//! tabs + single italic preview tab), in-place `Code | Diff` mode switch,
//! Markdown rendered/source, in-file find, `Open in`, live `Viewed` sync, and
//! scoped fallback watch when neither `Files` nor `Changes` rail is mounted.

mod changes;
mod diff;
mod find;
mod open_in;
mod read;
mod render;
mod rendered;
mod saved;
mod tab_nav;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
mod uncommitted;
mod viewed;
mod watch;

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, linear_color_stop, linear_gradient, px, rems, svg, AnyElement, App, Bounds,
    ClickEvent, Context, Entity, EntityId, FocusHandle, FontWeight, ImageSource, KeyDownEvent,
    MouseButton, Pixels, Resource, ScrollHandle, SharedString, Subscription, Task, Window,
};
use runner_app::ui::menu::{popup_layer_right_aligned, popup_layer_sized, PopupWidth};
use runner_app::ui::{Button, ButtonVariant, TextField};
use runner_core::protocol::changes::{
    ChangedFile, ChangesDiff, ChangesTarget, ChangesViewed, DiffCounts, DiffHunk, ModeChange,
};
use runner_core::protocol::files::{
    FileContent, FileStamp, FileStatus, FilesChanged, FilesRoot, FilesRootOutcome, FilesSubject,
};
use runner_core::protocol::{ClientError, DaemonClient};

use super::code::CodeView;
use super::document::{self, Document, FileQuery, Found, MATCH_LIMIT};
use super::image::{self, DecodeOutcome, DecodedImage, ImageCanvas};
use super::panel_view::{absolute, letter_color};
use super::tabs::{FileTab, FileTabs, OpenPreviewOutcome, TabKey};
use super::tree::PreparedStatus;
use super::{highlight, settings, FilesEvent, FilesHub, Look};
use crate::app_settings::{DiffViewMode, FileLinkEditor, MarkdownOpenAs};
#[cfg(not(test))]
use crate::file_links::{self, FileLinkTarget};
use crate::surfaces::changes::model::{
    abbreviate_folder, display_base_ref, file_name, reconcile_file_selection,
    status_letter_and_color, ListRow, SelectedTarget,
};
use crate::surfaces::changes::{
    prepare_diff_body, LoadedDiffKind, PreparedDiffBody, UnifiedDiffView,
};
use crate::surfaces::side_panel::{
    self, MaximizeControl, PanelCallback, PanelContext, PanelMaximizeCallback, PanelSubject,
    Widened,
};
use crate::terminal::find::{find_bar_panel, FindBarParts};
use crate::theme;
use crate::ui::markdown::{self, LinkTarget, MarkdownView, Page};
pub(crate) use open_in::{editor_icon, file_link_editor_options, found_editors};
pub(super) use open_in::{reveal, reveal_label};
#[cfg(test)]
use read::{read_raster_file, read_text_or_svg_file, Read};
pub(crate) use saved::SavedViewerState;

pub(crate) type TitlebarWrapper = Rc<dyn Fn(&'static str, gpui::Div, &mut App) -> AnyElement>;
pub(crate) type TabActivatedCallback = Rc<dyn Fn(Option<&TabKey>, &mut App)>;
pub(crate) type ViewedToggledCallback =
    Rc<dyn Fn(&Path, &SelectedTarget, &str, Option<&str>, Option<&str>, bool, &mut App)>;
pub(crate) type ViewedErrorCallback = Rc<dyn Fn(String, &mut App)>;

/// Which representation an open tab currently displays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DocMode {
    #[default]
    Code,
    Diff,
}

/// Request from the Changes rail to open a changed file in the unified viewer.
#[derive(Clone, Debug)]
pub(crate) struct OpenChangesRequest {
    pub(crate) root: FilesRoot,
    pub(crate) path: String,
    pub(crate) target: SelectedTarget,
    pub(crate) chosen_base: Option<String>,
    pub(crate) resolved_base: Option<String>,
    pub(crate) branch_name: Option<String>,
    pub(crate) commit_short_id: Option<String>,
    pub(crate) file: ChangedFile,
    pub(crate) has_head: bool,
}

/// Ephemeral per-frame chrome passed by `FilesPanel` when composing the
/// unified viewer beside the right rail.
#[derive(Default)]
pub(crate) struct ViewerChrome {
    pub(crate) show_rail_column: bool,
    pub(crate) hidden_rail_focus: Option<FocusHandle>,
    pub(crate) finder_host: Option<gpui::WeakEntity<super::panel_view::FilesView>>,
    pub(crate) maximize_control: Option<MaximizeControl>,
    pub(crate) collapse_when_rail_hidden: Option<AnyElement>,
    pub(crate) macos_titlebar_inset: Option<(f32, Option<AnyElement>)>,
    pub(crate) wrap_titlebar: Option<TitlebarWrapper>,
    pub(crate) on_escape_to_rail: Option<PanelCallback>,
    pub(crate) on_diff_focus_in: Option<PanelCallback>,
    pub(crate) on_tab_activated: Option<TabActivatedCallback>,
    pub(crate) on_viewed_toggled: Option<ViewedToggledCallback>,
    pub(crate) on_viewed_error: Option<ViewedErrorCallback>,
}

/// Preserved body kept dimmed beneath the banner when a file is deleted while open.
pub(super) enum DeletedContent {
    Text(Entity<CodeView>),
    Image {
        canvas: Entity<ImageCanvas>,
        width: u32,
        height: u32,
        size: u64,
    },
}

/// What a tab's working-tree file body holds.
pub(super) enum Content {
    /// Working-tree file has not been read yet (e.g., opened straight into Diff mode).
    Idle,
    Loading,
    Text(Entity<CodeView>),
    Image {
        canvas: Entity<ImageCanvas>,
        width: u32,
        height: u32,
        size: u64,
    },
    ImageTooLarge {
        width: u32,
        height: u32,
        size: u64,
    },
    ImageFailed,
    Binary {
        size: u64,
    },
    TooLarge {
        size: u64,
    },
    /// Deleted while open: the last version read, if any, dimmed.
    Deleted(Option<DeletedContent>),
    Failed(SharedString),
}

pub(crate) struct LoadedFileDiff {
    pub(crate) path: String,
    pub(crate) target: SelectedTarget,
    pub(crate) base: Option<String>,
    pub(crate) old_blob_id: Option<String>,
    pub(crate) new_blob_id: Option<String>,
    pub(crate) status: FileStatus,
    pub(crate) counts: DiffCounts,
    pub(crate) mode_change: Option<ModeChange>,
    pub(crate) conflicted: bool,
    pub(crate) has_head: bool,
    pub(crate) viewed: bool,
    pub(crate) changed_since_viewed: bool,
    pub(crate) hunks: Vec<DiffHunk>,
    pub(crate) kind: LoadedDiffKind,
}

impl LoadedFileDiff {
    pub(crate) fn set_viewed_state(
        &mut self,
        viewed: bool,
        changed_since_viewed: bool,
        cx: &mut App,
    ) {
        self.viewed = viewed;
        self.changed_since_viewed = changed_since_viewed;
        match &self.kind {
            LoadedDiffKind::Unified(entity)
            | LoadedDiffKind::Svg {
                unified: entity, ..
            } => {
                entity.update(cx, |v, cx| {
                    v.set_viewed_state(viewed, changed_since_viewed, cx);
                });
            }
            _ => {}
        }
    }

    pub(crate) fn hunk_new_lines(&self) -> Vec<u32> {
        self.hunks.iter().map(|h| h.new_start.max(1)).collect()
    }
}

#[derive(Default)]
pub(super) enum FileTabDiff {
    #[default]
    Idle,
    Loading,
    Loaded(Box<LoadedFileDiff>),
    NoChanges,
    Failed(SharedString),
}

impl FileTabDiff {
    fn drop_image_textures(&self, window: Option<&mut Window>, cx: &mut App) {
        if let Self::Loaded(loaded) = self {
            match &loaded.kind {
                LoadedDiffKind::Image { canvas, .. } | LoadedDiffKind::Svg { canvas, .. } => {
                    canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(window, cx));
                }
                _ => {}
            }
        }
    }
}

pub(super) struct OpenDocument {
    pub(super) content: Content,
    read: u64,
    reading: bool,
    stamp: Option<Stamp>,
    image_hash: Option<u64>,
    ignored: bool,
    rendered: Option<Entity<MarkdownView>>,
    svg_canvas: Option<Entity<ImageCanvas>>,
    pub(super) svg_too_large: Option<(u32, u32, u64)>,
    pub(super) svg_meta: Option<(u32, u32, u64)>,
    pub(super) svg_invalid: bool,
    source: bool,
    /// Line cited by a ⌘-clicked terminal link (selects line & switches MD to source).
    line: Option<u32>,
    /// Line preserved when flipping `Diff → Code` before the working-tree file finishes reading.
    pending_code_line: Option<u32>,
    /// Line preserved when flipping `Code → Diff` before the diff finishes loading.
    pending_diff_line: Option<u32>,
    pub(super) mode: DocMode,
    /// Whether this tab was opened from Changes (so `Code | Diff` stays available for its target).
    pub(super) has_changes_target: bool,
    pub(super) diff: FileTabDiff,
    diff_read: u64,
    diff_reading: bool,
    pub(super) diff_base: Option<String>,
    pub(super) resolved_base: Option<String>,
    pub(super) branch_name: Option<String>,
    /// True once branch metadata was explicitly resolved (distinguishing detached HEAD `None`
    /// from an unresolved initial state so we never fall back to the original checkout branch).
    pub(super) branch_known: bool,
    pub(super) commit_short_id: Option<String>,
    pub(super) has_head: bool,
    pub(super) hint_status: Option<FileStatus>,
    pub(super) hint_counts: Option<DiffCounts>,
    pub(super) hint_viewed: Option<(bool, bool)>,
    pub(super) hint_old_blob_id: Option<String>,
    pub(super) hint_new_blob_id: Option<String>,
    /// 1-based new-file hunk start lines for `[` / `]` in Code mode (populated
    /// either from a loaded diff or a lightweight Code-mode diff-hunk fetch).
    hunk_lines: Vec<u32>,
    /// `(old_blob_id, new_blob_id)` that `hunk_lines` was computed for, so unrelated
    /// checkout edits do not re-fetch `changes_diff` for unchanged open Code tabs.
    hunk_blobs: Option<(Option<String>, Option<String>)>,
    hunk_read: u64,
    /// Set across reconnect (`FilesEvent::Resync`) or hidden intervals so stale
    /// pre-disconnect viewed hints are never reused without refreshing from daemon.
    viewed_stale: bool,
    /// `Some(true)` only when the file is absent on disk in the working tree;
    /// distinguishes historical commit deletions from currently missing files.
    working_tree_missing: Option<bool>,
    /// Newest viewed mutation/event `(rev, old_blob, new_blob, viewed, changed_since)`
    /// so an in-flight diff load cannot overwrite a newer viewed update for the same blobs.
    #[allow(clippy::type_complexity)]
    latest_viewed: Option<(u64, Option<String>, Option<String>, bool, bool)>,
}

impl OpenDocument {
    fn new_code(source: bool, has_head: bool) -> Self {
        Self {
            content: Content::Loading,
            read: 0,
            reading: false,
            stamp: None,
            image_hash: None,
            ignored: false,
            rendered: None,
            svg_canvas: None,
            svg_too_large: None,
            svg_meta: None,
            svg_invalid: false,
            source,
            line: None,
            pending_code_line: None,
            pending_diff_line: None,
            mode: DocMode::Code,
            has_changes_target: false,
            diff: FileTabDiff::Idle,
            diff_read: 0,
            diff_reading: false,
            diff_base: None,
            resolved_base: None,
            branch_name: None,
            branch_known: false,
            commit_short_id: None,
            has_head,
            hint_status: None,
            hint_counts: None,
            hint_viewed: None,
            hint_old_blob_id: None,
            hint_new_blob_id: None,
            hunk_lines: Vec::new(),
            hunk_blobs: None,
            hunk_read: 0,
            viewed_stale: false,
            working_tree_missing: None,
            latest_viewed: None,
        }
    }

    fn new_diff(req: &OpenChangesRequest, source: bool) -> Self {
        Self {
            content: Content::Idle,
            read: 0,
            reading: false,
            stamp: None,
            image_hash: None,
            ignored: false,
            rendered: None,
            svg_canvas: None,
            svg_too_large: None,
            svg_meta: None,
            svg_invalid: false,
            source,
            line: None,
            pending_code_line: None,
            pending_diff_line: None,
            mode: DocMode::Diff,
            has_changes_target: true,
            diff: FileTabDiff::Loading,
            diff_read: 0,
            diff_reading: false,
            diff_base: req.chosen_base.clone(),
            resolved_base: req.resolved_base.clone(),
            branch_name: req.branch_name.clone(),
            branch_known: true,
            commit_short_id: req.commit_short_id.clone(),
            has_head: req.has_head,
            hint_status: Some(req.file.status.clone()),
            hint_counts: Some(req.file.counts),
            hint_viewed: Some((req.file.viewed, req.file.changed_since_viewed)),
            hint_old_blob_id: req.file.old_blob_id.clone(),
            hint_new_blob_id: req.file.new_blob_id.clone(),
            hunk_lines: Vec::new(),
            hunk_blobs: None,
            hunk_read: 0,
            viewed_stale: false,
            working_tree_missing: (req.target == SelectedTarget::Uncommitted)
                .then_some(req.file.status == FileStatus::Deleted),
            latest_viewed: None,
        }
    }

    fn apply_changes_hints(&mut self, req: &OpenChangesRequest) {
        self.has_changes_target = true;
        self.diff_base = req.chosen_base.clone();
        self.resolved_base = req.resolved_base.clone();
        self.branch_name = req.branch_name.clone();
        self.branch_known = true;
        self.commit_short_id = req.commit_short_id.clone();
        self.has_head = req.has_head;
        self.hint_status = Some(req.file.status.clone());
        self.hint_counts = Some(req.file.counts);
        self.hint_viewed = Some((req.file.viewed, req.file.changed_since_viewed));
        self.hint_old_blob_id = req.file.old_blob_id.clone();
        self.hint_new_blob_id = req.file.new_blob_id.clone();
        self.viewed_stale = false;
        if req.target == SelectedTarget::Uncommitted {
            self.working_tree_missing = Some(req.file.status == FileStatus::Deleted);
        }
    }

    /// Whether the file is genuinely unavailable in the working tree (not merely
    /// deleted in an older historical commit that a later commit recreated).
    pub(super) fn is_working_tree_deleted(&self, key: &TabKey) -> bool {
        if matches!(self.content, Content::Deleted(_)) {
            return true;
        }
        if matches!(
            self.content,
            Content::Text(_)
                | Content::Image { .. }
                | Content::ImageTooLarge { .. }
                | Content::ImageFailed
                | Content::Binary { .. }
                | Content::TooLarge { .. }
        ) {
            return false;
        }
        if let Some(missing) = self.working_tree_missing {
            return missing;
        }
        if key.target == SelectedTarget::Uncommitted {
            match &self.diff {
                FileTabDiff::Loaded(loaded) => loaded.status == FileStatus::Deleted,
                _ => self.hint_status.as_ref() == Some(&FileStatus::Deleted),
            }
        } else {
            false
        }
    }

    fn drop_image_textures(&self, mut window: Option<&mut Window>, cx: &mut App) {
        match &self.content {
            Content::Image { canvas, .. }
            | Content::Deleted(Some(DeletedContent::Image { canvas, .. })) => {
                canvas.update(cx, |canvas, cx| {
                    canvas.drop_atlas_textures(window.as_deref_mut(), cx)
                });
            }
            _ => {}
        }
        if let Some(canvas) = &self.svg_canvas {
            canvas.update(cx, |canvas, cx| {
                canvas.drop_atlas_textures(window.as_deref_mut(), cx)
            });
        }
        self.diff.drop_image_textures(window, cx);
    }

    fn expected_base(&self, key: &TabKey) -> Option<&str> {
        matches!(key.target, SelectedTarget::Branch)
            .then_some(self.diff_base.as_deref())
            .flatten()
    }

    fn clear_uncommitted_diff_hints(&mut self) {
        self.has_changes_target = false;
        self.hint_status = None;
        self.hint_counts = None;
        self.hint_viewed = None;
        self.hint_old_blob_id = None;
        self.hint_new_blob_id = None;
        self.viewed_stale = false;
        self.working_tree_missing = None;
        self.hunk_lines.clear();
        self.hunk_blobs = None;
        self.hunk_read += 1;
        self.diff = FileTabDiff::Idle;
        self.diff_reading = false;
        self.diff_read += 1;
    }

    /// Atomically binds `entry`'s blob pair and viewed state to this document's hints
    /// (and loaded diff when blobs match). A newer viewed event/toggle supersedes the
    /// snapshot's mark ONLY when it is for the exact same `(old_blob_id, new_blob_id)`.
    fn apply_list_entry_viewed(
        &mut self,
        entry: &ChangedFile,
        started_rev: u64,
        viewed_revision: &mut u64,
        cx: &mut App,
    ) -> bool {
        let prev_old = self.hint_old_blob_id.clone();
        let prev_new = self.hint_new_blob_id.clone();
        let prev_viewed = self.hint_viewed;
        let was_stale = self.viewed_stale;

        self.hint_status = Some(entry.status.clone());
        self.hint_counts = Some(entry.counts);
        self.hint_old_blob_id = entry.old_blob_id.clone();
        self.hint_new_blob_id = entry.new_blob_id.clone();

        let newer_for_same_pair = self.latest_viewed.as_ref().and_then(
            |(rev, old_blob, new_blob, viewed, changed_since)| {
                (*rev > started_rev
                    && *old_blob == entry.old_blob_id
                    && *new_blob == entry.new_blob_id)
                    .then_some((*viewed, *changed_since))
            },
        );
        let newer_for_other_pair =
            self.latest_viewed
                .as_ref()
                .is_some_and(|(rev, old_blob, new_blob, ..)| {
                    *rev > started_rev
                        && (*old_blob != entry.old_blob_id || *new_blob != entry.new_blob_id)
                });

        let mut changed = false;
        if let Some((viewed, changed_since)) = newer_for_same_pair {
            self.hint_viewed = Some((viewed, changed_since));
            self.viewed_stale = false;
            if let FileTabDiff::Loaded(loaded) = &mut self.diff
                && loaded.old_blob_id == entry.old_blob_id
                && loaded.new_blob_id == entry.new_blob_id
            {
                loaded.set_viewed_state(viewed, changed_since, cx);
                changed = true;
            }
        } else if newer_for_other_pair {
            // A viewed mutation landed for another version (e.g. displayed A) while this
            // list snapshot for B was in flight: never attach A's mark to B's blob IDs,
            // and mark `viewed_stale` so B's diff load fetches B's post-mutation flags.
            self.hint_viewed = None;
            self.viewed_stale = true;
            changed = true;
        } else {
            let next_viewed = (entry.viewed, entry.changed_since_viewed);
            self.hint_viewed = Some(next_viewed);
            self.viewed_stale = false;
            if was_stale
                || prev_old != entry.old_blob_id
                || prev_new != entry.new_blob_id
                || prev_viewed != Some(next_viewed)
            {
                *viewed_revision += 1;
                self.latest_viewed = Some((
                    *viewed_revision,
                    entry.old_blob_id.clone(),
                    entry.new_blob_id.clone(),
                    entry.viewed,
                    entry.changed_since_viewed,
                ));
                changed = true;
            }
            if let FileTabDiff::Loaded(loaded) = &mut self.diff
                && loaded.old_blob_id == entry.old_blob_id
                && loaded.new_blob_id == entry.new_blob_id
            {
                loaded.set_viewed_state(entry.viewed, entry.changed_since_viewed, cx);
                changed = true;
            }
        }
        changed
    }
}

/// A Markdown file's page and the local images it shows.
type Rendered = (Arc<Page>, HashMap<String, Arc<gpui::Image>>);

type Stamp = (u64, Option<chrono::DateTime<chrono::Utc>>);

/// How often an open ignored file is read again: the watcher stays quiet about
/// ignored paths, where a build changes thousands.
pub(super) const IGNORED_REFRESH: std::time::Duration = std::time::Duration::from_secs(3);

struct Find {
    field: Entity<TextField>,
    query: FileQuery,
    found: Found,
    searched: Option<Arc<Document>>,
    current: Option<usize>,
    search: u64,
    _observe: Subscription,
}

fn subject_protocol(context: &PanelContext) -> FilesSubject {
    match &context.subject {
        PanelSubject::Session(id) => FilesSubject::Session(id.clone()),
        PanelSubject::Mission(id) => FilesSubject::Mission(id.clone()),
    }
}

fn is_markdown(path: &str) -> bool {
    super::highlight::grammar_for_path(path).is_some_and(|grammar| grammar.name == "markdown")
}

fn is_svg(path: &str) -> bool {
    markdown::image_format(path) == Some(gpui::ImageFormat::Svg)
}

fn raster_image_format(path: &str) -> Option<gpui::ImageFormat> {
    markdown::image_format(path).filter(|&fmt| fmt != gpui::ImageFormat::Svg)
}

/// `bytes` for people: `512 B`, `1.2 MB`, `214 MB`.
pub(crate) fn format_size(bytes: u64) -> String {
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

pub(crate) struct UnifiedViewer {
    context: PanelContext,
    client: DaemonClient,
    pub(crate) root_focus: FocusHandle,
    pub(crate) viewer_focus: FocusHandle,
    _focus_lost: Option<Subscription>,
    root: Option<FilesRoot>,
    generation: u64,
    shown: bool,
    rail_watches: bool,
    changes_rail_shown: bool,
    watched: Option<PathBuf>,
    watch_chain: Option<Task<()>>,
    ignored_refresh: Option<Task<()>>,
    status: Option<Arc<PreparedStatus>>,
    status_stale: bool,
    uncommitted_files: Option<HashMap<String, ChangedFile>>,
    uncommitted_seq: u64,
    non_uncommitted_seq: u64,
    pub(crate) tabs: FileTabs,
    pub(super) docs: HashMap<TabKey, OpenDocument>,
    pub(crate) tree_hidden: bool,
    pub(crate) widened: Option<Widened>,
    pub(crate) maximized: bool,
    pub(crate) on_request_maximize: Option<PanelMaximizeCallback>,
    pub(super) diff_mode_pref: DiffViewMode,
    pub(super) look: Option<super::Look>,
    owner: Option<EntityId>,
    reported: (Option<bool>, bool, bool),
    pending_restore: Option<SavedViewerState>,
    pub(crate) tree_resizing: bool,
    next_read: u64,
    next_diff_read: u64,
    next_hunk_read: u64,
    next_search: u64,
    viewed_revision: u64,
    find: Option<Find>,
    list_open: bool,
    open_in_open: bool,
    strip: ScrollHandle,
    list_anchor: Rc<Cell<Bounds<Pixels>>>,
    open_in_anchor: Rc<Cell<Bounds<Pixels>>>,
    chrome: ViewerChrome,
    _hub: Subscription,
    _settings: Subscription,
    #[cfg(test)]
    pub(super) resolve_gate: Option<futures::channel::oneshot::Receiver<()>>,
    #[cfg(test)]
    pub(super) hold_search: bool,
    #[cfg(test)]
    pub(super) hold_diffs: bool,
    #[cfg(test)]
    pub(super) held_diffs: Vec<futures::channel::oneshot::Sender<()>>,
    #[cfg(test)]
    pub(super) hold_uncommitted_lists: bool,
    #[cfg(test)]
    pub(super) held_uncommitted_lists: Vec<futures::channel::oneshot::Sender<()>>,
    #[cfg(test)]
    pub(super) hold_non_uncommitted_refreshes: bool,
    #[cfg(test)]
    pub(super) held_non_uncommitted_refreshes: Vec<futures::channel::oneshot::Sender<()>>,
    #[cfg(test)]
    pub(super) fail_next_viewed_write: Option<String>,
    #[cfg(test)]
    pub(super) launched: Vec<(Option<FileLinkEditor>, PathBuf, Option<u32>)>,
    #[cfg(test)]
    pub(super) opened_urls: Vec<String>,
}

impl UnifiedViewer {
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn new(context: PanelContext, cx: &mut Context<Self>) -> Self {
        #[cfg(test)]
        {
            Self::new_with_resolve_gate(context, None, cx)
        }
        #[cfg(not(test))]
        {
            Self::new_inner(context, cx)
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_resolve_gate(
        context: PanelContext,
        resolve_gate: Option<futures::channel::oneshot::Receiver<()>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_inner(context, resolve_gate, cx)
    }

    fn new_inner(
        context: PanelContext,
        #[cfg(test)] resolve_gate: Option<futures::channel::oneshot::Receiver<()>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let client = crate::app_store::global_app_store(cx)
            .read(cx)
            .client
            .clone();
        let hub = super::hub(&client, cx);
        let hub_subscription = cx.subscribe(&hub, |this, _: Entity<FilesHub>, event, cx| {
            this.on_files_event(event, cx)
        });
        let store = crate::app_store::global_app_store(cx);
        let settings_subscription = cx.observe(&store, |this, _, cx| this.settings_changed(cx));
        let mut viewer = Self {
            context,
            client,
            root_focus: cx.focus_handle(),
            viewer_focus: cx.focus_handle(),
            _focus_lost: None,
            root: None,
            generation: 0,
            shown: false,
            rail_watches: false,
            changes_rail_shown: false,
            watched: None,
            watch_chain: None,
            ignored_refresh: None,
            status: None,
            status_stale: true,
            uncommitted_files: None,
            uncommitted_seq: 0,
            non_uncommitted_seq: 0,
            tabs: FileTabs::default(),
            docs: HashMap::new(),
            tree_hidden: false,
            widened: None,
            maximized: false,
            on_request_maximize: None,
            diff_mode_pref: super::settings(cx)
                .map_or(DiffViewMode::Unified, |settings| settings.changes_diff_mode),
            look: super::Look::current(cx),
            owner: None,
            reported: (None, false, false),
            pending_restore: None,
            tree_resizing: false,
            next_read: 0,
            next_diff_read: 0,
            next_hunk_read: 0,
            next_search: 0,
            viewed_revision: 0,
            find: None,
            list_open: false,
            open_in_open: false,
            strip: ScrollHandle::new(),
            list_anchor: Rc::default(),
            open_in_anchor: Rc::default(),
            chrome: ViewerChrome::default(),
            _hub: hub_subscription,
            _settings: settings_subscription,
            #[cfg(test)]
            resolve_gate,
            #[cfg(test)]
            hold_search: false,
            #[cfg(test)]
            hold_diffs: false,
            #[cfg(test)]
            held_diffs: Vec::new(),
            #[cfg(test)]
            hold_uncommitted_lists: false,
            #[cfg(test)]
            held_uncommitted_lists: Vec::new(),
            #[cfg(test)]
            hold_non_uncommitted_refreshes: false,
            #[cfg(test)]
            held_non_uncommitted_refreshes: Vec::new(),
            #[cfg(test)]
            fail_next_viewed_write: None,
            #[cfg(test)]
            launched: Vec::new(),
            #[cfg(test)]
            opened_urls: Vec::new(),
        };
        viewer.resolve(cx);
        cx.on_release(|this: &mut Self, cx| {
            for doc in this.docs.values() {
                doc.drop_image_textures(None, cx);
            }
            let previous = this.watch_chain.take();
            let root = this.watched.take();
            let client = this.client.clone();
            cx.background_spawn(async move {
                if let Some(previous) = previous {
                    previous.await;
                }
                if let Some(root) = root {
                    let _ = client.files_unwatch(&root);
                }
            })
            .detach();
        })
        .detach();
        viewer
    }

    pub(crate) fn set_context(&mut self, context: PanelContext, cx: &mut Context<Self>) {
        if self.context == context {
            return;
        }
        let moved =
            self.context.remote != context.remote || self.context.project != context.project;
        self.context = context;
        if moved {
            self.resolve(cx);
        }
        cx.notify();
    }

    pub(crate) fn set_owner(&mut self, owner: Option<EntityId>) {
        self.owner = owner;
    }

    pub(crate) fn set_chrome(&mut self, chrome: ViewerChrome) {
        self.chrome = chrome;
    }

    pub(super) fn notify_owner(&self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
    }

    pub(crate) fn set_widened(&mut self, widened: Option<Widened>, cx: &mut Context<Self>) {
        if self.widened != widened {
            self.widened = widened;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        if self.maximized != maximized {
            self.maximized = maximized;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_on_request_maximize(&mut self, handler: Option<PanelMaximizeCallback>) {
        self.on_request_maximize = handler;
    }

    pub(crate) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    pub(crate) fn widening(&self) -> Option<bool> {
        if self.context.remote {
            return None;
        }
        if self.has_tabs() {
            return Some(!self.tree_hidden);
        }
        if self.root.is_none() {
            return self
                .pending_restore
                .as_ref()
                .filter(|saved| !saved.tabs.is_empty())
                .map(|saved| !saved.tree_hidden);
        }
        None
    }

    pub(crate) fn root_path(&self) -> Option<&Path> {
        self.root.as_ref().map(|r| r.path.as_path())
    }

    pub(crate) fn changes_root_is_git(&self) -> Option<bool> {
        if self.context.remote {
            return Some(true);
        }
        self.root.as_ref().map(|root| root.git.is_some())
    }

    pub(crate) fn root_has_head(&self) -> bool {
        self.root
            .as_ref()
            .and_then(|r| r.git.as_ref())
            .is_some_and(|g| g.commit.is_some())
    }

    pub(crate) fn has_dirty_changes(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.status.changed > 0)
    }

    /// Authoritative live dirty flag while `UnifiedViewer` itself owns the watch
    /// (e.g., mission on Sessions or rail hidden while viewing a document).
    pub(crate) fn fresh_dirty_changes(&self) -> Option<bool> {
        if !self.shown || self.rail_watches || self.status_stale || self.watched.is_none() {
            return None;
        }
        match &self.root {
            Some(root) if root.git.is_some() => self.status.as_ref().map(|s| s.status.changed > 0),
            Some(_) => Some(false),
            None => None,
        }
    }

    pub(crate) fn has_uncommitted_diff(&self, path: &str) -> Option<bool> {
        let Some(root) = &self.root else {
            return Some(false);
        };
        if root.git.is_none() {
            return Some(false);
        }
        self.uncommitted_files
            .as_ref()
            .map(|files| files.contains_key(path))
    }
}

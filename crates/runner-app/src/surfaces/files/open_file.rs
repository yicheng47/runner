//! Unified File + Diff viewer (#1421): shared per-subject tab strip (pinned
//! tabs + single italic preview tab), in-place `Code | Diff` mode switch,
//! Markdown rendered/source, in-file find, `Open in`, live `Viewed` sync, and
//! scoped fallback watch when neither `Files` nor `Changes` rail is mounted.

mod find;
mod open_in;
mod render;

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
use super::{FilesEvent, FilesHub};
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
pub(crate) use open_in::{editor_icon, found_editors};
pub(super) use open_in::{reveal, reveal_label};

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
        if let LoadedDiffKind::Unified(entity) = &self.kind {
            entity.update(cx, |v, cx| {
                v.set_viewed_state(viewed, changed_since_viewed, cx);
            });
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
            canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(window, cx));
        }
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

#[derive(Clone, Debug)]
pub(crate) struct SavedViewerTab {
    pub(crate) key: TabKey,
    pub(crate) preview: bool,
    pub(crate) mode: DocMode,
    pub(crate) has_changes_target: bool,
    pub(crate) diff_base: Option<String>,
    pub(crate) resolved_base: Option<String>,
    pub(crate) branch_name: Option<String>,
    pub(crate) commit_short_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SavedViewerState {
    pub(crate) root: PathBuf,
    pub(crate) tabs: Vec<SavedViewerTab>,
    pub(crate) active: Option<TabKey>,
    pub(crate) tree_hidden: bool,
}

/// A Markdown file's page and the local images it shows.
type Rendered = (Arc<Page>, HashMap<String, Arc<gpui::Image>>);

type Stamp = (u64, Option<chrono::DateTime<chrono::Utc>>);

enum Read {
    Text(
        Arc<Document>,
        Option<Rendered>,
        Option<DecodeOutcome>,
        Stamp,
        Option<u64>,
    ),
    Image(DecodedImage, Stamp, u64),
    ImageTooLarge {
        width: u32,
        height: u32,
        size: u64,
        stamp: Stamp,
        hash: u64,
    },
    ImageFailed {
        stamp: Stamp,
        hash: u64,
    },
    Unchanged(Option<Stamp>),
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

fn stamp_filter(since: Option<Stamp>) -> Option<FileStamp> {
    since.and_then(|(size, modified)| {
        modified.map(|modified| FileStamp {
            size,
            modified: Some(modified),
        })
    })
}

fn read_raster_file(
    client: &DaemonClient,
    root: &Path,
    relative: &str,
    format: gpui::ImageFormat,
    since: Option<Stamp>,
    prev_hash: Option<u64>,
) -> Result<Read, ClientError> {
    let content = if let Some((prev_size, prev_modified)) = since {
        match client.files_read(root, relative, false, stamp_filter(since))? {
            FileContent::Unchanged => return Ok(Read::Unchanged(None)),
            FileContent::Binary {
                size,
                modified: Some(modified),
                ..
            } if prev_size == size && prev_modified == Some(modified) => {
                return Ok(Read::Unchanged(None));
            }
            FileContent::Binary { .. } => client.files_read(root, relative, true, None)?,
            FileContent::Text {
                size,
                modified: Some(modified),
                ..
            } if prev_size == size && prev_modified == Some(modified) => {
                return Ok(Read::Unchanged(None));
            }
            other => other,
        }
    } else {
        client.files_read(root, relative, true, None)?
    };

    Ok(match content {
        FileContent::Unchanged => Read::Unchanged(None),
        FileContent::Binary {
            size,
            bytes: Some(bytes),
            modified,
        } => {
            let hash = image::bytes_hash(&bytes.0);
            let stamp = (size, modified);
            if prev_hash == Some(hash) {
                return Ok(Read::Unchanged(Some(stamp)));
            }
            match image::decode_raster_image(format, &bytes.0, size) {
                DecodeOutcome::Ready(decoded) => Read::Image(decoded, stamp, hash),
                DecodeOutcome::TooManyPixels {
                    width,
                    height,
                    size,
                } => Read::ImageTooLarge {
                    width,
                    height,
                    size,
                    stamp,
                    hash,
                },
                DecodeOutcome::Failed => Read::ImageFailed { stamp, hash },
            }
        }
        FileContent::Binary { size, .. } => Read::Binary(size),
        FileContent::Text {
            bytes,
            size,
            modified,
        } => {
            let hash = image::bytes_hash(&bytes.0);
            let stamp = (size, modified);
            if let Some(detected) = image::detect_raster_magic(&bytes.0) {
                if prev_hash == Some(hash) {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                match image::decode_raster_image(detected, &bytes.0, size) {
                    DecodeOutcome::Ready(decoded) => Read::Image(decoded, stamp, hash),
                    DecodeOutcome::TooManyPixels {
                        width,
                        height,
                        size,
                    } => Read::ImageTooLarge {
                        width,
                        height,
                        size,
                        stamp,
                        hash,
                    },
                    DecodeOutcome::Failed if std::str::from_utf8(&bytes.0).is_err() => {
                        Read::ImageFailed { stamp, hash }
                    }
                    DecodeOutcome::Failed => {
                        let document = Arc::new(Document::decode(&bytes.0));
                        Read::Text(document, None, None, stamp, None)
                    }
                }
            } else if std::str::from_utf8(&bytes.0).is_err() {
                if prev_hash == Some(hash) {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                Read::ImageFailed { stamp, hash }
            } else {
                let document = Arc::new(Document::decode(&bytes.0));
                Read::Text(document, None, None, stamp, None)
            }
        }
        FileContent::TooLarge { size } => Read::TooLarge(size),
        FileContent::Missing => Read::Missing,
    })
}

fn read_text_or_svg_file(
    client: &DaemonClient,
    root: &Path,
    relative: &str,
    markdown: bool,
    svg: bool,
    since: Option<Stamp>,
    prev_hash: Option<u64>,
) -> Result<Read, ClientError> {
    Ok(
        match client.files_read(root, relative, false, stamp_filter(since))? {
            FileContent::Unchanged => Read::Unchanged(None),
            FileContent::Text { size, modified, .. }
                if since == Some((size, modified)) && (!svg || modified.is_some()) =>
            {
                Read::Unchanged(None)
            }
            FileContent::Text {
                bytes,
                size,
                modified,
            } => {
                let stamp = (size, modified);
                let svg_hash = svg.then(|| image::bytes_hash(&bytes.0));
                if svg && prev_hash.is_some() && prev_hash == svg_hash {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                let document = Arc::new(Document::decode(&bytes.0));
                let rendered = markdown.then(|| {
                    let page = Page::parse(document.text(), relative);
                    let images = page
                        .images()
                        .into_iter()
                        .filter_map(|image| {
                            let decoded = read_image(client, root, &image)?;
                            Some((image, decoded))
                        })
                        .collect();
                    (Arc::new(page), images)
                });
                let svg_decoded = svg.then(|| image::decode_svg_image(&bytes.0, size));
                Read::Text(document, rendered, svg_decoded, stamp, svg_hash)
            }
            FileContent::Binary { size, .. } => Read::Binary(size),
            FileContent::TooLarge { size } => Read::TooLarge(size),
            FileContent::Missing => Read::Missing,
        },
    )
}

fn read_image(client: &DaemonClient, root: &Path, path: &str) -> Option<Arc<gpui::Image>> {
    let format = markdown::image_format(path)?;
    let bytes = match client.files_read(root, path, true, None).ok()? {
        FileContent::Text { bytes, .. } => bytes.0,
        FileContent::Binary {
            bytes: Some(bytes), ..
        } => bytes.0,
        _ => return None,
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
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
    pub(crate) fn new(context: PanelContext, cx: &mut Context<Self>) -> Self {
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

    // ── Root & Watch Management ────────────────────────────────────────

    fn request<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) -> Result<T, ClientError> + Send + 'static,
        then: impl FnOnce(&mut Self, Result<T, ClientError>, &mut Context<Self>) + 'static,
    ) {
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move { call(&client) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    then(this, result, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.root = None;
            self.reset_tabs(None, cx);
            self.sync_watch(cx);
            cx.notify();
            return;
        }
        let subject = subject_protocol(&self.context);
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| {
                if let Ok(outcome) = result {
                    this.apply_root_outcome(outcome, cx);
                }
            },
        );
    }

    pub(crate) fn sync_root(&mut self, outcome: FilesRootOutcome, cx: &mut Context<Self>) {
        match (&outcome, &self.root) {
            (FilesRootOutcome::Root(next), Some(current))
                if runner_core::app_paths::same_checkout_path(&current.path, &next.path) =>
            {
                if current != next {
                    self.root = Some(next.clone());
                    cx.notify();
                }
                return;
            }
            (FilesRootOutcome::ChooseFolder | FilesRootOutcome::Missing { .. }, None)
                if self.pending_restore.is_none() && self.tabs.is_empty() =>
            {
                return;
            }
            _ => {}
        }
        self.apply_root_outcome(outcome, cx);
    }

    fn apply_root_outcome(&mut self, outcome: FilesRootOutcome, cx: &mut Context<Self>) {
        let same = matches!(
            (&outcome, &self.root),
            (FilesRootOutcome::Root(next), Some(current))
                if runner_core::app_paths::same_checkout_path(&current.path, &next.path)
        );
        if !same {
            self.generation += 1;
        }
        match outcome {
            FilesRootOutcome::Root(next_root) => {
                if same {
                    self.root = Some(next_root);
                    self.read_again_after_resolve(cx);
                    self.refresh_uncommitted_paths(true, cx);
                } else {
                    self.reset_tabs(None, cx);
                    self.status = None;
                    self.status_stale = true;
                    self.uncommitted_files = None;
                    self.root = Some(next_root);
                    if let Some(saved) = self.pending_restore.take() {
                        self.apply_saved_state(saved, cx);
                    }
                    self.notify_owner(cx);
                }
            }
            FilesRootOutcome::ChooseFolder | FilesRootOutcome::Missing { .. } => {
                self.pending_restore = None;
                self.root = None;
                self.status = None;
                self.uncommitted_files = None;
                self.reset_tabs(None, cx);
                self.notify_owner(cx);
            }
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    pub(crate) fn set_status_snapshot(
        &mut self,
        root: &Path,
        status: Arc<PreparedStatus>,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_some_and(|r| runner_core::app_paths::same_checkout_path(r, root))
        {
            self.apply_status(status, cx);
        }
    }

    pub(crate) fn set_shown(
        &mut self,
        shown: bool,
        rail_watches: bool,
        changes_rail_shown: bool,
        cx: &mut Context<Self>,
    ) {
        let was_shown = self.shown;
        if self.shown == shown
            && self.rail_watches == rail_watches
            && self.changes_rail_shown == changes_rail_shown
        {
            return;
        }
        self.shown = shown;
        self.rail_watches = rail_watches;
        self.changes_rail_shown = changes_rail_shown;
        if !shown {
            self.list_open = false;
            self.open_in_open = false;
            self.status_stale = true;
            self.ignored_refresh = None;
            for doc in self.docs.values() {
                doc.drop_image_textures(None, cx);
            }
        } else {
            if self.ignored_refresh.is_none() && self.has_tabs() {
                self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
            }
            if !was_shown && self.has_tabs() {
                for doc in self.docs.values_mut() {
                    doc.viewed_stale = true;
                }
                if rail_watches
                    && changes_rail_shown
                    && self.status_stale
                    && let Some(root) = self.root_path().map(PathBuf::from)
                {
                    self.request(
                        cx,
                        move |client| client.files_status(&root).map(PreparedStatus::new),
                        |this, result, cx| {
                            if let Ok(status) = result {
                                this.apply_status(status, cx);
                            }
                        },
                    );
                }
                self.reload_changed(None, cx);
                self.refresh_uncommitted_paths(true, cx);
                self.refresh_non_uncommitted_targets(false, cx);
            }
        }
        self.sync_watch(cx);
    }

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        let same = match (self.watched.as_deref(), self.root_path()) {
            (Some(a), Some(b)) => runner_core::app_paths::same_checkout_path(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.unwatch(cx);
        }
        self.sync_watch(cx);
    }

    fn chain_watch(
        &mut self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) + Send + 'static,
    ) {
        let previous = self.watch_chain.take();
        let client = self.client.clone();
        self.watch_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            call(&client);
        }));
    }

    fn unwatch(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.watched.take() {
            self.chain_watch(cx, move |client| {
                let _ = client.files_unwatch(&root);
            });
        }
    }

    fn sync_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = (self.shown && self.has_tabs() && !self.rail_watches)
            .then(|| self.root_path().map(Path::to_path_buf))
            .flatten();
        let same = match (self.watched.as_deref(), wanted.as_deref()) {
            (Some(a), Some(b)) => runner_core::app_paths::same_checkout_path(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        self.watched = Some(root.clone());
        let (status_tx, status_rx) = futures::channel::oneshot::channel();
        self.chain_watch(cx, move |client| {
            let _ = status_tx.send(
                client
                    .files_watch(&root)
                    .and_then(|()| client.files_status(&root))
                    .map(PreparedStatus::new),
            );
        });
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let Ok(Ok(status)) = status_rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.watched.is_some() {
                    this.apply_status(status, cx);
                    this.notify_owner(cx);
                }
            });
        })
        .detach();
    }

    fn apply_status(&mut self, status: Arc<PreparedStatus>, cx: &mut Context<Self>) {
        if self
            .status
            .as_ref()
            .is_some_and(|cur| Arc::ptr_eq(cur, &status))
        {
            self.status_stale = false;
            return;
        }
        self.status = Some(status);
        self.status_stale = false;
        if self.has_tabs() {
            self.note_ignored_tabs();
            self.reconcile_diff_tabs(false, None, false, cx);
            self.refresh_uncommitted_paths(true, cx);
            self.refresh_non_uncommitted_targets(true, cx);
        }
        cx.notify();
    }

    pub(crate) fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if !self.shown || !self.has_tabs() {
                    return;
                }
                if self
                    .root_path()
                    .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, &changed.root))
                {
                    return;
                }
                self.reload_changed(Some(changed), cx);
                self.refresh_uncommitted_paths(false, cx);
            }
            FilesEvent::Status { root, status } => {
                if self.shown
                    && self
                        .root_path()
                        .is_some_and(|r| runner_core::app_paths::same_checkout_path(r, root))
                {
                    self.apply_status(status.clone(), cx);
                    if self.watched.is_some() {
                        self.notify_owner(cx);
                    }
                }
            }
            FilesEvent::Viewed(viewed) => {
                if self.shown {
                    self.apply_viewed_event(viewed, cx);
                }
            }
            FilesEvent::RootChanged { subject, origin } => {
                if &self.context.subject == subject && cx.entity_id() != *origin {
                    self.resolve(cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if self.context.remote {
                    return;
                }
                if self.root.is_none() {
                    self.resolve(cx);
                    return;
                }
                for doc in self.docs.values_mut() {
                    doc.viewed_stale = true;
                }
                if !self.shown {
                    return;
                }
                if *rewatch {
                    self.watched = None;
                    self.sync_watch(cx);
                } else if let Some(root) = self.watched.clone() {
                    self.request(
                        cx,
                        move |client| client.files_status(&root).map(PreparedStatus::new),
                        |this, result, cx| {
                            if let Ok(status) = result {
                                this.apply_status(status, cx);
                            }
                        },
                    );
                }
                self.reload_changed(None, cx);
                self.refresh_uncommitted_paths(true, cx);
                self.refresh_non_uncommitted_targets(false, cx);
            }
        }
    }

    // ── Settings & Resize ──────────────────────────────────────────────

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let diff_mode = super::settings(cx)
            .map_or(DiffViewMode::Unified, |settings| settings.changes_diff_mode);
        if diff_mode != self.diff_mode_pref {
            self.diff_mode_pref = diff_mode;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
        let look = super::Look::current(cx);
        if look == self.look {
            return;
        }
        self.look = look;
        for doc in self.docs.values() {
            match &doc.content {
                Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => {
                    view.update(cx, |_, cx| cx.notify());
                }
                Content::Image { canvas, .. }
                | Content::Deleted(Some(DeletedContent::Image { canvas, .. })) => {
                    canvas.update(cx, |_, cx| cx.notify());
                }
                _ => {}
            }
            if let Some(rendered) = &doc.rendered {
                rendered.update(cx, |_, cx| cx.notify());
            }
            if let Some(canvas) = &doc.svg_canvas {
                canvas.update(cx, |_, cx| cx.notify());
            }
            if let FileTabDiff::Loaded(loaded) = &doc.diff
                && let LoadedDiffKind::Unified(diff) = &loaded.kind
            {
                diff.update(cx, |_, cx| cx.notify());
            }
        }
        cx.notify();
    }

    pub(crate) fn diff_width(&self) -> f32 {
        self.widened
            .map_or(crate::app_settings::FILE_VIEWER_DEFAULT, |w| w.file)
    }

    pub(super) fn effective_diff_mode(&self, cx: &App) -> DiffViewMode {
        let preferred = super::settings(cx).map_or(self.diff_mode_pref, |s| s.changes_diff_mode);
        side_panel::effective_diff_mode(preferred, self.diff_width(), self.maximized)
    }

    fn sync_effective_diff_mode(&self, cx: &mut Context<Self>) {
        let mode = self.effective_diff_mode(cx);
        for doc in self.docs.values() {
            if let FileTabDiff::Loaded(loaded) = &doc.diff
                && let LoadedDiffKind::Unified(diff) = &loaded.kind
            {
                diff.update(cx, |v, cx| v.set_mode(mode, cx));
            }
        }
    }

    pub(super) fn select_diff_mode(
        &mut self,
        mode: DiffViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diff_mode_pref = mode;
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_diff_mode != mode;
                    settings.changes_diff_mode = mode;
                    changed
                },
                true,
                cx,
            );
        });
        if !self.root_focus.contains_focused(window, cx) {
            self.focus_active(window, cx);
        }
        if mode == DiffViewMode::Split
            && !self.maximized
            && self.diff_width() < side_panel::SPLIT_MIN_DIFF_WIDTH
            && let Some(request) = self.on_request_maximize.clone()
        {
            self.maximized = true;
            request(true, window, cx);
        }
        self.sync_effective_diff_mode(cx);
        cx.notify();
    }

    /// Sizes the right rail `tree` wide from dragging the inner divider, trading
    /// width with the unified viewer so the total panel width stays constant across
    /// Files, Changes, and Sessions (#1421).
    pub(crate) fn resize_tree(&mut self, tree: f32, cx: &mut Context<Self>) {
        let Some(widened) = self.widened.filter(|w| w.tree) else {
            return;
        };
        self.tree_resizing = true;
        if self.maximized {
            let total = widened.file + widened.tree_width;
            let tree = crate::app_settings::clamp_file_tree_width(tree)
                .min((total - widened.standard).max(crate::app_settings::FILE_TREE_MIN));
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed = settings.file_tree_width != tree;
                        settings.file_tree_width = tree;
                        changed
                    },
                    false,
                    cx,
                );
            });
        } else {
            let (file, tree) = widened.resize_tree(tree);
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed =
                            settings.file_tree_width != tree || settings.file_viewer_width != file;
                        settings.file_tree_width = tree;
                        settings.file_viewer_width = file;
                        changed
                    },
                    false,
                    cx,
                );
            });
        }
        self.notify_owner(cx);
        cx.notify();
    }

    pub(crate) fn finish_tree_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tree_resizing) {
            crate::app_store::global_app_store(cx)
                .read(cx)
                .save_settings();
            cx.notify();
        }
    }

    // ── Opening, Switching, Pinning & Closing Tabs ─────────────────────

    pub(crate) fn open_pinned_file(
        &mut self,
        path: String,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.tabs.open_pinned(&path);
        let key = outcome.key().clone();
        self.ensure_uncommitted_paths(cx);
        let source = !is_svg(&key.path)
            && super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let has_head = self.root_has_head();
        let need_read = if outcome.is_new() || !self.docs.contains_key(&key) {
            self.docs
                .insert(key.clone(), OpenDocument::new_code(source, has_head));
            true
        } else if let Some(doc) = self.docs.get_mut(&key) {
            doc.mode = DocMode::Code;
            matches!(doc.content, Content::Idle)
        } else {
            false
        };
        if let Some(line) = line
            && let Some(doc) = self.docs.get_mut(&key)
        {
            doc.line = Some(line);
            doc.mode = DocMode::Code;
            if is_markdown(&key.path) || is_svg(&key.path) {
                doc.source = true;
            }
        }
        self.note_ignored_tabs();
        if need_read {
            self.read_file(key.clone(), cx);
        } else if line.is_some() {
            self.show_line(&key, cx);
        }
        if self.has_uncommitted_diff(&key.path) == Some(true) {
            self.load_code_hunks(key.clone(), cx);
        }
        if self.shown && self.ignored_refresh.is_none() {
            self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
        }
        self.sync_watch(cx);
        self.tab_shown(cx);
    }

    pub(crate) fn open_changes_file(
        &mut self,
        req: OpenChangesRequest,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        // Ensure our root matches the Changes rail before opening.
        let same_root = self
            .root
            .as_ref()
            .is_some_and(|r| runner_core::app_paths::same_checkout_path(&r.path, &req.root.path));
        if !same_root {
            self.apply_root_outcome(FilesRootOutcome::Root(req.root.clone()), cx);
        } else {
            self.root = Some(req.root.clone());
        }
        let outcome = self.tabs.open_preview(&req.path, req.target.clone());
        if pin && let Some(active_idx) = self.tabs.active_index() {
            self.tabs.pin(active_idx);
        }
        let source = !is_svg(&req.path)
            && super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let key = outcome.key().clone();
        if req.target == SelectedTarget::Uncommitted
            && let Some(files) = &mut self.uncommitted_files
        {
            files.insert(req.path.clone(), req.file.clone());
        }
        if req.target == SelectedTarget::Branch {
            self.non_uncommitted_seq += 1;
        }
        let need_diff = match outcome {
            OpenPreviewOutcome::ReplacedPreview { replaced, .. } => {
                if self.tabs.position_key(&replaced).is_none()
                    && let Some(old_doc) = self.docs.remove(&replaced)
                {
                    old_doc.drop_image_textures(None, cx);
                }
                self.docs
                    .insert(key.clone(), OpenDocument::new_diff(&req, source));
                true
            }
            OpenPreviewOutcome::CreatedPreview(_) => {
                self.docs
                    .insert(key.clone(), OpenDocument::new_diff(&req, source));
                true
            }
            OpenPreviewOutcome::FocusedPinned(_) | OpenPreviewOutcome::AlreadyPreview(_) => {
                if let Some(doc) = self.docs.get_mut(&key) {
                    let prev_base = doc.expected_base(&key).map(str::to_owned);
                    let prev_old = doc.hint_old_blob_id.clone();
                    let prev_new = doc.hint_new_blob_id.clone();
                    let expected_base = matches!(key.target, SelectedTarget::Branch)
                        .then_some(req.chosen_base.as_deref())
                        .flatten();
                    let base_changed = matches!(key.target, SelectedTarget::Branch)
                        && prev_base.as_deref() != expected_base;
                    let blobs_changed =
                        prev_old != req.file.old_blob_id || prev_new != req.file.new_blob_id;
                    let matches_loaded = matches!(
                        &doc.diff,
                        FileTabDiff::Loaded(loaded)
                            if loaded.target == key.target
                                && loaded.base.as_deref() == expected_base
                                && loaded.old_blob_id == req.file.old_blob_id
                                && loaded.new_blob_id == req.file.new_blob_id
                    );
                    doc.apply_changes_hints(&req);
                    doc.mode = DocMode::Diff;
                    if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                        && matches_loaded
                    {
                        loaded.set_viewed_state(req.file.viewed, req.file.changed_since_viewed, cx);
                        false
                    } else {
                        if base_changed {
                            doc.diff = FileTabDiff::Loading;
                            doc.hunk_lines.clear();
                            doc.hunk_blobs = None;
                        }
                        !doc.diff_reading
                            || base_changed
                            || blobs_changed
                            || matches!(
                                doc.diff,
                                FileTabDiff::Idle | FileTabDiff::NoChanges | FileTabDiff::Failed(_)
                            )
                    }
                } else {
                    self.docs
                        .insert(key.clone(), OpenDocument::new_diff(&req, source));
                    true
                }
            }
        };
        if need_diff {
            self.load_tab_diff(key, cx);
        }
        if self.shown && self.ignored_refresh.is_none() {
            self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
        }
        self.sync_watch(cx);
        self.tab_shown(cx);
    }

    /// Keeps any open `All changes` (`SelectedTarget::Branch`) tabs in sync when the
    /// Changes rail changes base — including while in `Code` mode, while viewing another
    /// target, or when a base change collapses `All changes` down to a single commit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_branch_base(
        &mut self,
        root: &Path,
        chosen_base: Option<&str>,
        resolved_base: Option<&str>,
        branch_name: Option<&str>,
        has_head: bool,
        reload_now: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        // Supersede any in-flight status-triggered branch refresh that captured an older base.
        self.non_uncommitted_seq += 1;
        let branch_keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.key.target == SelectedTarget::Branch)
            .map(|t| t.key.clone())
            .collect();
        let mut to_reload = Vec::new();
        let mut to_load_hunks = Vec::new();
        let mut changed_any = false;
        for key in branch_keys {
            let Some(doc) = self.docs.get_mut(&key) else {
                continue;
            };
            let prev_base = doc.expected_base(&key).map(str::to_owned);
            let base_changed = prev_base.as_deref() != chosen_base
                || (chosen_base.is_none()
                    && resolved_base.is_some()
                    && doc.resolved_base.as_deref() != resolved_base);
            let pending_diff = doc.mode == DocMode::Diff
                && !doc.diff_reading
                && matches!(doc.diff, FileTabDiff::Loading | FileTabDiff::Idle);
            doc.diff_base = chosen_base.map(str::to_owned);
            if resolved_base.is_some() {
                doc.resolved_base = resolved_base.map(str::to_owned);
            }
            doc.branch_name = branch_name.map(str::to_owned);
            doc.branch_known = true;
            doc.has_head = has_head;
            if base_changed {
                changed_any = true;
                doc.hint_status = None;
                doc.hint_counts = None;
                doc.hint_viewed = None;
                doc.hint_old_blob_id = None;
                doc.hint_new_blob_id = None;
                doc.viewed_stale = false;
                doc.hunk_lines.clear();
                doc.hunk_blobs = None;
                doc.hunk_read += 1;
                doc.diff_reading = false;
                doc.diff_read += 1;
                if doc.mode == DocMode::Diff {
                    doc.diff = FileTabDiff::Loading;
                    if reload_now {
                        to_reload.push(key);
                    }
                } else {
                    doc.diff = FileTabDiff::Idle;
                    if reload_now {
                        to_load_hunks.push(key);
                    }
                }
            } else if reload_now && pending_diff {
                // Complete an invalidation queued by `pick_base` when the follow-up snapshot
                // collapsed `All changes` (e.g. 0 commits ahead -> Uncommitted).
                to_reload.push(key);
            }
        }
        for key in to_reload {
            self.load_tab_diff(key, cx);
        }
        for key in to_load_hunks {
            self.load_code_hunks(key, cx);
        }
        if changed_any {
            cx.notify();
        }
    }

    /// Synchronizes open tabs with a refreshed file list from the Changes rail:
    /// updates `viewed` / `changed_since_viewed` / base metadata, reloads diffs
    /// whose blobs or base changed, and reconciles a live preview tab when its
    /// file disappears while `Changes` is active.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_changes_files(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        chosen_base: Option<&str>,
        resolved_base: Option<&str>,
        branch_name: Option<&str>,
        has_head: bool,
        files: &[ChangedFile],
        list_rows: &[ListRow],
        tree_mode: bool,
        reload_same_diff: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        let expected_base = matches!(target, SelectedTarget::Branch)
            .then_some(chosen_base)
            .flatten();
        if *target == SelectedTarget::Uncommitted {
            self.uncommitted_seq += 1;
            self.uncommitted_files = Some(
                files
                    .iter()
                    .map(|entry| (entry.path.clone(), entry.clone()))
                    .collect(),
            );
        } else if *target == SelectedTarget::Branch {
            // Supersede any in-flight status-triggered branch refresh that captured an older scope.
            self.non_uncommitted_seq += 1;
        }

        // 1. Only reconcile a live Changes preview tab's disappearance while Changes
        //    is already active — switching rails to Changes must never close or replace
        //    an open viewer document, and an empty Changes list must never collapse the
        //    active tab away without an explicit close action.
        if allow_preview_replace
            && self.changes_rail_shown
            && let Some(preview_idx) = self.tabs.preview_index()
        {
            let preview_key = self.tabs.tabs()[preview_idx].key.clone();
            if &preview_key.target == target && !files.iter().any(|f| f.path == preview_key.path) {
                let preview_was_active = self.tabs.active_index() == Some(preview_idx);
                if !preview_was_active {
                    // Never yank focus away from another tab the user is actively reading.
                    if let Some(closed) = self.tabs.close(preview_idx)
                        && let Some(old_doc) = self.docs.remove(&closed)
                    {
                        old_doc.drop_image_textures(None, cx);
                    }
                } else {
                    let replacement = reconcile_file_selection(
                        Some(&preview_key.path),
                        files,
                        list_rows,
                        tree_mode,
                    );
                    if let Some(next_path) = replacement
                        && let Some(next_file) = files.iter().find(|f| f.path == next_path)
                        && let Some(root_obj) = self.root.clone()
                    {
                        let next_key = TabKey::new(&next_path, target.clone());
                        // If the replacement is already pinned, remove the orphaned preview first
                        // so `open_changes_file` focuses the pinned tab cleanly with no orphan left.
                        if self.tabs.position_key(&next_key).is_some()
                            && let Some(closed) = self.tabs.close(preview_idx)
                            && let Some(old_doc) = self.docs.remove(&closed)
                        {
                            old_doc.drop_image_textures(None, cx);
                        }
                        let req = OpenChangesRequest {
                            root: root_obj,
                            path: next_path,
                            target: target.clone(),
                            chosen_base: chosen_base.map(str::to_owned),
                            resolved_base: resolved_base.map(str::to_owned),
                            branch_name: branch_name.map(str::to_owned),
                            commit_short_id: match target {
                                SelectedTarget::Commit(id) => {
                                    Some(id[..id.len().min(7)].to_owned())
                                }
                                _ => None,
                            },
                            file: next_file.clone(),
                            has_head,
                        };
                        self.open_changes_file(req, false, cx);
                    }
                }
            }
        }

        // 2. Sync existing open tabs matching this target.
        let matching_keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| &t.key.target == target)
            .map(|t| t.key.clone())
            .collect();
        let mut to_reload = Vec::new();
        let mut to_read = Vec::new();
        let mut to_load_hunks = Vec::new();
        for key in matching_keys {
            if let Some(file) = files.iter().find(|f| f.path == key.path) {
                if let Some(doc) = self.docs.get_mut(&key) {
                    let prev_base = doc.expected_base(&key).map(str::to_owned);
                    let prev_old = doc.hint_old_blob_id.clone();
                    let prev_new = doc.hint_new_blob_id.clone();
                    let prev_viewed = doc.hint_viewed;
                    let base_changed = matches!(key.target, SelectedTarget::Branch)
                        && prev_base.as_deref() != expected_base;
                    let blobs_changed =
                        prev_old != file.old_blob_id || prev_new != file.new_blob_id;
                    let next_viewed = (file.viewed, file.changed_since_viewed);
                    doc.diff_base = chosen_base.map(str::to_owned);
                    doc.resolved_base = resolved_base.map(str::to_owned);
                    doc.branch_name = branch_name.map(str::to_owned);
                    doc.branch_known = true;
                    doc.has_head = has_head;
                    doc.hint_status = Some(file.status.clone());
                    doc.hint_counts = Some(file.counts);
                    doc.hint_viewed = Some(next_viewed);
                    doc.hint_old_blob_id = file.old_blob_id.clone();
                    doc.hint_new_blob_id = file.new_blob_id.clone();
                    doc.viewed_stale = false;
                    if blobs_changed || prev_viewed != Some(next_viewed) {
                        self.viewed_revision += 1;
                        doc.latest_viewed = Some((
                            self.viewed_revision,
                            file.old_blob_id.clone(),
                            file.new_blob_id.clone(),
                            file.viewed,
                            file.changed_since_viewed,
                        ));
                    }
                    if *target == SelectedTarget::Uncommitted {
                        doc.working_tree_missing = Some(file.status == FileStatus::Deleted);
                    }
                    let matches_loaded = match &mut doc.diff {
                        FileTabDiff::Loaded(loaded) => {
                            let same = loaded.target == key.target
                                && loaded.base.as_deref() == expected_base
                                && loaded.old_blob_id == file.old_blob_id
                                && loaded.new_blob_id == file.new_blob_id;
                            if same {
                                loaded.set_viewed_state(file.viewed, file.changed_since_viewed, cx);
                            }
                            same
                        }
                        _ => false,
                    };
                    if !matches_loaded {
                        if doc.mode == DocMode::Diff {
                            if base_changed {
                                doc.diff = FileTabDiff::Loading;
                                doc.hunk_lines.clear();
                                doc.hunk_blobs = None;
                            }
                            if !doc.diff_reading
                                || base_changed
                                || blobs_changed
                                || reload_same_diff
                            {
                                to_reload.push(key);
                            }
                        } else {
                            // Invalidate stale loaded/in-flight diff while in Code mode so
                            // switching back to Diff reloads against the new base/blobs.
                            doc.diff = FileTabDiff::Idle;
                            doc.diff_reading = false;
                            doc.diff_read += 1;
                            if base_changed || blobs_changed || doc.hunk_lines.is_empty() {
                                doc.hunk_lines.clear();
                                doc.hunk_blobs = None;
                                to_load_hunks.push(key);
                            }
                        }
                    } else if doc.mode == DocMode::Diff && reload_same_diff {
                        to_reload.push(key);
                    }
                }
            } else if *target == SelectedTarget::Uncommitted
                && let Some(doc) = self.docs.get_mut(&key)
            {
                // Uncommitted changes cleared on a working-tree tab: return to clean Code
                // and promote out of the Changes diff-preview slot so future Changes
                // updates do not treat this open Code document as an orphaned diff preview.
                let was_diff = doc.mode == DocMode::Diff;
                doc.mode = DocMode::Code;
                doc.clear_uncommitted_diff_hints();
                if was_diff || matches!(doc.content, Content::Idle) {
                    to_read.push(key.clone());
                }
                if let Some(idx) = self.tabs.position_key(&key) {
                    self.tabs.pin(idx);
                }
            } else if *target == SelectedTarget::Branch
                && let Some(doc) = self.docs.get_mut(&key)
            {
                // Under the newly chosen base this file is no longer changed in `All changes`:
                // settle cleanly to `NoChanges` in Diff mode (or Idle in Code) with no stale hunks.
                doc.diff_base = chosen_base.map(str::to_owned);
                doc.resolved_base = resolved_base.map(str::to_owned);
                doc.branch_name = branch_name.map(str::to_owned);
                doc.branch_known = true;
                doc.has_head = has_head;
                doc.hint_status = None;
                doc.hint_counts = None;
                doc.hint_viewed = None;
                doc.hint_old_blob_id = None;
                doc.hint_new_blob_id = None;
                doc.viewed_stale = false;
                doc.hunk_lines.clear();
                doc.hunk_blobs = None;
                doc.hunk_read += 1;
                doc.diff = if doc.mode == DocMode::Diff {
                    FileTabDiff::NoChanges
                } else {
                    FileTabDiff::Idle
                };
                doc.diff_reading = false;
                doc.diff_read += 1;
            }
        }
        for key in to_read {
            self.read_file(key, cx);
        }
        for key in to_reload {
            self.load_tab_diff(key, cx);
        }
        for key in to_load_hunks {
            self.load_code_hunks(key, cx);
        }
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_changes_viewed_optimistic(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        path: &str,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        viewed: bool,
        changed_since: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        self.viewed_revision += 1;
        let rev = self.viewed_revision;
        let mut updated = false;
        for (key, doc) in &mut self.docs {
            if key.path != path {
                continue;
            }
            if &key.target == target
                || (doc.hint_old_blob_id.as_deref() == old_blob
                    && doc.hint_new_blob_id.as_deref() == new_blob)
            {
                doc.latest_viewed = Some((
                    rev,
                    old_blob.map(str::to_owned),
                    new_blob.map(str::to_owned),
                    viewed,
                    changed_since,
                ));
            }
            if &key.target == target
                && doc.hint_old_blob_id.as_deref() == old_blob
                && doc.hint_new_blob_id.as_deref() == new_blob
            {
                doc.hint_viewed = Some((viewed, changed_since));
            }
            if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                && loaded.old_blob_id.as_deref() == old_blob
                && loaded.new_blob_id.as_deref() == new_blob
            {
                loaded.set_viewed_state(viewed, changed_since, cx);
                updated = true;
            }
        }
        if updated {
            cx.notify();
        }
    }

    pub(crate) fn pin_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.pin(index) {
            cx.notify();
        }
    }

    pub(crate) fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tabs.activate(index);
        self.tab_shown(cx);
    }

    pub(crate) fn cycle_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.cycle(back);
        self.tab_shown(cx);
        self.focus_active(window, cx);
    }

    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.tabs.close(index)
            && let Some(old_doc) = self.docs.remove(&key)
        {
            old_doc.drop_image_textures(Some(window), cx);
        }
        if self.tabs.is_empty() {
            let was_maximized = self.maximized;
            self.reset_tabs(Some(window), cx);
            if was_maximized {
                self.maximized = false;
                if let Some(request) = self.on_request_maximize.clone() {
                    request(false, window, cx);
                }
            }
            self.sync_watch(cx);
            self.tab_shown(cx);
            self.notify_owner(cx);
            if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
                escape(window, cx);
            }
            return;
        }
        self.tab_shown(cx);
        self.focus_active(window, cx);
    }

    fn tab_shown(&mut self, cx: &mut Context<Self>) {
        let active_key = self.tabs.active().map(|t| t.key.clone());
        if let Some(on_tab) = self.chrome.on_tab_activated.clone() {
            let key_clone = active_key.clone();
            cx.defer(move |cx| on_tab(key_clone.as_ref(), cx));
        }
        let Some(index) = self.tabs.active_index() else {
            self.find = None;
            cx.notify();
            return;
        };
        self.strip.scroll_to_item(index);
        if let Some(key) = active_key {
            let (needs_read, needs_diff, needs_hunks) =
                self.docs.get(&key).map_or((false, false, false), |doc| {
                    let needs_read =
                        doc.mode == DocMode::Code && matches!(doc.content, Content::Idle);
                    let needs_diff =
                        doc.mode == DocMode::Diff && matches!(doc.diff, FileTabDiff::Idle);
                    let needs_hunks = doc.mode == DocMode::Code
                        && doc.hunk_lines.is_empty()
                        && !matches!(doc.diff, FileTabDiff::Loaded(_))
                        && !doc.is_working_tree_deleted(&key)
                        && self.tab_can_diff(&key, doc);
                    (needs_read, needs_diff, needs_hunks)
                });
            if needs_read {
                self.read_file(key.clone(), cx);
            }
            if needs_diff {
                self.load_tab_diff(key.clone(), cx);
            } else if needs_hunks {
                self.load_code_hunks(key.clone(), cx);
            }
            self.refresh_ignored(key, cx);
        }
        self.search(cx);
        self.notify_owner(cx);
        cx.notify();
    }

    fn reset_tabs(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        for doc in self.docs.values() {
            doc.drop_image_textures(window.as_deref_mut(), cx);
        }
        self.tabs = FileTabs::default();
        self.docs.clear();
        self.find = None;
        self.list_open = false;
        self.open_in_open = false;
        self.ignored_refresh = None;
    }

    // ── Focus & Keyboard ───────────────────────────────────────────────

    fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ours = window
            .focus_lost_restore_target(cx)
            .is_some_and(|target| target == self.root_focus || target == self.viewer_focus);
        if ours {
            if self.has_tabs() {
                self.focus_active(window, cx);
            } else if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
                escape(window, cx);
            }
        }
    }

    pub(crate) fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(diff) = self.active_diff_view() {
            diff.read(cx).focus_handle().clone().focus(window, cx);
            if let Some(on_in) = self.chrome.on_diff_focus_in.clone() {
                on_in(window, cx);
            }
        } else if self.active_showing_diff() {
            self.viewer_focus.focus(window, cx);
            if let Some(on_in) = self.chrome.on_diff_focus_in.clone() {
                on_in(window, cx);
            }
        } else if let Some(view) = self.active_view() {
            view.read(cx).focus_handle().clone().focus(window, cx);
        } else if let Some(page) = self.active_rendered() {
            page.read(cx).focus_handle().clone().focus(window, cx);
        } else if let Some(canvas) = self.active_image_canvas() {
            canvas.read(cx).focus_handle().clone().focus(window, cx);
        } else {
            self.viewer_focus.focus(window, cx);
        }
    }

    pub(super) fn escape_to_rail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
            escape(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if let Some(diff) = self.active_diff_view().cloned() {
            return diff.update(cx, |v, cx| v.jump_hunk(forward, cx));
        }
        let hunk_lines = self.active_hunk_lines();
        if !hunk_lines.is_empty()
            && let Some(view) = self.active_view().cloned()
        {
            return view.update(cx, |v, cx| v.jump_to_hunk(&hunk_lines, forward, cx));
        }
        false
    }

    pub(super) fn on_viewer_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match keystroke.key.as_str() {
            "escape" if !m.shift => {
                if self.find.is_some() {
                    return;
                }
                if self.chrome.show_rail_column {
                    cx.stop_propagation();
                    self.escape_to_rail(window, cx);
                }
            }
            "]" if !m.shift && self.jump_hunk(true, cx) => {
                cx.stop_propagation();
            }
            "[" if !m.shift && self.jump_hunk(false, cx) => {
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    pub(super) fn zoom(
        &mut self,
        step: i16,
        app: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewer_focus.contains_focused(window, cx)
            || self.root_focus.contains_focused(window, cx)
        {
            if let Some(canvas) = self.active_image_canvas().cloned() {
                canvas.update(cx, |canvas, cx| canvas.step_zoom(step > 0, cx));
            } else {
                self.zoom_text(step, cx);
            }
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

    // ── Code | Diff Mode Switch ────────────────────────────────────────

    fn tab_can_diff(&self, key: &TabKey, doc: &OpenDocument) -> bool {
        if doc.has_changes_target || key.target != SelectedTarget::Uncommitted {
            return true;
        }
        !doc.is_working_tree_deleted(key) && self.has_uncommitted_diff(&key.path) == Some(true)
    }

    pub(crate) fn active_can_diff(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return false;
        };
        self.tab_can_diff(&tab.key, doc)
    }

    pub(crate) fn active_showing_diff(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        self.active_can_diff()
            && self
                .docs
                .get(&tab.key)
                .is_some_and(|doc| doc.mode == DocMode::Diff)
    }

    pub(crate) fn active_deleted(&self) -> bool {
        self.tabs.active().is_some_and(|tab| {
            self.docs
                .get(&tab.key)
                .is_some_and(|doc| doc.is_working_tree_deleted(&tab.key))
        })
    }

    pub(crate) fn active_diff_view(&self) -> Option<&Entity<UnifiedDiffView>> {
        let tab = self.tabs.active()?;
        if !self.active_can_diff() {
            return None;
        }
        let doc = self.docs.get(&tab.key)?;
        if doc.mode != DocMode::Diff {
            return None;
        }
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Unified(entity) => Some(entity),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn active_hunk_lines(&self) -> Vec<u32> {
        let Some(tab) = self.tabs.active() else {
            return Vec::new();
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return Vec::new();
        };
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => loaded.hunk_new_lines(),
            _ => doc.hunk_lines.clone(),
        }
    }

    pub(crate) fn set_active_mode(
        &mut self,
        target_mode: DocMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_idx) = self.tabs.active_index() else {
            return;
        };
        let key = self.tabs.tabs()[active_idx].key.clone();
        match target_mode {
            DocMode::Code => {
                if self.active_deleted() {
                    return;
                }
                // Flipping Diff -> Code promotes an italic preview tab to pinned (#1421).
                self.tabs.pin(active_idx);
                let target_line = self
                    .active_diff_view()
                    .and_then(|diff| diff.read(cx).current_new_line())
                    .or_else(|| self.active_hunk_lines().first().copied());
                let (need_read, existing_view) = match self.docs.get_mut(&key) {
                    Some(doc) => {
                        doc.mode = DocMode::Code;
                        let need_read = matches!(doc.content, Content::Idle);
                        if need_read {
                            doc.pending_code_line = target_line;
                        }
                        let view = match &doc.content {
                            Content::Text(v) => Some(v.clone()),
                            _ => None,
                        };
                        (need_read, view)
                    }
                    None => return,
                };
                if need_read {
                    self.read_file(key, cx);
                } else if let (Some(view), Some(line)) = (existing_view, target_line) {
                    view.update(cx, |v, cx| v.scroll_to_line(line, cx));
                }
                self.search(cx);
                self.focus_active(window, cx);
            }
            DocMode::Diff => {
                if !self.active_can_diff() {
                    return;
                }
                let code_line = self.active_view().map(|v| v.read(cx).line_in_view());
                if let Some(view) = self.active_view().cloned() {
                    view.update(cx, |v, cx| v.set_find(Arc::default(), None, cx));
                }
                self.find = None;
                let (need_load, existing_diff) = match self.docs.get_mut(&key) {
                    Some(doc) => {
                        doc.mode = DocMode::Diff;
                        let expected_base = doc.expected_base(&key).map(str::to_owned);
                        let has_blob_hints =
                            doc.hint_old_blob_id.is_some() || doc.hint_new_blob_id.is_some();
                        let loaded_valid = !doc.viewed_stale
                            && matches!(
                                &doc.diff,
                                FileTabDiff::Loaded(loaded)
                                    if loaded.target == key.target
                                        && loaded.base == expected_base
                                        && (!has_blob_hints
                                            || (loaded.old_blob_id == doc.hint_old_blob_id
                                                && loaded.new_blob_id == doc.hint_new_blob_id))
                            );
                        let existing = if loaded_valid {
                            match &doc.diff {
                                FileTabDiff::Loaded(loaded) => match &loaded.kind {
                                    LoadedDiffKind::Unified(e) => Some(e.clone()),
                                    _ => None,
                                },
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let need_load = !loaded_valid;
                        if need_load {
                            doc.pending_diff_line = code_line;
                        }
                        (need_load, existing)
                    }
                    None => return,
                };
                if need_load {
                    self.load_tab_diff(key, cx);
                } else if let (Some(diff), Some(line)) = (existing_diff, code_line) {
                    diff.update(cx, |v, cx| v.scroll_to_new_line(line, cx));
                }
                self.focus_active(window, cx);
            }
        }
        cx.notify();
    }

    // ── File Reading & Live Reloads ────────────────────────────────────

    fn show_line(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Content::Text(view) = &doc.content else {
            return;
        };
        if let Some(line) = doc.line.take() {
            doc.pending_code_line = None;
            view.update(cx, |view, cx| view.go_to_line(line, cx));
        } else if let Some(line) = doc.pending_code_line.take() {
            view.update(cx, |view, cx| view.scroll_to_line(line, cx));
        }
        cx.notify();
    }

    fn note_ignored_tabs(&mut self) {
        let Some(status) = self.status.as_ref() else {
            return;
        };
        for (key, doc) in &mut self.docs {
            if status.status.is_ignored(&key.path) {
                doc.ignored = true;
            }
        }
    }

    fn refresh_ignored(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(&key) else {
            return;
        };
        if doc.ignored && !doc.reading && !matches!(doc.content, Content::Idle) {
            let stamp = doc.stamp;
            let prev_hash = (!matches!(doc.content, Content::Deleted(_)))
                .then_some(doc.image_hash)
                .flatten();
            self.read_file_since(key, stamp, prev_hash, cx);
        }
    }

    fn refresh_ignored_tabs(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(IGNORED_REFRESH).await;
            let refreshed = this.update(cx, |this, cx| {
                let keys: Vec<TabKey> = this.tabs.tabs().iter().map(|t| t.key.clone()).collect();
                for key in keys {
                    this.refresh_ignored(key, cx);
                }
            });
            if refreshed.is_err() {
                break;
            }
        })
    }

    fn read_again_after_resolve(&mut self, cx: &mut Context<Self>) {
        let waiting: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(_, doc)| doc.reading)
            .map(|(k, _)| k.clone())
            .collect();
        let waiting_diffs: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(_, doc)| {
                doc.diff_reading
                    || (doc.mode == DocMode::Diff
                        && matches!(doc.diff, FileTabDiff::Idle | FileTabDiff::Loading))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in waiting {
            self.read_file(key, cx);
        }
        for key in waiting_diffs {
            self.load_tab_diff(key, cx);
        }
    }

    fn read_file(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let prev_hash = self.docs.get(&key).and_then(|doc| {
            (!matches!(doc.content, Content::Deleted(_)))
                .then_some(doc.image_hash)
                .flatten()
        });
        self.read_file_since(key, None, prev_hash, cx);
    }

    fn read_file_since(
        &mut self,
        key: TabKey,
        since: Option<Stamp>,
        prev_hash: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.next_read += 1;
        let read = self.next_read;
        if let Some(doc) = self.docs.get_mut(&key) {
            doc.read = read;
            doc.reading = true;
            if matches!(doc.content, Content::Idle) {
                doc.content = Content::Loading;
            }
        }
        let relative = key.path.clone();
        let markdown = is_markdown(&key.path);
        let svg = is_svg(&key.path);
        let raster_format = raster_image_format(&key.path);
        self.request(
            cx,
            move |client| {
                if let Some(format) = raster_format {
                    return read_raster_file(client, &root, &relative, format, since, prev_hash);
                }
                read_text_or_svg_file(client, &root, &relative, markdown, svg, since, prev_hash)
            },
            move |this, result, cx| this.apply_read(key, read, result, cx),
        );
    }

    fn apply_read(
        &mut self,
        key: TabKey,
        read: u64,
        result: Result<Read, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let open_line = self.open_line_handler(key.clone(), cx);
        let open_link = self.open_link_handler(cx);
        let active = self.tabs.active().is_some_and(|tab| tab.key == key);
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        if doc.read != read {
            return;
        }
        doc.reading = false;
        if let Ok(Read::Unchanged(new_stamp)) = &result {
            if let Some(stamp) = new_stamp {
                doc.stamp = Some(*stamp);
            }
            return;
        }
        let same = match (&result, &doc.content) {
            (Ok(Read::Missing), Content::Deleted(_)) => true,
            (Ok(Read::Binary(new)), Content::Binary { size })
            | (Ok(Read::TooLarge(new)), Content::TooLarge { size }) => new == size,
            _ => false,
        };
        if same {
            return;
        }
        doc.stamp = match &result {
            Ok(Read::Text(_, _, _, stamp, _))
            | Ok(Read::Image(_, stamp, _))
            | Ok(Read::ImageTooLarge { stamp, .. })
            | Ok(Read::ImageFailed { stamp, .. }) => Some(*stamp),
            _ => None,
        };
        doc.image_hash = match &result {
            Ok(Read::Text(_, _, _, _, svg_hash)) => *svg_hash,
            Ok(Read::Image(_, _, hash))
            | Ok(Read::ImageTooLarge { hash, .. })
            | Ok(Read::ImageFailed { hash, .. }) => Some(*hash),
            _ => None,
        };
        let previous = std::mem::replace(&mut doc.content, Content::Loading);
        let (kept_text, kept_image) = match previous {
            Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => {
                (Some(view), None)
            }
            Content::Image {
                canvas,
                width,
                height,
                size,
            }
            | Content::Deleted(Some(DeletedContent::Image {
                canvas,
                width,
                height,
                size,
            })) => (None, Some((canvas, width, height, size))),
            _ => (None, None),
        };
        if let Ok(Read::Text(_, Some((page, images)), _, _, _)) = &result {
            let (page, images) = (page.clone(), images.clone());
            match &doc.rendered {
                Some(view) => view.update(cx, |view, cx| {
                    view.set_page(page, images, cx);
                    view.set_deleted(false, cx);
                }),
                None => {
                    let relative = key.path.clone();
                    doc.rendered = Some(cx.new(|cx| {
                        let mut view = MarkdownView::new(page, relative, images, cx);
                        view.set_open_link(open_link);
                        view
                    }));
                }
            }
        }
        if let (Ok(Read::Missing), Some(view)) = (&result, &doc.rendered) {
            view.update(cx, |view, cx| view.set_deleted(true, cx));
        }
        if let (Ok(Read::Missing), Some(canvas)) = (&result, &doc.svg_canvas) {
            canvas.update(cx, |canvas, cx| canvas.set_deleted(true, cx));
        }
        doc.working_tree_missing = match &result {
            Ok(
                Read::Text(..)
                | Read::Image(..)
                | Read::ImageTooLarge { .. }
                | Read::ImageFailed { .. }
                | Read::Binary(_)
                | Read::TooLarge(_),
            ) => Some(false),
            Ok(Read::Missing) => Some(true),
            _ => doc.working_tree_missing,
        };
        let drop_kept_image = |kept: Option<(Entity<ImageCanvas>, u32, u32, u64)>, cx: &mut App| {
            if let Some((canvas, ..)) = kept {
                canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(None, cx));
            }
        };
        let clear_svg_canvas = |doc: &mut OpenDocument, cx: &mut App| {
            if let Some(canvas) = doc.svg_canvas.take() {
                canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(None, cx));
            }
            doc.svg_too_large = None;
            doc.svg_meta = None;
        };
        doc.content = match result {
            Ok(Read::Text(document, _, svg_decoded, _, _)) => {
                drop_kept_image(kept_image, cx);
                match svg_decoded {
                    Some(DecodeOutcome::Ready(decoded)) => {
                        doc.svg_too_large = None;
                        doc.svg_meta = Some((decoded.width, decoded.height, decoded.size));
                        match &doc.svg_canvas {
                            Some(canvas) => canvas.update(cx, |canvas, cx| {
                                canvas.set_single_image(decoded, cx);
                                canvas.set_deleted(false, cx);
                            }),
                            None => {
                                doc.svg_canvas =
                                    Some(cx.new(|cx| ImageCanvas::new_single(decoded, cx)));
                            }
                        }
                        if std::mem::take(&mut doc.svg_invalid) {
                            doc.source = false;
                        }
                    }
                    Some(DecodeOutcome::TooManyPixels {
                        width,
                        height,
                        size,
                    }) => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_too_large = Some((width, height, size));
                        doc.svg_meta = Some((width, height, size));
                        if std::mem::take(&mut doc.svg_invalid) {
                            doc.source = false;
                        }
                    }
                    Some(DecodeOutcome::Failed) => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_invalid = true;
                        doc.source = true;
                    }
                    None => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_invalid = false;
                    }
                }
                match kept_text {
                    Some(view) => {
                        view.update(cx, |view, cx| {
                            view.set_document(document, cx);
                            view.set_deleted(false, cx);
                        });
                        Content::Text(view)
                    }
                    None => Content::Text(cx.new(|cx| {
                        let mut view = CodeView::new(
                            document,
                            super::highlight::grammar_for_path(&key.path),
                            cx,
                        );
                        view.set_open_line(open_line);
                        view
                    })),
                }
            }
            Ok(Read::Image(decoded, ..)) => {
                clear_svg_canvas(doc, cx);
                let (width, height, size) = (decoded.width, decoded.height, decoded.size);
                let canvas = match kept_image {
                    Some((canvas, ..)) => {
                        canvas.update(cx, |canvas, cx| {
                            canvas.set_single_image(decoded, cx);
                            canvas.set_deleted(false, cx);
                        });
                        canvas
                    }
                    None => cx.new(|cx| ImageCanvas::new_single(decoded, cx)),
                };
                Content::Image {
                    canvas,
                    width,
                    height,
                    size,
                }
            }
            Ok(Read::ImageTooLarge {
                width,
                height,
                size,
                ..
            }) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::ImageTooLarge {
                    width,
                    height,
                    size,
                }
            }
            Ok(Read::ImageFailed { .. }) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::ImageFailed
            }
            Ok(Read::Unchanged(_)) => unreachable!(),
            Ok(Read::Binary(size)) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::Binary { size }
            }
            Ok(Read::TooLarge(size)) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::TooLarge { size }
            }
            Ok(Read::Missing) => {
                let kept = if let Some(view) = kept_text {
                    view.update(cx, |view, cx| view.set_deleted(true, cx));
                    Some(DeletedContent::Text(view))
                } else if let Some((canvas, width, height, size)) = kept_image {
                    canvas.update(cx, |canvas, cx| canvas.set_deleted(true, cx));
                    Some(DeletedContent::Image {
                        canvas,
                        width,
                        height,
                        size,
                    })
                } else {
                    None
                };
                if !doc.has_changes_target {
                    doc.mode = DocMode::Code;
                    doc.diff = FileTabDiff::Idle;
                    doc.diff_reading = false;
                }
                if active {
                    self.open_in_open = false;
                }
                Content::Deleted(kept)
            }
            Err(error) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::Failed(error.to_string().into())
            }
        };
        self.show_line(&key, cx);
        if active {
            self.search(cx);
        }
        cx.notify();
    }

    pub(crate) fn reload_changed(
        &mut self,
        changed: Option<&FilesChanged>,
        cx: &mut Context<Self>,
    ) {
        let rescan = changed.is_none_or(|changed| changed.paths.is_empty());
        let keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .map(|t| t.key.clone())
            .filter(|key| {
                rescan
                    || changed.is_some_and(|changed| {
                        changed.paths.iter().any(|changed| {
                            changed == &key.path
                                || key
                                    .path
                                    .strip_prefix(changed.as_str())
                                    .is_some_and(|rest| rest.starts_with('/'))
                        })
                    })
            })
            .collect();
        for key in keys {
            let (read_code, read_diff, load_hunks, reprobe_exists) =
                self.docs
                    .get(&key)
                    .map_or((false, false, false, false), |doc| {
                        let read_code = !matches!(doc.content, Content::Idle);
                        let read_diff = doc.mode == DocMode::Diff
                            || (key.target == SelectedTarget::Uncommitted
                                && matches!(doc.diff, FileTabDiff::Loaded(_)));
                        let load_hunks = !read_diff
                            && doc.mode == DocMode::Code
                            && key.target == SelectedTarget::Uncommitted
                            && self.has_uncommitted_diff(&key.path) == Some(true);
                        let reprobe_exists = !read_code
                            && key.target != SelectedTarget::Uncommitted
                            && doc.working_tree_missing.is_some();
                        (read_code, read_diff, load_hunks, reprobe_exists)
                    });
            if read_code {
                self.read_file(key.clone(), cx);
            }
            if read_diff {
                self.load_tab_diff(key.clone(), cx);
            } else if load_hunks {
                self.load_code_hunks(key.clone(), cx);
            }
            if reprobe_exists {
                self.reprobe_working_tree_exists(key, cx);
            }
        }
    }

    fn reprobe_working_tree_exists(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        let relative = key.path.clone();
        self.request(
            cx,
            move |client| {
                Ok(matches!(
                    client.files_read(&root, &relative, false, None),
                    Ok(FileContent::Missing)
                ))
            },
            move |this, result, cx| {
                if let Ok(missing) = result
                    && let Some(doc) = this.docs.get_mut(&key)
                    && doc.working_tree_missing != Some(missing)
                {
                    doc.working_tree_missing = Some(missing);
                    cx.notify();
                }
            },
        );
    }

    // ── Uncommitted Paths & Diff Loading ───────────────────────────────

    fn clear_uncommitted_path(&mut self, path: &str) {
        if let Some(files) = &mut self.uncommitted_files {
            files.remove(path);
        }
    }

    fn ensure_uncommitted_paths(&mut self, cx: &mut Context<Self>) {
        if self.uncommitted_files.is_none() && self.root.is_some() {
            self.refresh_uncommitted_paths(false, cx);
        }
    }

    pub(crate) fn refresh_uncommitted_paths(
        &mut self,
        refresh_active_diffs: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.has_tabs() {
            self.uncommitted_files = None;
            self.uncommitted_seq += 1;
            return;
        }
        let Some(root) = &self.root else {
            return;
        };
        if root.git.is_none() {
            self.uncommitted_files = Some(HashMap::new());
            self.reconcile_diff_tabs(false, None, true, cx);
            return;
        }
        self.uncommitted_seq += 1;
        let seq = self.uncommitted_seq;
        let started_viewed_rev = self.viewed_revision;
        let root_path = root.path.clone();
        #[cfg(test)]
        let hold_rx = self.hold_uncommitted_lists.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_uncommitted_lists.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            client.changes_files(&root_path, &ChangesTarget::Uncommitted, None)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.uncommitted_seq != seq {
                    return;
                }
                if let Ok(files) = result {
                    this.uncommitted_files = Some(
                        files
                            .files
                            .into_iter()
                            .map(|entry| (entry.path.clone(), entry))
                            .collect(),
                    );
                    this.reconcile_diff_tabs(
                        refresh_active_diffs,
                        Some(started_viewed_rev),
                        true,
                        cx,
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Refreshes open non-`Uncommitted` tabs (`All changes` / commit) against daemon:
    /// - `branch_only = true`: live `FilesEvent::Status` / `FilesEvent::Viewed` path. Resolves
    ///   the current branch and its saved/effective base via `changes_commits` + settings (matching
    ///   `ChangesView::load_snapshot_inner`) so branch checkouts and moving comparison refs update
    ///   branch metadata, comparison base, `Loaded <-> NoChanges`, and authoritative viewed flags
    ///   while `Files` or `Sessions` is active.
    /// - `branch_only = false`: reconnect (`FilesEvent::Resync`) or re-show after hidden
    ///   (also refreshes authoritative viewed marks across open `Commit` tabs).
    fn refresh_non_uncommitted_targets(&mut self, branch_only: bool, cx: &mut Context<Self>) {
        struct BranchRefresh {
            branch_name: Option<String>,
            effective_base: Option<String>,
            resolved_base: Option<String>,
            has_head: bool,
            files: Option<Vec<ChangedFile>>,
        }

        let Some(root) = &self.root else {
            return;
        };
        if root.git.is_none() {
            return;
        }
        let has_branch_tabs = self.docs.keys().any(|k| k.target == SelectedTarget::Branch);
        let mut commit_targets: Vec<SelectedTarget> = Vec::new();
        if !branch_only {
            for key in self.docs.keys() {
                if matches!(key.target, SelectedTarget::Commit(_))
                    && !commit_targets.contains(&key.target)
                {
                    commit_targets.push(key.target.clone());
                }
            }
        }
        if !has_branch_tabs && commit_targets.is_empty() {
            return;
        }
        let settings_snapshot = super::settings(cx).cloned();
        let initial_base = self
            .docs
            .iter()
            .find(|(k, _)| k.target == SelectedTarget::Branch)
            .and_then(|(k, d)| d.expected_base(k).map(str::to_owned));
        self.non_uncommitted_seq += 1;
        let seq = self.non_uncommitted_seq;
        let started_viewed_rev = self.viewed_revision;
        let root_path = root.path.clone();
        #[cfg(test)]
        let hold_rx = self.hold_non_uncommitted_refreshes.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_non_uncommitted_refreshes.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let branch_refresh = if has_branch_tabs {
                client
                    .changes_commits(&root_path, initial_base.as_deref())
                    .ok()
                    .map(|mut commits| {
                        let wanted_base = settings_snapshot
                            .as_ref()
                            .and_then(|s| {
                                s.changes_base(&commits.checkout_top, commits.branch.as_deref())
                            })
                            .map(str::to_owned);
                        if wanted_base != initial_base
                            && let Ok(refreshed) =
                                client.changes_commits(&root_path, wanted_base.as_deref())
                        {
                            commits = refreshed;
                        }
                        let effective_base = (!commits.base_missing)
                            .then_some(wanted_base.as_deref())
                            .flatten()
                            .map(str::to_owned);
                        let resolved_base =
                            commits.base.clone().or_else(|| commits.auto_base.clone());
                        let files = client
                            .changes_files(
                                &root_path,
                                &ChangesTarget::Branch,
                                effective_base.as_deref(),
                            )
                            .ok()
                            .map(|f| f.files);
                        BranchRefresh {
                            branch_name: commits.branch,
                            effective_base,
                            resolved_base,
                            has_head: commits.head_commit.is_some(),
                            files,
                        }
                    })
            } else {
                None
            };
            let mut commit_results = Vec::with_capacity(commit_targets.len());
            for target in commit_targets {
                let proto = target.to_protocol();
                if let Ok(files) = client.changes_files(&root_path, &proto, None) {
                    commit_results.push((target, files.files));
                }
            }
            (branch_refresh, commit_results)
        });
        cx.spawn(async move |this, cx| {
            let (branch_refresh, commit_results) = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.non_uncommitted_seq != seq {
                    return;
                }
                let mut to_reload = Vec::new();
                let mut to_load_hunks = Vec::new();
                let mut changed = false;

                if let Some(branch) = branch_refresh {
                    let branch_keys: Vec<TabKey> = this
                        .docs
                        .keys()
                        .filter(|k| k.target == SelectedTarget::Branch)
                        .cloned()
                        .collect();
                    for key in branch_keys {
                        let Some(doc) = this.docs.get_mut(&key) else {
                            continue;
                        };
                        let prev_base = doc.expected_base(&key).map(str::to_owned);
                        let base_changed = prev_base != branch.effective_base
                            || (branch.effective_base.is_none()
                                && branch.resolved_base.is_some()
                                && doc.resolved_base != branch.resolved_base);
                        if doc.branch_name != branch.branch_name
                            || !doc.branch_known
                            || doc.resolved_base != branch.resolved_base
                            || doc.diff_base != branch.effective_base
                            || doc.has_head != branch.has_head
                        {
                            changed = true;
                        }
                        doc.diff_base = branch.effective_base.clone();
                        doc.resolved_base = branch.resolved_base.clone();
                        doc.branch_name = branch.branch_name.clone();
                        doc.branch_known = true;
                        doc.has_head = branch.has_head;

                        let Some(files) = branch.files.as_ref() else {
                            continue;
                        };
                        if let Some(entry) = files.iter().find(|f| f.path == key.path) {
                            let prev_old = doc.hint_old_blob_id.clone();
                            let prev_new = doc.hint_new_blob_id.clone();
                            let blobs_changed =
                                prev_old != entry.old_blob_id || prev_new != entry.new_blob_id;
                            if doc.apply_list_entry_viewed(
                                entry,
                                started_viewed_rev,
                                &mut this.viewed_revision,
                                cx,
                            ) {
                                changed = true;
                            }
                            let matches_loaded = matches!(
                                &doc.diff,
                                FileTabDiff::Loaded(loaded)
                                    if loaded.base == branch.effective_base
                                        && loaded.old_blob_id == entry.old_blob_id
                                        && loaded.new_blob_id == entry.new_blob_id
                            );
                            if !matches_loaded {
                                if doc.mode == DocMode::Diff {
                                    if base_changed {
                                        doc.diff = FileTabDiff::Loading;
                                        doc.hunk_lines.clear();
                                        doc.hunk_blobs = None;
                                    }
                                    if !doc.diff_reading
                                        || base_changed
                                        || blobs_changed
                                        || matches!(
                                            doc.diff,
                                            FileTabDiff::NoChanges
                                                | FileTabDiff::Idle
                                                | FileTabDiff::Failed(_)
                                        )
                                    {
                                        to_reload.push(key);
                                    }
                                } else {
                                    doc.diff = FileTabDiff::Idle;
                                    doc.diff_reading = false;
                                    doc.diff_read += 1;
                                    let entry_blobs =
                                        (entry.old_blob_id.clone(), entry.new_blob_id.clone());
                                    if base_changed || doc.hunk_blobs.as_ref() != Some(&entry_blobs)
                                    {
                                        doc.hunk_lines.clear();
                                        doc.hunk_blobs = None;
                                        to_load_hunks.push(key);
                                    }
                                }
                            }
                        } else {
                            doc.hint_status = None;
                            doc.hint_counts = None;
                            doc.hint_viewed = None;
                            doc.hint_old_blob_id = None;
                            doc.hint_new_blob_id = None;
                            doc.viewed_stale = false;
                            doc.hunk_lines.clear();
                            doc.hunk_blobs = None;
                            doc.hunk_read += 1;
                            doc.diff = if doc.mode == DocMode::Diff {
                                FileTabDiff::NoChanges
                            } else {
                                FileTabDiff::Idle
                            };
                            doc.diff_reading = false;
                            doc.diff_read += 1;
                            changed = true;
                        }
                    }
                }

                for (target, files) in commit_results {
                    let matching_keys: Vec<TabKey> = this
                        .docs
                        .keys()
                        .filter(|k| k.target == target)
                        .cloned()
                        .collect();
                    for key in matching_keys {
                        let Some(doc) = this.docs.get_mut(&key) else {
                            continue;
                        };
                        if let Some(entry) = files.iter().find(|f| f.path == key.path)
                            && doc.apply_list_entry_viewed(
                                entry,
                                started_viewed_rev,
                                &mut this.viewed_revision,
                                cx,
                            )
                        {
                            changed = true;
                        }
                    }
                }
                for key in to_reload {
                    this.load_tab_diff(key, cx);
                }
                for key in to_load_hunks {
                    this.load_code_hunks(key, cx);
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn reconcile_diff_tabs(
        &mut self,
        refresh_active_diffs: bool,
        viewed_snapshot_rev: Option<u64>,
        authoritative_uncommitted: bool,
        cx: &mut Context<Self>,
    ) {
        let clean_keys: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(key, doc)| {
                key.target == SelectedTarget::Uncommitted
                    && ((!doc.has_changes_target && matches!(doc.content, Content::Deleted(_)))
                        || ((!doc.has_changes_target || authoritative_uncommitted)
                            && self.has_uncommitted_diff(&key.path) == Some(false)))
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut to_read = Vec::new();
        for key in clean_keys {
            if let Some(doc) = self.docs.get_mut(&key) {
                let was_diff = doc.mode == DocMode::Diff;
                doc.mode = DocMode::Code;
                doc.clear_uncommitted_diff_hints();
                if was_diff || matches!(doc.content, Content::Idle) {
                    to_read.push(key.clone());
                }
            }
            if !self.changes_rail_shown
                && let Some(idx) = self.tabs.position_key(&key)
            {
                self.tabs.pin(idx);
            }
        }
        for key in to_read {
            self.read_file(key, cx);
        }
        // Apply fresh authoritative metadata and viewed marks from `changes_files(Uncommitted)`
        // atomically per blob pair so a newer viewed toggle on an older version can never
        // attach to a newer blob pair.
        if let Some(started_rev) = viewed_snapshot_rev
            && let Some(files) = self.uncommitted_files.as_ref()
        {
            for (key, doc) in &mut self.docs {
                if key.target != SelectedTarget::Uncommitted {
                    continue;
                }
                let Some(entry) = files.get(&key.path) else {
                    continue;
                };
                doc.working_tree_missing = Some(entry.status == FileStatus::Deleted);
                doc.apply_list_entry_viewed(entry, started_rev, &mut self.viewed_revision, cx);
            }
        }
        // Only fetch Code-mode hunks for modified files whose blob pair is not yet cached,
        // so unrelated edits elsewhere in the checkout do not re-run `changes_diff`.
        let code_hunk_keys: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(key, doc)| {
                if doc.mode != DocMode::Code
                    || key.target != SelectedTarget::Uncommitted
                    || doc.is_working_tree_deleted(key)
                {
                    return false;
                }
                let Some(entry) = self
                    .uncommitted_files
                    .as_ref()
                    .and_then(|files| files.get(&key.path))
                else {
                    return false;
                };
                let blobs_known = entry.old_blob_id.is_some() || entry.new_blob_id.is_some();
                if !blobs_known {
                    return refresh_active_diffs || doc.hunk_lines.is_empty();
                }
                doc.hunk_blobs.as_ref()
                    != Some(&(entry.old_blob_id.clone(), entry.new_blob_id.clone()))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in code_hunk_keys {
            self.load_code_hunks(key, cx);
        }
        if refresh_active_diffs {
            let active_diff_keys: Vec<TabKey> = self
                .docs
                .iter()
                .filter(|(key, doc)| {
                    if key.target != SelectedTarget::Uncommitted || doc.mode != DocMode::Diff {
                        return false;
                    }
                    // If `Loaded` already matches the fresh blob pair from `changes_files`,
                    // its viewed state was updated above and no diff re-parse is needed.
                    if let Some(entry) = self
                        .uncommitted_files
                        .as_ref()
                        .and_then(|files| files.get(&key.path))
                        && (entry.old_blob_id.is_some() || entry.new_blob_id.is_some())
                        && matches!(
                            &doc.diff,
                            FileTabDiff::Loaded(loaded)
                                if loaded.old_blob_id == entry.old_blob_id
                                    && loaded.new_blob_id == entry.new_blob_id
                        )
                    {
                        return false;
                    }
                    true
                })
                .map(|(k, _)| k.clone())
                .collect();
            for key in active_diff_keys {
                self.load_tab_diff(key, cx);
            }
        }
    }

    /// Lightweight diff-hunk fetch for Code mode so modified files opened
    /// directly from `Files` show the `[ ]` hunk indicator and support `[`/`]`
    /// without waiting for the user to visit Diff mode first.
    fn load_code_hunks(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.next_hunk_read += 1;
        let seq = self.next_hunk_read;
        let requested_base = match self.docs.get_mut(&key) {
            Some(doc) if doc.mode == DocMode::Code => {
                doc.hunk_read = seq;
                doc.expected_base(&key).map(str::to_owned)
            }
            _ => return,
        };
        let relative = key.path.clone();
        let proto_target = key.target.to_protocol();
        let req_base = requested_base.clone();
        self.request(
            cx,
            move |client| {
                let diff =
                    client.changes_diff(&root, &proto_target, &relative, req_base.as_deref())?;
                let lines = match &diff.body {
                    runner_core::protocol::changes::DiffBody::Text { hunks, .. } => {
                        hunks.iter().map(|h| h.new_start.max(1)).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                Ok((lines, diff.old_blob_id, diff.new_blob_id))
            },
            move |this, result, cx| {
                let Some(doc) = this.docs.get_mut(&key) else {
                    return;
                };
                if doc.hunk_read != seq || requested_base.as_deref() != doc.expected_base(&key) {
                    return;
                }
                if let Ok((lines, old_blob, new_blob)) = result {
                    doc.hunk_lines = lines;
                    doc.hunk_blobs = Some((old_blob, new_blob));
                    cx.notify();
                }
            },
        );
    }

    pub(crate) fn load_tab_diff(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        let default_has_head = self.root_has_head();
        self.next_diff_read += 1;
        let seq = self.next_diff_read;
        let started_viewed_rev = self.viewed_revision;
        let (requested_base, need_commits, hint_snapshot, viewed_stale, doc_has_head) =
            match self.docs.get_mut(&key) {
                Some(doc) => {
                    doc.diff_read = seq;
                    doc.diff_reading = true;
                    let req_base = doc.expected_base(&key).map(str::to_owned);
                    let keep_existing = matches!(
                        &doc.diff,
                        FileTabDiff::Loaded(loaded)
                            if loaded.target == key.target
                                && loaded.base == req_base
                                && loaded.path == key.path
                    );
                    if !keep_existing {
                        doc.diff = FileTabDiff::Loading;
                    }
                    let need_commits = matches!(key.target, SelectedTarget::Branch)
                        && (!doc.branch_known || doc.resolved_base.is_none());
                    let hint_snapshot = (
                        doc.hint_old_blob_id.clone(),
                        doc.hint_new_blob_id.clone(),
                        doc.hint_viewed,
                    );
                    (
                        req_base,
                        need_commits,
                        hint_snapshot,
                        doc.viewed_stale,
                        doc.has_head,
                    )
                }
                None => return,
            };
        #[cfg(test)]
        let hold_rx = self.hold_diffs.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_diffs.push(tx);
            rx
        });
        let relative = key.path.clone();
        let target_is_uncommitted = key.target == SelectedTarget::Uncommitted;
        let proto_target = key.target.to_protocol();
        let req_base = requested_base.clone();
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let diff = client.changes_diff(&root, &proto_target, &relative, req_base.as_deref())?;
            // Only query commits/files when rail metadata or fresh matching version hints are
            // absent; normal Changes rail previews supply both and issue only `changes_diff`.
            let (has_head, branch_update, resolved_base) = if need_commits {
                let commits_opt = client.changes_commits(&root, req_base.as_deref()).ok();
                (
                    commits_opt
                        .as_ref()
                        .map_or(default_has_head || doc_has_head, |c| {
                            c.head_commit.is_some()
                        }),
                    commits_opt.as_ref().map(|c| c.branch.clone()),
                    commits_opt
                        .as_ref()
                        .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone())),
                )
            } else {
                (default_has_head || doc_has_head, None, None)
            };
            let hint_matches = !viewed_stale
                && hint_snapshot.2.is_some()
                && hint_snapshot.0 == diff.old_blob_id
                && hint_snapshot.1 == diff.new_blob_id;
            let viewed_state = if hint_matches {
                hint_snapshot.2
            } else {
                client
                    .changes_files(&root, &proto_target, req_base.as_deref())
                    .ok()
                    .and_then(|files| {
                        files
                            .files
                            .into_iter()
                            .find(|f| {
                                f.path == diff.path
                                    && f.old_blob_id == diff.old_blob_id
                                    && f.new_blob_id == diff.new_blob_id
                            })
                            .map(|f| (f.viewed, f.changed_since_viewed))
                    })
            };
            let working_tree_missing = if target_is_uncommitted {
                Some(diff.status == FileStatus::Deleted)
            } else if diff.status == FileStatus::Deleted {
                Some(matches!(
                    client.files_read(&root, &relative, false, None),
                    Ok(FileContent::Missing)
                ))
            } else {
                None
            };
            let prepared = prepare_diff_body(&diff);
            Ok((
                diff,
                prepared,
                has_head,
                branch_update,
                resolved_base,
                viewed_state,
                working_tree_missing,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_tab_diff(key, seq, started_viewed_rev, requested_base, result, cx);
                }
            });
        })
        .detach();
    }

    #[allow(clippy::type_complexity)]
    fn apply_tab_diff(
        &mut self,
        key: TabKey,
        seq: u64,
        started_viewed_rev: u64,
        requested_base: Option<String>,
        result: Result<
            (
                ChangesDiff,
                PreparedDiffBody,
                bool,
                Option<Option<String>>,
                Option<String>,
                Option<(bool, bool)>,
                Option<bool>,
            ),
            ClientError,
        >,
        cx: &mut Context<Self>,
    ) {
        let effective_mode = self.effective_diff_mode(cx);
        let weak = cx.weak_entity();
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        if doc.diff_read != seq || requested_base.as_deref() != doc.expected_base(&key) {
            return;
        }
        doc.diff_reading = false;
        let mut became_clean = false;
        let mut refresh_target_viewed = false;
        match result {
            Ok((
                diff,
                prepared,
                has_head,
                branch_update,
                resolved_base,
                viewed_state,
                working_tree_missing,
            )) => {
                if let Some(branch_name) = branch_update {
                    doc.branch_name = branch_name;
                    doc.branch_known = true;
                }
                if resolved_base.is_some() {
                    doc.resolved_base = resolved_base;
                }
                doc.has_head = has_head;
                if let Some(missing) = working_tree_missing {
                    doc.working_tree_missing = Some(missing);
                }
                // Bind viewed strictly to `(diff.old_blob_id, diff.new_blob_id)` and preserve
                // any newer viewed toggle/event that landed while this diff was in flight.
                let newer_event_viewed = doc.latest_viewed.as_ref().and_then(
                    |(rev, old_blob, new_blob, viewed, changed_since)| {
                        (*rev > started_viewed_rev
                            && *old_blob == diff.old_blob_id
                            && *new_blob == diff.new_blob_id)
                            .then_some((*viewed, *changed_since))
                    },
                );
                let matching_hint_viewed = (!doc.viewed_stale
                    && doc.hint_old_blob_id == diff.old_blob_id
                    && doc.hint_new_blob_id == diff.new_blob_id)
                    .then_some(doc.hint_viewed)
                    .flatten();
                // If a viewed mark changed on another blob pair of the same path — or cleared on
                // the same pair (where an earlier tracked mark may still keep it amber) — while
                // this diff was in flight, refresh authoritative flags from daemon `changes_files`.
                refresh_target_viewed = matches!(
                    key.target,
                    SelectedTarget::Uncommitted | SelectedTarget::Branch
                ) && doc.latest_viewed.as_ref().is_some_and(
                    |(rev, old_blob, new_blob, viewed, _)| {
                        *rev > started_viewed_rev
                            && (!*viewed
                                || *old_blob != diff.old_blob_id
                                || *new_blob != diff.new_blob_id)
                    },
                );
                let (viewed, changed_since_viewed) = newer_event_viewed
                    .or(viewed_state)
                    .or(matching_hint_viewed)
                    .unwrap_or((false, false));
                doc.viewed_stale = false;
                let file_label: SharedString = file_name(&diff.path).to_owned().into();
                let grammar = super::highlight::grammar_for_path(&diff.path);
                let hunks_copy = match &diff.body {
                    runner_core::protocol::changes::DiffBody::Text { hunks, .. } => hunks.clone(),
                    _ => Vec::new(),
                };
                doc.hunk_lines = hunks_copy.iter().map(|h| h.new_start.max(1)).collect();
                doc.hunk_blobs = Some((diff.old_blob_id.clone(), diff.new_blob_id.clone()));
                let pending_line = doc.pending_diff_line.take();
                let kind = match prepared {
                    PreparedDiffBody::Unified {
                        old_doc,
                        new_doc,
                        hunks,
                    } => {
                        if let FileTabDiff::Loaded(existing) = &doc.diff
                            && let LoadedDiffKind::Unified(entity) = &existing.kind
                        {
                            let entity = entity.clone();
                            entity.update(cx, |v, cx| {
                                v.set_show_viewed_footer(true);
                                v.set_mode(effective_mode, cx);
                                v.update_content(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_label,
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                                if let Some(line) = pending_line {
                                    v.scroll_to_new_line(line, cx);
                                }
                            });
                            LoadedDiffKind::Unified(entity)
                        } else {
                            let esc_weak = weak.clone();
                            let toggle_weak = weak.clone();
                            let focus_weak = weak;
                            let toggle_key = key.clone();
                            let entity = cx.new(|cx| {
                                let mut v = UnifiedDiffView::new(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_label,
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                                v.set_show_viewed_footer(true);
                                v.set_mode(effective_mode, cx);
                                if let Some(line) = pending_line {
                                    v.scroll_to_new_line(line, cx);
                                }
                                v.set_callbacks(
                                    Rc::new(move |window, cx| {
                                        let _ = esc_weak.update(cx, |this, cx| {
                                            this.escape_to_rail(window, cx);
                                        });
                                    }),
                                    Some(Rc::new(move |_, cx| {
                                        let toggle_key = toggle_key.clone();
                                        let _ = toggle_weak.update(cx, |this, cx| {
                                            this.toggle_viewed_for_tab(&toggle_key, cx);
                                        });
                                    })),
                                    Rc::new(move |window, cx| {
                                        let _ = focus_weak.update(cx, |this, cx| {
                                            if let Some(on_in) =
                                                this.chrome.on_diff_focus_in.clone()
                                            {
                                                on_in(window, cx);
                                            }
                                        });
                                    }),
                                );
                                v
                            });
                            LoadedDiffKind::Unified(entity)
                        }
                    }
                    PreparedDiffBody::CrlfOnly => LoadedDiffKind::CrlfOnly,
                    PreparedDiffBody::Binary { old_size, new_size } => {
                        LoadedDiffKind::Binary { old_size, new_size }
                    }
                    PreparedDiffBody::TooLarge { old_size, new_size } => {
                        LoadedDiffKind::TooLarge { old_size, new_size }
                    }
                    PreparedDiffBody::ModeOnly { old_mode, new_mode } => {
                        LoadedDiffKind::ModeOnly { old_mode, new_mode }
                    }
                    PreparedDiffBody::Submodule {
                        old_commit,
                        new_commit,
                    } => LoadedDiffKind::Submodule {
                        old_commit,
                        new_commit,
                    },
                };
                doc.hint_status = Some(diff.status.clone());
                doc.hint_counts = Some(diff.counts);
                doc.hint_viewed = Some((viewed, changed_since_viewed));
                doc.hint_old_blob_id = diff.old_blob_id.clone();
                doc.hint_new_blob_id = diff.new_blob_id.clone();
                doc.diff = FileTabDiff::Loaded(Box::new(LoadedFileDiff {
                    path: diff.path,
                    target: key.target.clone(),
                    base: requested_base,
                    old_blob_id: diff.old_blob_id,
                    new_blob_id: diff.new_blob_id,
                    status: diff.status,
                    counts: diff.counts,
                    mode_change: diff.mode_change,
                    conflicted: diff.conflicted,
                    has_head,
                    viewed,
                    changed_since_viewed,
                    hunks: hunks_copy,
                    kind,
                }));
            }
            Err(error) => {
                let message = error.to_string();
                if message.contains("has no changes in target") {
                    if key.target == SelectedTarget::Uncommitted {
                        let was_diff = doc.mode == DocMode::Diff;
                        doc.mode = DocMode::Code;
                        doc.clear_uncommitted_diff_hints();
                        became_clean = true;
                        if was_diff || matches!(doc.content, Content::Idle) {
                            self.read_file(key.clone(), cx);
                        }
                    } else {
                        doc.diff = FileTabDiff::NoChanges;
                        doc.hint_status = None;
                        doc.hint_counts = None;
                        doc.hint_viewed = None;
                        doc.hint_old_blob_id = None;
                        doc.hint_new_blob_id = None;
                        doc.hunk_lines.clear();
                        doc.hunk_blobs = None;
                        doc.viewed_stale = false;
                    }
                } else {
                    doc.diff = FileTabDiff::Failed(message.into());
                }
            }
        }
        if became_clean {
            if !self.changes_rail_shown
                && let Some(idx) = self.tabs.position_key(&key)
            {
                self.tabs.pin(idx);
            }
            self.clear_uncommitted_path(&key.path);
            self.refresh_uncommitted_paths(false, cx);
        } else if refresh_target_viewed {
            match key.target {
                SelectedTarget::Uncommitted => self.refresh_uncommitted_paths(false, cx),
                SelectedTarget::Branch => self.refresh_non_uncommitted_targets(true, cx),
                SelectedTarget::Commit(_) => {}
            }
        }
        cx.notify();
    }

    // ── Viewed Sync ────────────────────────────────────────────────────

    pub(super) fn toggle_viewed_for_tab(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        let Some(root_path) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.viewed_revision += 1;
        let write_rev = self.viewed_revision;
        let (old_blob, new_blob, prev_viewed, prev_changed, next_viewed) = {
            let Some(doc) = self.docs.get_mut(key) else {
                return;
            };
            let FileTabDiff::Loaded(loaded) = &mut doc.diff else {
                return;
            };
            let prev_viewed = loaded.viewed;
            let prev_changed = loaded.changed_since_viewed;
            let next_viewed = !prev_viewed;
            let next_changed = if next_viewed { false } else { prev_changed };
            loaded.set_viewed_state(next_viewed, next_changed, cx);
            if doc.hint_old_blob_id == loaded.old_blob_id
                && doc.hint_new_blob_id == loaded.new_blob_id
            {
                doc.hint_viewed = Some((next_viewed, next_changed));
            }
            doc.latest_viewed = Some((
                write_rev,
                loaded.old_blob_id.clone(),
                loaded.new_blob_id.clone(),
                next_viewed,
                next_changed,
            ));
            (
                loaded.old_blob_id.clone(),
                loaded.new_blob_id.clone(),
                prev_viewed,
                prev_changed,
                next_viewed,
            )
        };
        if let Some(on_viewed) = self.chrome.on_viewed_toggled.clone() {
            on_viewed(
                &root_path,
                &key.target,
                &key.path,
                old_blob.as_deref(),
                new_blob.as_deref(),
                next_viewed,
                cx,
            );
        }
        let target = key.target.to_protocol();
        let req_path = key.path.clone();
        let err_key = key.clone();
        let err_root = root_path.clone();
        let err_old = old_blob.clone();
        let err_new = new_blob.clone();
        #[cfg(test)]
        let forced_error = self.fail_next_viewed_write.take();
        self.request(
            cx,
            move |client| {
                #[cfg(test)]
                if let Some(msg) = forced_error {
                    return Err(ClientError::msg(msg));
                }
                client.changes_set_viewed(
                    &root_path,
                    &target,
                    &req_path,
                    old_blob.as_deref(),
                    new_blob.as_deref(),
                    next_viewed,
                )
            },
            move |this, result, cx| {
                if let Err(error) = result {
                    this.reconcile_failed_viewed_write(
                        &err_root,
                        &err_key,
                        write_rev,
                        err_old.as_deref(),
                        err_new.as_deref(),
                        prev_viewed,
                        prev_changed,
                        error.to_string(),
                        cx,
                    );
                }
            },
        );
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_failed_viewed_write(
        &mut self,
        root_path: &Path,
        key: &TabKey,
        write_rev: u64,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        prev_viewed: bool,
        prev_changed: bool,
        error_msg: String,
        cx: &mut Context<Self>,
    ) {
        let mut reverted = false;
        if let Some(doc) = self.docs.get_mut(key) {
            let superseded = doc.latest_viewed.as_ref().is_some_and(|(rev, o, n, ..)| {
                *rev > write_rev && o.as_deref() == old_blob && n.as_deref() == new_blob
            });
            if !superseded {
                if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                    && loaded.old_blob_id.as_deref() == old_blob
                    && loaded.new_blob_id.as_deref() == new_blob
                {
                    loaded.set_viewed_state(prev_viewed, prev_changed, cx);
                    reverted = true;
                }
                if doc.hint_old_blob_id.as_deref() == old_blob
                    && doc.hint_new_blob_id.as_deref() == new_blob
                {
                    doc.hint_viewed = Some((prev_viewed, prev_changed));
                    reverted = true;
                }
                if reverted {
                    self.viewed_revision += 1;
                    doc.latest_viewed = Some((
                        self.viewed_revision,
                        old_blob.map(str::to_owned),
                        new_blob.map(str::to_owned),
                        prev_viewed,
                        prev_changed,
                    ));
                }
            }
        }
        if reverted && let Some(on_viewed) = self.chrome.on_viewed_toggled.clone() {
            on_viewed(
                root_path,
                &key.target,
                &key.path,
                old_blob,
                new_blob,
                prev_viewed,
                cx,
            );
        }
        if let Some(on_err) = self.chrome.on_viewed_error.clone() {
            on_err(error_msg, cx);
        }
        cx.notify();
    }

    fn apply_viewed_event(&mut self, event: &ChangesViewed, cx: &mut Context<Self>) {
        let Some(root_path) = self.root_path() else {
            return;
        };
        let event_top = runner_core::app_paths::normalize_checkout_path(&event.checkout_top);
        let root_norm = runner_core::app_paths::normalize_checkout_path(root_path);
        let event_abs = absolute(&event_top, &event.path);
        self.viewed_revision += 1;
        let rev = self.viewed_revision;
        let mut updated = false;
        let mut refresh_uncommitted = false;
        let mut refresh_branch = false;
        for (key, doc) in &mut self.docs {
            let doc_abs = absolute(&root_norm, &key.path);
            if !runner_core::app_paths::same_normalized_checkout_path(&doc_abs, &event_abs) {
                continue;
            }
            let changed = if event.viewed {
                false
            } else if let FileTabDiff::Loaded(loaded) = &doc.diff
                && loaded.old_blob_id == event.old_blob_id
                && loaded.new_blob_id == event.new_blob_id
            {
                loaded.changed_since_viewed
            } else if doc.hint_old_blob_id == event.old_blob_id
                && doc.hint_new_blob_id == event.new_blob_id
            {
                doc.hint_viewed.is_some_and(|(_, c)| c)
            } else {
                false
            };
            // Always record the latest viewed event for this path/blob pair even while
            // a diff load is still in flight so the returning read cannot clobber it.
            doc.latest_viewed = Some((
                rev,
                event.old_blob_id.clone(),
                event.new_blob_id.clone(),
                event.viewed,
                changed,
            ));
            let same_loaded = matches!(
                &doc.diff,
                FileTabDiff::Loaded(loaded)
                    if loaded.old_blob_id == event.old_blob_id
                        && loaded.new_blob_id == event.new_blob_id
            );
            let same_hint = doc.hint_old_blob_id == event.old_blob_id
                && doc.hint_new_blob_id == event.new_blob_id;
            if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                && same_loaded
            {
                loaded.set_viewed_state(event.viewed, changed, cx);
                if same_hint {
                    doc.hint_viewed = Some((event.viewed, changed));
                }
                updated = true;
            } else if same_hint {
                doc.hint_viewed = Some((event.viewed, changed));
                updated = true;
            }
            // Refresh authoritative daemon flags for mutable targets whenever either:
            // - another blob pair changed (set or clear), OR
            // - the viewed mark was cleared (even for the same pair, because an earlier
            //   tracked mark like v1 can still make an unchecked v2 `changed_since_viewed = true`).
            if !event.viewed || (!same_loaded && !same_hint) {
                match key.target {
                    SelectedTarget::Uncommitted => refresh_uncommitted = true,
                    SelectedTarget::Branch => refresh_branch = true,
                    SelectedTarget::Commit(_) => {}
                }
            }
        }
        if refresh_uncommitted {
            self.refresh_uncommitted_paths(false, cx);
        }
        if refresh_branch {
            self.refresh_non_uncommitted_targets(true, cx);
        }
        if updated {
            cx.notify();
        }
    }

    // ── Active Views & Markdown ────────────────────────────────────────

    pub(super) fn active_is_image(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        let Some(format) = markdown::image_format(&tab.key.path) else {
            return false;
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return false;
        };
        if format == gpui::ImageFormat::Svg {
            return true;
        }
        !matches!(
            doc.content,
            Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
        ) && !matches!(
            &doc.diff,
            FileTabDiff::Loaded(loaded) if matches!(loaded.kind, LoadedDiffKind::Unified(_))
        )
    }

    pub(super) fn active_image_canvas(&self) -> Option<&Entity<ImageCanvas>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        if doc.mode == DocMode::Diff {
            return None;
        }
        if self.shows_svg_rendered(doc) {
            return doc.svg_canvas.as_ref();
        }
        match &doc.content {
            Content::Image { canvas, .. }
            | Content::Deleted(Some(DeletedContent::Image { canvas, .. })) => Some(canvas),
            _ => None,
        }
    }

    pub(super) fn active_view(&self) -> Option<&Entity<CodeView>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        if doc.mode == DocMode::Diff || self.shows_rendered(doc) {
            return None;
        }
        match &doc.content {
            Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => Some(view),
            _ => None,
        }
    }

    pub(super) fn shows_markdown_rendered(&self, doc: &OpenDocument) -> bool {
        doc.mode == DocMode::Code
            && doc.rendered.is_some()
            && !doc.source
            && matches!(
                doc.content,
                Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
            )
    }

    pub(super) fn shows_svg_rendered(&self, doc: &OpenDocument) -> bool {
        doc.mode == DocMode::Code
            && (doc.svg_canvas.is_some() || doc.svg_too_large.is_some())
            && !doc.source
            && matches!(
                doc.content,
                Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
            )
    }

    fn shows_rendered(&self, doc: &OpenDocument) -> bool {
        self.shows_markdown_rendered(doc) || self.shows_svg_rendered(doc)
    }

    pub(super) fn active_rendered(&self) -> Option<&Entity<MarkdownView>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        self.shows_markdown_rendered(doc)
            .then_some(doc.rendered.as_ref())
            .flatten()
    }

    pub(super) fn active_svg_rendered(&self) -> Option<bool> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        self.shows_svg_rendered(doc).then_some(true)
    }

    fn active_markdown(&self) -> Option<bool> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        (doc.rendered.is_some() || doc.svg_canvas.is_some() || doc.svg_too_large.is_some())
            .then_some(doc.source)
    }

    pub(super) fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.active_markdown() else {
            return;
        };
        if self.active_deleted() {
            return;
        }
        if source {
            if let Some(view) = self.active_view().cloned() {
                view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            }
            self.find = None;
        }
        if let Some(doc) = self
            .tabs
            .active()
            .and_then(|tab| self.docs.get_mut(&tab.key))
        {
            doc.source = !source;
        }
        self.search(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn open_link_handler(&self, cx: &mut Context<Self>) -> markdown::OpenLink {
        let root = cx.weak_entity();
        Rc::new(move |target, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_link(target, cx));
        })
    }

    fn open_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        if self.active_deleted() {
            return;
        }
        match target {
            LinkTarget::Url(url) => {
                #[cfg(test)]
                self.opened_urls.push(url);
                #[cfg(not(test))]
                cx.open_url(&url);
            }
            LinkTarget::File(path) => self.open_pinned_file(path, None, cx),
            LinkTarget::None => {}
        }
    }

    // ── LRU Eviction State Persistence ─────────────────────────────────

    pub(crate) fn saved_state(&self) -> Option<SavedViewerState> {
        let Some(root) = &self.root else {
            return self.pending_restore.clone();
        };
        if self.tabs.is_empty() && !self.tree_hidden {
            return None;
        }
        let tabs = self
            .tabs
            .tabs()
            .iter()
            .map(|tab| {
                let doc = self.docs.get(&tab.key);
                SavedViewerTab {
                    key: tab.key.clone(),
                    preview: tab.preview,
                    mode: doc.map_or(DocMode::Code, |d| d.mode),
                    has_changes_target: doc.is_some_and(|d| d.has_changes_target),
                    diff_base: doc.and_then(|d| d.diff_base.clone()),
                    resolved_base: doc.and_then(|d| d.resolved_base.clone()),
                    branch_name: doc.and_then(|d| d.branch_name.clone()),
                    commit_short_id: doc.and_then(|d| d.commit_short_id.clone()),
                }
            })
            .collect();
        Some(SavedViewerState {
            root: root.path.clone(),
            tabs,
            active: self.tabs.active().map(|t| t.key.clone()),
            tree_hidden: self.tree_hidden,
        })
    }

    pub(crate) fn restore_saved_state(&mut self, saved: SavedViewerState, cx: &mut Context<Self>) {
        if self.root.is_some() {
            self.apply_saved_state(saved, cx);
        } else {
            self.pending_restore = Some(saved);
        }
    }

    fn apply_saved_state(&mut self, saved: SavedViewerState, cx: &mut Context<Self>) {
        let same_root = self
            .root
            .as_ref()
            .is_some_and(|r| runner_core::app_paths::same_checkout_path(&r.path, &saved.root));
        if !same_root {
            self.notify_owner(cx);
            cx.notify();
            return;
        }
        self.tree_hidden = saved.tree_hidden;
        let markdown_source = super::settings(cx)
            .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let has_head = self.root_has_head();
        for item in saved.tabs {
            let source = !is_svg(&item.key.path) && markdown_source;
            self.tabs.push_restored(item.key.clone(), item.preview);
            let mut doc = OpenDocument::new_code(source, has_head);
            doc.mode = item.mode;
            doc.has_changes_target = item.has_changes_target;
            doc.diff_base = item.diff_base;
            doc.resolved_base = item.resolved_base;
            doc.branch_name = item.branch_name;
            doc.commit_short_id = item.commit_short_id;
            if doc.mode == DocMode::Diff {
                doc.content = Content::Idle;
                doc.diff = FileTabDiff::Loading;
            }
            self.docs.insert(item.key.clone(), doc);
            if item.mode == DocMode::Diff {
                self.load_tab_diff(item.key, cx);
            } else {
                self.read_file(item.key, cx);
            }
        }
        if let Some(active) = saved.active
            && let Some(index) = self.tabs.position_key(&active)
        {
            self.tabs.activate(index);
        }
        self.ensure_uncommitted_paths(cx);
        self.tab_shown(cx);
    }
}

#[cfg(test)]
impl UnifiedViewer {
    pub(crate) fn open_tabs(&self) -> Vec<String> {
        self.tabs
            .tabs()
            .iter()
            .map(|tab| tab.key.path.clone())
            .collect()
    }

    pub(crate) fn active_tab(&self) -> Option<String> {
        self.tabs.active().map(|tab| tab.key.path.clone())
    }

    pub(crate) fn active_tab_entry(&self) -> Option<&FileTab> {
        self.tabs.active()
    }

    pub(crate) fn code_view(&self) -> Option<Entity<CodeView>> {
        self.active_view().cloned()
    }

    #[allow(dead_code)]
    pub(crate) fn image_canvas(&self) -> Option<Entity<ImageCanvas>> {
        self.active_image_canvas().cloned()
    }

    #[allow(dead_code)]
    pub(crate) fn active_is_image_tab(&self) -> bool {
        self.active_is_image()
    }

    pub(crate) fn showing_diff(&self) -> bool {
        self.active_showing_diff()
    }

    pub(crate) fn tab_diff_view(&self) -> Option<Entity<UnifiedDiffView>> {
        self.active_diff_view().cloned()
    }

    pub(crate) fn read_seq(&self, path: &str) -> Option<u64> {
        self.docs
            .iter()
            .find(|(k, _)| k.path == path)
            .map(|(_, d)| d.read)
    }

    pub(crate) fn reply_text(&mut self, path: &str, read: u64, text: &str, cx: &mut Context<Self>) {
        let Some(key) = self.docs.keys().find(|k| k.path == path).cloned() else {
            return;
        };
        let document = Arc::new(Document::decode(text.as_bytes()));
        self.apply_read(
            key,
            read,
            Ok(Read::Text(
                document,
                None,
                None,
                (text.len() as u64, None),
                None,
            )),
            cx,
        );
    }

    pub(crate) fn reading(&self, path: &str) -> bool {
        self.docs.iter().any(|(k, d)| k.path == path && d.reading)
    }

    pub(crate) fn last_search(&self) -> Option<u64> {
        self.find.as_ref().map(|find| find.search)
    }

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
        let Some(key) = self.docs.keys().find(|k| k.path == path).cloned() else {
            return;
        };
        self.apply_read(key, read, Ok(Read::Binary(size)), cx);
    }

    pub(crate) fn hold_search(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.hold_search = hold;
        if !hold {
            self.search(cx);
        }
    }

    pub(crate) fn set_hold_diffs(&mut self, hold: bool) {
        self.hold_diffs = hold;
    }

    pub(crate) fn status_stale(&self) -> bool {
        self.status_stale
    }

    pub(crate) fn set_fail_next_viewed_write(&mut self, message: Option<String>) {
        self.fail_next_viewed_write = message;
    }

    pub(crate) fn release_held_diffs(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_diffs.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_diffs_len(&self) -> usize {
        self.held_diffs.len()
    }

    pub(crate) fn set_hold_uncommitted_lists(&mut self, hold: bool) {
        self.hold_uncommitted_lists = hold;
    }

    pub(crate) fn release_held_uncommitted_lists(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_uncommitted_lists.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_uncommitted_lists_len(&self) -> usize {
        self.held_uncommitted_lists.len()
    }

    pub(crate) fn set_hold_non_uncommitted_refreshes(&mut self, hold: bool) {
        self.hold_non_uncommitted_refreshes = hold;
    }

    pub(crate) fn release_held_non_uncommitted_refreshes(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_non_uncommitted_refreshes.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_non_uncommitted_refreshes_len(&self) -> usize {
        self.held_non_uncommitted_refreshes.len()
    }

    pub(crate) fn active_diff_viewed(&self) -> Option<(bool, bool)> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => Some((loaded.viewed, loaded.changed_since_viewed)),
            _ => None,
        }
    }

    pub(crate) fn is_diff_loading(&self) -> bool {
        self.tabs
            .active()
            .and_then(|t| self.docs.get(&t.key))
            .is_some_and(|d| d.mode == DocMode::Diff && matches!(d.diff, FileTabDiff::Loading))
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn loaded_diff_meta(
        &self,
    ) -> Option<(
        &SelectedTarget,
        Option<&str>,
        &str,
        Option<&str>,
        Option<&str>,
    )> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => Some((
                &loaded.target,
                loaded.base.as_deref(),
                &loaded.path,
                loaded.old_blob_id.as_deref(),
                loaded.new_blob_id.as_deref(),
            )),
            _ => None,
        }
    }

    pub(crate) fn step(&mut self, next: bool, cx: &mut Context<Self>) {
        self.step_find(next, cx);
    }

    pub(crate) fn launched(&self) -> Vec<(Option<FileLinkEditor>, PathBuf, Option<u32>)> {
        self.launched.clone()
    }

    pub(crate) fn rendered_view(&self) -> Option<Entity<MarkdownView>> {
        self.active_rendered().cloned()
    }

    pub(crate) fn opened_urls(&self) -> Vec<String> {
        self.opened_urls.clone()
    }

    pub(crate) fn follow_link(&mut self, href: &str, cx: &mut Context<Self>) {
        let Some(from) = self.active_tab() else {
            return;
        };
        self.open_link(markdown::route(href, &from), cx);
    }

    pub(crate) fn find_state(&self) -> Option<(Option<String>, Option<usize>)> {
        let find = self.find.as_ref()?;
        Some((
            Self::find_count(find).map(|(label, _)| label.to_string()),
            find.current,
        ))
    }

    pub(crate) fn watched(&self) -> Option<&Path> {
        self.watched.as_deref()
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

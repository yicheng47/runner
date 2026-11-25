//! The side panel's Changes view (#1141): changed files, branch commits since
//! merge base, Unified diff with two-version syntax highlighting, and viewed marks.

pub(crate) mod diff;
pub(crate) mod model;
mod render;

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, EntityId, FocusHandle, KeyDownEvent, PathPromptOptions, Pixels,
    ScrollHandle, ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle,
    Window,
};
use runner_app::ui::{Scrollbar, TextField};
use runner_core::protocol::changes::{
    ChangedFile, ChangesBranches, ChangesCommits, ChangesDiff, ChangesFiles, ChangesTotals,
    ChangesViewed, DiffBody, DiffHunk,
};
use runner_core::protocol::files::{FilesRoot, FilesRootOutcome, FilesSubject, WorktreeEntry};
use runner_core::protocol::{ClientError, DaemonClient};

use crate::surfaces::files::document::Document;
use crate::surfaces::files::image::{
    self, DecodeOutcome, DiffCaptionKind, DiffSide, DiffSideState, DiffSides, ImageCanvas,
};
use crate::surfaces::files::open_file::{OpenChangesRequest, UnifiedViewer};
use crate::surfaces::files::tabs::TabKey;
use crate::surfaces::files::{self, absolute, picker, FilesEvent, FilesHub, Look};
use crate::surfaces::side_panel::{PanelContext, PanelSurface, Widened};
use crate::theme;
use crate::ui::markdown;

pub(crate) use diff::UnifiedDiffView;
use model::{
    build_base_picker_items, build_checkout_picker_items, build_commit_rows, build_flat_rows,
    build_tree_rows, choose_initial_target, default_base_highlight, default_checkout_highlight,
    filter_base_picker_items, filter_checkout_picker_items, has_all_changes, is_crlf_only_change,
    is_viewing_other_checkout, live_worktrees, reconcile_target, BasePickerItem,
    CheckoutPickerItem, CommitRow, FocusPlace, ListRow, SelectedTarget,
};
pub(crate) use render::{
    render_diff_viewed_footer, render_shared_diff_header, render_shared_diff_placeholder,
    DiffHeaderProps, DiffModeCallback, PlaceholderViewedFooter,
};

fn subject(context: &PanelContext) -> FilesSubject {
    match &context.subject {
        crate::surfaces::side_panel::PanelSubject::Session(id) => FilesSubject::Session(id.clone()),
        crate::surfaces::side_panel::PanelSubject::Mission(id) => FilesSubject::Mission(id.clone()),
    }
}

pub(super) enum Body {
    Loading,
    Remote,
    Choose,
    Missing { path: PathBuf },
    NotInGit,
    Root(Box<RootChanges>),
    Failed(SharedString),
}

pub(super) struct RootChanges {
    pub(super) root: FilesRoot,
    pub(super) commits: Option<ChangesCommits>,
    pub(super) chosen_base: Option<String>,
    pub(super) missing_base_notice: Option<String>,
    pub(super) selected_target: SelectedTarget,
    pub(super) initial_target_chosen: bool,
    pub(super) uncommitted_count: usize,
    pub(super) branch_count: usize,
    pub(super) files: Vec<ChangedFile>,
    pub(super) totals: ChangesTotals,
    pub(super) collapsed_folders: BTreeSet<String>,
    pub(super) list_rows: Vec<ListRow>,
    pub(super) selected_path: Option<String>,
    pub(super) highlighted_list_row: usize,
    pub(super) snapshot_seq: u64,
    pub(super) files_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowIdentity {
    Folder(String),
    File(String),
}

fn row_identity(row: &ListRow) -> RowIdentity {
    match row {
        ListRow::Folder { path, .. } => RowIdentity::Folder(path.clone()),
        ListRow::File { path, .. } => RowIdentity::File(path.clone()),
    }
}

fn find_nearest_visible_ancestor_folder(path: &str, rows: &[ListRow]) -> Option<usize> {
    let mut current = path;
    while let Some((parent, _)) = current.rsplit_once('/') {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::Folder { path: p, .. } if p == parent))
        {
            return Some(idx);
        }
        current = parent;
    }
    None
}

fn resolve_highlighted_row(
    wanted: Option<&RowIdentity>,
    selected_path: Option<&str>,
    fallback_idx: usize,
    rows: &[ListRow],
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    if let Some(identity) = wanted {
        match identity {
            RowIdentity::Folder(folder) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::Folder { path, .. } if path == folder))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(folder, rows) {
                    return idx;
                }
                let prefix = format!("{folder}/");
                if let Some(idx) = rows.iter().position(
                    |r| matches!(r, ListRow::File { path, .. } if path.starts_with(&prefix)),
                ) {
                    return idx;
                }
            }
            RowIdentity::File(file) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::File { path, .. } if path == file))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(file, rows) {
                    return idx;
                }
            }
        }
    }
    if let Some(selected) = selected_path {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::File { path, .. } if path == selected))
        {
            return idx;
        }
        if let Some(idx) = find_nearest_visible_ancestor_folder(selected, rows) {
            return idx;
        }
    }
    fallback_idx.min(rows.len().saturating_sub(1))
}

pub(crate) enum LoadedDiffKind {
    Unified(Entity<UnifiedDiffView>),
    Image {
        canvas: Entity<ImageCanvas>,
        one_sided: bool,
    },
    Svg {
        canvas: Entity<ImageCanvas>,
        unified: Entity<UnifiedDiffView>,
        source: bool,
        one_sided: bool,
    },
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) enum PreparedDiffBody {
    Unified {
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: Vec<DiffHunk>,
    },
    Image {
        sides: DiffSides,
    },
    Svg {
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: Vec<DiffHunk>,
        sides: DiffSides,
    },
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) fn prepare_diff_body(diff: &ChangesDiff) -> PreparedDiffBody {
    let name = model::file_name(&diff.path).to_owned();
    let decode_raster_side = |caption: DiffCaptionKind,
                              format: gpui::ImageFormat,
                              bytes: &[u8],
                              fallback_size: Option<u64>| {
        let size = fallback_size.unwrap_or(bytes.len() as u64);
        let state = match image::decode_raster_image(format, bytes, size) {
            DecodeOutcome::Ready(decoded) => DiffSideState::Ready(decoded),
            DecodeOutcome::TooManyPixels {
                width,
                height,
                size,
            } => DiffSideState::TooManyPixels {
                width,
                height,
                size,
            },
            DecodeOutcome::Failed => DiffSideState::Failed,
        };
        DiffSide {
            caption,
            file_name: name.clone(),
            state,
        }
    };
    match &diff.body {
        DiffBody::Text { old, new, hunks } => {
            if markdown::image_format(&diff.path).is_some_and(|fmt| fmt != gpui::ImageFormat::Svg) {
                let old_present = diff.old_blob_id.is_some() || !old.0.is_empty();
                let new_present = diff.new_blob_id.is_some() || !new.0.is_empty();
                let old_magic = old_present
                    .then(|| image::detect_raster_magic(&old.0))
                    .flatten();
                let new_magic = new_present
                    .then(|| image::detect_raster_magic(&new.0))
                    .flatten();
                if old_magic.is_some() || new_magic.is_some() {
                    let decode_text_raster_side =
                        |caption: DiffCaptionKind,
                         bytes: &[u8],
                         detected: Option<gpui::ImageFormat>| {
                            if let Some(fmt) = detected {
                                decode_raster_side(caption, fmt, bytes, None)
                            } else {
                                DiffSide {
                                    caption,
                                    file_name: name.clone(),
                                    state: DiffSideState::Failed,
                                }
                            }
                        };
                    let sides = match (old_present, new_present) {
                        (false, true) => DiffSides::One(Box::new(decode_text_raster_side(
                            DiffCaptionKind::Added,
                            &new.0,
                            new_magic,
                        ))),
                        (true, false) => DiffSides::One(Box::new(decode_text_raster_side(
                            DiffCaptionKind::Deleted,
                            &old.0,
                            old_magic,
                        ))),
                        _ => DiffSides::Two {
                            before: Box::new(decode_text_raster_side(
                                DiffCaptionKind::Before,
                                &old.0,
                                old_magic,
                            )),
                            after: Box::new(decode_text_raster_side(
                                DiffCaptionKind::After,
                                &new.0,
                                new_magic,
                            )),
                        },
                    };
                    // Match file-tab behavior: when neither side is an actual image (nor hits
                    // pixel limits) and every present side is valid UTF-8 (e.g. plain text
                    // beginning with "BM..."), keep the normal text line diff.
                    let all_valid_utf8 = (!old_present || std::str::from_utf8(&old.0).is_ok())
                        && (!new_present || std::str::from_utf8(&new.0).is_ok());
                    if sides.has_drawable() || sides.has_too_many_pixels() || !all_valid_utf8 {
                        return PreparedDiffBody::Image { sides };
                    }
                }
            }
            if is_crlf_only_change(&old.0, &new.0, diff.mode_change.as_ref(), diff.conflicted) {
                PreparedDiffBody::CrlfOnly
            } else {
                let old_doc = Arc::new(Document::decode(&old.0));
                let new_doc = Arc::new(Document::decode(&new.0));
                if markdown::image_format(&diff.path) == Some(gpui::ImageFormat::Svg) {
                    let decode_svg_side = |caption: DiffCaptionKind, bytes: &[u8]| DiffSide {
                        caption,
                        file_name: name.clone(),
                        state: match image::decode_svg_image(bytes, bytes.len() as u64) {
                            DecodeOutcome::Ready(decoded) => DiffSideState::Ready(decoded),
                            DecodeOutcome::TooManyPixels {
                                width,
                                height,
                                size,
                            } => DiffSideState::TooManyPixels {
                                width,
                                height,
                                size,
                            },
                            DecodeOutcome::Failed => DiffSideState::Failed,
                        },
                    };
                    let sides = match (diff.old_blob_id.is_some(), diff.new_blob_id.is_some()) {
                        (false, true) => DiffSides::One(Box::new(decode_svg_side(
                            DiffCaptionKind::Added,
                            &new.0,
                        ))),
                        (true, false) => DiffSides::One(Box::new(decode_svg_side(
                            DiffCaptionKind::Deleted,
                            &old.0,
                        ))),
                        _ => DiffSides::Two {
                            before: Box::new(decode_svg_side(DiffCaptionKind::Before, &old.0)),
                            after: Box::new(decode_svg_side(DiffCaptionKind::After, &new.0)),
                        },
                    };
                    if sides.has_drawable() || sides.has_too_many_pixels() {
                        return PreparedDiffBody::Svg {
                            old_doc,
                            new_doc,
                            hunks: hunks.clone(),
                            sides,
                        };
                    }
                }
                PreparedDiffBody::Unified {
                    old_doc,
                    new_doc,
                    hunks: hunks.clone(),
                }
            }
        }
        DiffBody::Binary {
            old_size,
            new_size,
            old,
            new,
        } => {
            if (old.is_some() || new.is_some())
                && let Some(format) =
                    markdown::image_format(&diff.path).filter(|&fmt| fmt != gpui::ImageFormat::Svg)
            {
                let sides = match (old.as_ref(), new.as_ref()) {
                    (Some(old_b), Some(new_b)) => DiffSides::Two {
                        before: Box::new(decode_raster_side(
                            DiffCaptionKind::Before,
                            format,
                            &old_b.0,
                            *old_size,
                        )),
                        after: Box::new(decode_raster_side(
                            DiffCaptionKind::After,
                            format,
                            &new_b.0,
                            *new_size,
                        )),
                    },
                    (None, Some(new_b)) => DiffSides::One(Box::new(decode_raster_side(
                        DiffCaptionKind::Added,
                        format,
                        &new_b.0,
                        *new_size,
                    ))),
                    (Some(old_b), None) => DiffSides::One(Box::new(decode_raster_side(
                        DiffCaptionKind::Deleted,
                        format,
                        &old_b.0,
                        *old_size,
                    ))),
                    (None, None) => unreachable!(),
                };
                return PreparedDiffBody::Image { sides };
            }
            PreparedDiffBody::Binary {
                old_size: *old_size,
                new_size: *new_size,
            }
        }
        DiffBody::TooLarge { old_size, new_size } => PreparedDiffBody::TooLarge {
            old_size: *old_size,
            new_size: *new_size,
        },
        DiffBody::ModeOnly { old_mode, new_mode } => PreparedDiffBody::ModeOnly {
            old_mode: old_mode.clone(),
            new_mode: new_mode.clone(),
        },
        DiffBody::Submodule {
            old_commit,
            new_commit,
        } => PreparedDiffBody::Submodule {
            old_commit: old_commit.clone(),
            new_commit: new_commit.clone(),
        },
    }
}

struct SnapshotPayload {
    commits: ChangesCommits,
    wanted_base: Option<String>,
    uncommitted: ChangesFiles,
    branch: Option<ChangesFiles>,
    commit_files: Option<(String, ChangesFiles)>,
}

pub(super) struct BasePickerState {
    pub(super) field: Entity<TextField>,
    pub(super) query: String,
    pub(super) branches: ChangesBranches,
    pub(super) items: Vec<BasePickerItem>,
    pub(super) highlighted: usize,
    pub(super) moved: bool,
    pub(super) scroll: ScrollHandle,
    pub(super) reveal: Cell<bool>,
    pub(super) return_focus: FocusHandle,
    _observe: Subscription,
}

pub(super) struct CheckoutPickerState {
    pub(super) field: Entity<TextField>,
    pub(super) query: String,
    pub(super) worktrees: Vec<WorktreeEntry>,
    pub(super) items: Vec<CheckoutPickerItem>,
    pub(super) highlighted: usize,
    pub(super) moved: bool,
    pub(super) scroll: ScrollHandle,
    pub(super) reveal: Cell<bool>,
    pub(super) return_focus: FocusHandle,
    _observe: Subscription,
}

#[derive(Clone, Debug)]
pub(crate) struct SavedChangesState {
    pub(crate) root: PathBuf,
    pub(crate) selected_target: SelectedTarget,
}

pub(crate) struct ChangesView {
    pub(super) context: PanelContext,
    client: DaemonClient,
    pub(crate) viewer: Entity<UnifiedViewer>,
    pub(crate) root_focus: FocusHandle,
    pub(super) list_focus: FocusHandle,
    pub(super) checkout_button_focus: FocusHandle,
    pub(super) checkout_return_focus: FocusHandle,
    pub(super) base_chip_focus: FocusHandle,
    pub(super) focus_place: FocusPlace,
    pub(super) _focus_lost: Option<Subscription>,
    pub(super) body: Body,
    generation: u64,
    root_change: u64,
    pub(super) shown: bool,
    watched: Option<PathBuf>,
    watch_chain: Option<Task<()>>,
    root_chain: Option<Task<()>>,
    pub(super) error: Option<SharedString>,
    pub(super) recent: Vec<PathBuf>,
    pub(super) choose_field: Entity<TextField>,
    pub(super) base_picker: Option<BasePickerState>,
    pub(super) base_picker_anchor: Rc<Cell<Bounds<Pixels>>>,
    pub(super) checkout_picker: Option<CheckoutPickerState>,
    pub(super) checkout_picker_anchor: Rc<Cell<Bounds<Pixels>>>,
    pub(super) removed_checkout_notice: Option<String>,
    pub(super) commits_scroll: ScrollHandle,
    pub(super) files_scroll: UniformListScrollHandle,
    pub(super) files_scrollbar: Entity<Scrollbar>,
    pub(super) widened: Option<Widened>,
    pub(super) owner: Option<EntityId>,
    pub(super) reported_dirty: bool,
    pub(super) status_stale: bool,
    pub(super) pending_restore: Option<SavedChangesState>,
    look: Option<Look>,
    tree_view_pref: bool,
    #[cfg(test)]
    hold_snapshots: bool,
    #[cfg(test)]
    held_snapshots: Vec<futures::channel::oneshot::Sender<()>>,
    _hub: Subscription,
    _settings: Subscription,
}

impl ChangesView {
    pub(crate) fn new(
        context: PanelContext,
        viewer: Entity<UnifiedViewer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let client = crate::app_store::global_app_store(cx)
            .read(cx)
            .client
            .clone();
        let hub = files::hub(&client, cx);
        let hub_subscription = cx.subscribe(&hub, |this, _: Entity<FilesHub>, event, cx| {
            this.on_files_event(event, cx)
        });
        let store = crate::app_store::global_app_store(cx);
        let settings_subscription = cx.observe(&store, |this, _, cx| this.settings_changed(cx));
        let root_weak = cx.weak_entity();
        let choose_field = cx.new(move |field_cx| {
            TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("ui.type_a_path"),
                true,
            )
            .text_size(theme::text_ui())
            .right_padding(32.)
            .key_interceptor(Rc::new(move |event, window, cx| {
                if event.keystroke.key != "enter" {
                    return false;
                }
                let root_weak = root_weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = root_weak.update(cx, |this, cx| this.apply_typed_path(window, cx));
                });
                true
            }))
        });
        let files_scroll = UniformListScrollHandle::new();
        let entity_id = cx.entity_id();
        let base = files_scroll.0.borrow().base_handle.clone();
        let files_scrollbar = cx.new(|_| Scrollbar::app(base, entity_id));
        let tree_view_pref = files::settings(cx).is_some_and(|s| s.changes_tree_view);

        let mut view = Self {
            body: if context.remote {
                Body::Remote
            } else {
                Body::Loading
            },
            context,
            client,
            viewer,
            root_focus: cx.focus_handle(),
            list_focus: cx.focus_handle().tab_stop(true),
            checkout_button_focus: cx.focus_handle().tab_stop(true),
            checkout_return_focus: cx.focus_handle().tab_stop(true),
            base_chip_focus: cx.focus_handle().tab_stop(true),
            focus_place: FocusPlace::Files,
            _focus_lost: None,
            generation: 0,
            root_change: 0,
            shown: false,
            watched: None,
            watch_chain: None,
            root_chain: None,
            error: None,
            recent: Vec::new(),
            choose_field,
            base_picker: None,
            base_picker_anchor: Rc::default(),
            checkout_picker: None,
            checkout_picker_anchor: Rc::default(),
            removed_checkout_notice: None,
            commits_scroll: ScrollHandle::new(),
            files_scroll,
            files_scrollbar,
            widened: None,
            owner: None,
            reported_dirty: false,
            status_stale: false,
            pending_restore: None,
            look: Look::current(cx),
            tree_view_pref,
            #[cfg(test)]
            hold_snapshots: false,
            #[cfg(test)]
            held_snapshots: Vec::new(),
            _hub: hub_subscription,
            _settings: settings_subscription,
        };
        view.resolve(cx);
        cx.on_release(|this: &mut Self, cx| {
            if let Some(chain) = this.root_chain.take() {
                chain.detach();
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
        view
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

    pub(crate) fn set_widened(&mut self, widened: Option<Widened>, cx: &mut Context<Self>) {
        if self.widened != widened {
            self.widened = widened;
            cx.notify();
        }
    }

    pub(crate) fn is_not_in_git(&self) -> bool {
        matches!(self.body, Body::NotInGit)
    }

    pub(crate) fn focus_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root_focus.contains_focused(window, cx) {
            return;
        }
        if matches!(self.body, Body::Root(_)) {
            self.focus_list(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
    }

    pub(crate) fn set_owner(&mut self, owner: Option<EntityId>) {
        self.owner = owner;
    }

    pub(super) fn notify_owner(&self, cx: &mut Context<Self>) {
        if let Some(owner) = self.owner {
            cx.defer(move |cx| cx.notify(owner));
        }
    }

    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if !shown {
            self.base_picker = None;
            self.checkout_picker = None;
            self.status_stale = true;
            if let Body::Root(state) = &mut self.body {
                // Invalidate in-flight snapshot / target-list responses so a
                // hidden Changes rail cannot mutate the shared viewer after handoff.
                state.snapshot_seq += 1;
                state.files_seq += 1;
            }
        }
        self.sync_watch(cx);
    }

    pub(crate) fn has_dirty_changes(&self) -> bool {
        matches!(
            &self.body,
            Body::Root(state) if state.uncommitted_count > 0
        )
    }

    /// Authoritative live uncommitted-changes flag while this view is watching.
    pub(crate) fn fresh_dirty_changes(&self) -> Option<bool> {
        if !self.shown || self.status_stale {
            return None;
        }
        match &self.body {
            Body::Root(state) if state.commits.is_some() => Some(state.uncommitted_count > 0),
            Body::Root(_) | Body::Loading | Body::Failed(_) => None,
            Body::Remote | Body::Choose | Body::Missing { .. } | Body::NotInGit => Some(false),
        }
    }

    pub(crate) fn saved_state(&self) -> Option<SavedChangesState> {
        match &self.body {
            Body::Root(state) => Some(SavedChangesState {
                root: state.root.path.clone(),
                selected_target: state.selected_target.clone(),
            }),
            Body::Loading => self.pending_restore.clone(),
            _ => None,
        }
    }

    pub(crate) fn restore_saved_state(&mut self, saved: SavedChangesState, cx: &mut Context<Self>) {
        if matches!(&self.body, Body::Root(state) if state.commits.is_some()) {
            self.apply_saved_changes(saved, cx);
        } else {
            self.pending_restore = Some(saved);
        }
    }

    fn apply_saved_changes(&mut self, saved: SavedChangesState, cx: &mut Context<Self>) {
        let same_root = matches!(
            &self.body,
            Body::Root(state) if picker::same_checkout_path(&state.root.path, &saved.root)
        );
        if !same_root {
            return;
        }
        let needs_target_switch = matches!(
            &self.body,
            Body::Root(state) if state.selected_target != saved.selected_target
        );
        if needs_target_switch {
            self.select_target(saved.selected_target, cx);
        }
    }

    pub(crate) fn sync_active_tab(&mut self, active: Option<&TabKey>, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        let next = active
            .filter(|key| {
                key.target == state.selected_target
                    && state.files.iter().any(|f| f.path == key.path)
            })
            .map(|key| key.path.clone());
        if state.selected_path != next {
            state.selected_path = next;
            if let Some(path) = &state.selected_path
                && let Some(row_idx) = state
                    .list_rows
                    .iter()
                    .position(|r| matches!(r, ListRow::File { path: p, .. } if p == path))
            {
                state.highlighted_list_row = row_idx;
                self.files_scroll
                    .scroll_to_item(row_idx, ScrollStrategy::Nearest);
            }
            cx.notify();
        }
    }

    pub(crate) fn on_viewer_viewed_error(&mut self, msg: String, cx: &mut Context<Self>) {
        if self.recover_from_root_error(&msg, cx) {
            return;
        }
        self.error = Some(msg.into());
        self.load_snapshot_inner(false, false, false, cx);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_viewer_viewed_optimistic(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        path: &str,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        viewed: bool,
        cx: &mut Context<Self>,
    ) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if !picker::same_checkout_path(&state.root.path, root) || &state.selected_target != target {
            return;
        }
        let mut updated = false;
        for file in &mut state.files {
            if file.path == path
                && file.old_blob_id.as_deref() == old_blob
                && file.new_blob_id.as_deref() == new_blob
            {
                file.viewed = viewed;
                if viewed {
                    file.changed_since_viewed = false;
                }
                updated = true;
            }
        }
        if updated {
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            cx.notify();
        }
    }

    pub(crate) fn on_viewer_diff_focused(&mut self, cx: &mut Context<Self>) {
        if self.focus_place != FocusPlace::Diff {
            self.focus_place = FocusPlace::Diff;
            cx.notify();
        }
    }

    fn watched_git_root(&self) -> Option<&Path> {
        match &self.body {
            Body::Root(state) => Some(&state.root.path),
            _ => None,
        }
    }

    pub(super) fn noun(&self) -> String {
        crate::i18n::text(match self.context.surface {
            PanelSurface::Chat => "noun.chat",
            PanelSurface::Terminal => "noun.terminal",
            PanelSurface::Mission => "noun.mission",
        })
        .to_owned()
    }

    pub(super) fn projects(&self, cx: &App) -> Vec<(String, PathBuf)> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .filter(|project| !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
            .collect()
    }

    pub(super) fn own_project(&self, cx: &App) -> Option<(String, PathBuf)> {
        let id = self.context.project.as_deref()?;
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .find(|project| project.id == id && !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
    }

    pub(super) fn home(cx: &App) -> Option<PathBuf> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .home_dir
            .clone()
    }

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

    pub(super) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.body = Body::Remote;
            self.sync_watch(cx);
            cx.notify();
            return;
        }
        let subject = subject(&self.context);
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| this.apply_outcome(result, cx),
        );
    }

    pub(super) fn set_root(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.context.remote {
            return;
        }
        self.base_picker = None;
        self.checkout_picker = None;
        self.removed_checkout_notice = None;
        self.root_change += 1;
        let change = self.root_change;
        let subject = subject(&self.context);
        let client = self.client.clone();
        let previous = self.root_chain.take();
        let (reply, result) = futures::channel::oneshot::channel();
        self.root_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = reply.send(client.files_set_root(&subject, path.as_deref()));
        }));
        cx.spawn(async move |this, cx| {
            let Ok(result) = result.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.root_change != change {
                    return;
                }
                match result {
                    Ok(outcome) => {
                        let panel_subject = this.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = this.client.clone();
                        this.apply_outcome(Ok(outcome), cx);
                        files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_outcome(
        &mut self,
        outcome: Result<FilesRootOutcome, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let old_removed_other: Option<String> = match &self.body {
            Body::Root(old)
                if is_viewing_other_checkout(&old.root)
                    && !runner_core::app_paths::is_existing_dir(&old.root.path) =>
            {
                Some(
                    old.commits
                        .as_ref()
                        .and_then(|c| c.branch.clone())
                        .or_else(|| old.root.git.as_ref().and_then(|g| g.branch.clone()))
                        .unwrap_or_else(|| old.root.name.clone()),
                )
            }
            _ => None,
        };
        let same = matches!(
            (&outcome, &self.body),
            (Ok(FilesRootOutcome::Root(root)), Body::Root(state))
                if root.path == state.root.path && root.git.is_some()
        );
        if !same {
            self.generation += 1;
            self.base_picker = None;
            self.checkout_picker = None;
        }
        let sync_outcome = outcome.as_ref().ok().cloned();
        self.error = None;
        match outcome {
            Ok(FilesRootOutcome::Root(root)) => {
                if is_viewing_other_checkout(&root) {
                    self.removed_checkout_notice = None;
                } else if let Some(removed) =
                    root.removed_checkout.clone().or(old_removed_other.clone())
                {
                    self.removed_checkout_notice = Some(removed);
                    if old_removed_other.is_some() {
                        let panel_subject = self.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = self.client.clone();
                        files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
                }
                if root.git.is_none() {
                    self.pending_restore = None;
                    self.body = Body::NotInGit;
                    self.notify_owner(cx);
                } else if let Body::Root(state) = &mut self.body
                    && same
                {
                    state.root = root;
                    if self.shown {
                        self.load_snapshot_inner(true, true, false, cx);
                    }
                } else {
                    self.files_scroll = UniformListScrollHandle::new();
                    let entity_id = cx.entity_id();
                    let base = self.files_scroll.0.borrow().base_handle.clone();
                    self.files_scrollbar = cx.new(|_| Scrollbar::app(base, entity_id));
                    self.commits_scroll = ScrollHandle::new();
                    let chosen_base = root.git.as_ref().and_then(|git| {
                        files::settings(cx)?
                            .changes_base(&git.top, git.branch.as_deref())
                            .map(str::to_owned)
                    });
                    self.body = Body::Root(Box::new(RootChanges {
                        root,
                        commits: None,
                        chosen_base,
                        missing_base_notice: None,
                        selected_target: SelectedTarget::Uncommitted,
                        initial_target_chosen: false,
                        uncommitted_count: 0,
                        branch_count: 0,
                        files: Vec::new(),
                        totals: ChangesTotals::default(),
                        collapsed_folders: BTreeSet::new(),
                        list_rows: Vec::new(),
                        selected_path: None,
                        highlighted_list_row: 0,
                        snapshot_seq: 0,
                        files_seq: 0,
                    }));
                    if let Body::Root(state) = &self.body
                        && self.pending_restore.as_ref().is_some_and(|pending| {
                            !picker::same_checkout_path(&state.root.path, &pending.root)
                        })
                    {
                        self.pending_restore = None;
                        self.notify_owner(cx);
                    }
                    if self.shown {
                        self.load_snapshot_inner(true, true, false, cx);
                    }
                }
            }
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.pending_restore = None;
                self.body = Body::Choose;
                self.load_recent(cx);
                self.notify_owner(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.pending_restore = None;
                self.body = Body::Missing { path };
                self.notify_owner(cx);
            }
            Err(error) => {
                self.pending_restore = None;
                self.body = Body::Failed(error.to_string().into());
                self.notify_owner(cx);
            }
        }
        if let Some(root_outcome) = sync_outcome {
            self.viewer
                .update(cx, |viewer, cx| viewer.sync_root(root_outcome, cx));
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    pub(super) fn show_choose_screen(&mut self, cx: &mut Context<Self>) {
        self.body = Body::Choose;
        self.load_recent(cx);
        self.watched_root_changed(cx);
        cx.notify();
    }

    fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.request(
            cx,
            |client| client.files_recent_folders(),
            |this, result, cx| {
                if let Ok(recent) = result {
                    this.recent = recent;
                    cx.notify();
                }
            },
        );
    }

    pub(super) fn apply_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.choose_field.read(cx).text().to_owned();
        match picker::typed_path(&text, Self::home(cx).as_deref()) {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.list_focus.focus(window, cx);
            }
            None => {
                self.error = Some(crate::i18n::text("ui.type_a_full_path").into());
                cx.notify();
            }
        }
    }

    pub(super) fn browse(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.choose_folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.set_root(Some(path), cx));
        })
        .detach();
    }

    // ── Watch & live events ────────────────────────────────────────────

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        if self.watched.as_deref() != self.watched_git_root() {
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
        let wanted = self
            .shown
            .then(|| self.watched_git_root().map(Path::to_path_buf))
            .flatten();
        if let Some(root) = &wanted
            && !runner_core::app_paths::is_existing_dir(root)
        {
            self.unwatch(cx);
            self.resolve(cx);
            return;
        }
        if self.watched == wanted {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        let has_root = matches!(&self.body, Body::Root(_));
        self.watched = Some(root.clone());
        self.chain_watch(cx, move |client| {
            let _ = client.files_watch(&root);
        });
        if has_root {
            self.load_snapshot_inner(true, true, false, cx);
        }
    }

    fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if self.shown
                    && self
                        .watched_git_root()
                        .is_some_and(|watched| picker::same_checkout_path(watched, &changed.root))
                {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Status { root, .. } => {
                if self.shown
                    && self
                        .watched_git_root()
                        .is_some_and(|watched| picker::same_checkout_path(watched, root))
                {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Viewed(viewed) => {
                self.apply_viewed_event(viewed, cx);
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
                if !matches!(self.body, Body::Root(_)) {
                    self.resolve(cx);
                    return;
                }
                if !self.shown {
                    return;
                }
                if *rewatch {
                    self.watched = None;
                    self.sync_watch(cx);
                }
                self.load_snapshot(cx);
            }
        }
    }

    // ── Snapshot & diff loading ────────────────────────────────────────

    fn recover_from_root_error(&mut self, msg: &str, cx: &mut Context<Self>) -> bool {
        let root_missing = matches!(
            &self.body,
            Body::Root(state) if !runner_core::app_paths::is_existing_dir(&state.root.path)
        ) || msg.contains("is not an existing folder");
        if root_missing {
            self.unwatch(cx);
            self.resolve(cx);
            return true;
        }
        if msg.contains("not inside a git checkout") || msg.contains("git is not on PATH") {
            self.body = Body::NotInGit;
            self.watched_root_changed(cx);
            cx.notify();
            return true;
        }
        false
    }

    fn load_snapshot(&mut self, cx: &mut Context<Self>) {
        self.load_snapshot_inner(true, true, true, cx);
    }

    fn load_snapshot_inner(
        &mut self,
        reload_same_diff: bool,
        clear_error: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        let settings_snapshot = files::settings(cx).cloned();
        let Body::Root(state) = &mut self.body else {
            return;
        };
        state.snapshot_seq += 1;
        state.files_seq += 1;
        let seq = state.snapshot_seq;
        let files_seq = state.files_seq;
        let root_path = state.root.path.clone();
        let known_top_and_branch = state
            .commits
            .as_ref()
            .map(|c| (c.checkout_top.clone(), c.branch.clone()))
            .or_else(|| {
                state
                    .root
                    .git
                    .as_ref()
                    .map(|g| (g.top.clone(), g.branch.clone()))
            });
        let initial_base = known_top_and_branch.and_then(|(top, branch)| {
            settings_snapshot
                .as_ref()?
                .changes_base(&top, branch.as_deref())
                .map(str::to_owned)
        });
        let wanted_commit = match &state.selected_target {
            SelectedTarget::Commit(id) => Some(id.clone()),
            _ => None,
        };
        #[cfg(test)]
        let hold_rx = self.hold_snapshots.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_snapshots.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let mut commits = client.changes_commits(&root_path, initial_base.as_deref())?;
            let wanted_base = settings_snapshot
                .as_ref()
                .and_then(|s| s.changes_base(&commits.checkout_top, commits.branch.as_deref()))
                .map(str::to_owned);
            if wanted_base != initial_base {
                commits = client.changes_commits(&root_path, wanted_base.as_deref())?;
            }
            let effective_base = (!commits.base_missing)
                .then_some(wanted_base.as_deref())
                .flatten();
            let uncommitted = client.changes_files(
                &root_path,
                &runner_core::protocol::changes::ChangesTarget::Uncommitted,
                None,
            )?;
            let branch = if has_all_changes(&commits) {
                Some(client.changes_files(
                    &root_path,
                    &runner_core::protocol::changes::ChangesTarget::Branch,
                    effective_base,
                )?)
            } else {
                None
            };
            let commit_files = if let Some(id) = wanted_commit {
                if commits.commits.iter().any(|c| c.id == id) {
                    client
                        .changes_files(
                            &root_path,
                            &runner_core::protocol::changes::ChangesTarget::Commit(id.clone()),
                            None,
                        )
                        .ok()
                        .map(|files| (id, files))
                } else {
                    None
                }
            } else {
                None
            };
            Ok(SnapshotPayload {
                commits,
                wanted_base,
                uncommitted,
                branch,
                commit_files,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_snapshot(
                        seq,
                        files_seq,
                        reload_same_diff,
                        clear_error,
                        allow_preview_replace,
                        result,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_snapshot(
        &mut self,
        seq: u64,
        files_seq: u64,
        reload_same_diff: bool,
        clear_error: bool,
        allow_preview_replace: bool,
        result: Result<SnapshotPayload, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let is_current = match &self.body {
            Body::Root(state) => self.shown && seq >= state.snapshot_seq,
            _ => false,
        };
        if !is_current {
            return;
        }
        let payload = match result {
            Ok(payload) => payload,
            Err(error) => {
                let msg = error.to_string();
                if self.recover_from_root_error(&msg, cx) {
                    return;
                }
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if clear_error {
            self.error = None;
        }
        let prev_top_and_branch = state
            .commits
            .as_ref()
            .map(|c| (c.checkout_top.clone(), c.branch.clone()))
            .or_else(|| {
                state
                    .root
                    .git
                    .as_ref()
                    .map(|g| (g.top.clone(), g.branch.clone()))
            });
        let new_top_and_branch = (
            payload.commits.checkout_top.clone(),
            payload.commits.branch.clone(),
        );
        let branch_changed = prev_top_and_branch
            .as_ref()
            .is_some_and(|(prev_top, prev_branch)| {
                !picker::same_checkout_path(prev_top, &new_top_and_branch.0)
                    || prev_branch != &new_top_and_branch.1
            });
        let mut forget_missing_base: Option<(PathBuf, Option<String>)> = None;
        if payload.commits.base_missing {
            if let Some(missing_ref) = payload.wanted_base.clone() {
                state.missing_base_notice = Some(missing_ref);
                state.chosen_base = None;
                forget_missing_base = Some(new_top_and_branch);
            } else if branch_changed {
                state.missing_base_notice = None;
                state.chosen_base = None;
            }
        } else {
            if branch_changed
                || payload.wanted_base.is_some()
                || payload.wanted_base != state.chosen_base
            {
                state.missing_base_notice = None;
            }
            state.chosen_base = payload.wanted_base;
        }
        let uncommitted_count = payload.uncommitted.files.len();
        let branch_count = payload.branch.as_ref().map_or(0, |b| b.files.len());
        let prev_target = state.selected_target.clone();
        let prev_files_seq = state.files_seq;
        let next_target = if !state.initial_target_chosen {
            state.initial_target_chosen = true;
            choose_initial_target(&payload.commits, uncommitted_count)
        } else {
            reconcile_target(&state.selected_target, &payload.commits, uncommitted_count)
        };

        let active_files_opt = match &next_target {
            SelectedTarget::Uncommitted => Some(payload.uncommitted),
            SelectedTarget::Branch => Some(payload.branch.unwrap_or(ChangesFiles {
                checkout_top: payload.commits.checkout_top.clone(),
                files: Vec::new(),
                totals: ChangesTotals::default(),
            })),
            SelectedTarget::Commit(id) => payload
                .commit_files
                .filter(|(loaded_id, _)| loaded_id == id)
                .map(|(_, files)| files),
        };

        state.commits = Some(payload.commits);
        state.uncommitted_count = uncommitted_count;
        state.branch_count = branch_count;
        state.selected_target = next_target.clone();
        self.status_stale = false;
        if let Some((top, branch)) = forget_missing_base {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| settings.set_changes_base(&top, branch.as_deref(), None),
                    true,
                    cx,
                );
            });
        }
        if self.base_picker.is_some() {
            self.rebuild_base_picker();
        }
        if self.checkout_picker.is_some() {
            self.rebuild_checkout_picker(cx);
        }
        if next_target != SelectedTarget::Branch
            && let Body::Root(state) = &self.body
        {
            let root_path = state.root.path.clone();
            let chosen_base = state.chosen_base.clone();
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            self.viewer.update(cx, |viewer, cx| {
                viewer.sync_branch_base(
                    &root_path,
                    chosen_base.as_deref(),
                    resolved_base.as_deref(),
                    branch_name.as_deref(),
                    has_head,
                    true,
                    cx,
                );
            });
        }
        if files_seq < prev_files_seq && next_target == prev_target && active_files_opt.is_none() {
            cx.notify();
            return;
        }
        let active_files = active_files_opt.unwrap_or_else(|| {
            let Body::Root(state) = &self.body else {
                unreachable!()
            };
            ChangesFiles {
                checkout_top: state
                    .commits
                    .as_ref()
                    .map(|c| c.checkout_top.clone())
                    .unwrap_or_default(),
                files: Vec::new(),
                totals: ChangesTotals::default(),
            }
        });
        if let Body::Root(state) = &mut self.body {
            state.files_seq += 1;
        }
        self.apply_files_list(
            active_files,
            reload_same_diff && files_seq >= prev_files_seq,
            allow_preview_replace,
            cx,
        );
    }

    pub(super) fn select_target(&mut self, target: SelectedTarget, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if state.selected_target == target {
            return;
        }
        state.selected_target = target.clone();
        state.selected_path = None;
        state.highlighted_list_row = 0;
        state.files_seq += 1;
        let files_seq = state.files_seq;
        let root_path = state.root.path.clone();
        let chosen_base = matches!(target, SelectedTarget::Branch)
            .then(|| state.chosen_base.clone())
            .flatten();
        let proto = target.to_protocol();
        self.request(
            cx,
            move |client| client.changes_files(&root_path, &proto, chosen_base.as_deref()),
            move |this, result, cx| {
                let is_current = match &this.body {
                    Body::Root(state) => this.shown && files_seq >= state.files_seq,
                    _ => false,
                };
                if !is_current {
                    return;
                }
                match result {
                    Ok(files) => {
                        if let Body::Root(state) = &mut this.body {
                            match state.selected_target {
                                SelectedTarget::Uncommitted => {
                                    state.uncommitted_count = files.files.len();
                                }
                                SelectedTarget::Branch => {
                                    state.branch_count = files.files.len();
                                }
                                SelectedTarget::Commit(_) => {}
                            }
                        }
                        this.apply_files_list(files, false, false, cx);
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        if this.recover_from_root_error(&msg, cx) {
                            return;
                        }
                        this.error = Some(msg.into());
                        cx.notify();
                    }
                }
            },
        );
        cx.notify();
    }

    fn apply_files_list(
        &mut self,
        mut changes_files: ChangesFiles,
        reload_same_diff: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        changes_files.files.sort_by(|a, b| a.path.cmp(&b.path));
        let tree_mode = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        self.tree_view_pref = tree_mode;

        let reconcile = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let prev_path = state.selected_path.clone();
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_highlight_idx = state.highlighted_list_row;
            state.files = changes_files.files;
            state.totals = changes_files.totals;
            state.list_rows = if tree_mode {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            (
                state.root.path.clone(),
                state.selected_target.clone(),
                state.chosen_base.clone(),
                resolved_base,
                branch_name,
                has_head,
                state.files.clone(),
                state.list_rows.clone(),
                prev_path,
                prev_highlight,
                prev_highlight_idx,
            )
        };
        let (
            root_path,
            target,
            chosen_base,
            resolved_base,
            branch_name,
            has_head,
            files_snapshot,
            rows_snapshot,
            prev_path,
            prev_highlight,
            prev_highlight_idx,
        ) = reconcile;

        self.viewer.update(cx, |viewer, cx| {
            viewer.reconcile_changes_files(
                &root_path,
                &target,
                chosen_base.as_deref(),
                resolved_base.as_deref(),
                branch_name.as_deref(),
                has_head,
                &files_snapshot,
                &rows_snapshot,
                tree_mode,
                reload_same_diff,
                allow_preview_replace,
                cx,
            );
        });

        let active_key = self
            .viewer
            .read(cx)
            .tabs
            .active()
            .map(|tab| tab.key.clone());
        if let Body::Root(state) = &mut self.body {
            let next_path = active_key
                .filter(|key| {
                    key.target == state.selected_target
                        && state.files.iter().any(|f| f.path == key.path)
                })
                .map(|key| key.path);
            let same_path = next_path.is_some() && next_path == prev_path;
            state.selected_path = next_path;
            if state.selected_path.is_some() {
                let wanted = if same_path {
                    prev_highlight
                } else {
                    state
                        .selected_path
                        .as_ref()
                        .map(|p| RowIdentity::File(p.clone()))
                };
                state.highlighted_list_row = resolve_highlighted_row(
                    wanted.as_ref(),
                    state.selected_path.as_deref(),
                    prev_highlight_idx,
                    &state.list_rows,
                );
                if !same_path {
                    self.files_scroll
                        .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
                }
            } else {
                state.highlighted_list_row = if state.list_rows.is_empty() {
                    0
                } else {
                    resolve_highlighted_row(
                        prev_highlight.as_ref(),
                        None,
                        prev_highlight_idx,
                        &state.list_rows,
                    )
                };
            }
        }
        if let Some(saved) = self.pending_restore.take() {
            self.apply_saved_changes(saved, cx);
        }
        cx.notify();
    }

    pub(super) fn select_file_by_path(&mut self, path: String, pin: bool, cx: &mut Context<Self>) {
        let req = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let Some(file) = state.files.iter().find(|f| f.path == path).cloned() else {
                return;
            };
            if let Some(row_idx) = state
                .list_rows
                .iter()
                .position(|r| matches!(r, ListRow::File { path: p, .. } if p == &path))
            {
                state.highlighted_list_row = row_idx;
                self.files_scroll
                    .scroll_to_item(row_idx, ScrollStrategy::Nearest);
            }
            state.selected_path = Some(path.clone());
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let commit_short_id = match &state.selected_target {
                SelectedTarget::Commit(id) => state
                    .commits
                    .as_ref()
                    .and_then(|c| c.commits.iter().find(|commit| &commit.id == id))
                    .map(|c| c.short_id.clone())
                    .or_else(|| Some(id[..id.len().min(7)].to_owned())),
                _ => None,
            };
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            OpenChangesRequest {
                root: state.root.clone(),
                path,
                target: state.selected_target.clone(),
                chosen_base: state.chosen_base.clone(),
                resolved_base,
                branch_name,
                commit_short_id,
                file,
                has_head,
            }
        };
        self.viewer
            .update(cx, |viewer, cx| viewer.open_changes_file(req, pin, cx));
        cx.notify();
    }

    // ── Viewed marks & flat/tree toggle ────────────────────────────────

    pub(super) fn toggle_viewed_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let (root_path, selected_target, old_blob, new_blob, next_viewed, next_changed) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let Some(file) = state.files.iter_mut().find(|f| f.path == path) else {
                return;
            };
            let next_viewed = !file.viewed;
            file.viewed = next_viewed;
            if next_viewed {
                file.changed_since_viewed = false;
            }
            let next_changed = file.changed_since_viewed;
            let old_blob = file.old_blob_id.clone();
            let new_blob = file.new_blob_id.clone();
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            (
                state.root.path.clone(),
                state.selected_target.clone(),
                old_blob,
                new_blob,
                next_viewed,
                next_changed,
            )
        };
        self.viewer.update(cx, |viewer, cx| {
            viewer.apply_changes_viewed_optimistic(
                &root_path,
                &selected_target,
                path,
                old_blob.as_deref(),
                new_blob.as_deref(),
                next_viewed,
                next_changed,
                cx,
            );
        });
        let req_path = path.to_owned();
        let target = selected_target.to_protocol();
        self.send_viewed_toggle(
            root_path,
            target,
            req_path,
            old_blob,
            new_blob,
            next_viewed,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_viewed_toggle(
        &mut self,
        root_path: PathBuf,
        target: runner_core::protocol::changes::ChangesTarget,
        req_path: String,
        old_blob: Option<String>,
        new_blob: Option<String>,
        next_viewed: bool,
        cx: &mut Context<Self>,
    ) {
        self.request(
            cx,
            move |client| {
                client.changes_set_viewed(
                    &root_path,
                    &target,
                    &req_path,
                    old_blob.as_deref(),
                    new_blob.as_deref(),
                    next_viewed,
                )
            },
            |this, result, cx| {
                if let Err(error) = result {
                    let msg = error.to_string();
                    if this.recover_from_root_error(&msg, cx) {
                        return;
                    }
                    this.error = Some(msg.into());
                    this.load_snapshot_inner(false, false, false, cx);
                }
            },
        );
        cx.notify();
    }

    fn apply_viewed_event(&mut self, event: &ChangesViewed, cx: &mut Context<Self>) {
        let mut matched_checkout = false;
        if let Body::Root(state) = &mut self.body
            && state
                .commits
                .as_ref()
                .is_some_and(|c| picker::same_checkout_path(&c.checkout_top, &event.checkout_top))
        {
            matched_checkout = true;
            let event_top = runner_core::app_paths::normalize_checkout_path(&event.checkout_top);
            let root_norm = runner_core::app_paths::normalize_checkout_path(&state.root.path);
            let event_abs = absolute(&event_top, &event.path);
            for file in &mut state.files {
                let file_abs = absolute(&root_norm, &file.path);
                if runner_core::app_paths::same_normalized_checkout_path(&file_abs, &event_abs)
                    && file.old_blob_id == event.old_blob_id
                    && file.new_blob_id == event.new_blob_id
                {
                    file.viewed = event.viewed;
                    if event.viewed {
                        file.changed_since_viewed = false;
                    }
                }
            }
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
        }
        if matched_checkout {
            if self.shown {
                self.load_snapshot_inner(false, true, false, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn set_tree_view(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_tree_view != tree_view;
                    settings.changes_tree_view = tree_view;
                    changed
                },
                true,
                cx,
            );
        });
        self.rebuild_list_rows(tree_view, cx);
    }

    fn rebuild_list_rows(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        self.tree_view_pref = tree_view;
        if let Body::Root(state) = &mut self.body {
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_idx = state.highlighted_list_row;
            state.list_rows = if tree_view {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };
            state.highlighted_list_row = resolve_highlighted_row(
                prev_highlight.as_ref(),
                state.selected_path.as_deref(),
                prev_idx,
                &state.list_rows,
            );
            self.files_scroll
                .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    pub(super) fn toggle_folder(&mut self, folder_path: &str, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if !state.collapsed_folders.remove(folder_path) {
            state.collapsed_folders.insert(folder_path.to_owned());
        }
        let tree_mode = self.tree_view_pref;
        self.rebuild_list_rows(tree_mode, cx);
    }

    // ── Base & Checkout pickers ────────────────────────────────────────

    pub(super) fn toggle_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.base_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.checkout_picker = None;
        let root_path = match &mut self.body {
            Body::Root(state) => {
                state.missing_base_notice = None;
                state.root.path.clone()
            }
            _ => return,
        };
        let return_focus = if self.base_chip_focus.is_focused(window) {
            self.base_chip_focus.clone()
        } else {
            self.list_focus.clone()
        };
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_branches"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| this.on_base_picker_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.base_picker
                && picker.query != query
            {
                picker.query = query;
                picker.moved = false;
                let visible = filter_base_picker_items(&picker.items, &picker.query);
                picker.highlighted = default_base_highlight(&picker.items, &visible, &picker.query);
                picker.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.base_picker = Some(BasePickerState {
            field,
            query: String::new(),
            branches: ChangesBranches::default(),
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        self.rebuild_base_picker();
        self.request(
            cx,
            move |client| client.changes_branches(&root_path),
            |this, result, cx| {
                if let (Some(picker), Ok(branches)) = (&mut this.base_picker, result) {
                    picker.branches = branches;
                    this.rebuild_base_picker();
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn close_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.base_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn rebuild_base_picker(&mut self) {
        let (chosen_base, auto_base, current_branch) = match &self.body {
            Body::Root(state) => {
                let auto = state.commits.as_ref().and_then(|c| c.auto_base.clone());
                let branch = state
                    .commits
                    .as_ref()
                    .and_then(|c| c.branch.clone())
                    .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
                (state.chosen_base.clone(), auto, branch)
            }
            _ => return,
        };
        let Some(picker) = &mut self.base_picker else {
            return;
        };
        picker.items = build_base_picker_items(
            chosen_base.as_deref(),
            auto_base.as_deref(),
            current_branch.as_deref(),
            &picker.branches,
            crate::i18n::text("changes.base_auto"),
            crate::i18n::text("changes.base_none"),
        );
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.len().saturating_sub(1))
        } else {
            default_base_highlight(&picker.items, &visible, &picker.query)
        };
    }

    fn on_base_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = &mut self.base_picker else {
            return false;
        };
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.moved = true;
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.moved = true;
                picker.reveal.set(true);
            }
            "escape" => {
                self.close_base_picker(window, cx);
                return true;
            }
            "enter" => {
                if let Some(index) = visible.get(picker.highlighted).copied()
                    && let Some(item) = picker.items.get(index).cloned()
                {
                    self.pick_base(item.base_ref, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn pick_base(
        &mut self,
        next_base: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = self.base_picker.take().map(|p| p.return_focus);
        let (top_and_branch, sync_info) = match &mut self.body {
            Body::Root(state) => {
                state.missing_base_notice = None;
                state.chosen_base = next_base.clone();
                let top_and_branch = state
                    .commits
                    .as_ref()
                    .map(|c| (c.checkout_top.clone(), c.branch.clone()))
                    .or_else(|| {
                        state
                            .root
                            .git
                            .as_ref()
                            .map(|g| (g.top.clone(), g.branch.clone()))
                    });
                let branch_name = state
                    .commits
                    .as_ref()
                    .and_then(|c| c.branch.clone())
                    .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
                let has_head = state.commits.as_ref().map_or_else(
                    || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                    |c| c.head_commit.is_some(),
                );
                let sync_info = (
                    state.root.path.clone(),
                    branch_name,
                    has_head,
                    state.selected_target != SelectedTarget::Branch,
                );
                (top_and_branch, Some(sync_info))
            }
            _ => (None, None),
        };
        if let Some((root_path, branch_name, has_head, reload_now)) = sync_info {
            self.viewer.update(cx, |viewer, cx| {
                viewer.sync_branch_base(
                    &root_path,
                    next_base.as_deref(),
                    next_base.as_deref(),
                    branch_name.as_deref(),
                    has_head,
                    reload_now,
                    cx,
                );
            });
        }
        if let Some((top, branch)) = top_and_branch {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        settings.set_changes_base(&top, branch.as_deref(), next_base.as_deref())
                    },
                    true,
                    cx,
                );
            });
        }
        if let Some(focus) = return_focus {
            focus.focus(window, cx);
        }
        self.load_snapshot_inner(true, true, false, cx);
        cx.notify();
    }

    pub(super) fn toggle_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.base_picker = None;
        self.removed_checkout_notice = None;
        let (root_path, initial_worktrees) = match &self.body {
            Body::Root(state) => {
                let wts = state
                    .commits
                    .as_ref()
                    .map(|c| c.worktrees.clone())
                    .filter(|w| !w.is_empty())
                    .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.clone()))
                    .unwrap_or_default();
                (state.root.path.clone(), wts)
            }
            _ => return,
        };
        let return_focus = if self.checkout_button_focus.is_focused(window) {
            self.checkout_button_focus.clone()
        } else {
            self.list_focus.clone()
        };
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_checkouts"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| {
                    this.on_checkout_picker_key(event, window, cx)
                })
                .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.checkout_picker
                && picker.query != query
            {
                picker.query = query;
                picker.moved = false;
                let visible = filter_checkout_picker_items(&picker.items, &picker.query);
                picker.highlighted =
                    default_checkout_highlight(&picker.items, &visible, &picker.query);
                picker.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.checkout_picker = Some(CheckoutPickerState {
            field,
            query: String::new(),
            worktrees: initial_worktrees,
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        self.rebuild_checkout_picker(cx);
        self.request(
            cx,
            move |client| client.files_worktrees(&root_path),
            |this, result, cx| {
                if let (Some(picker), Ok(worktrees)) = (&mut this.checkout_picker, result) {
                    picker.worktrees = worktrees;
                    this.rebuild_checkout_picker(cx);
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn close_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn rebuild_checkout_picker(&mut self, cx: &App) {
        let (current_top, own_checkout_top) = match &self.body {
            Body::Root(state) => {
                let current_top = state
                    .commits
                    .as_ref()
                    .map(|c| c.checkout_top.clone())
                    .or_else(|| state.root.git.as_ref().map(|g| g.top.clone()))
                    .unwrap_or_else(|| state.root.path.clone());
                (current_top, state.root.own_checkout_top.clone())
            }
            _ => return,
        };
        let home = Self::home(cx);
        let context = self.context.clone();
        let Some(picker) = &mut self.checkout_picker else {
            return;
        };
        let session_labels = picker::collect_checkout_session_labels(
            &context,
            own_checkout_top.as_deref(),
            &picker.worktrees,
            cx,
        );
        picker.items = build_checkout_picker_items(
            &current_top,
            &picker.worktrees,
            &session_labels,
            home.as_deref(),
            crate::i18n::text("ui.detached"),
        );
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.len().saturating_sub(1))
        } else {
            default_checkout_highlight(&picker.items, &visible, &picker.query)
        };
    }

    fn on_checkout_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = &mut self.checkout_picker else {
            return false;
        };
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.moved = true;
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.moved = true;
                picker.reveal.set(true);
            }
            "escape" => {
                self.close_checkout_picker(window, cx);
                return true;
            }
            "enter" => {
                if let Some(index) = visible.get(picker.highlighted).copied()
                    && let Some(item) = picker.items.get(index).cloned()
                {
                    self.pick_checkout(item.path, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn pick_checkout(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = self.checkout_picker.take().map(|p| p.return_focus);
        let is_own = matches!(
            &self.body,
            Body::Root(state)
                if state
                    .root
                    .own_checkout_top
                    .as_deref()
                    .is_some_and(|own| picker::same_checkout_path(own, &path))
        );
        self.set_root((!is_own).then_some(path), cx);
        if let Some(focus) = return_focus {
            focus.focus(window, cx);
        } else {
            self.list_focus.focus(window, cx);
        }
    }

    // ── Settings, focus & keyboard ─────────────────────────────────────

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if self.checkout_picker.is_some() {
            self.rebuild_checkout_picker(cx);
        }
        let external_base_change = match &mut self.body {
            Body::Root(state) => {
                let top_and_branch = state
                    .commits
                    .as_ref()
                    .map(|c| (&c.checkout_top, c.branch.as_deref()))
                    .or_else(|| {
                        state
                            .root
                            .git
                            .as_ref()
                            .map(|g| (&g.top, g.branch.as_deref()))
                    });
                if let Some((top, branch)) = top_and_branch {
                    let next = files::settings(cx)
                        .and_then(|s| s.changes_base(top, branch))
                        .map(str::to_owned);
                    if next != state.chosen_base {
                        state.chosen_base = next;
                        state.missing_base_notice = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if external_base_change {
            self.load_snapshot_inner(true, true, false, cx);
        }
        let look = Look::current(cx);
        let tree_pref = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        if tree_pref != self.tree_view_pref {
            self.rebuild_list_rows(tree_pref, cx);
        }
        if look != self.look {
            self.look = look;
            cx.notify();
        }
    }

    pub(super) fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = window.focus_lost_restore_target(cx) else {
            return;
        };
        if target == self.root_focus
            || target == self.checkout_button_focus
            || target == self.checkout_return_focus
            || target == self.base_chip_focus
        {
            self.focus_list(window, cx);
        }
    }

    pub(crate) fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_place == FocusPlace::Diff {
            self.focus_place = FocusPlace::Files;
        }
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn enter_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_place = FocusPlace::Diff;
        self.viewer
            .update(cx, |viewer, cx| viewer.focus_active(window, cx));
        cx.notify();
    }

    pub(super) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        self.viewer
            .update(cx, |viewer, cx| viewer.jump_hunk(forward, cx))
    }

    fn has_checkout_trigger(&self) -> bool {
        let Body::Root(state) = &self.body else {
            return false;
        };
        let viewing_other = is_viewing_other_checkout(&state.root);
        let worktrees_slice = state
            .commits
            .as_ref()
            .map(|c| c.worktrees.as_slice())
            .filter(|w| !w.is_empty())
            .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.as_slice()))
            .unwrap_or(&[]);
        live_worktrees(worktrees_slice).len() > 1 || viewing_other
    }

    fn is_viewing_other(&self) -> bool {
        matches!(&self.body, Body::Root(state) if is_viewing_other_checkout(&state.root))
    }

    fn cycle_list_focus(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum ListStop {
            Files,
            Commits,
            Base,
            Checkout,
            CheckoutReturn,
        }

        let mut stops = vec![ListStop::Files, ListStop::Commits, ListStop::Base];
        if self.has_checkout_trigger() {
            stops.push(ListStop::Checkout);
            if self.is_viewing_other() {
                stops.push(ListStop::CheckoutReturn);
            }
        }
        let current = if self.checkout_return_focus.is_focused(window) {
            ListStop::CheckoutReturn
        } else if self.checkout_button_focus.is_focused(window) {
            ListStop::Checkout
        } else if self.base_chip_focus.is_focused(window) {
            ListStop::Base
        } else if self.focus_place == FocusPlace::Commits {
            ListStop::Commits
        } else {
            ListStop::Files
        };
        let pos = stops.iter().position(|&s| s == current).unwrap_or(0);
        let next_pos = if backwards {
            pos.checked_sub(1).unwrap_or(stops.len() - 1)
        } else {
            (pos + 1) % stops.len()
        };
        match stops[next_pos] {
            ListStop::Files => {
                self.focus_place = FocusPlace::Files;
                self.list_focus.focus(window, cx);
            }
            ListStop::Commits => {
                self.focus_place = FocusPlace::Commits;
                self.list_focus.focus(window, cx);
            }
            ListStop::Base => {
                self.base_chip_focus.focus(window, cx);
            }
            ListStop::Checkout => {
                self.checkout_button_focus.focus(window, cx);
            }
            ListStop::CheckoutReturn => {
                self.checkout_return_focus.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn on_list_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        if event.keystroke.key == "tab" {
            cx.stop_propagation();
            self.cycle_list_focus(modifiers.shift, window, cx);
            return;
        }
        if !self.list_focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" | "j" => {
                cx.stop_propagation();
                self.step_list(1, cx);
            }
            "up" | "k" => {
                cx.stop_propagation();
                self.step_list(-1, cx);
            }
            "left" if self.focus_place == FocusPlace::Files => {
                if self.move_list_left(cx) {
                    cx.stop_propagation();
                }
            }
            "right" if self.focus_place == FocusPlace::Files => {
                if self.move_list_right(cx) {
                    cx.stop_propagation();
                }
            }
            "enter" => {
                cx.stop_propagation();
                if self.focus_place == FocusPlace::Files
                    && let Some((folder_path, _)) = self.highlighted_folder()
                {
                    self.toggle_folder(&folder_path, cx);
                } else if self.focus_place == FocusPlace::Files {
                    let open_path = match &self.body {
                        Body::Root(state) if state.selected_path.is_none() => {
                            match state.list_rows.get(state.highlighted_list_row) {
                                Some(ListRow::File { path, .. }) => Some(path.clone()),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(path) = open_path {
                        self.select_file_by_path(path, false, cx);
                    } else if self.viewer.read(cx).has_tabs() {
                        self.enter_diff(window, cx);
                    }
                } else if self.viewer.read(cx).has_tabs() {
                    self.enter_diff(window, cx);
                }
            }
            "space" if self.focus_place == FocusPlace::Files => {
                cx.stop_propagation();
                if let Some((folder_path, _)) = self.highlighted_folder() {
                    self.toggle_folder(&folder_path, cx);
                } else if let Body::Root(state) = &self.body {
                    let path = match state.list_rows.get(state.highlighted_list_row) {
                        Some(ListRow::File { path, .. }) => Some(path.clone()),
                        _ => state.selected_path.clone(),
                    };
                    if let Some(path) = path {
                        self.toggle_viewed_for_path(&path, cx);
                    }
                }
            }
            "]" => {
                cx.stop_propagation();
                self.jump_hunk(true, cx);
            }
            "[" => {
                cx.stop_propagation();
                self.jump_hunk(false, cx);
            }
            _ => {}
        }
    }

    fn move_list_left(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((folder_path, true)) = self.highlighted_folder() {
            self.toggle_folder(&folder_path, cx);
            return true;
        }
        let parent_idx = {
            let Body::Root(state) = &self.body else {
                return false;
            };
            let Some(row) = state.list_rows.get(state.highlighted_list_row) else {
                return false;
            };
            find_nearest_visible_ancestor_folder(row.path(), &state.list_rows)
        };
        if let Some(idx) = parent_idx
            && let Body::Root(state) = &mut self.body
        {
            state.highlighted_list_row = idx;
            self.files_scroll
                .scroll_to_item(idx, ScrollStrategy::Nearest);
            cx.notify();
            return true;
        }
        false
    }

    fn move_list_right(&mut self, cx: &mut Context<Self>) -> bool {
        match self.highlighted_folder() {
            Some((folder_path, false)) => {
                self.toggle_folder(&folder_path, cx);
                true
            }
            Some((_, true)) => {
                let has_visible_child = match &self.body {
                    Body::Root(state) => match (
                        state.list_rows.get(state.highlighted_list_row),
                        state.list_rows.get(state.highlighted_list_row + 1),
                    ) {
                        (
                            Some(ListRow::Folder { depth, .. }),
                            Some(
                                ListRow::Folder {
                                    depth: child_depth, ..
                                }
                                | ListRow::File {
                                    depth: child_depth, ..
                                },
                            ),
                        ) => child_depth > depth,
                        _ => false,
                    },
                    _ => false,
                };
                if has_visible_child {
                    self.step_list(1, cx);
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    fn highlighted_folder(&self) -> Option<(String, bool)> {
        let Body::Root(state) = &self.body else {
            return None;
        };
        match state.list_rows.get(state.highlighted_list_row)? {
            ListRow::Folder { path, expanded, .. } => Some((path.clone(), *expanded)),
            ListRow::File { .. } => None,
        }
    }

    fn step_list(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.focus_place {
            FocusPlace::Commits => {
                let next_target = {
                    let Body::Root(state) = &self.body else {
                        return;
                    };
                    let Some(commits) = &state.commits else {
                        return;
                    };
                    let rows: Vec<CommitRow> =
                        build_commit_rows(commits, state.branch_count, state.uncommitted_count);
                    if rows.is_empty() {
                        return;
                    }
                    let current_idx = rows
                        .iter()
                        .position(|r| r.target() == state.selected_target)
                        .unwrap_or(0);
                    let next_idx = (current_idx as isize + delta)
                        .clamp(0, rows.len().saturating_sub(1) as isize)
                        as usize;
                    self.commits_scroll.scroll_to_item(next_idx);
                    rows[next_idx].target()
                };
                self.select_target(next_target, cx);
            }
            FocusPlace::Files | FocusPlace::Diff => {
                self.focus_place = FocusPlace::Files;
                let file_to_select = {
                    let Body::Root(state) = &mut self.body else {
                        return;
                    };
                    if state.list_rows.is_empty() {
                        return;
                    }
                    let next_idx = (state.highlighted_list_row as isize + delta)
                        .clamp(0, state.list_rows.len().saturating_sub(1) as isize)
                        as usize;
                    state.highlighted_list_row = next_idx;
                    self.files_scroll
                        .scroll_to_item(next_idx, ScrollStrategy::Nearest);
                    match &state.list_rows[next_idx] {
                        ListRow::File { path, .. } => Some(path.clone()),
                        _ => None,
                    }
                };
                if let Some(path) = file_to_select {
                    self.select_file_by_path(path, false, cx);
                } else {
                    cx.notify();
                }
            }
        }
    }
}

#[cfg(test)]
impl ChangesView {
    pub(crate) fn receive(&mut self, event: FilesEvent, cx: &mut Context<Self>) {
        self.on_files_event(&event, cx);
        self.viewer
            .update(cx, |viewer, cx| viewer.on_files_event(&event, cx));
    }

    pub(crate) fn viewer(&self) -> Entity<UnifiedViewer> {
        self.viewer.clone()
    }

    pub(crate) fn widening(&self, cx: &App) -> Option<bool> {
        self.viewer.read(cx).widening()
    }

    pub(crate) fn selected_target(&self) -> Option<&SelectedTarget> {
        match &self.body {
            Body::Root(state) => Some(&state.selected_target),
            _ => None,
        }
    }

    pub(crate) fn selected_path(&self) -> Option<&str> {
        match &self.body {
            Body::Root(state) => state.selected_path.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn focus_place(&self) -> FocusPlace {
        self.focus_place
    }

    pub(crate) fn unified_diff_view(&self, cx: &App) -> Option<Entity<UnifiedDiffView>> {
        self.viewer.read(cx).tab_diff_view()
    }

    pub(crate) fn set_hold_diffs(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |viewer, _| viewer.set_hold_diffs(hold));
    }

    pub(crate) fn release_held_diffs(&mut self, cx: &mut Context<Self>) -> usize {
        self.viewer
            .update(cx, |viewer, _| viewer.release_held_diffs())
    }

    pub(crate) fn set_hold_snapshots(&mut self, hold: bool) {
        self.hold_snapshots = hold;
    }

    pub(crate) fn held_snapshots_len(&self) -> usize {
        self.held_snapshots.len()
    }

    pub(crate) fn release_held_snapshots(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_snapshots.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn reload_diff_for_test(&mut self, path: &str, cx: &mut Context<Self>) {
        let target = match &self.body {
            Body::Root(state) => state.selected_target.clone(),
            _ => SelectedTarget::Uncommitted,
        };
        let key = TabKey::new(path, target);
        self.viewer
            .update(cx, |viewer, cx| viewer.load_tab_diff(key, cx));
    }

    pub(crate) fn highlighted_row(&self) -> Option<&ListRow> {
        match &self.body {
            Body::Root(state) => state.list_rows.get(state.highlighted_list_row),
            _ => None,
        }
    }

    pub(crate) fn is_diff_loading(&self, cx: &App) -> bool {
        self.viewer.read(cx).is_diff_loading()
    }

    pub(crate) fn watched(&self) -> Option<&Path> {
        self.watched.as_deref()
    }
}

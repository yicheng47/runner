//! The side panel's Changes view (#1141): changed files, branch commits since
//! merge base, Unified diff with two-version syntax highlighting, and viewed marks.

pub(crate) mod diff;
mod diff_body;
mod list;
pub(crate) mod model;
mod pickers;
mod render;
mod root;
mod snapshot;
#[cfg(test)]
mod test_support;
mod viewed;

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
pub(crate) use diff_body::{prepare_diff_body, LoadedDiffKind, PreparedDiffBody};
use list::{resolve_highlighted_row, row_identity, RowIdentity};
use model::{
    build_base_picker_items, build_checkout_picker_items, build_commit_rows, build_flat_rows,
    build_tree_rows, choose_initial_target, default_base_highlight, default_checkout_highlight,
    filter_base_picker_items, filter_checkout_picker_items, has_all_changes, is_crlf_only_change,
    is_viewing_other_checkout, live_worktrees, reconcile_target, BasePickerItem,
    CheckoutPickerItem, CommitRow, FocusPlace, ListRow, SelectedTarget,
};
use pickers::{BasePickerState, CheckoutPickerState};
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

    pub(crate) fn changes_root_is_git(&self) -> Option<bool> {
        match &self.body {
            Body::Loading => None,
            Body::Remote | Body::Root(_) => Some(true),
            Body::NotInGit | Body::Choose | Body::Missing { .. } | Body::Failed(_) => Some(false),
        }
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
                !key.is_outside()
                    && key.target == state.selected_target
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
}

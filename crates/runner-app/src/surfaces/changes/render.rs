//! Rendering for `ChangesView`: diff header & placeholders, Commits section,
//! flat/tree file list, and root edge states.

mod branch;
mod choose;
mod list;
mod shared_diff;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, px, relative, svg, uniform_list, AnyElement, App, ClickEvent, Context,
    FontWeight, ImageSource, KeyDownEvent, MouseButton, Resource, SharedString, Window,
};
use runner_app::ui::{
    menu::popup_layer_right_aligned, tooltip::tooltip_view, Button, ButtonVariant, IconButton,
};
use runner_core::protocol::changes::{DiffCounts, ModeChange};
use runner_core::protocol::files::{FileStatus, WorktreeKind};

use crate::app_settings::{DiffViewMode, FileLinkEditor};
use crate::file_icons::{self, IconSet};
use crate::surfaces::files::{
    self, absolute, compact_missing_path, editor_icon, format_size, found_editors, picker,
};
use crate::surfaces::side_panel::PanelSurface;
use crate::text_util::{abbreviate_home, HomePath};
use crate::theme;

use super::diff;
use super::diff::render_viewed_checkbox;
use super::model::{
    abbreviate_folder, base_chip_label, branch_header_label, build_commit_rows, display_base_ref,
    file_name, filter_base_picker_items, filter_checkout_picker_items, is_viewing_other_checkout,
    live_worktrees, older_commits_hidden, status_letter_and_color, CommitRow, FocusPlace, ListRow,
};
use super::{Body, ChangesView, LoadedDiffKind, RootChanges};
pub(crate) use shared_diff::{
    render_diff_viewed_footer, render_shared_diff_header, render_shared_diff_placeholder,
    DiffHeaderProps, DiffModeCallback, PlaceholderViewedFooter,
};

const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;
const BASE_PICKER_WIDTH: f32 = 300.;
const CHECKOUT_PICKER_WIDTH: f32 = 420.;

impl Render for ChangesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.has_dirty_changes();
        if dirty != self.reported_dirty {
            self.reported_dirty = dirty;
            self.notify_owner(cx);
        }
        if self._focus_lost.is_none() {
            self._focus_lost = Some(cx.on_focus_lost(window, Self::on_focus_lost));
        }
        let context = self.context.clone();

        let inner = match &self.body {
            Body::Loading => self.render_loading(),
            Body::Remote => self.render_message(
                "CHANGES_REMOTE",
                "laptop.svg",
                crate::i18n::text("changes.arent_available_here"),
                crate::i18n::tf("changes.on_another_computer", &[("subject", self.noun())]),
                None,
            ),
            Body::Choose => self.render_choose(cx),
            Body::Missing { path } => {
                let path = path.clone();
                self.render_missing(&path, cx)
            }
            Body::NotInGit => self.render_message(
                "CHANGES_NOT_GIT",
                "git-branch.svg",
                crate::i18n::text("changes.not_a_git_repo"),
                crate::i18n::text("changes.not_a_git_repo_body").to_owned(),
                None,
            ),
            Body::Failed(error) => {
                let retry = cx.weak_entity();
                let error = error.clone();
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "CHANGES_FAILED".into())
                    })
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::danger())
                            .child(error),
                    )
                    .child(
                        Button::new("changes-retry", crate::i18n::text("ui.try_again"))
                            .variant(ButtonVariant::Secondary)
                            .debug_selector("CHANGES_RETRY")
                            .on_press(move |_, cx| {
                                let _ = retry.update(cx, |this, cx| this.resolve(cx));
                            }),
                    )
                    .into_any_element()
            }
            Body::Root(_) => self.render_root_view(window, cx),
        };

        div()
            .when(cfg!(test), |view| {
                view.debug_selector(|| format!("CHANGES_VIEW {context:?}"))
            })
            .key_context("FileViewer")
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|this, _: &crate::CloseFileTab, window, cx| {
                let active = this.viewer.read(cx).tabs.active_index();
                match active {
                    Some(index) => {
                        this.viewer
                            .update(cx, |viewer, cx| viewer.close_tab(index, window, cx));
                    }
                    None => cx.propagate(),
                }
            }))
            .on_action(cx.listener(|this, _: &crate::NextFileTab, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.cycle_tab(false, window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::PreviousFileTab, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.cycle_tab(true, window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::FindInFile, window, cx| {
                if this.viewer.read(cx).has_tabs() {
                    this.viewer
                        .update(cx, |viewer, cx| viewer.open_find(window, cx));
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|_, _: &crate::ZoomFileIn, window, cx| {
                window.dispatch_action(Box::new(crate::ZoomIn), cx);
            }))
            .on_action(cx.listener(|_, _: &crate::ZoomFileOut, window, cx| {
                window.dispatch_action(Box::new(crate::ZoomOut), cx);
            }))
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .relative()
            .child(inner)
    }
}

impl ChangesView {
    fn render_loading(&self) -> AnyElement {
        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_LOADING".into())
            })
            .flex_1()
            .min_h(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(theme::text_ui())
            .text_color(theme::faint())
            .child(crate::i18n::text("changes.loading"))
            .into_any_element()
    }

    fn render_root_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let base_picker = self.render_base_picker(window, cx);
        let checkout_picker = self.render_checkout_picker(window, cx);
        let list_column = self.render_list_column(window, cx);
        div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .relative()
            .child(list_column)
            .children(base_picker)
            .children(checkout_picker)
            .into_any_element()
    }
}

fn render_centered_message(
    selector: &'static str,
    icon: &'static str,
    title: String,
    body: String,
    actions: Option<AnyElement>,
) -> AnyElement {
    let title_shared: SharedString = title.into();
    div()
        .when(cfg!(test), |el| el.debug_selector(move || selector.into()))
        .flex_1()
        .min_h(px(0.))
        .px(px(28.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .text_center()
        .child(svg().size(px(28.)).path(icon).text_color(theme::faint()))
        .child(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(title_shared),
        )
        .when(!body.is_empty(), |el| {
            el.child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(body),
            )
        })
        .children(actions.map(|actions| div().pt(px(8.)).child(actions)))
        .into_any_element()
}

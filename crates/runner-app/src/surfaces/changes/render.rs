//! Rendering for `ChangesView`: diff header & placeholders, Commits section,
//! flat/tree file list, and root edge states.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, img, px, relative, rems, svg, uniform_list, AnyElement, App, Context, FontWeight,
    ImageSource, KeyDownEvent, MouseButton, Resource, SharedString, Window,
};
use runner_app::ui::{
    menu::popup_layer_right_aligned,
    resize::{resize_strip, resize_strip_inset, ResizeAxis},
    tooltip::tooltip_view,
    Button, ButtonVariant, IconButton,
};
use runner_core::protocol::changes::{DiffCounts, ModeChange};
use runner_core::protocol::files::{FileStatus, WorktreeKind};

use crate::app_settings::{DiffViewMode, FileLinkEditor};
use crate::file_icons::{self, IconSet};
use crate::surfaces::files::{
    self, absolute, compact_missing_path, editor_icon, format_size, found_editors, picker,
    FileTreeResizeDrag,
};
use crate::surfaces::side_panel::PanelSurface;
use crate::text_util::{abbreviate_home, HomePath};
use crate::theme;

use super::diff::render_viewed_checkbox;
use super::model::{
    abbreviate_folder, base_chip_label, branch_header_label, build_commit_rows, display_base_ref,
    file_name, filter_base_picker_items, filter_checkout_picker_items, is_viewing_other_checkout,
    live_worktrees, older_commits_hidden, status_letter_and_color, CommitRow, FocusPlace, ListRow,
    SelectedTarget,
};
use super::{ActiveDiff, Body, ChangesView, LoadedDiff, LoadedDiffKind, RootChanges};

const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;
const BASE_PICKER_WIDTH: f32 = 300.;
const CHECKOUT_PICKER_WIDTH: f32 = 420.;

impl Render for ChangesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reported = self.widening();
        if reported != self.reported_wide {
            self.reported_wide = reported;
            if let Some(owner) = self.owner {
                cx.defer(move |cx| cx.notify(owner));
            }
        }
        if self._focus_lost.is_none() {
            self._focus_lost = Some(cx.on_focus_lost(window, Self::on_focus_lost));
        }
        if self.focus_place == FocusPlace::Diff
            && self.diff_focus.is_focused(window)
            && let Body::Root(state) = &self.body
            && let ActiveDiff::Loaded(loaded) = &state.diff_state
            && let LoadedDiffKind::Unified(entity) = &loaded.kind
        {
            entity.read(cx).focus_handle().clone().focus(window, cx);
        }
        let context = self.context.clone();
        let zoom = crate::app_store::global_app_store(cx)
            .read(cx)
            .settings
            .app_zoom;

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
            Body::Root(_) => self.render_root_view(zoom, window, cx),
        };

        div()
            .when(cfg!(test), |view| {
                view.debug_selector(|| format!("CHANGES_VIEW {context:?}"))
            })
            .key_context("FileViewer")
            .track_focus(&self.root_focus)
            .on_action(cx.listener(|_, _: &crate::CloseFileTab, _, cx| cx.propagate()))
            .on_action(cx.listener(|_, _: &crate::NextFileTab, _, cx| cx.propagate()))
            .on_action(cx.listener(|_, _: &crate::PreviousFileTab, _, cx| cx.propagate()))
            .on_action(cx.listener(|_, _: &crate::FindInFile, _, cx| cx.propagate()))
            .on_action(cx.listener(|this, _: &crate::ZoomFileIn, window, cx| {
                this.zoom(1, Box::new(crate::ZoomIn), window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::ZoomFileOut, window, cx| {
                this.zoom(-1, Box::new(crate::ZoomOut), window, cx)
            }))
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .relative()
            .child(inner)
            .on_drag_move::<FileTreeResizeDrag>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<FileTreeResizeDrag>, _, cx| {
                    let tree = f32::from(event.bounds.right() - event.event.position.x) / zoom;
                    this.resize_tree(tree, cx);
                },
            ))
            .on_drop(cx.listener(|this, _: &FileTreeResizeDrag, _, cx| this.finish_tree_resize(cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tree_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tree_resize(cx)),
            )
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

    fn render_root_view(
        &mut self,
        zoom: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tree_width = self
            .widened
            .map_or(crate::app_settings::FILE_TREE_DEFAULT, |w| w.tree_width);
        let diff_column = self.render_diff_column(window, cx);
        let list_column = self.render_list_column(tree_width, window, cx);
        let base_picker = self.render_base_picker(window, cx);
        let checkout_picker = self.render_checkout_picker(window, cx);
        let resize = resize_strip(
            "changes-tree-resize",
            ResizeAxis::Columns,
            self.tree_resizing,
            zoom,
            None,
            false,
        )
        .id("changes-tree-resize")
        .when(cfg!(test), |handle| {
            handle.debug_selector(|| "CHANGES_TREE_GUTTER".into())
        })
        .map(|handle| {
            #[cfg(any(windows, target_os = "linux"))]
            let handle = handle.occlude();
            handle
        })
        .absolute()
        .top_0()
        .right(px(tree_width * zoom - resize_strip_inset(zoom)))
        .on_drag(
            FileTreeResizeDrag,
            |drag: &FileTreeResizeDrag, _, _, cx: &mut gpui::App| cx.new(|_| drag.clone()),
        );

        div()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .flex_row()
            .relative()
            .child(diff_column)
            .child(list_column)
            .child(resize)
            .children(base_picker)
            .children(checkout_picker)
            .into_any_element()
    }

    // ── List column (Commits + Files) ──────────────────────────────────

    fn render_list_column(
        &mut self,
        tree_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list_focused = self.list_focus.is_focused(window);
        let commits_focused = list_focused && self.focus_place == FocusPlace::Commits;
        let files_focused = list_focused && self.focus_place == FocusPlace::Files;
        let tree_view = files::settings(cx).is_some_and(|s| s.changes_tree_view);

        let Body::Root(state) = &self.body else {
            return div().into_any_element();
        };

        let branch_header = self.render_branch_header(state, window, cx);
        let base_row = self.render_base_row(state, window, cx);
        let commits_section = self.render_commits_section(state, commits_focused, cx);
        let summary_row = self.render_files_summary(state, tree_view, cx);
        let row_count = state.list_rows.len();

        let files_list = uniform_list(
            "changes-files-list",
            row_count,
            cx.processor(move |this, range: Range<usize>, window, cx| {
                let list_focused =
                    this.list_focus.is_focused(window) && this.focus_place == FocusPlace::Files;
                let Body::Root(state) = &this.body else {
                    return Vec::new();
                };
                let rows: Vec<ListRow> = state.list_rows[range.clone()].to_vec();
                rows.into_iter()
                    .zip(range)
                    .map(|(row, idx)| this.render_list_row(idx, row, list_focused, cx))
                    .collect()
            }),
        )
        .track_scroll(&self.files_scroll)
        .py(px(4.))
        .size_full();

        let error_banner = self.error.clone().map(|err| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_ERROR".into())
                })
                .px(px(10.))
                .py(px(6.))
                .text_size(theme::text_meta())
                .text_color(theme::danger())
                .child(err)
        });

        div()
            .id("changes-list-column")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_LIST_COLUMN".into())
            })
            .track_focus(&self.list_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(Self::on_list_key))
            .flex_none()
            .w(rems(tree_width / 16.))
            .h_full()
            .min_h(px(0.))
            .bg(theme::panel())
            .border_l_1()
            .border_color(theme::border())
            .rounded_br(rems(11. / 16.))
            .flex()
            .flex_col()
            .child(branch_header)
            .child(base_row)
            .children(error_banner)
            .child(commits_section)
            .child(summary_row)
            .child(
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(move || {
                            if files_focused {
                                "CHANGES_FILES_LIST focused".into()
                            } else {
                                "CHANGES_FILES_LIST".into()
                            }
                        })
                    })
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(files_list)
                    .child(self.files_scrollbar.clone()),
            )
            .into_any_element()
    }

    fn render_branch_header(
        &self,
        state: &RootChanges,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewing_other = is_viewing_other_checkout(&state.root);
        let worktrees_slice = state
            .commits
            .as_ref()
            .map(|c| c.worktrees.as_slice())
            .filter(|w| !w.is_empty())
            .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.as_slice()))
            .unwrap_or(&[]);
        let multiple_checkouts = live_worktrees(worktrees_slice).len() > 1 || viewing_other;

        let (branch_name, commit_count) = match &state.commits {
            Some(commits) => (
                branch_header_label(commits),
                commits.base.is_some().then(|| {
                    if viewing_other {
                        commits.total.to_string()
                    } else {
                        crate::i18n::tn("changes.commits", commits.total)
                    }
                }),
            ),
            None => (
                state
                    .root
                    .git
                    .as_ref()
                    .and_then(|g| g.branch.clone())
                    .unwrap_or_else(|| state.root.name.clone()),
                None,
            ),
        };

        let removed_notice = self.removed_checkout_notice.as_ref().map(|name| {
            let notice_key = match self.context.surface {
                PanelSurface::Chat => "changes.checkout_removed_chat",
                PanelSurface::Terminal => "changes.checkout_removed_terminal",
                PanelSurface::Mission => "changes.checkout_removed_mission",
            };
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_REMOVED_CHECKOUT_NOTICE".into())
                })
                .px(px(10.))
                .pb(px(6.))
                .flex()
                .items_start()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .mt(px(2.))
                        .size(px(12.))
                        .path("triangle-alert.svg")
                        .text_color(theme::warning()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::text_meta())
                        .line_height(relative(1.4))
                        .text_color(theme::warning())
                        .child(crate::i18n::tf(notice_key, &[("name", name.clone())])),
                )
        });

        if !multiple_checkouts {
            let plain_row = div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_BRANCH_HEADER".into())
                })
                .flex_none()
                .h(px(36.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .size(px(13.))
                        .path("git-branch.svg")
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(branch_name),
                )
                .child(div().flex_1())
                .children(commit_count.map(|count| {
                    div()
                        .flex_none()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(count)
                }));
            return div()
                .flex_none()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(theme::border())
                .child(plain_row)
                .children(removed_notice)
                .into_any_element();
        }

        let is_main = state
            .root
            .git
            .as_ref()
            .is_some_and(|g| g.worktree == WorktreeKind::Main);
        let kind_icon = if is_main {
            "house.svg"
        } else {
            "folder-git-2.svg"
        };
        let kind_selector = if is_main {
            "CHANGES_CHECKOUT_KIND_MAIN"
        } else {
            "CHANGES_CHECKOUT_KIND_LINKED"
        };
        let picker_open = self.checkout_picker.is_some();
        let button_focused = self.checkout_button_focus.is_focused(window);
        let anchor = self.checkout_picker_anchor.clone();
        let branch_button = div()
            .id("changes-checkout-button")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_CHECKOUT_BUTTON".into())
            })
            .track_focus(&self.checkout_button_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let m = event.keystroke.modifiers;
                if m.platform || m.control || m.alt {
                    return;
                }
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_checkout_picker(window, cx);
                }
            }))
            .relative()
            .min_w(px(0.))
            .flex_shrink(1.)
            .py(px(2.))
            .pl(px(4.))
            .pr(px(5.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .border_1()
            .border_color(if picker_open || button_focused {
                theme::accent()
            } else {
                gpui::transparent_black()
            })
            .when(picker_open, |el| {
                el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
            })
            .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_checkout_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_ui())
                    .text_color(theme::text())
                    .child(branch_name),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(11.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );

        let return_tip_key = match self.context.surface {
            PanelSurface::Chat => "changes.back_to_chat_checkout",
            PanelSurface::Terminal => "changes.back_to_terminal_checkout",
            PanelSurface::Mission => "changes.back_to_mission_checkout",
        };
        let return_focused = self.checkout_return_focus.is_focused(window);
        let return_button = viewing_other.then(|| {
            div()
                .id("changes-checkout-return")
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_CHECKOUT_RETURN".into())
                })
                .track_focus(&self.checkout_return_focus)
                .tab_index(0)
                .tab_stop(true)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let m = event.keystroke.modifiers;
                    if m.platform || m.control || m.alt {
                        return;
                    }
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        cx.stop_propagation();
                        this.set_root(None, cx);
                        this.checkout_button_focus.focus(window, cx);
                    }
                }))
                .flex_none()
                .size(px(22.))
                .rounded(px(4.))
                .border_1()
                .border_color(if return_focused {
                    theme::accent()
                } else {
                    gpui::transparent_black()
                })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::raised(), 0.7)))
                .tooltip(move |_, cx| tooltip_view(crate::i18n::t(return_tip_key), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| this.set_root(None, cx)))
                .child(
                    svg()
                        .size(px(13.))
                        .path("undo-2.svg")
                        .text_color(theme::muted()),
                )
        });

        let row = div()
            .when(cfg!(test), |el| {
                el.debug_selector(move || {
                    if viewing_other {
                        "CHANGES_BRANCH_HEADER other".into()
                    } else {
                        "CHANGES_BRANCH_HEADER".into()
                    }
                })
            })
            .flex_none()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(5.))
            .when(viewing_other, |el| el.bg(theme::sidebar_selected()))
            .child(
                div()
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || kind_selector.into())
                    })
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .size(px(13.))
                            .path(kind_icon)
                            .text_color(theme::muted()),
                    ),
            )
            .child(branch_button)
            .child(div().flex_1())
            .children(commit_count.map(|count| {
                div()
                    .when(cfg!(test), {
                        let c = count.clone();
                        move |el| el.debug_selector(move || format!("CHANGES_BRANCH_COUNT {c}"))
                    })
                    .flex_none()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(count)
            }))
            .children(return_button);

        div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border())
            .child(row)
            .children(removed_notice)
            .into_any_element()
    }

    fn render_base_row(
        &self,
        state: &RootChanges,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (chip_text, is_custom) =
            base_chip_label(state.chosen_base.as_deref(), state.commits.as_ref());
        let picker_open = self.base_picker.is_some();
        let chip_focused = self.base_chip_focus.is_focused(window);
        let anchor = self.base_picker_anchor.clone();
        let chip = div()
            .id("changes-base-chip")
            .when(cfg!(test), {
                let text = chip_text.clone();
                move |el| el.debug_selector(move || format!("CHANGES_BASE_CHIP {text}"))
            })
            .track_focus(&self.base_chip_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let m = event.keystroke.modifiers;
                if m.platform || m.control || m.alt {
                    return;
                }
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_base_picker(window, cx);
                }
            }))
            .relative()
            .min_w(px(0.))
            .flex_shrink(1.)
            .py(px(2.))
            .pl(px(8.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .border_1()
            .border_color(if picker_open || chip_focused {
                theme::accent()
            } else {
                theme::border()
            })
            .bg(theme::raised())
            .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_base_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(if is_custom {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .child(chip_text),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(11.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );

        let missing_notice = state.missing_base_notice.as_ref().map(|missing| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_MISSING_BASE_NOTICE".into())
                })
                .px(px(10.))
                .pb(px(6.))
                .flex()
                .items_start()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .mt(px(2.))
                        .size(px(12.))
                        .path("triangle-alert.svg")
                        .text_color(theme::warning()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::text_meta())
                        .line_height(relative(1.4))
                        .text_color(theme::warning())
                        .child(crate::i18n::tf(
                            "changes.base_missing_notice",
                            &[("base", display_base_ref(missing).to_owned())],
                        )),
                )
        });

        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_BASE_ROW".into())
            })
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("changes.base")),
                    )
                    .child(div().flex_1())
                    .child(chip),
            )
            .children(missing_notice)
            .into_any_element()
    }

    fn render_base_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.base_picker.as_ref()?;
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        let entity = cx.entity();
        let content = crate::surfaces::worktree_pickers::base_picker_card(
            crate::surfaces::worktree_pickers::PickerListView {
                id_prefix: "changes-base-picker-item",
                selector_prefix: "CHANGES_BASE_PICKER_ITEM",
                title: crate::i18n::text("changes.compare_against").into(),
                field: picker.field.clone(),
                visible: &visible,
                highlighted: picker.highlighted,
                scroll: &picker.scroll,
                reveal: &picker.reveal,
            },
            &picker.items,
            Rc::new(move |base_ref, window, cx| {
                entity.update(cx, |this, cx| this.pick_base(base_ref, window, cx));
            }),
            "CHANGES_BASE_PICKER",
            "changes-base-picker-list",
            "CHANGES_BASE_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.base_picker_anchor.get(),
            window,
            px(BASE_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_base_picker(window, cx));
            }),
        ))
    }

    fn render_checkout_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.checkout_picker.as_ref()?;
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        let entity = cx.entity();
        let content = crate::surfaces::worktree_pickers::checkout_picker_card(
            crate::surfaces::worktree_pickers::PickerListView {
                id_prefix: "changes-checkout-picker-item",
                selector_prefix: "CHANGES_CHECKOUT_PICKER_ITEM",
                title: crate::i18n::text("changes.show_changes_in").into(),
                field: picker.field.clone(),
                visible: &visible,
                highlighted: picker.highlighted,
                scroll: &picker.scroll,
                reveal: &picker.reveal,
            },
            &picker.items,
            false,
            None,
            Rc::new(move |pick_path, window, cx| {
                entity.update(cx, |this, cx| this.pick_checkout(pick_path, window, cx));
            }),
            "CHANGES_CHECKOUT_PICKER",
            "changes-checkout-picker-list",
            "CHANGES_CHECKOUT_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.checkout_picker_anchor.get(),
            window,
            px(CHECKOUT_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_checkout_picker(window, cx));
            }),
        ))
    }

    fn render_commits_section(
        &self,
        state: &RootChanges,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(commits) = &state.commits else {
            return div()
                .flex_none()
                .h(px(34.))
                .border_b_1()
                .border_color(theme::border())
                .into_any_element();
        };
        let rows = build_commit_rows(commits, state.branch_count, state.uncommitted_count);
        let hidden_older = older_commits_hidden(commits);
        let rendered_rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let target = row.target();
                let selected = target == state.selected_target;
                let click_target = target.clone();
                let (selector, leading, title, count, tooltip_text) = match row {
                    CommitRow::AllChanges { count } => (
                        "CHANGES_COMMIT_ALL".to_owned(),
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path("git-pull-request.svg")
                            .text_color(if selected {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .into_any_element(),
                        crate::i18n::text("changes.all_changes").to_owned(),
                        Some(count),
                        None,
                    ),
                    CommitRow::Uncommitted { count } => (
                        "CHANGES_COMMIT_UNCOMMITTED".to_owned(),
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path("circle-dot.svg")
                            .text_color(if count > 0 {
                                theme::warning()
                            } else {
                                theme::faint()
                            })
                            .into_any_element(),
                        crate::i18n::text("changes.uncommitted_changes").to_owned(),
                        Some(count),
                        None,
                    ),
                    CommitRow::Commit(summary) => {
                        let tip = format!(
                            "{} · {}",
                            summary.author,
                            summary.date.format("%Y-%m-%d %H:%M UTC")
                        );
                        let short = summary.short_id.clone();
                        (
                            format!("CHANGES_COMMIT {}", summary.short_id),
                            div()
                                .flex_none()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::faint())
                                .child(short)
                                .into_any_element(),
                            summary.subject,
                            None,
                            Some(tip),
                        )
                    }
                };

                div()
                    .id(("changes-commit-row", index))
                    .when(cfg!(test), move |el| {
                        let s = selector.clone();
                        el.debug_selector(move || s.clone())
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme::sidebar_selected()))
                    .when(selected && focused, |el| el.border_color(theme::faint()))
                    .when(!selected, |el| {
                        el.hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    })
                    .when_some(tooltip_text, |el, tip| {
                        el.tooltip(move |_, cx| tooltip_view(tip.clone(), cx))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_place = FocusPlace::Commits;
                        this.list_focus.focus(window, cx);
                        this.select_target(click_target.clone(), cx);
                    }))
                    .child(leading)
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .truncate()
                            .text_size(theme::text_ui())
                            .when(selected, |t| {
                                t.font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                            })
                            .when(!selected, |t| t.text_color(theme::muted()))
                            .child(title),
                    )
                    .children(count.map(|c| {
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(c.to_string())
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .id("changes-commits-scroll")
            .when(cfg!(test), |el| {
                el.debug_selector(move || {
                    if focused {
                        "CHANGES_COMMITS focused".into()
                    } else {
                        "CHANGES_COMMITS".into()
                    }
                })
            })
            .flex_none()
            .max_h(px(186.))
            .overflow_y_scroll()
            .track_scroll(&self.commits_scroll)
            .py(px(4.))
            .border_b_1()
            .border_color(theme::border())
            .flex()
            .flex_col()
            .children(rendered_rows)
            .when(hidden_older > 0, |el| {
                el.child(
                    div()
                        .when(cfg!(test), |d| {
                            d.debug_selector(|| "CHANGES_OLDER_COMMITS_NOTE".into())
                        })
                        .px(px(10.))
                        .py(px(4.))
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::tn(
                            "changes.older_commits_not_shown",
                            hidden_older,
                        )),
                )
            })
            .into_any_element()
    }

    fn render_files_summary(
        &self,
        state: &RootChanges,
        tree_view: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files_text = crate::i18n::tn("changes.files", state.totals.files);
        let summary_text = if state.totals.viewed > 0 {
            crate::i18n::tf(
                "changes.files_viewed",
                &[
                    ("files", files_text),
                    ("viewed", state.totals.viewed.to_string()),
                ],
            )
        } else {
            files_text
        };
        let show_counts = state.totals.files > 0
            && (state.totals.added > 0
                || state.totals.removed > 0
                || state
                    .files
                    .iter()
                    .any(|f| matches!(f.counts, DiffCounts::Lines { .. })));

        let mode_button = |id: &'static str,
                           selector: &'static str,
                           icon: &'static str,
                           tip_key: &'static str,
                           active: bool,
                           target_tree: bool,
                           cx: &mut Context<Self>| {
            div()
                .id(id)
                .when(cfg!(test), move |el| {
                    el.debug_selector(move || selector.into())
                })
                .flex_none()
                .size(px(22.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(active, |el| el.bg(theme::raised()))
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .tooltip(move |_, cx| tooltip_view(crate::i18n::t(tip_key), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| this.set_tree_view(target_tree, cx)))
                .child(svg().size(px(13.)).path(icon).text_color(if active {
                    theme::text()
                } else {
                    theme::faint()
                }))
        };

        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_FILES_SUMMARY".into())
            })
            .flex_none()
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(summary_text),
            )
            .when(show_counts, |el| {
                el.child(
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::accent())
                        .child(format!("+{}", state.totals.added)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::danger())
                        .child(format!("−{}", state.totals.removed)),
                )
            })
            .child(div().flex_1())
            .child(mode_button(
                "changes-mode-flat",
                "CHANGES_MODE_FLAT",
                "list.svg",
                "changes.flat_list",
                !tree_view,
                false,
                cx,
            ))
            .child(mode_button(
                "changes-mode-tree",
                "CHANGES_MODE_TREE",
                "list-tree.svg",
                "changes.tree_view",
                tree_view,
                true,
                cx,
            ))
            .into_any_element()
    }

    fn render_list_row(
        &self,
        row_index: usize,
        row: ListRow,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Body::Root(state) = &self.body else {
            return div().into_any_element();
        };
        let highlighted = row_index == state.highlighted_list_row;
        match row {
            ListRow::Folder {
                path,
                name,
                depth,
                expanded,
            } => {
                let folder_path = path.clone();
                let icon_set = files::settings(cx).map_or(IconSet::default(), |s| s.file_icons);
                let dark = !theme::active_variant().is_light();
                let folder_icon = match file_icons::folder_icon(icon_set, dark, &name, expanded) {
                    Some(icon) if icon.monochrome => svg()
                        .flex_none()
                        .size(px(14.))
                        .path(icon.path)
                        .text_color(theme::muted())
                        .into_any_element(),
                    Some(icon) => img(ImageSource::Resource(Resource::Embedded(icon.path)))
                        .flex_none()
                        .size(px(14.))
                        .into_any_element(),
                    None => svg()
                        .flex_none()
                        .size(px(14.))
                        .path("folder.svg")
                        .text_color(theme::muted())
                        .into_any_element(),
                };
                div()
                    .id(("changes-folder-row", row_index))
                    .when(cfg!(test), move |el| {
                        let p = path.clone();
                        el.debug_selector(move || format!("CHANGES_FOLDER {p}"))
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .pl(px(10. + depth as f32 * INDENT))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(highlighted && list_focused, |el| {
                        el.border_color(theme::faint())
                    })
                    .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_place = FocusPlace::Files;
                        this.list_focus.focus(window, cx);
                        if let Body::Root(state) = &mut this.body {
                            state.highlighted_list_row = row_index;
                        }
                        this.toggle_folder(&folder_path, cx);
                    }))
                    .child(
                        svg()
                            .flex_none()
                            .size(px(12.))
                            .path(if expanded {
                                "chevron-down.svg"
                            } else {
                                "chevron-right.svg"
                            })
                            .text_color(theme::faint()),
                    )
                    .child(folder_icon)
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .child(name),
                    )
                    .into_any_element()
            }
            ListRow::File {
                file_index,
                path,
                name,
                folder_hint,
                depth,
            } => {
                let Some(file) = state.files.get(file_index) else {
                    return div().into_any_element();
                };
                let selected = state.selected_path.as_deref() == Some(path.as_str());
                let viewed = file.viewed;
                let changed_since = file.changed_since_viewed;
                let (letter, letter_color) = status_letter_and_color(&file.status);
                let counts = file.counts;
                let tooltip_str = match &file.status {
                    FileStatus::Renamed { from } => crate::i18n::tf(
                        "changes.renamed_from",
                        &[("path", path.clone()), ("from", from.clone())],
                    ),
                    _ => path.clone(),
                };
                let hint = if changed_since {
                    Some((
                        crate::i18n::text("changes.changed_since_viewed").to_owned(),
                        theme::warning(),
                    ))
                } else {
                    folder_hint.map(|h| (h, theme::faint()))
                };

                let select_path = path.clone();
                let check_path = path.clone();
                let check_tip = if viewed {
                    "changes.viewed"
                } else {
                    "changes.mark_viewed"
                };

                let counts_el = match counts {
                    DiffCounts::Binary => div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child("bin")
                        .into_any_element(),
                    DiffCounts::Lines { added, removed } => div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .when(viewed, |el| el.opacity(0.55))
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::accent())
                                .child(format!("+{added}")),
                        )
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::danger())
                                .child(format!("−{removed}")),
                        )
                        .into_any_element(),
                };

                div()
                    .id(("changes-file-row", row_index))
                    .when(cfg!(test), {
                        let p = path.clone();
                        move |el| el.debug_selector(move || format!("CHANGES_ROW {p}"))
                    })
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .pl(px(10. + depth as f32 * INDENT))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme::sidebar_selected()))
                    .when(highlighted && list_focused, |el| {
                        el.border_color(theme::faint())
                    })
                    .when(!selected, |el| {
                        el.hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    })
                    .tooltip(move |_, cx| tooltip_view(tooltip_str.clone(), cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_place = FocusPlace::Files;
                        this.list_focus.focus(window, cx);
                        this.select_file_by_path(select_path.clone(), cx);
                    }))
                    .child(
                        div()
                            .id(("changes-row-viewed", row_index))
                            .when(cfg!(test), {
                                let p = path;
                                move |el| {
                                    el.debug_selector(move || format!("CHANGES_VIEWED_BOX {p}"))
                                }
                            })
                            .flex_none()
                            .p(px(2.))
                            .m(px(-2.))
                            .cursor_pointer()
                            .tooltip(move |_, cx| tooltip_view(crate::i18n::t(check_tip), cx))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_viewed_for_path(&check_path, cx);
                            }))
                            .child(render_viewed_checkbox(viewed, changed_since)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(10.))
                            .text_center()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(letter_color)
                            .child(letter),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .text_color(if viewed {
                                theme::faint()
                            } else if selected {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(name),
                    )
                    .children(hint.map(|(text, color)| {
                        div()
                            .min_w(px(0.))
                            .flex_shrink(1.)
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(color)
                            .child(text)
                    }))
                    .child(div().flex_1())
                    .child(counts_el)
                    .into_any_element()
            }
        }
    }

    // ── Diff column ────────────────────────────────────────────────────

    fn render_diff_column(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let effective_mode = self.effective_diff_mode(cx);
        let Body::Root(state) = &self.body else {
            return div().flex_1().into_any_element();
        };

        let header = self.render_diff_header(state, effective_mode, cx);
        let body = match &state.diff_state {
            ActiveDiff::Empty => self.render_empty_target(state),
            ActiveDiff::Loading => self.render_loading(),
            ActiveDiff::Failed(error) => div()
                .flex_1()
                .p_4()
                .text_size(theme::text_ui())
                .text_color(theme::danger())
                .child(error.clone())
                .into_any_element(),
            ActiveDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Unified(entity) => {
                    entity.update(cx, |v, cx| v.set_mode(effective_mode, cx));
                    entity.clone().into_any_element()
                }
                _ => self.render_diff_placeholder(state, loaded, cx),
            },
        };

        div()
            .id("changes-diff-column")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_DIFF_COLUMN".into())
            })
            .track_focus(&self.diff_focus)
            .on_key_down(cx.listener(Self::on_placeholder_diff_key))
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_diff_header(
        &self,
        state: &RootChanges,
        effective_mode: DiffViewMode,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selected_file = state
            .selected_path
            .as_deref()
            .and_then(|p| state.files.iter().find(|f| f.path == p));

        let (path_opt, status_opt, counts_opt, mode_change, conflicted): (
            Option<&str>,
            Option<&FileStatus>,
            Option<DiffCounts>,
            Option<&ModeChange>,
            bool,
        ) = match &state.diff_state {
            ActiveDiff::Loaded(loaded) => (
                Some(loaded.path.as_str()),
                Some(&loaded.status),
                Some(loaded.counts),
                loaded.mode_change.as_ref(),
                loaded.conflicted,
            ),
            _ => (
                selected_file.map(|f| f.path.as_str()),
                selected_file.map(|f| &f.status),
                selected_file.map(|f| f.counts),
                None,
                false,
            ),
        };

        let mode_switch = path_opt.is_some().then(|| {
            let weak = cx.weak_entity();
            let handler: DiffModeCallback = Rc::new(move |mode, window, cx| {
                let _ = weak.update(cx, |this, cx| this.select_diff_mode(mode, window, cx));
            });
            (effective_mode, handler)
        });

        render_shared_diff_header(DiffHeaderProps {
            path: path_opt,
            status: status_opt,
            counts: counts_opt,
            mode_change,
            conflicted,
            compare_label: None,
            mode_switch,
        })
    }

    fn render_empty_target(&self, state: &RootChanges) -> AnyElement {
        let Some(commits) = &state.commits else {
            return self.render_loading();
        };
        if !state.files.is_empty() {
            return self.render_message(
                "CHANGES_SELECT_FILE",
                "git-compare.svg",
                crate::i18n::text("ui.changes"),
                crate::i18n::text("changes.select_file").to_owned(),
                None,
            );
        }
        match &state.selected_target {
            SelectedTarget::Branch => {
                let base = commits.base.clone().unwrap_or_else(|| "main".to_owned());
                let title = crate::i18n::tf("changes.no_changes_since_base", &[("base", base)]);
                self.render_dynamic_message(
                    "CHANGES_EMPTY",
                    "git-compare.svg",
                    title,
                    String::new(),
                    None,
                )
            }
            SelectedTarget::Uncommitted => {
                if let Some(base) = &commits.base
                    && commits.total == 0
                {
                    let title =
                        crate::i18n::tf("changes.no_changes_since_base", &[("base", base.clone())]);
                    self.render_dynamic_message(
                        "CHANGES_EMPTY",
                        "git-compare.svg",
                        title,
                        crate::i18n::text("changes.no_uncommitted_body").to_owned(),
                        None,
                    )
                } else {
                    self.render_message(
                        "CHANGES_EMPTY",
                        "git-compare.svg",
                        crate::i18n::text("changes.no_uncommitted"),
                        crate::i18n::text("changes.no_uncommitted_body").to_owned(),
                        None,
                    )
                }
            }
            SelectedTarget::Commit(id) => {
                let short = commits
                    .commits
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| c.short_id.clone())
                    .unwrap_or_else(|| id[..id.len().min(7)].to_owned());
                let title = crate::i18n::tf("changes.no_changes_in_commit", &[("sha", short)]);
                self.render_dynamic_message(
                    "CHANGES_EMPTY",
                    "git-compare.svg",
                    title,
                    String::new(),
                    None,
                )
            }
        }
    }

    fn render_diff_placeholder(
        &self,
        state: &RootChanges,
        loaded: &LoadedDiff,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_entity = cx.entity();
        let toggle_entity = cx.entity();
        render_shared_diff_placeholder(
            &state.root.path,
            &loaded.path,
            &loaded.status,
            &loaded.kind,
            Rc::new(move |editor, path, cx| {
                open_entity.update(cx, |this, cx| {
                    this.launch_external(editor, path, cx);
                });
            }),
            Some(PlaceholderViewedFooter {
                viewed: loaded.viewed,
                changed_since_viewed: loaded.changed_since_viewed,
                on_toggle: Rc::new(move |_, cx| {
                    toggle_entity.update(cx, |this, cx| {
                        this.toggle_viewed_for_loaded_diff(cx);
                    });
                }),
            }),
            cx,
        )
    }

    // ── Shared edge-state screens (Choose, Missing, Message) ───────────

    fn render_message(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: &'static str,
        body: String,
        actions: Option<AnyElement>,
    ) -> AnyElement {
        render_centered_message(selector, icon, title.to_owned(), body, actions)
    }

    fn render_dynamic_message(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: String,
        body: String,
        actions: Option<AnyElement>,
    ) -> AnyElement {
        render_centered_message(selector, icon, title, body, actions)
    }

    fn render_choose(&self, cx: &mut Context<Self>) -> AnyElement {
        let home = Self::home(cx);
        let entry = |id: (&'static str, usize),
                     name: String,
                     path: PathBuf,
                     cx: &mut Context<Self>| {
            let subtitle = abbreviate_home(HomePath::Native(&path, home.as_deref()));
            div()
                .id(id)
                .when(cfg!(test), |el| {
                    el.debug_selector(move || format!("CHANGES_CHOOSE {} {}", id.0, id.1))
                })
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_root(Some(path.clone()), cx)))
                .child(
                    svg()
                        .flex_none()
                        .size(px(16.))
                        .path("folder.svg")
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_ui())
                                .text_color(theme::text())
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_meta())
                                .text_color(theme::faint())
                                .child(subtitle),
                        ),
                )
                .into_any_element()
        };
        let section = |label: &'static str| {
            div()
                .px(px(12.))
                .pt(px(14.))
                .pb(px(4.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(label)
        };
        let projects = self
            .projects(cx)
            .into_iter()
            .enumerate()
            .map(|(index, (name, path))| entry(("changes-choose-project", index), name, path, cx))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                entry(
                    ("changes-choose-recent", index),
                    picker::folder_name(&path),
                    path,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let browse = cx.entity();
        div()
            .id("changes-choose")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_CHOOSE".into())
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(px(16.))
            .pt(px(28.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        svg()
                            .size(px(22.))
                            .path("folder-open.svg")
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .pt(px(4.))
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(crate::i18n::text("ui.choose_a_folder_to_show")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "changes.choose_folder_body",
                                &[("subject", self.noun())],
                            )),
                    ),
            )
            .child(
                div()
                    .pt(px(16.))
                    .relative()
                    .child(self.choose_field.clone())
                    .child(
                        div()
                            .absolute()
                            .top(px(16.))
                            .right(px(4.))
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(
                                IconButton::new("changes-browse", "folder-search.svg")
                                    .tooltip(crate::i18n::text("ui.browse"))
                                    .on_press(move |_, cx| {
                                        browse.update(cx, |this, cx| this.browse(cx))
                                    }),
                            ),
                    ),
            )
            .when(!projects.is_empty(), |el| {
                el.child(section(crate::i18n::text("ui.projects")))
                    .children(projects)
            })
            .when(!recent.is_empty(), |el| {
                el.child(section(crate::i18n::text("ui.recent")))
                    .children(recent)
            })
            .into_any_element()
    }

    fn render_missing(&self, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let project = self.own_project(cx);
        let show_root = cx.entity();
        let choose_root = cx.entity();
        let actions = div()
            .flex()
            .gap_2()
            .justify_center()
            .children(project.map(|(name, path)| {
                Button::new(
                    "changes-show-main-checkout",
                    crate::i18n::tf("format.show_folder", &[("name", name)]),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("CHANGES_SHOW_MAIN")
                .on_press(move |_, cx| {
                    let path = path.clone();
                    show_root.update(cx, |this, cx| this.set_root(Some(path), cx));
                })
            }))
            .child(
                Button::new(
                    "changes-choose-root",
                    crate::i18n::text("ui.choose_ellipsis"),
                )
                .variant(ButtonVariant::Outline)
                .on_press(move |_, cx| {
                    choose_root.update(cx, |this, cx| this.show_choose_screen(cx));
                }),
            )
            .into_any_element();
        let shown = compact_missing_path(path);
        self.render_message(
            "CHANGES_MISSING",
            "folder-x.svg",
            crate::i18n::text("ui.this_folder_no_longer_exists"),
            crate::i18n::tf("format.folder_was_moved_or_deleted", &[("path", shown)]),
            Some(actions),
        )
    }
}

pub(crate) type DiffModeCallback = Rc<dyn Fn(DiffViewMode, &mut Window, &mut App)>;
pub(crate) type OpenExternalCallback = Rc<dyn Fn(Option<FileLinkEditor>, PathBuf, &mut App)>;

pub(crate) struct DiffHeaderProps<'a> {
    pub(crate) path: Option<&'a str>,
    pub(crate) status: Option<&'a FileStatus>,
    pub(crate) counts: Option<DiffCounts>,
    pub(crate) mode_change: Option<&'a ModeChange>,
    pub(crate) conflicted: bool,
    pub(crate) compare_label: Option<String>,
    pub(crate) mode_switch: Option<(DiffViewMode, DiffModeCallback)>,
}

pub(crate) struct PlaceholderViewedFooter {
    pub(crate) viewed: bool,
    pub(crate) changed_since_viewed: bool,
    pub(crate) on_toggle: super::diff::DiffCallback,
}

pub(crate) fn render_shared_diff_header(props: DiffHeaderProps<'_>) -> AnyElement {
    let renamed_from = match props.status {
        Some(FileStatus::Renamed { from }) => Some(format!("{from} →")),
        _ => None,
    };
    let name = props.path.map(|p| file_name(p).to_owned());
    let folder = props.path.and_then(abbreviate_folder);
    let mode_badge = props.mode_change.map(|m| {
        crate::i18n::tf(
            "changes.mode_badge",
            &[("old", m.old.clone()), ("new", m.new.clone())],
        )
    });

    let mode_switch = props.mode_switch.map(|(active_mode, on_select)| {
        let pill = |id: &'static str,
                    selector: &'static str,
                    label_key: &'static str,
                    target: DiffViewMode,
                    on_select: DiffModeCallback| {
            let active = active_mode == target;
            div()
                .id(id)
                .when(cfg!(test), move |el| {
                    el.debug_selector(move || {
                        if active {
                            format!("{selector} active")
                        } else {
                            selector.into()
                        }
                    })
                })
                .h(px(22.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(theme::text_meta())
                .when(active, |el| {
                    el.bg(theme::raised())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                })
                .when(!active, |el| {
                    el.text_color(theme::muted())
                        .hover(|h| h.text_color(theme::text()))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| on_select(target, window, cx))
                .child(crate::i18n::text(label_key))
        };

        div()
            .when(cfg!(test), |d| {
                d.debug_selector(|| "CHANGES_DIFF_MODE".into())
            })
            .flex_none()
            .p(px(2.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .flex()
            .items_center()
            .gap(px(2.))
            .child(pill(
                "changes-diff-mode-unified",
                "CHANGES_DIFF_MODE_UNIFIED",
                "changes.unified",
                DiffViewMode::Unified,
                on_select.clone(),
            ))
            .child(pill(
                "changes-diff-mode-split",
                "CHANGES_DIFF_MODE_SPLIT",
                "changes.split",
                DiffViewMode::Split,
                on_select,
            ))
    });

    let left_group = div()
        .min_w(px(36.))
        .flex_grow(1.)
        .flex_shrink(1.)
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(6.))
        .children(renamed_from.map(|old| {
            div()
                .min_w(px(0.))
                .flex_shrink(3.)
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(old)
        }))
        .children(name.map(|n| {
            div()
                .min_w(px(20.))
                .flex_shrink(1.)
                .truncate()
                .text_size(theme::text_ui())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(n)
        }))
        .children(folder.map(|f| {
            div()
                .min_w(px(0.))
                .flex_shrink(3.)
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(f)
        }))
        .children(props.counts.map(|counts| {
            match counts {
                DiffCounts::Binary => div()
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_caption())
                    .text_color(theme::faint())
                    .child("bin")
                    .into_any_element(),
                DiffCounts::Lines { added, removed } => div()
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::accent())
                            .child(format!("+{added}")),
                    )
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::danger())
                            .child(format!("−{removed}")),
                    )
                    .into_any_element(),
            }
        }))
        .children(mode_badge.map(|badge| {
            div()
                .min_w(px(0.))
                .flex_shrink(2.)
                .truncate()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(theme::raised())
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::muted())
                .child(badge)
        }))
        .when(props.conflicted, |el| {
            el.child(
                div()
                    .when(cfg!(test), |d| {
                        d.debug_selector(|| "CHANGES_CONFLICTED_BADGE".into())
                    })
                    .min_w(px(0.))
                    .flex_shrink(2.)
                    .truncate()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(theme::with_alpha(theme::danger(), 0.14))
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::danger())
                    .child(crate::i18n::text("changes.conflicted")),
            )
        });

    div()
        .when(cfg!(test), |el| {
            el.debug_selector(|| "CHANGES_DIFF_HEADER".into())
        })
        .flex_none()
        .h(px(36.))
        .pl(px(10.))
        .pr(px(8.))
        .overflow_x_hidden()
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(theme::border())
        .child(left_group)
        .children(mode_switch)
        .children(props.compare_label.map(|label| {
            div()
                .when(cfg!(test), |d| {
                    d.debug_selector(|| "CHANGES_COMPARE_LABEL".into())
                })
                .min_w(px(0.))
                .max_w(px(240.))
                .flex_shrink(2.)
                .truncate()
                .px(px(7.))
                .py(px(3.))
                .rounded(px(4.))
                .bg(theme::raised())
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(label)
        }))
        .into_any_element()
}

pub(crate) fn render_shared_diff_placeholder(
    root_path: &Path,
    rel_path: &str,
    status: &FileStatus,
    kind: &LoadedDiffKind,
    on_open_external: OpenExternalCallback,
    viewed_footer: Option<PlaceholderViewedFooter>,
    cx: &App,
) -> AnyElement {
    let full_path = absolute(root_path, rel_path);
    let exists_on_disk = *status != FileStatus::Deleted && full_path.exists();

    let size_line = |old_size: Option<u64>, new_size: Option<u64>| -> String {
        match (old_size, new_size) {
            (Some(old), Some(new)) => crate::i18n::tf(
                "changes.binary_sizes",
                &[("old", format_size(old)), ("new", format_size(new))],
            ),
            (None, Some(new)) => {
                crate::i18n::tf("changes.binary_added_size", &[("size", format_size(new))])
            }
            (Some(old), None) => {
                crate::i18n::tf("changes.binary_deleted_size", &[("size", format_size(old))])
            }
            (None, None) => String::new(),
        }
    };

    let center = match kind {
        LoadedDiffKind::Unified(_) => div().into_any_element(),
        LoadedDiffKind::CrlfOnly => render_centered_message(
            "CHANGES_PLACEHOLDER_CRLF",
            "file-text.svg",
            crate::i18n::text("changes.only_line_endings").to_owned(),
            String::new(),
            None,
        ),
        LoadedDiffKind::ModeOnly { old_mode, new_mode } => render_centered_message(
            "CHANGES_PLACEHOLDER_MODE",
            "file-text.svg",
            crate::i18n::tf(
                "changes.mode_changed",
                &[("old", old_mode.clone()), ("new", new_mode.clone())],
            ),
            String::new(),
            None,
        ),
        LoadedDiffKind::Submodule {
            old_commit,
            new_commit,
        } => {
            let short = |s: &str| s[..s.len().min(7)].to_owned();
            let msg = match (old_commit.as_deref(), new_commit.as_deref()) {
                (Some(old), Some(new)) => crate::i18n::tf(
                    "changes.submodule_moved",
                    &[("old", short(old)), ("new", short(new))],
                ),
                (None, Some(new)) => {
                    crate::i18n::tf("changes.submodule_added", &[("new", short(new))])
                }
                (Some(old), None) => {
                    crate::i18n::tf("changes.submodule_removed", &[("old", short(old))])
                }
                (None, None) => String::new(),
            };
            render_centered_message(
                "CHANGES_PLACEHOLDER_SUBMODULE",
                "folder-git-2.svg",
                msg,
                String::new(),
                None,
            )
        }
        LoadedDiffKind::Binary { old_size, new_size } => {
            let target_path = full_path.clone();
            let open = on_open_external;
            let action = exists_on_disk.then(|| {
                Button::new(
                    "changes-open-default-app",
                    crate::i18n::text("changes.open_in_default_app"),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("CHANGES_OPEN_DEFAULT_APP")
                .on_press(move |_, cx| {
                    open(Some(FileLinkEditor::DefaultApp), target_path.clone(), cx);
                })
                .into_any_element()
            });
            render_centered_message(
                "CHANGES_PLACEHOLDER_BINARY",
                "file-text.svg",
                crate::i18n::text("changes.binary_file").to_owned(),
                size_line(*old_size, *new_size),
                action,
            )
        }
        LoadedDiffKind::TooLarge { old_size, new_size } => {
            let (_, effective_editor) = found_editors(cx);
            let target_path = full_path;
            let open = on_open_external;
            let action = exists_on_disk.then(|| {
                let label = crate::i18n::tf(
                    "format.open_in_editor",
                    &[("editor", effective_editor.label().to_owned())],
                );
                div()
                    .id("changes-open-large-in-editor")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "CHANGES_OPEN_IN_EDITOR".into())
                    })
                    .px(px(12.))
                    .h(px(28.))
                    .rounded_md()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::raised())
                    .hover(|el| el.bg(theme::sidebar_selected()))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .on_click(move |_, _, cx| {
                        open(Some(effective_editor), target_path.clone(), cx);
                    })
                    .child(editor_icon(effective_editor, 14.))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::text())
                            .child(label),
                    )
                    .into_any_element()
            });
            let sizes = size_line(*old_size, *new_size);
            let limit_note = crate::i18n::text("changes.diffs_up_to_limit");
            let body_str = if sizes.is_empty() {
                limit_note.to_owned()
            } else {
                format!("{sizes}\n{limit_note}")
            };
            render_centered_message(
                "CHANGES_PLACEHOLDER_TOO_LARGE",
                "file-text.svg",
                crate::i18n::text("changes.file_too_large").to_owned(),
                body_str,
                action,
            )
        }
    };

    let footer = viewed_footer.map(|footer| {
        let mark_label = crate::i18n::tf(
            "changes.mark_file_viewed",
            &[("name", file_name(rel_path).to_owned())],
        );
        let on_toggle = footer.on_toggle;
        div()
            .id("changes-placeholder-mark-viewed")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_DIFF_VIEWED".into())
            })
            .flex_none()
            .h(px(40.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_t_1()
            .border_color(theme::border())
            .cursor_pointer()
            .hover(|row| row.bg(theme::with_alpha(theme::sidebar_selected(), 0.35)))
            .on_click(move |_, window, cx| on_toggle(window, cx))
            .child(render_viewed_checkbox(
                footer.viewed,
                footer.changed_since_viewed,
            ))
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(mark_label),
            )
    });

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(center)
        .children(footer)
        .into_any_element()
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

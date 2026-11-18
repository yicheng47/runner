use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    canvas, div, px, rems, svg, AnyElement, Bounds, KeyDownEvent, MouseButton, Pixels, ScrollHandle,
};
use runner_app::theme;
use runner_app::ui::menu::{popup_layer, popup_layer_right_aligned};
use runner_app::ui::tooltip::tooltip_view;
use runner_core::app_paths::{normalize_checkout_path, same_normalized_checkout_path};
use runner_core::protocol::changes::ChangesBranches;
use runner_core::protocol::files::{WorktreeEntry, WorktreeKind};
use runner_core::protocol::worktree::{WorktreeCheckout, WorktreeNameCheck, WorktreeSpec};

use super::*;
use crate::surfaces::changes::model::{
    build_base_picker_items, build_checkout_picker_items, default_base_highlight,
    default_checkout_highlight, display_base_ref, filter_base_picker_items,
    filter_checkout_picker_items, BasePickerItem, CheckoutPickerItem,
};
use crate::surfaces::files::picker;
use crate::surfaces::worktree_pickers::{
    base_picker_card, checkout_picker_card, PickerListView, BASE_PICKER_WIDTH,
    CHECKOUT_PICKER_WIDTH,
};

#[cfg(test)]
#[derive(Default)]
pub(super) struct WorktreeTestGates {
    pub(super) worktree_check: std::collections::VecDeque<crate::test_support::StagedReplyGate>,
    pub(super) default_base: std::collections::VecDeque<crate::test_support::StagedReplyGate>,
    pub(super) checkout_picker: std::collections::VecDeque<crate::test_support::StagedReplyGate>,
    pub(super) base_picker: std::collections::VecDeque<crate::test_support::StagedReplyGate>,
}

pub(super) struct LauncherCheckoutPicker {
    pub(super) id: u64,
    pub(super) repo_top: PathBuf,
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

pub(super) struct LauncherBasePicker {
    pub(super) id: u64,
    pub(super) repo_dir: PathBuf,
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

pub(super) struct NewWorktreeState {
    pub(super) form_gen: u64,
    pub(super) repo_dir: PathBuf,
    pub(super) main_checkout: PathBuf,
    pub(super) branch_field: Entity<TextField>,
    pub(super) branch_query: String,
    pub(super) base: Option<String>,
    pub(super) default_base: Option<String>,
    pub(super) check: Option<WorktreeNameCheck>,
    pub(super) check_gen: u64,
    pub(super) check_pending: bool,
    pub(super) base_picker: Option<LauncherBasePicker>,
    pub(super) base_anchor: Rc<Cell<Bounds<Pixels>>>,
    _observe_branch: Subscription,
}

impl NewWorktreeState {
    pub(super) fn can_start(&self) -> bool {
        !self.branch_query.trim().is_empty()
            && !self.check_pending
            && matches!(
                self.check,
                Some(
                    WorktreeNameCheck::NewBranch { .. }
                        | WorktreeNameCheck::ExistingBranch { .. }
                        | WorktreeNameCheck::AlreadyCheckedOut { .. }
                )
            )
    }

    pub(super) fn effective_base(&self) -> Option<String> {
        self.base.clone().or_else(|| self.default_base.clone())
    }

    pub(super) fn base_dimmed(&self) -> bool {
        matches!(
            self.check,
            Some(
                WorktreeNameCheck::ExistingBranch { .. }
                    | WorktreeNameCheck::AlreadyCheckedOut { .. }
            )
        )
    }

    pub(super) fn is_invalid(&self) -> bool {
        matches!(self.check, Some(WorktreeNameCheck::Invalid { .. }))
    }

    fn base_label(&self) -> String {
        let raw = self.effective_base();
        display_base_ref(raw.as_deref().unwrap_or("HEAD")).to_owned()
    }
}

pub(super) fn rewrite_cwd_for_checkout(
    current_cwd: &str,
    current_top: Option<&Path>,
    target_top: &Path,
) -> PathBuf {
    if let Some(current_top) = current_top {
        let norm_cwd = normalize_checkout_path(Path::new(current_cwd));
        let norm_top = normalize_checkout_path(current_top);
        if let Ok(rel) = norm_cwd.strip_prefix(&norm_top) {
            if !rel.as_os_str().is_empty() {
                let candidate = target_top.join(rel);
                if candidate.is_dir() {
                    return candidate;
                }
            }
        }
    }
    target_top.to_path_buf()
}

fn strip_windows_path_prefix(main: &str, path: &str) -> Option<String> {
    let main_norm = crate::surfaces::remote_client::model::normalize_remote_host_path(main);
    let prefix = main_norm.trim_end_matches('/');
    let path_norm = crate::surfaces::remote_client::model::normalize_remote_host_path(path);
    let remainder = path_norm.strip_prefix(prefix)?;
    if remainder.is_empty() || !remainder.starts_with('/') {
        return None;
    }
    let rel_count = remainder.trim_start_matches('/').split('/').count();
    let original_parts: Vec<&str> = path
        .trim_end_matches(['/', '\\'])
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    if original_parts.len() < rel_count {
        return None;
    }
    Some(original_parts[original_parts.len() - rel_count..].join("\\"))
}

fn format_worktree_target_path(main_checkout: &Path, path: &Path, home: Option<&Path>) -> String {
    let main_raw = main_checkout.to_string_lossy();
    let path_raw = path.to_string_lossy();
    if cfg!(not(windows))
        && crate::surfaces::remote_client::model::looks_like_windows_path(&main_raw)
        && crate::surfaces::remote_client::model::looks_like_windows_path(&path_raw)
    {
        if let Some(rel) = strip_windows_path_prefix(&main_raw, &path_raw)
            && !rel.is_empty()
        {
            return rel;
        }
    } else {
        let norm_main = normalize_checkout_path(main_checkout);
        let norm_path = normalize_checkout_path(path);
        if let Ok(rel) = norm_path.strip_prefix(&norm_main) {
            if !rel.as_os_str().is_empty() {
                return rel.to_string_lossy().into_owned();
            }
        }
    }
    picker::shorten_checkout_path(path, home)
}

pub(super) fn format_new_worktree_hint(
    branch_query: &str,
    check: Option<&WorktreeNameCheck>,
    main_checkout: &Path,
    home: Option<&Path>,
) -> (&'static str, String, gpui::Hsla) {
    match check {
        _ if branch_query.trim().is_empty() => (
            "git-branch-plus.svg",
            crate::i18n::text("launcher.worktree_hint_empty").to_owned(),
            theme::muted(),
        ),
        Some(WorktreeNameCheck::NewBranch { base, path }) => (
            "git-branch-plus.svg",
            crate::i18n::tf(
                "launcher.worktree_hint_new",
                &[
                    ("base", base.clone()),
                    (
                        "path",
                        format_worktree_target_path(main_checkout, path, home),
                    ),
                ],
            ),
            theme::muted(),
        ),
        Some(WorktreeNameCheck::ExistingBranch { commit, path }) => (
            "git-branch.svg",
            crate::i18n::tf(
                "launcher.worktree_hint_existing",
                &[
                    ("commit", commit.clone()),
                    (
                        "path",
                        format_worktree_target_path(main_checkout, path, home),
                    ),
                ],
            ),
            theme::muted(),
        ),
        Some(WorktreeNameCheck::AlreadyCheckedOut { path }) => (
            "corner-down-right.svg",
            crate::i18n::tf(
                "launcher.worktree_hint_checked_out",
                &[
                    ("branch", branch_query.trim().to_owned()),
                    ("path", picker::shorten_checkout_path(path, home)),
                ],
            ),
            theme::muted(),
        ),
        Some(WorktreeNameCheck::Invalid { reason }) => {
            ("circle-alert.svg", reason.clone(), theme::danger())
        }
        None => (
            "git-branch-plus.svg",
            crate::i18n::text("launcher.worktree_hint_empty").to_owned(),
            theme::muted(),
        ),
    }
}

impl ChatLauncher {
    pub(super) fn new_worktree_allows_start(&self) -> bool {
        self.new_worktree
            .as_ref()
            .is_none_or(NewWorktreeState::can_start)
    }

    pub(super) fn worktree_spec(&self) -> Option<WorktreeSpec> {
        let wt = self.new_worktree.as_ref()?;
        let branch = wt.branch_query.trim();
        if branch.is_empty() || !wt.can_start() {
            return None;
        }
        let base = match &wt.check {
            Some(WorktreeNameCheck::NewBranch { base, .. }) => {
                wt.base.clone().or_else(|| Some(base.clone()))
            }
            _ => None,
        };
        Some(WorktreeSpec {
            branch: branch.to_owned(),
            base,
        })
    }

    pub(super) fn worktree_repo_cwd(&self) -> Option<String> {
        self.new_worktree
            .as_ref()
            .map(|wt| wt.repo_dir.to_string_lossy().into_owned())
    }

    pub(super) fn refresh_checkout_state(&mut self, cx: &mut Context<Self>) {
        if !self.default_page || self.client.is_remote() {
            self.checkout = None;
            self.checkout_picker = None;
            self.new_worktree = None;
            return;
        }
        let cwd_text = self.cwd.read(cx).text().trim().to_owned();
        if cwd_text.is_empty() {
            self.checkout = None;
            return;
        }
        self.checkout_gen = self.checkout_gen.wrapping_add(1);
        let generation = self.checkout_gen;
        let daemon = self.client.local_daemon().clone();
        let project_id = self.project.scope().project_id().map(str::to_owned);
        let query_cwd = PathBuf::from(&cwd_text);
        let task = cx.background_spawn(async move {
            daemon.worktree_checkout(&query_cwd, project_id.as_deref())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.checkout_gen != generation {
                    return;
                }
                match result {
                    Ok(checkout) => {
                        if let WorktreeCheckout::Git { main_checkout, .. } = &checkout {
                            let main_str = main_checkout.to_string_lossy().into_owned();
                            this.project.follow_external_repo(&main_str, &cwd_text, cx);
                        }
                        this.checkout = Some(checkout);
                    }
                    Err(_) => {
                        this.checkout = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn location_display_path(&self, cx: &App) -> (String, bool) {
        if let Some(wt) = &self.new_worktree {
            let home = runner_core::app_paths::home_dir();
            match &wt.check {
                Some(WorktreeNameCheck::NewBranch { path, .. })
                | Some(WorktreeNameCheck::ExistingBranch { path, .. }) => {
                    return (
                        format_worktree_target_path(&wt.main_checkout, path, home.as_deref()),
                        true,
                    );
                }
                Some(WorktreeNameCheck::AlreadyCheckedOut { path }) => {
                    return (picker::shorten_checkout_path(path, home.as_deref()), true);
                }
                _ => {}
            }
        }
        (self.cwd.read(cx).text().to_owned(), false)
    }

    pub(super) fn render_checkout_control(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.client.is_remote() || !self.default_page {
            return None;
        }
        let Some(WorktreeCheckout::Git {
            kind,
            branch,
            commit,
            ..
        }) = &self.checkout
        else {
            return None;
        };

        let in_new_worktree = self.new_worktree.is_some();
        let active = in_new_worktree || self.checkout_picker.is_some();
        let (icon_path, label, icon_color, text_color) = if in_new_worktree {
            (
                "folder-plus.svg",
                crate::i18n::text("launcher.new_worktree").to_owned(),
                theme::accent(),
                theme::accent(),
            )
        } else {
            let icon = match kind {
                WorktreeKind::Main => "house.svg",
                WorktreeKind::Linked => "folder-git-2.svg",
            };
            let label = match (branch, commit) {
                (Some(b), _) => b.clone(),
                (None, Some(sha)) => {
                    crate::i18n::tf("changes.detached_at", &[("sha", sha.clone())])
                }
                (None, None) => crate::i18n::text("ui.detached").to_owned(),
            };
            (icon, label, theme::muted(), theme::text())
        };
        let anchor = self.checkout_anchor.clone();

        Some(
            div()
                .id("launcher-checkout")
                .debug_selector(|| "LAUNCHER_CHECKOUT".into())
                .relative()
                .h(rems(22. / 16.))
                .max_w(rems(160. / 16.))
                .px(rems(8. / 16.))
                .flex_none()
                .flex()
                .items_center()
                .gap(rems(5. / 16.))
                .rounded(rems(6. / 16.))
                .border_1()
                .border_color(if active {
                    theme::accent()
                } else {
                    theme::border()
                })
                .bg(if in_new_worktree {
                    theme::with_alpha(theme::accent(), 0.14)
                } else {
                    theme::panel()
                })
                .cursor_pointer()
                .hover(|el| {
                    el.bg(if in_new_worktree {
                        theme::with_alpha(theme::accent(), 0.2)
                    } else {
                        theme::raised()
                    })
                })
                .tooltip(move |_, cx| {
                    tooltip_view(crate::i18n::text("launcher.checkout_tooltip"), cx)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_checkout_picker(window, cx);
                }))
                .child(
                    svg()
                        .flex_none()
                        .size(rems(12. / 16.))
                        .path(icon_path)
                        .text_color(icon_color),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(rems(11. / 16.))
                        .font_weight(if in_new_worktree {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(text_color)
                        .child(label),
                )
                .child(
                    svg()
                        .flex_none()
                        .size(rems(10. / 16.))
                        .path("chevron-down.svg")
                        .text_color(if in_new_worktree {
                            theme::accent()
                        } else {
                            theme::muted()
                        }),
                )
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, _| {
                            anchor.set(bounds);
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_new_worktree_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.client.is_remote() || !self.default_page {
            return None;
        }
        let wt = self.new_worktree.as_ref()?;
        let invalid = wt.is_invalid();
        let base_dimmed = wt.base_dimmed();
        let home = runner_core::app_paths::home_dir();

        let (hint_icon, hint_text, hint_color) = format_new_worktree_hint(
            &wt.branch_query,
            wt.check.as_ref(),
            &wt.main_checkout,
            home.as_deref(),
        );

        let base_anchor = wt.base_anchor.clone();
        let base_picker_open = wt.base_picker.is_some();
        let base_chip = div()
            .id("launcher-worktree-base")
            .debug_selector(|| "LAUNCHER_WORKTREE_BASE".into())
            .relative()
            .h(rems(26. / 16.))
            .max_w(rems(140. / 16.))
            .px(rems(8. / 16.))
            .flex_none()
            .flex()
            .items_center()
            .gap(rems(5. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(if base_picker_open && !base_dimmed {
                theme::accent()
            } else {
                theme::border()
            })
            .bg(theme::raised())
            .when(base_dimmed, |el| el.opacity(0.45))
            .when(!base_dimmed, |el| {
                el.cursor_pointer()
                    .hover(|el| el.bg(theme::sidebar_selected()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_base_picker(window, cx);
                    }))
            })
            .child(
                svg()
                    .flex_none()
                    .size(rems(11. / 16.))
                    .path("git-branch.svg")
                    .text_color(theme::muted()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(rems(11. / 16.))
                    .text_color(theme::text())
                    .child(wt.base_label()),
            )
            .child(
                svg()
                    .flex_none()
                    .size(rems(10. / 16.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, _| {
                        base_anchor.set(bounds);
                    },
                )
                .absolute()
                .inset_0(),
            );

        Some(
            div()
                .debug_selector(|| "LAUNCHER_NEW_WORKTREE".into())
                .w_full()
                .px(rems(10. / 16.))
                .py(rems(8. / 16.))
                .rounded(rems(8. / 16.))
                .border_1()
                .border_color(if invalid {
                    theme::danger()
                } else {
                    theme::border()
                })
                .bg(theme::panel())
                .flex()
                .flex_col()
                .gap(rems(6. / 16.))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .h(rems(28. / 16.))
                                .px(rems(8. / 16.))
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .rounded(rems(6. / 16.))
                                .border_1()
                                .border_color(if invalid {
                                    theme::danger()
                                } else {
                                    theme::border()
                                })
                                .bg(theme::bg())
                                .child(
                                    svg()
                                        .flex_none()
                                        .size(rems(12. / 16.))
                                        .path("git-branch.svg")
                                        .text_color(if invalid {
                                            theme::danger()
                                        } else {
                                            theme::muted()
                                        }),
                                )
                                .child(div().flex_1().min_w(px(0.)).child(wt.branch_field.clone())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::faint())
                                .child(crate::i18n::text("launcher.worktree_from")),
                        )
                        .child(base_chip),
                )
                .child(
                    div()
                        .debug_selector(|| "LAUNCHER_NEW_WORKTREE_HINT".into())
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .child(
                            svg()
                                .flex_none()
                                .size(rems(12. / 16.))
                                .path(hint_icon)
                                .text_color(hint_color),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_normal()
                                .text_size(rems(11. / 16.))
                                .text_color(hint_color)
                                .child(hint_text),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn toggle_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        let Some(WorktreeCheckout::Git { top, .. }) = &self.checkout else {
            return;
        };
        if let Some(wt) = &mut self.new_worktree {
            wt.base_picker = None;
        }
        let repo_top = top.clone();
        let return_focus = self
            .new_worktree
            .as_ref()
            .filter(|wt| wt.branch_field.read(cx).focus_handle().is_focused(window))
            .map(|wt| wt.branch_field.read(cx).focus_handle().clone())
            .unwrap_or_else(|| self.field().read(cx).focus_handle().clone());
        let entity = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_checkouts"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                entity
                    .update(cx, |this, cx| {
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
                    if this.new_worktree.is_some() && picker.query.trim().is_empty() {
                        visible.len()
                    } else {
                        default_checkout_highlight(&picker.items, &visible, &picker.query)
                    };
                picker.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.checkout_picker_gen = self.checkout_picker_gen.wrapping_add(1);
        let picker_id = self.checkout_picker_gen;
        self.checkout_picker = Some(LauncherCheckoutPicker {
            id: picker_id,
            repo_top: repo_top.clone(),
            field,
            query: String::new(),
            worktrees: Vec::new(),
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        let daemon = self.client.local_daemon().clone();
        let query_top = repo_top.clone();
        let task = cx.background_spawn(async move { daemon.files_worktrees(&query_top) });
        #[cfg(test)]
        let test_gate = self.worktree_test_gates.checkout_picker.pop_front();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update(cx, |this, cx| {
                #[cfg(test)]
                let _applied = applied;
                let checkout_matches = matches!(
                    &this.checkout,
                    Some(WorktreeCheckout::Git { top, .. })
                        if same_normalized_checkout_path(top, &repo_top)
                );
                if !checkout_matches {
                    return;
                }
                if let (Some(picker), Ok(worktrees)) = (&mut this.checkout_picker, result)
                    && picker.id == picker_id
                    && same_normalized_checkout_path(&picker.repo_top, &repo_top)
                {
                    picker.worktrees = worktrees;
                    this.rebuild_checkout_picker(cx);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn close_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn rebuild_checkout_picker(&mut self, cx: &App) {
        let Some(WorktreeCheckout::Git { top, .. }) = &self.checkout else {
            return;
        };
        let current_top = top.clone();
        let home = runner_core::app_paths::home_dir();
        let in_new_worktree = self.new_worktree.is_some();
        let Some(picker) = &mut self.checkout_picker else {
            return;
        };
        let session_labels =
            picker::collect_launcher_checkout_session_labels(&picker.worktrees, cx);
        let mut items = build_checkout_picker_items(
            &current_top,
            &picker.worktrees,
            &session_labels,
            home.as_deref(),
            crate::i18n::text("ui.detached"),
        );
        if in_new_worktree {
            for item in &mut items {
                item.current = false;
            }
        }
        picker.items = items;
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.len())
        } else if in_new_worktree && picker.query.trim().is_empty() {
            visible.len()
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
        let max_index = visible.len(); // `visible.len()` is the `New worktree…` footer row.
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(max_index);
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
                if picker.highlighted == max_index {
                    self.enter_new_worktree(window, cx);
                } else if let Some(&item_index) = visible.get(picker.highlighted) {
                    let path = picker.items[item_index].path.clone();
                    self.pick_checkout(path, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn pick_checkout(&mut self, target_top: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let current_top = match &self.checkout {
            Some(WorktreeCheckout::Git { top, .. }) => Some(top.as_path()),
            _ => None,
        };
        let current_cwd = self.cwd.read(cx).text().to_owned();
        let next_cwd = rewrite_cwd_for_checkout(&current_cwd, current_top, &target_top);
        let next_str = next_cwd.to_string_lossy().into_owned();
        let return_focus = self
            .checkout_picker
            .take()
            .map(|p| p.return_focus)
            .unwrap_or_else(|| self.field().read(cx).focus_handle().clone());
        self.new_worktree = None;
        self.error = None;
        self.project.picked(&next_str, cx);
        self.cwd.update(cx, |input, cx| input.reset(next_str, cx));
        self.changed_directory = true;
        self.sync_sample_prompt_to_cwd(cx);
        self.refresh_checkout_state(cx);
        let _ = return_focus;
        self.focus(window, cx);
        cx.notify();
    }

    pub(super) fn enter_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(WorktreeCheckout::Git {
            top, main_checkout, ..
        }) = self.checkout.clone()
        else {
            return;
        };
        self.checkout_picker = None;
        self.error = None;
        if let Some(wt) = &self.new_worktree {
            wt.branch_field.read(cx).focus_handle().focus(window, cx);
            cx.notify();
            return;
        }
        let entity = cx.weak_entity();
        let branch_field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("launcher.worktree_branch_placeholder"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                entity
                    .update(cx, |this, cx| this.on_branch_field_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe_branch = cx.observe(&branch_field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            let changed = this.new_worktree.as_mut().is_some_and(|wt| {
                if wt.branch_query == query {
                    false
                } else {
                    wt.branch_query = query;
                    true
                }
            });
            if changed {
                this.error = None;
                this.run_worktree_check(cx);
            }
        });
        branch_field.read(cx).focus_handle().focus(window, cx);
        let repo_dir = top.clone();
        self.new_worktree_gen = self.new_worktree_gen.wrapping_add(1);
        self.new_worktree_check_gen = self.new_worktree_check_gen.wrapping_add(1);
        let form_gen = self.new_worktree_gen;
        let check_gen = self.new_worktree_check_gen;
        self.new_worktree = Some(NewWorktreeState {
            form_gen,
            repo_dir: repo_dir.clone(),
            main_checkout,
            branch_field,
            branch_query: String::new(),
            base: None,
            default_base: None,
            check: None,
            check_gen,
            check_pending: false,
            base_picker: None,
            base_anchor: Rc::new(Cell::new(Bounds::default())),
            _observe_branch: observe_branch,
        });
        let daemon = self.client.local_daemon().clone();
        let task = cx.background_spawn(async move { daemon.worktree_default_base(&repo_dir) });
        #[cfg(test)]
        let test_gate = self.worktree_test_gates.default_base.pop_front();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update(cx, |this, cx| {
                #[cfg(test)]
                let _applied = applied;
                if let (Some(wt), Ok(Some(base))) = (&mut this.new_worktree, result) {
                    if wt.form_gen == form_gen
                        && same_normalized_checkout_path(&wt.main_checkout, &base.main_checkout)
                    {
                        wt.default_base = Some(base.branch);
                        if !wt.branch_query.trim().is_empty() {
                            this.run_worktree_check(cx);
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn exit_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_worktree = None;
        self.error = None;
        self.focus(window, cx);
        cx.notify();
    }

    fn on_branch_field_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
        {
            return false;
        }
        match event.keystroke.key.as_str() {
            "down" => {
                self.selected = 0;
                self.focus(window, cx);
                cx.notify();
                true
            }
            "enter" => {
                if !self.new_worktree_allows_start() {
                    return true;
                }
                match self.step {
                    Step::Goal(_) if !event.keystroke.modifiers.shift => {
                        self.start_mission(window, cx);
                    }
                    Step::Start | Step::Crews => {
                        let (rows, _) = self.rows(cx);
                        if let Some(row) = rows.get(self.selected) {
                            self.start(row.choice.clone(), row.unavailable, window, cx);
                        }
                    }
                    _ => {}
                }
                true
            }
            "escape" => {
                self.exit_new_worktree(window, cx);
                true
            }
            _ => false,
        }
    }

    fn run_worktree_check(&mut self, cx: &mut Context<Self>) {
        if self.new_worktree.is_none() {
            return;
        }
        self.new_worktree_check_gen = self.new_worktree_check_gen.wrapping_add(1);
        let generation = self.new_worktree_check_gen;
        let wt = self.new_worktree.as_mut().unwrap();
        let trimmed = wt.branch_query.trim().to_owned();
        if trimmed.is_empty() {
            wt.check = None;
            wt.check_pending = false;
            wt.check_gen = generation;
            cx.notify();
            return;
        }
        let form_gen = wt.form_gen;
        wt.check_gen = generation;
        wt.check_pending = true;
        let repo_dir = wt.repo_dir.clone();
        let base = wt.base.clone();
        let check_repo = repo_dir.clone();
        let check_trimmed = trimmed.clone();
        let check_base = base.clone();
        let daemon = self.client.local_daemon().clone();
        let task = cx.background_spawn(async move {
            daemon.worktree_check(&check_repo, &check_trimmed, check_base.as_deref())
        });
        #[cfg(test)]
        let test_gate = self.worktree_test_gates.worktree_check.pop_front();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update(cx, |this, cx| {
                #[cfg(test)]
                let _applied = applied;
                let Some(wt) = &mut this.new_worktree else {
                    return;
                };
                if wt.form_gen != form_gen
                    || wt.check_gen != generation
                    || !same_normalized_checkout_path(&wt.repo_dir, &repo_dir)
                    || wt.branch_query.trim() != trimmed
                    || wt.base != base
                {
                    return;
                }
                wt.check_pending = false;
                wt.check = match result {
                    Ok(check) => {
                        if wt.base.is_none()
                            && let WorktreeNameCheck::NewBranch { ref base, .. } = check
                        {
                            wt.default_base = Some(base.clone());
                        }
                        Some(check)
                    }
                    Err(error) => Some(WorktreeNameCheck::Invalid {
                        reason: error.to_string(),
                    }),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wt) = &mut self.new_worktree else {
            return;
        };
        if wt.base_dimmed() {
            return;
        }
        if let Some(picker) = wt.base_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.checkout_picker = None;
        let repo_dir = wt.repo_dir.clone();
        let form_gen = wt.form_gen;
        let return_focus = wt.branch_field.read(cx).focus_handle().clone();
        let entity = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_branches"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                entity
                    .update(cx, |this, cx| this.on_base_picker_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(wt) = &mut this.new_worktree
                && let Some(picker) = &mut wt.base_picker
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
        self.base_picker_gen = self.base_picker_gen.wrapping_add(1);
        let picker_id = self.base_picker_gen;
        if let Some(wt) = &mut self.new_worktree {
            wt.base_picker = Some(LauncherBasePicker {
                id: picker_id,
                repo_dir: repo_dir.clone(),
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
        }
        self.rebuild_base_picker();
        let daemon = self.client.local_daemon().clone();
        let query_dir = repo_dir.clone();
        let task = cx.background_spawn(async move { daemon.changes_branches(&query_dir) });
        #[cfg(test)]
        let test_gate = self.worktree_test_gates.base_picker.pop_front();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update(cx, |this, cx| {
                #[cfg(test)]
                let _applied = applied;
                if let (Some(wt), Ok(branches)) = (&mut this.new_worktree, result)
                    && wt.form_gen == form_gen
                    && same_normalized_checkout_path(&wt.repo_dir, &repo_dir)
                    && let Some(picker) = &mut wt.base_picker
                    && picker.id == picker_id
                    && same_normalized_checkout_path(&picker.repo_dir, &repo_dir)
                {
                    picker.branches = branches;
                    this.rebuild_base_picker();
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn close_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(wt) = &mut self.new_worktree
            && let Some(picker) = wt.base_picker.take()
        {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn rebuild_base_picker(&mut self) {
        let Some(wt) = &mut self.new_worktree else {
            return;
        };
        let chosen_base = wt.base.clone();
        let default_base = wt.default_base.clone();
        let Some(picker) = &mut wt.base_picker else {
            return;
        };
        let mut all_branches = picker.branches.clone();
        all_branches.upstream = None;
        picker.items = build_base_picker_items(
            chosen_base.as_deref(),
            default_base.as_deref(),
            None,
            &all_branches,
            crate::i18n::text("launcher.worktree_base_default"),
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
        let Some(wt) = &mut self.new_worktree else {
            return false;
        };
        let Some(picker) = &mut wt.base_picker else {
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
                if let Some(&item_index) = visible.get(picker.highlighted) {
                    let base_ref = picker.items[item_index].base_ref.clone();
                    self.pick_base(base_ref, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn pick_base(&mut self, base_ref: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wt) = &mut self.new_worktree else {
            return;
        };
        wt.base = base_ref.map(|r| display_base_ref(&r).to_owned());
        if let Some(picker) = wt.base_picker.take() {
            picker.return_focus.focus(window, cx);
        }
        self.run_worktree_check(cx);
    }

    pub(super) fn render_checkout_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.checkout_picker.as_ref()?;
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        let new_wt_highlighted = picker.highlighted == visible.len();
        if new_wt_highlighted && picker.reveal.replace(false) {
            picker.scroll.scroll_to_item(visible.len());
        }
        let entity = cx.entity();
        let new_wt_entity = entity.clone();
        let footer = div()
            .flex_none()
            .flex()
            .flex_col()
            .child(div().mx(px(8.)).h(px(1.)).bg(theme::border()))
            .child(
                div().p(px(4.)).child(
                    div()
                        .id("launcher-checkout-new-worktree")
                        .debug_selector(|| "LAUNCHER_CHECKOUT_NEW_WORKTREE".into())
                        .h(px(30.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(new_wt_highlighted, |el| el.bg(theme::sidebar_selected()))
                        .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                        .on_click(move |_, window, cx| {
                            new_wt_entity.update(cx, |this, cx| {
                                this.enter_new_worktree(window, cx);
                            });
                        })
                        .child(div().flex_none().size(px(12.)))
                        .child(
                            svg()
                                .flex_none()
                                .size(px(13.))
                                .path("plus.svg")
                                .text_color(theme::accent()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(theme::text_ui())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                                .child(crate::i18n::text("launcher.new_worktree")),
                        ),
                ),
            )
            .into_any_element();

        let content = checkout_picker_card(
            PickerListView {
                id_prefix: "launcher-checkout-picker-item",
                selector_prefix: "LAUNCHER_CHECKOUT_PICKER_ITEM",
                title: crate::i18n::text("launcher.start_in").into(),
                field: picker.field.clone(),
                visible: &visible,
                highlighted: picker.highlighted,
                scroll: &picker.scroll,
                reveal: &picker.reveal,
            },
            &picker.items,
            true,
            Some(footer),
            Rc::new(move |pick_path, window, cx| {
                entity.update(cx, |this, cx| this.pick_checkout(pick_path, window, cx));
            }),
            "LAUNCHER_CHECKOUT_PICKER",
            "launcher-checkout-picker-list",
            "LAUNCHER_CHECKOUT_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer(
            self.checkout_anchor.get(),
            window,
            px(CHECKOUT_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_checkout_picker(window, cx));
            }),
        ))
    }

    pub(super) fn render_base_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let wt = self.new_worktree.as_ref()?;
        let picker = wt.base_picker.as_ref()?;
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        let entity = cx.entity();
        let content = base_picker_card(
            PickerListView {
                id_prefix: "launcher-base-picker-item",
                selector_prefix: "LAUNCHER_BASE_PICKER_ITEM",
                title: crate::i18n::text("launcher.start_from").into(),
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
            "LAUNCHER_BASE_PICKER",
            "launcher-base-picker-list",
            "LAUNCHER_BASE_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer_right_aligned(
            wt.base_anchor.get(),
            window,
            px(BASE_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_base_picker(window, cx));
            }),
        ))
    }

    #[cfg(test)]
    pub(crate) fn has_checkout(&self) -> bool {
        matches!(self.checkout, Some(WorktreeCheckout::Git { .. }))
    }

    #[cfg(test)]
    pub(crate) fn new_worktree_check_state(&self) -> Option<&WorktreeNameCheck> {
        self.new_worktree.as_ref().and_then(|wt| wt.check.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn new_worktree_is_open(&self) -> bool {
        self.new_worktree.is_some()
    }

    #[cfg(test)]
    pub(crate) fn new_worktree_base_label_for_test(&self) -> Option<String> {
        self.new_worktree.as_ref().map(NewWorktreeState::base_label)
    }

    #[cfg(test)]
    pub(crate) fn new_worktree_default_base_for_test(&self) -> Option<&str> {
        self.new_worktree
            .as_ref()
            .and_then(|wt| wt.default_base.as_deref())
    }

    #[cfg(test)]
    pub(crate) fn checkout_picker_labels_for_test(&self) -> Option<Vec<String>> {
        self.checkout_picker.as_ref().map(|picker| {
            picker
                .items
                .iter()
                .map(|i| i.branch_label.clone())
                .collect()
        })
    }

    #[cfg(test)]
    pub(crate) fn base_picker_labels_for_test(&self) -> Option<Vec<String>> {
        self.new_worktree
            .as_ref()
            .and_then(|wt| wt.base_picker.as_ref())
            .map(|picker| picker.items.iter().map(|i| i.label.clone()).collect())
    }

    #[cfg(test)]
    pub(crate) fn stage_worktree_check_gate_for_test(
        &mut self,
    ) -> crate::test_support::StagedReplyHandle {
        let (gate, handle) = crate::test_support::StagedReplyGate::pair();
        self.worktree_test_gates.worktree_check.push_back(gate);
        handle
    }

    #[cfg(test)]
    pub(crate) fn stage_default_base_gate_for_test(
        &mut self,
    ) -> crate::test_support::StagedReplyHandle {
        let (gate, handle) = crate::test_support::StagedReplyGate::pair();
        self.worktree_test_gates.default_base.push_back(gate);
        handle
    }

    #[cfg(test)]
    pub(crate) fn stage_checkout_picker_gate_for_test(
        &mut self,
    ) -> crate::test_support::StagedReplyHandle {
        let (gate, handle) = crate::test_support::StagedReplyGate::pair();
        self.worktree_test_gates.checkout_picker.push_back(gate);
        handle
    }

    #[cfg(test)]
    pub(crate) fn stage_base_picker_gate_for_test(
        &mut self,
    ) -> crate::test_support::StagedReplyHandle {
        let (gate, handle) = crate::test_support::StagedReplyGate::pair();
        self.worktree_test_gates.base_picker.push_back(gate);
        handle
    }

    #[cfg(test)]
    pub(crate) fn toggle_base_picker_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_base_picker(window, cx);
    }

    #[cfg(test)]
    pub(crate) fn set_new_worktree_branch_for_test(&self, text: &str, cx: &mut App) {
        if let Some(wt) = &self.new_worktree {
            wt.branch_field
                .update(cx, |field, cx| field.reset(text, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_hint_lines_and_targets_preserve_worktree_suffix() {
        runner_app::i18n::testing::with_locale(crate::i18n::Locale::English, || {
            let main = Path::new(r"C:\Users\dev\repo");
            let target = Path::new(r"C:\Users\dev\repo\.worktrees\feat-win");

            let (_, new_hint, _) = format_new_worktree_hint(
                "feat/win",
                Some(&WorktreeNameCheck::NewBranch {
                    base: "main".into(),
                    path: target.to_path_buf(),
                }),
                main,
                None,
            );
            assert!(
                new_hint.contains(r".worktrees\feat-win"),
                "expected Windows worktree suffix in hint: {new_hint}"
            );
            assert!(new_hint.contains("main"));

            let (_, existing_hint, _) = format_new_worktree_hint(
                "feat/win",
                Some(&WorktreeNameCheck::ExistingBranch {
                    commit: "abc1234".into(),
                    path: target.to_path_buf(),
                }),
                main,
                None,
            );
            assert!(
                existing_hint.contains(r".worktrees\feat-win"),
                "expected Windows worktree suffix in existing hint: {existing_hint}"
            );
            assert!(existing_hint.contains("abc1234"));

            let (_, checked_out_hint, _) = format_new_worktree_hint(
                "feat/win",
                Some(&WorktreeNameCheck::AlreadyCheckedOut {
                    path: target.to_path_buf(),
                }),
                main,
                None,
            );
            assert!(
                checked_out_hint.contains("…/feat-win"),
                "expected shortened .worktrees path in hint: {checked_out_hint}"
            );
        });
    }
}

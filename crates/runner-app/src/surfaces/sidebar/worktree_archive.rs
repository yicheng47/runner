use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use gpui::{div, prelude::*, px, rems, svg, AnyElement, FontWeight, MouseButton, Window};
use runner_core::app_paths::{normalize_checkout_path, same_normalized_checkout_path};
use runner_core::protocol::files::WorktreeKind;
use runner_core::protocol::worktree::WorktreeCheckout;

use super::archive::linked_missions_for_local_chats;
use crate::app_store::AppStore;
use crate::surfaces::panes::{TerminalCloseConfirm, TerminalCloseTarget};
use crate::{theme, App, Context, NativeRoot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeRemoveState {
    Checking,
    Clean { checked: bool },
    Dirty { uncommitted: usize },
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeRemoveOption {
    pub(crate) main_checkout: PathBuf,
    pub(crate) top: PathBuf,
    pub(crate) branch: String,
    pub(crate) display_path: String,
    pub(crate) state: WorktreeRemoveState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeToRemove {
    pub(crate) main_checkout: PathBuf,
    pub(crate) top: PathBuf,
    pub(crate) branch: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeCleanScope {
    Chat,
    Tab,
    Mission,
}

fn format_worktree_display_path(main_checkout: &Path, top: &Path) -> String {
    let wt_folder = top
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("worktree");
    match main_checkout
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
    {
        Some(repo) => format!("{repo}/.worktrees/{wt_folder}"),
        None => format!(".worktrees/{wt_folder}"),
    }
}

fn extract_managed_candidate(
    checkout: &WorktreeCheckout,
) -> Option<(PathBuf, PathBuf, String, String)> {
    let WorktreeCheckout::Git {
        top,
        main_checkout,
        kind: WorktreeKind::Linked,
        managed: true,
        missing: false,
        branch,
        commit,
        ..
    } = checkout
    else {
        return None;
    };
    let norm_top = normalize_checkout_path(top);
    let norm_main = normalize_checkout_path(main_checkout);
    let label = branch
        .as_deref()
        .filter(|b| !b.is_empty())
        .or_else(|| commit.as_deref().filter(|c| !c.is_empty()))
        .or_else(|| norm_top.file_name().and_then(|name| name.to_str()))
        .unwrap_or("worktree")
        .to_owned();
    let display_path = format_worktree_display_path(&norm_main, &norm_top);
    Some((norm_main, norm_top, label, display_path))
}

/// Computes which Runner-managed `.worktrees/*` checkouts would become completely
/// unreferenced after archiving `session_ids` (plus linked missions) and `mission_ids`.
pub(crate) fn emptied_managed_worktrees_for_archive(
    store: &AppStore,
    session_ids: &[String],
    mission_ids: &[String],
) -> Vec<WorktreeRemoveOption> {
    let archiving_sessions = session_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let linked_mission_items = linked_missions_for_local_chats(
        session_ids,
        &store.sessions,
        &store.missions,
        &store.nodes,
    );
    let mut archiving_mission_ids = mission_ids.to_vec();
    for item in &linked_mission_items {
        if !archiving_mission_ids.contains(&item.id) {
            archiving_mission_ids.push(item.id.clone());
        }
    }
    let archiving_mission_set = archiving_mission_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    let mut archiving_mission_sessions = HashSet::new();
    for (mission_id, slots) in &store.mission_sessions {
        if archiving_mission_set.contains(mission_id.as_str()) {
            for slot in slots {
                archiving_mission_sessions.insert(slot.session.id.as_str());
            }
        }
    }

    let mut surviving_tops = BTreeSet::new();
    for session in &store.sessions {
        if session.archived_at.is_some()
            || archiving_sessions.contains(session.session_id.as_str())
            || archiving_mission_sessions.contains(session.session_id.as_str())
        {
            continue;
        }
        if let Some(cwd) = session.cwd.as_deref()
            && let Some(WorktreeCheckout::Git { top, .. }) =
                store.worktree_checkout_for(cwd, session.project_id.as_deref())
        {
            surviving_tops.insert(normalize_checkout_path(top));
        }
    }
    for summary in &store.missions {
        if summary.mission.archived_at.is_some()
            || archiving_mission_set.contains(summary.mission.id.as_str())
        {
            continue;
        }
        if let Some(cwd) = summary.mission.cwd.as_deref()
            && let Some(WorktreeCheckout::Git { top, .. }) =
                store.worktree_checkout_for(cwd, summary.mission.project_id.as_deref())
        {
            surviving_tops.insert(normalize_checkout_path(top));
        }
        if let Some(slots) = store.mission_sessions.get(&summary.mission.id) {
            for slot in slots {
                if let Some(cwd) = slot.session.cwd.as_deref()
                    && let Some(WorktreeCheckout::Git { top, .. }) =
                        store.worktree_checkout_for(cwd, summary.mission.project_id.as_deref())
                {
                    surviving_tops.insert(normalize_checkout_path(top));
                }
            }
        }
    }

    let mut seen_tops = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut push_checkout = |checkout: &WorktreeCheckout| {
        let Some((main_checkout, top, branch, display_path)) = extract_managed_candidate(checkout)
        else {
            return;
        };
        if surviving_tops.contains(&top) || !seen_tops.insert(top.clone()) {
            return;
        }
        candidates.push(WorktreeRemoveOption {
            main_checkout,
            top,
            branch,
            display_path,
            state: WorktreeRemoveState::Checking,
        });
    };

    for id in session_ids {
        if let Some(session) = store.sessions.iter().find(|s| s.session_id == *id)
            && let Some(cwd) = session.cwd.as_deref()
            && let Some(checkout) = store.worktree_checkout_for(cwd, session.project_id.as_deref())
        {
            push_checkout(checkout);
        }
    }
    for mission_id in &archiving_mission_ids {
        if let Some(summary) = store.missions.iter().find(|m| m.mission.id == *mission_id)
            && let Some(cwd) = summary.mission.cwd.as_deref()
            && let Some(checkout) =
                store.worktree_checkout_for(cwd, summary.mission.project_id.as_deref())
        {
            push_checkout(checkout);
        }
    }

    candidates
}

pub(crate) fn checked_worktrees_to_remove(confirm: &TerminalCloseConfirm) -> Vec<WorktreeToRemove> {
    confirm
        .worktree_options
        .iter()
        .filter(|option| matches!(option.state, WorktreeRemoveState::Clean { checked: true }))
        .map(|option| WorktreeToRemove {
            main_checkout: option.main_checkout.clone(),
            top: option.top.clone(),
            branch: option.branch.clone(),
        })
        .collect()
}

pub(crate) fn render_worktree_remove_options(
    options: &[WorktreeRemoveOption],
    scope: WorktreeCleanScope,
    root: gpui::Entity<NativeRoot>,
) -> Option<AnyElement> {
    let visible = options
        .iter()
        .enumerate()
        .filter(|(_, opt)| opt.state != WorktreeRemoveState::Unavailable)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return None;
    }

    Some(
        div()
            .id("confirm-worktree-options")
            .w(rems((420. - 2. * 22.) / 16.))
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .debug_selector(|| "CONFIRM_WORKTREE_OPTIONS".into())
            .children(visible.into_iter().map(|(index, option)| {
                let root = root.clone();
                let title = crate::i18n::tf(
                    "archive.remove_worktree",
                    &[("branch", option.branch.clone())],
                );
                let branch_tag = option.branch.clone();
                let clean_key = match scope {
                    WorktreeCleanScope::Chat => "archive.remove_worktree_clean_chat",
                    WorktreeCleanScope::Tab => "archive.remove_worktree_clean_tab",
                    WorktreeCleanScope::Mission => "archive.remove_worktree_clean_mission",
                };
                match option.state {
                    WorktreeRemoveState::Clean { checked } => div()
                        .id(gpui::SharedString::from(format!(
                            "confirm-worktree-{index}"
                        )))
                        .debug_selector({
                            let tag = branch_tag.clone();
                            move || format!("CONFIRM_WORKTREE_CLEAN {tag} {checked}")
                        })
                        .w_full()
                        .px(rems(10. / 16.))
                        .py(rems(8. / 16.))
                        .rounded(rems(8. / 16.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::raised())
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::sidebar_selected()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, cx| {
                                this.toggle_confirm_worktree(index, cx);
                            });
                        })
                        .flex()
                        .items_start()
                        .gap(rems(8. / 16.))
                        .child(
                            div()
                                .mt(rems(2. / 16.))
                                .size(rems(14. / 16.))
                                .flex_none()
                                .rounded(rems(4. / 16.))
                                .border_1()
                                .border_color(if checked {
                                    theme::accent()
                                } else {
                                    theme::faint()
                                })
                                .bg(if checked {
                                    theme::accent()
                                } else {
                                    gpui::transparent_black()
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(checked, |box_div| {
                                    box_div.child(
                                        svg()
                                            .path("check.svg")
                                            .size(rems(10. / 16.))
                                            .text_color(theme::accent_ink()),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(rems(2. / 16.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_ui())
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .whitespace_normal()
                                        .text_size(theme::text_meta())
                                        .line_height(rems(1.))
                                        .text_color(theme::faint())
                                        .child(crate::i18n::tf(
                                            clean_key,
                                            &[("path", option.display_path.clone())],
                                        )),
                                ),
                        )
                        .into_any_element(),
                    WorktreeRemoveState::Dirty { uncommitted } => div()
                        .debug_selector({
                            let tag = branch_tag.clone();
                            move || format!("CONFIRM_WORKTREE_DIRTY {tag}")
                        })
                        .w_full()
                        .px(rems(10. / 16.))
                        .py(rems(8. / 16.))
                        .rounded(rems(8. / 16.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::raised())
                        .flex()
                        .items_start()
                        .gap(rems(8. / 16.))
                        .child(
                            div()
                                .mt(rems(2. / 16.))
                                .size(rems(14. / 16.))
                                .flex_none()
                                .rounded(rems(4. / 16.))
                                .border_1()
                                .border_color(theme::border_strong())
                                .opacity(0.45),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(rems(3. / 16.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_ui())
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::muted())
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap(rems(5. / 16.))
                                        .text_size(theme::text_meta())
                                        .line_height(rems(1.))
                                        .text_color(theme::warning())
                                        .child(
                                            svg()
                                                .mt(rems(2. / 16.))
                                                .flex_none()
                                                .size(rems(12. / 16.))
                                                .path("circle-alert.svg")
                                                .text_color(theme::warning()),
                                        )
                                        .child(
                                            div().flex_1().min_w(px(0.)).whitespace_normal().child(
                                                crate::i18n::tn(
                                                    "archive.remove_worktree_dirty",
                                                    uncommitted,
                                                ),
                                            ),
                                        ),
                                ),
                        )
                        .into_any_element(),
                    WorktreeRemoveState::Checking => div()
                        .debug_selector({
                            let tag = branch_tag.clone();
                            move || format!("CONFIRM_WORKTREE_CHECKING {tag}")
                        })
                        .w_full()
                        .px(rems(10. / 16.))
                        .py(rems(8. / 16.))
                        .rounded(rems(8. / 16.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::raised())
                        .flex()
                        .items_start()
                        .gap(rems(8. / 16.))
                        .child(
                            div()
                                .mt(rems(2. / 16.))
                                .size(rems(14. / 16.))
                                .flex_none()
                                .rounded(rems(4. / 16.))
                                .border_1()
                                .border_color(theme::border_strong())
                                .opacity(0.45),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(rems(2. / 16.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_ui())
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::muted())
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .child(option.display_path.clone()),
                                ),
                        )
                        .into_any_element(),
                    WorktreeRemoveState::Unavailable => unreachable!(),
                }
            }))
            .into_any_element(),
    )
}

impl NativeRoot {
    pub(crate) fn emptied_managed_worktrees(
        &self,
        session_ids: &[String],
        mission_ids: &[String],
        cx: &App,
    ) -> Vec<WorktreeRemoveOption> {
        if self.front_page_client(cx).is_remote() {
            return Vec::new();
        }
        let store = self.app_store.read(cx);
        emptied_managed_worktrees_for_archive(store, session_ids, mission_ids)
    }

    pub(crate) fn open_archive_confirm(
        &mut self,
        target: TerminalCloseTarget,
        worktree_options: Vec<WorktreeRemoveOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panes.has_close_confirm() {
            return;
        }
        TerminalCloseConfirm::open(&mut self.panes.close_confirm, target, window, cx);
        let Some(confirm) = self.panes.close_confirm.as_mut() else {
            return;
        };
        if worktree_options.is_empty() {
            cx.notify();
            return;
        }
        confirm.worktree_options = worktree_options.clone();
        confirm.worktree_check_generation = confirm.worktree_check_generation.wrapping_add(1);
        let confirm_id = confirm.id;
        let generation = confirm.worktree_check_generation;
        let repos = worktree_options
            .iter()
            .map(|opt| opt.main_checkout.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let client = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            repos
                .into_iter()
                .map(|repo| {
                    let entries = client.files_worktrees(&repo).ok();
                    (repo, entries)
                })
                .collect::<Vec<_>>()
        });
        #[cfg(test)]
        let test_gate = self.archive_check_gates.pop_front();
        cx.spawn(async move |weak, cx| {
            let results = task.await;
            #[cfg(test)]
            let applied = match test_gate {
                Some(gate) => Some(gate.wait().await),
                None => None,
            };
            let _ = weak.update(cx, |this, cx| {
                #[cfg(test)]
                let _applied = applied;
                let Some(confirm) = this.panes.close_confirm.as_mut() else {
                    return;
                };
                if confirm.id != confirm_id || confirm.worktree_check_generation != generation {
                    return;
                }
                for opt in &mut confirm.worktree_options {
                    let entries = results
                        .iter()
                        .find(|(repo, _)| same_normalized_checkout_path(repo, &opt.main_checkout))
                        .and_then(|(_, entries)| entries.as_ref());
                    let Some(entries) = entries else {
                        opt.state = WorktreeRemoveState::Unavailable;
                        continue;
                    };
                    let Some(entry) = entries.iter().find(|entry| {
                        entry.kind == WorktreeKind::Linked
                            && same_normalized_checkout_path(&entry.path, &opt.top)
                    }) else {
                        opt.state = WorktreeRemoveState::Unavailable;
                        continue;
                    };
                    opt.state = match entry.uncommitted {
                        Some(0) => WorktreeRemoveState::Clean { checked: true },
                        Some(n) => WorktreeRemoveState::Dirty { uncommitted: n },
                        None => WorktreeRemoveState::Unavailable,
                    };
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn toggle_confirm_worktree(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(confirm) = self.panes.close_confirm.as_mut()
            && let Some(option) = confirm.worktree_options.get_mut(index)
            && let WorktreeRemoveState::Clean { checked } = &mut option.state
        {
            *checked = !*checked;
            cx.notify();
        }
    }

    pub(crate) fn request_archive_mission(
        &mut self,
        mission_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.front_page_client(cx).is_remote() {
            return;
        }
        let worktree_options =
            self.emptied_managed_worktrees(&[], std::slice::from_ref(&mission_id), cx);
        if worktree_options.is_empty() {
            self.execute_archive_mission(&mission_id, Vec::new(), window, cx);
            return;
        }
        self.open_archive_confirm(
            TerminalCloseTarget::ArchiveMission { mission_id },
            worktree_options,
            window,
            cx,
        );
    }

    pub(crate) fn remove_checked_worktrees(
        &mut self,
        worktrees: Vec<WorktreeToRemove>,
        cx: &mut Context<Self>,
    ) {
        if worktrees.is_empty() {
            return;
        }
        let client = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            worktrees
                .into_iter()
                .map(|wt| {
                    let target = wt.top.to_string_lossy().into_owned();
                    let result = client.worktree_remove(&wt.main_checkout, &target);
                    (wt, result.map_err(|err| err.to_string()))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |weak, cx| {
            let results = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.refresh_store(crate::app_store::StoreRefreshKind::All, cx);
                for (wt, result) in results {
                    if let Err(reason) = result {
                        this.show_toast(
                            crate::i18n::messagef(
                                "archive.remove_worktree_failed",
                                &[("name", wt.branch), ("reason", reason)],
                            ),
                            crate::toast::ToastTone::Error,
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

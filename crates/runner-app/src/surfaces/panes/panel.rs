use std::path::Path;

use runner_core::protocol::files::WorktreeKind;
use runner_core::protocol::model::Runtime;
use runner_core::protocol::worktree::WorktreeCheckout;

use super::*;
use crate::surfaces::mission_markdown::{parse_blocks, render_markdown};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InfoBranchRow {
    pub(crate) branch: String,
    pub(crate) top: Option<String>,
    pub(crate) marker: String,
    pub(crate) missing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InfoWorktreeRow {
    pub(crate) branch: String,
    pub(crate) path: Option<String>,
    pub(crate) marker: String,
    pub(crate) missing: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InfoLocationRows {
    pub(crate) worktree: Option<InfoWorktreeRow>,
    pub(crate) cwd: Option<String>,
}

/// `#1360` command de-duplication: hide `cmd` when it matches the runtime's
/// standard CLI executable (`claude`, `codex`, `agy`, `pi`, `opencode`, etc.).
pub(crate) fn should_show_cmd(runtime: &str, command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    match Runtime::parse(runtime).and_then(Runtime::command) {
        Some(default_cmd) => trimmed != default_cmd,
        None => true,
    }
}

fn strip_worktree_prefix(cwd: &str, top: &str) -> Option<String> {
    let cwd_trimmed = cwd.trim().trim_end_matches(['/', '\\']);
    let top_trimmed = top.trim().trim_end_matches(['/', '\\']);
    if cwd_trimmed.is_empty() || top_trimmed.is_empty() {
        return None;
    }
    if cwd_trimmed == top_trimmed {
        return Some(String::new());
    }
    if let Ok(rel) = Path::new(cwd_trimmed).strip_prefix(Path::new(top_trimmed)) {
        let rel = rel.to_string_lossy().replace('\\', "/");
        return Some(rel);
    }
    let cwd_norm = cwd_trimmed.replace('\\', "/");
    let top_norm = top_trimmed.replace('\\', "/");
    if cwd_norm == top_norm {
        return Some(String::new());
    }
    cwd_norm
        .strip_prefix(&format!("{top_norm}/"))
        .map(str::to_owned)
}

/// `#1360` location de-duplication:
/// - Inside a linked worktree, show a single `worktree` row (branch + copyable
///   full path + `from <base>` marker) and omit `cwd` unless `cwd` is a strict
///   subdirectory below the worktree root (then show the relative subdir).
/// - Outside a linked worktree (main checkout or non-git folder), show `cwd`
///   and no `worktree` row.
pub(crate) fn resolve_info_location_rows(
    cwd: Option<&str>,
    branch: Option<&InfoBranchRow>,
) -> InfoLocationRows {
    let cwd = cwd.map(str::trim).filter(|c| !c.is_empty());
    let Some(branch) = branch else {
        return InfoLocationRows {
            worktree: None,
            cwd: cwd.map(str::to_owned),
        };
    };
    let full_path = branch
        .top
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or(cwd)
        .map(str::to_owned);
    let relative_cwd = match (cwd, branch.top.as_deref()) {
        (Some(cwd), Some(top)) => strip_worktree_prefix(cwd, top).filter(|rel| !rel.is_empty()),
        _ => None,
    };
    InfoLocationRows {
        worktree: Some(InfoWorktreeRow {
            branch: branch.branch.clone(),
            path: full_path,
            marker: branch.marker.clone(),
            missing: branch.missing,
        }),
        cwd: relative_cwd,
    }
}

pub(crate) fn remote_info_branch_row(
    store: &Entity<AppStore>,
    computer: &str,
    mission_id: Option<&str>,
    session_id: Option<&str>,
    cwd: Option<&str>,
    cx: &App,
) -> Option<InfoBranchRow> {
    let clients = store.read(cx).remote_clients.read(cx);
    let entry = if let Some(mission_id) = mission_id {
        clients.remote_mission_checkout(computer, mission_id, cwd)
    } else if let Some(session_id) = session_id {
        clients.remote_session_checkout(computer, session_id, cwd)
    } else {
        None
    }?;
    info_branch_row_from_checkout(&entry.checkout, Some(entry.auto_base.as_deref()))
}

pub(crate) fn ensure_info_branch_row(
    store: &Entity<AppStore>,
    cwd: Option<&str>,
    project_id: Option<&str>,
    remote: bool,
    cx: &mut App,
) -> Option<InfoBranchRow> {
    if remote {
        return None;
    }
    let cwd = cwd.map(str::trim).filter(|c| !c.is_empty())?;
    let state = store.read(cx);
    let Some(checkout) = state.worktree_checkout_for(cwd, project_id).cloned() else {
        let store = store.clone();
        let cwd = cwd.to_owned();
        let project_id = project_id.map(str::to_owned);
        cx.defer(move |cx| {
            store.update(cx, |store, cx| {
                store.ensure_worktree_checkout(&cwd, project_id.as_deref(), cx);
            });
        });
        return None;
    };
    let auto_base = match &checkout {
        WorktreeCheckout::Git {
            kind: WorktreeKind::Linked,
            top,
            missing,
            ..
        } => {
            let loaded = state
                .worktree_auto_base_for(top)
                .map(|base| base.map(str::to_owned));
            if !*missing && loaded.is_none() {
                let store = store.clone();
                let top = top.clone();
                cx.defer(move |cx| {
                    store.update(cx, |store, cx| store.ensure_worktree_auto_base(&top, cx));
                });
            }
            loaded
        }
        _ => return None,
    };
    info_branch_row_from_checkout(&checkout, auto_base.as_ref().map(|b| b.as_deref()))
}

pub(crate) fn info_branch_row_from_checkout(
    checkout: &WorktreeCheckout,
    auto_base: Option<Option<&str>>,
) -> Option<InfoBranchRow> {
    let WorktreeCheckout::Git {
        kind: WorktreeKind::Linked,
        top,
        branch,
        commit,
        missing,
        ..
    } = checkout
    else {
        return None;
    };
    let top_string = top.to_string_lossy().into_owned();
    let branch_text = if *missing {
        let label = branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                crate::surfaces::remote_client::model::host_path_leaf(&top_string)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        crate::i18n::tf("sidebar.worktree_missing", &[("label", label)])
    } else {
        branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .or_else(|| commit.as_deref().filter(|c| !c.is_empty()))
            .unwrap_or("HEAD")
            .to_owned()
    };
    let marker = if !*missing {
        match auto_base {
            Some(Some(base)) => {
                crate::i18n::tf("info.worktree_from_base", &[("base", base.to_owned())])
            }
            _ => crate::i18n::text("info.worktree").to_owned(),
        }
    } else {
        crate::i18n::text("info.worktree").to_owned()
    };
    Some(InfoBranchRow {
        branch: branch_text,
        top: (!top_string.is_empty()).then_some(top_string),
        marker,
        missing: *missing,
    })
}

pub(crate) fn info_worktree_marker(marker: String) -> Div {
    div()
        .when(cfg!(test), |el| {
            let tag = marker.clone();
            el.debug_selector(move || format!("INFO_WORKTREE_MARKER {tag}"))
        })
        .flex()
        .items_center()
        .gap(rems(4. / 16.))
        .text_size(theme::text_caption())
        .text_color(theme::faint())
        .child(
            svg()
                .path("folder-git-2.svg")
                .size(rems(11. / 16.))
                .flex_none()
                .text_color(theme::faint()),
        )
        .child(marker)
}

pub(super) struct ChatInfoPopoverActions {
    pub(super) open_role: Option<ClickHandler>,
    pub(super) toggle_prompt: ClickHandler,
    pub(super) expanded: bool,
    pub(super) view: gpui::EntityId,
    pub(super) shortcut_hint: Option<String>,
}

fn render_worktree_row(
    worktree: InfoWorktreeRow,
    copy_button: Option<Entity<CopyValueButton>>,
) -> AnyElement {
    let tooltip_path = worktree.path.clone();
    let branch_label = div()
        .min_w(px(0.))
        .flex_1()
        .truncate()
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_color(if worktree.missing {
            theme::warning()
        } else {
            theme::text()
        })
        .when(cfg!(test), |col| {
            let text = worktree.branch.clone();
            col.debug_selector(move || format!("CHAT_PANEL_META {text}"))
        })
        .child(worktree.branch);
    let branch_with_tip = if let Some(full_path) = tooltip_path {
        Tooltip::new("s2-info-worktree-path", full_path, branch_label)
            .expand()
            .into_any_element()
    } else {
        branch_label.into_any_element()
    };

    side_panel_row(
        crate::i18n::text("info.worktree"),
        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHAT_PANEL_WORKTREE_ROW".into())
            })
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(2. / 16.))
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(branch_with_tip)
                    .children(copy_button),
            )
            .child(info_worktree_marker(worktree.marker)),
    )
}

pub(super) fn render_s2_remote_chat_info_card(
    session: SessionDisplay,
    branch: Option<InfoBranchRow>,
    worktree_copy: Option<Entity<CopyValueButton>>,
    shortcut_hint: Option<String>,
) -> AnyElement {
    let runtime = &session.agent_runtime;
    let location = resolve_info_location_rows(session.cwd.as_deref(), branch.as_ref());
    div()
        .when(cfg!(test), |el| {
            el.debug_selector(|| "S2_CHAT_INFO_CARD".into())
        })
        .p(rems(12. / 16.))
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap(rems(8. / 16.))
                        .child(runtime_mark(runtime, 24.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child(runtime_display_name(runtime)),
                        ),
                )
                .children(shortcut_hint.map(|hint| {
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_micro())
                        .text_color(theme::faint())
                        .child(hint)
                })),
        )
        .when(
            location.worktree.is_some() || location.cwd.is_some(),
            |el| {
                el.child(div().h(px(1.)).w_full().bg(theme::border()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .children(
                                location
                                    .worktree
                                    .map(|wt| render_worktree_row(wt, worktree_copy)),
                            )
                            .children(
                                location
                                    .cwd
                                    .map(|cwd| side_panel_row("cwd", side_panel_value(cwd))),
                            ),
                    )
            },
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_s2_chat_info_card(
    detail: &DirectSessionEntry,
    role: Option<&runner_core::protocol::model::Role>,
    session_key_copy: Entity<CopyValueButton>,
    worktree_copy: Option<Entity<CopyValueButton>>,
    actions: ChatInfoPopoverActions,
    account: Option<(String, Option<String>, String)>,
    branch: Option<InfoBranchRow>,
    cx: &App,
) -> AnyElement {
    let cwd = detail
        .cwd
        .clone()
        .or_else(|| role.and_then(|role| role.working_dir.clone()));
    let location = resolve_info_location_rows(cwd.as_deref(), branch.as_ref());
    let show_cmd = should_show_cmd(&detail.agent_runtime, &detail.agent_command);
    let system_prompt = role
        .and_then(|role| role.system_prompt.as_deref())
        .filter(|text| !text.trim().is_empty());

    let (model, model_default) = role_setting_label(detail.agent_model.as_deref());
    let (effort, effort_default) =
        crate::surfaces::roles::logic::role_effort_label(detail.agent_effort.as_deref());

    let header_left = div()
        .min_w(px(0.))
        .flex_1()
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .child(
            div()
                .flex_none()
                .size(rems(28. / 16.))
                .flex()
                .items_center()
                .justify_center()
                .when(cfg!(test), |identity| {
                    identity.debug_selector(|| {
                        role.map(|role| format!("CHAT_PANEL_AVATAR {}", role.handle))
                            .unwrap_or_else(|| format!("CHAT_PANEL_MARK {}", detail.agent_runtime))
                    })
                })
                .child(if let Some(role) = role {
                    RoleAvatar::new(role.handle.clone(), 28.).into_any_element()
                } else {
                    runtime_mark(&detail.agent_runtime, 24.)
                }),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .flex()
                .flex_col()
                .child({
                    let title = role
                        .map(|role| role.display_name.clone())
                        .unwrap_or_else(|| runtime_display_name(&detail.agent_runtime));
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_body())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .when(cfg!(test), |el| {
                            let tag = title.clone();
                            el.debug_selector(move || format!("COLUMN_TEXT {tag}"))
                        })
                        .child(title)
                })
                .children(role.map(|role| {
                    let handle = format!("@{}", role.handle);
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::muted())
                        .when(cfg!(test), |el| {
                            let tag = handle.clone();
                            el.debug_selector(move || format!("COLUMN_TEXT {tag}"))
                        })
                        .child(handle)
                })),
        );

    let header_right = if let Some(on_open) = actions.open_role {
        let key_open = on_open.clone();
        Some(
            div()
                .id("chat-panel-open-role")
                .when(cfg!(test), |link| {
                    link.debug_selector(|| "CHAT_PANEL_OPEN_ROLE".into())
                })
                .tab_index(0)
                .flex_none()
                .flex()
                .items_center()
                .gap(rems(3. / 16.))
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .cursor_pointer()
                .hover(|link| link.text_color(theme::text()))
                .focus_visible(|link| link.text_color(theme::text()).underline())
                .on_click(move |_, window, cx| on_open(window, cx))
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        key_open(window, cx);
                    }
                })
                .child(crate::i18n::text("ui.open_role"))
                .child(
                    svg()
                        .path("arrow-up-right.svg")
                        .size(rems(11. / 16.))
                        .text_color(theme::muted()),
                )
                .into_any_element(),
        )
    } else {
        actions.shortcut_hint.map(|hint| {
            div()
                .flex_none()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_micro())
                .text_color(theme::faint())
                .child(hint)
                .into_any_element()
        })
    };

    let setup_summary = {
        let mut parts = Vec::<AnyElement>::new();
        if role.is_some() {
            let runtime_name = runtime_display_name(&detail.agent_runtime);
            parts.push(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(rems(4. / 16.))
                    .text_color(theme::text())
                    .when(cfg!(test), |el| {
                        let tag = runtime_name.clone();
                        el.debug_selector(move || format!("COLUMN_TEXT {tag}"))
                    })
                    .child(runtime_mark(&detail.agent_runtime, 12.))
                    .child(runtime_name)
                    .into_any_element(),
            );
        }
        let dot = || {
            div()
                .flex_none()
                .text_color(theme::faint())
                .child("·")
                .into_any_element()
        };
        if !parts.is_empty() {
            parts.push(dot());
        }
        parts.push(
            div()
                .min_w(px(0.))
                .truncate()
                .when(!model_default, |el| {
                    el.font_family(theme::UI_MONOSPACE_FONT)
                })
                .text_color(if model_default {
                    theme::muted()
                } else {
                    theme::text()
                })
                .when(cfg!(test), |el| {
                    let tag = model.clone();
                    el.debug_selector(move || format!("COLUMN_TEXT {tag}"))
                })
                .child(model)
                .into_any_element(),
        );
        parts.push(dot());
        parts.push(
            div()
                .flex_none()
                .when(!effort_default, |el| {
                    el.font_family(theme::UI_MONOSPACE_FONT)
                })
                .text_color(if effort_default {
                    theme::muted()
                } else {
                    theme::text()
                })
                .when(cfg!(test), |el| {
                    let tag = effort.clone();
                    el.debug_selector(move || format!("COLUMN_TEXT {tag}"))
                })
                .child(effort)
                .into_any_element(),
        );

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rems(5. / 16.))
            .text_size(theme::text_meta())
            .children(parts)
    };

    let account_row = account.as_ref().map(|(name, email, _)| {
        side_panel_row(
            crate::i18n::text("accounts.account"),
            div()
                .debug_selector(|| "CHAT_ACCOUNT_DETAIL".into())
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_color(theme::text())
                        .child(name.clone()),
                )
                .children(email.as_ref().map(|email| {
                    div()
                        .truncate()
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child(email.clone())
                })),
        )
    });

    let metadata_rows = div()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .children(account_row)
        .children(
            location
                .worktree
                .map(|wt| render_worktree_row(wt, worktree_copy)),
        )
        .children(
            location
                .cwd
                .map(|cwd| side_panel_row("cwd", side_panel_value(cwd))),
        )
        .children(
            show_cmd.then(|| side_panel_row("cmd", side_panel_value(detail.agent_command.clone()))),
        )
        .children(account.map(|(_, _, folder)| side_panel_row("config", side_panel_value(folder))))
        .child(side_panel_row(
            "session_key",
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_color(theme::muted())
                        .child(
                            detail
                                .agent_session_key
                                .clone()
                                .unwrap_or_else(|| "NULL".into()),
                        ),
                )
                .child(session_key_copy),
        ));

    let prompt_section = system_prompt.map(|prompt| {
        let toggle = actions.toggle_prompt;
        let expanded = actions.expanded;
        div()
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .child(
                div()
                    .id("s2-info-prompt-toggle")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "CHAT_PANEL_PROMPT_TOGGLE".into())
                    })
                    .px(rems(4. / 16.))
                    .py(rems(4. / 16.))
                    .mx(rems(-4. / 16.))
                    .rounded(rems(4. / 16.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::sidebar_selected()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        toggle(window, cx);
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(4. / 16.))
                            .child(
                                svg()
                                    .flex_none()
                                    .size(rems(12. / 16.))
                                    .path(if expanded {
                                        "chevron-down.svg"
                                    } else {
                                        "chevron-right.svg"
                                    })
                                    .text_color(theme::faint()),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("ui.system_prompt")),
                            ),
                    )
                    .child(
                        div()
                            .when(cfg!(test), |meta| {
                                meta.debug_selector(|| "CHAT_PANEL_PROMPT_META".into())
                            })
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_caption())
                            .text_color(theme::faint())
                            .child(prompt_meta(prompt)),
                    ),
            )
            .when(expanded, |section| {
                section.child(
                    div()
                        .id("s2-info-prompt-scroll")
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "CHAT_PANEL_PROMPT".into())
                        })
                        .max_h(rems(180. / 16.))
                        .overflow_y_scroll()
                        .rounded(rems(6. / 16.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        .p(rems(8. / 16.))
                        .text_size(theme::text_meta())
                        .line_height(rems(18. / 16.))
                        .text_color(theme::muted())
                        .child(render_markdown(
                            &format!("s2-info-prompt-{}", detail.session_id),
                            &parse_blocks(prompt),
                            actions.view,
                            None,
                            theme::accent(),
                            None,
                            cx,
                        )),
                )
            })
    });

    div()
        .when(cfg!(test), |el| {
            el.debug_selector(|| "S2_CHAT_INFO_CARD".into())
        })
        .p(rems(12. / 16.))
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(header_left)
                .children(header_right),
        )
        .child(setup_summary)
        .child(div().h(px(1.)).w_full().bg(theme::border()))
        .child(metadata_rows)
        .children(prompt_section.map(|section| {
            div()
                .flex()
                .flex_col()
                .gap(rems(6. / 16.))
                .child(div().h(px(1.)).w_full().bg(theme::border()))
                .child(section)
        }))
        .into_any_element()
}

pub(super) fn account_chip(name: String) -> Div {
    div()
        .debug_selector(|| "CHAT_ACCOUNT_CHIP".into())
        .flex_none()
        .max_w(rems(160. / 16.))
        .truncate()
        .px_2()
        .rounded_sm()
        .bg(theme::raised())
        .text_size(theme::text_meta())
        .text_color(theme::muted())
        .child(name)
}

pub(super) fn side_panel_key(label: &'static str) -> AnyElement {
    div()
        .w(rems(72. / 16.))
        .flex_none()
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

pub(super) fn side_panel_value(value: String) -> AnyElement {
    div()
        .truncate()
        .flex_1()
        .min_w(px(0.))
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_color(theme::muted())
        .when(cfg!(test), |column| {
            column.debug_selector(|| format!("CHAT_PANEL_META {value}"))
        })
        .child(value)
        .into_any_element()
}

pub(super) fn side_panel_row(label: &'static str, value: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap_3()
        .text_size(theme::text_meta())
        .child(side_panel_key(label))
        .child(value)
        .into_any_element()
}

#[cfg(test)]
mod info_dedup_tests {
    use super::*;

    fn linked(top: &str, branch: &str) -> InfoBranchRow {
        InfoBranchRow {
            branch: branch.into(),
            top: Some(top.into()),
            marker: "from main".into(),
            missing: false,
        }
    }

    #[test]
    fn linked_worktree_at_root_replaces_cwd_and_branch_with_single_worktree_row() {
        let wt = linked("/repo/.worktrees/feat-1403", "feat/1403");
        let rows = resolve_info_location_rows(Some("/repo/.worktrees/feat-1403"), Some(&wt));
        assert_eq!(rows.cwd, None, "root cwd is not repeated beside worktree");
        let worktree = rows.worktree.expect("worktree row");
        assert_eq!(worktree.branch, "feat/1403");
        assert_eq!(worktree.path.as_deref(), Some("/repo/.worktrees/feat-1403"));
        assert_eq!(worktree.marker, "from main");
    }

    #[test]
    fn linked_worktree_subdirectory_shows_relative_cwd_below_worktree_root() {
        let wt = linked("/repo/.worktrees/feat-1403", "feat/1403");
        let rows = resolve_info_location_rows(
            Some("/repo/.worktrees/feat-1403/crates/runner-app"),
            Some(&wt),
        );
        assert_eq!(rows.cwd.as_deref(), Some("crates/runner-app"));
        assert_eq!(
            rows.worktree.as_ref().map(|w| w.branch.as_str()),
            Some("feat/1403")
        );
    }

    #[test]
    fn main_checkout_or_non_git_shows_cwd_and_no_worktree_row() {
        let rows = resolve_info_location_rows(Some("/repo"), None);
        assert_eq!(rows.worktree, None);
        assert_eq!(rows.cwd.as_deref(), Some("/repo"));
    }

    #[test]
    fn default_runtime_commands_are_hidden_and_custom_commands_are_shown() {
        assert!(!should_show_cmd("claude-code", "claude"));
        assert!(!should_show_cmd("codex", "codex"));
        assert!(!should_show_cmd("pi", "pi"));
        assert!(!should_show_cmd("antigravity", "agy"));
        assert!(!should_show_cmd("opencode", "opencode"));
        assert!(should_show_cmd("claude-code", "/usr/local/bin/claude-wrap"));
        assert!(should_show_cmd("codex", "codex-custom"));
        assert!(!should_show_cmd("claude-code", "   "));
    }
}

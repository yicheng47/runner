use super::*;
use runner_core::protocol::files::WorktreeKind;
use runner_core::protocol::worktree::WorktreeCheckout;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InfoBranchRow {
    pub(crate) branch: String,
    pub(crate) marker: String,
    pub(crate) missing: bool,
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
    let branch_text = if *missing {
        let label = branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                crate::surfaces::remote_client::model::host_path_leaf(&top.to_string_lossy())
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
        .text_size(theme::text_meta())
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

fn chat_panel_card() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .rounded_lg()
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::panel())
        .p(rems(14. / 16.))
}

pub(super) fn remote_chat_panel_content(
    session: SessionDisplay,
    branch: Option<InfoBranchRow>,
) -> AnyElement {
    let runtime = &session.agent_runtime;
    div()
        .flex()
        .flex_col()
        .gap(rems(10. / 16.))
        .child(side_panel_label(crate::i18n::text("ui.runtime")))
        .child(
            chat_panel_card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(12. / 16.))
                        .text_size(theme::text_title())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(runtime_mark(runtime, 40.))
                        .child(runtime_display_name(runtime)),
                )
                .children(
                    session
                        .cwd
                        .map(|cwd| side_panel_row("cwd", side_panel_value(cwd))),
                )
                .children(branch.map(chat_panel_branch)),
        )
        .into_any_element()
}

fn chat_panel_branch(branch: InfoBranchRow) -> AnyElement {
    side_panel_row(
        crate::i18n::text("ui.info_branch"),
        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHAT_PANEL_BRANCH_ROW".into())
            })
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(2. / 16.))
            .child(
                div()
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_color(if branch.missing {
                        theme::warning()
                    } else {
                        theme::muted()
                    })
                    .when(cfg!(test), |col| {
                        let text = branch.branch.clone();
                        col.debug_selector(move || format!("CHAT_PANEL_META {text}"))
                    })
                    .child(branch.branch),
            )
            .child(info_worktree_marker(branch.marker)),
    )
}

pub(super) struct ChatPanelActions {
    pub(super) open_role: Option<ClickHandler>,
    pub(super) toggle_prompt: ClickHandler,
    pub(super) expanded: bool,
    pub(super) view: gpui::EntityId,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn chat_panel_content(
    detail: &DirectSessionEntry,
    role: Option<&runner_core::protocol::model::Role>,
    width: f32,
    session_key_copy: Entity<CopyValueButton>,
    actions: ChatPanelActions,
    account: Option<(String, Option<String>, String)>,
    branch: Option<InfoBranchRow>,
    cx: &App,
) -> AnyElement {
    let column = width - 2. * 20. - 2. * 14. - 2.;
    let section_label = if role.is_some() {
        crate::i18n::text("ui.role")
    } else {
        crate::i18n::text("ui.runtime")
    };
    let cwd = detail
        .cwd
        .clone()
        .or_else(|| role.and_then(|role| role.working_dir.clone()));
    let system_prompt = role
        .and_then(|role| role.system_prompt.as_deref())
        .filter(|text| !text.trim().is_empty());
    div()
        .flex()
        .flex_col()
        .gap(rems(18. / 16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(10. / 16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(side_panel_label(section_label))
                        .children(actions.open_role.map(|on_open| {
                            let key_open = on_open.clone();
                            div()
                                .id("chat-panel-open-role")
                                .when(cfg!(test), |link| {
                                    link.debug_selector(|| "CHAT_PANEL_OPEN_ROLE".into())
                                })
                                .tab_index(0)
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(theme::text_ui())
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
                                        .size(rems(12. / 16.))
                                        .text_color(theme::muted()),
                                )
                        })),
                )
                .child(
                    chat_panel_card()
                        .child(chat_panel_setup(detail, role, column, account.as_ref()))
                        .child(div().h(rems(1. / 16.)).w_full().bg(theme::border()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(rems(6. / 16.))
                                .child(side_panel_row(
                                    "cmd",
                                    side_panel_value(detail.agent_command.clone()),
                                ))
                                .children(account.map(|(_, _, folder)| {
                                    side_panel_row("config", side_panel_value(folder))
                                }))
                                .children(
                                    cwd.map(|cwd| side_panel_row("cwd", side_panel_value(cwd))),
                                )
                                .children(branch.map(chat_panel_branch))
                                .child(side_panel_row(
                                    "session_key",
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .flex()
                                        .items_start()
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
                                )),
                        ),
                ),
        )
        .children(system_prompt.map(|prompt| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(side_panel_label(crate::i18n::text("ui.system_prompt")))
                        .child(
                            div()
                                .when(cfg!(test), |meta| {
                                    meta.debug_selector(|| "CHAT_PANEL_PROMPT_META".into())
                                })
                                .min_w(px(0.))
                                .truncate()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_caption())
                                .text_color(theme::faint())
                                .child(prompt_meta(prompt)),
                        ),
                )
                .child(
                    div()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border_strong())
                        .bg(theme::bg())
                        .p_3()
                        .text_size(theme::text_ui())
                        .line_height(rems(20. / 16.))
                        .text_color(theme::muted())
                        .child(clamped_markdown(
                            &format!("chat-panel-prompt-{}", detail.session_id),
                            prompt,
                            actions.expanded,
                            theme::bg(),
                            "CHAT_PANEL_PROMPT",
                            actions.toggle_prompt,
                            actions.view,
                            cx,
                        )),
                )
        }))
        .into_any_element()
}

pub(super) fn chat_panel_setup(
    detail: &DirectSessionEntry,
    role: Option<&runner_core::protocol::model::Role>,
    column: f32,
    account: Option<&(String, Option<String>, String)>,
) -> AnyElement {
    let identity_width = column - 40. - 12.;
    let half_column = (column - 16.) / 2.;
    let (model, model_default) = role_setting_label(detail.agent_model.as_deref());
    let (effort, effort_default) =
        crate::surfaces::roles::logic::role_effort_label(detail.agent_effort.as_deref());
    let identity = div()
        .flex()
        .items_center()
        .gap(rems(12. / 16.))
        .child(
            div()
                .flex_none()
                .size(rems(40. / 16.))
                .when(cfg!(test), |identity| {
                    identity.debug_selector(|| {
                        role.map(|role| format!("CHAT_PANEL_AVATAR {}", role.handle))
                            .unwrap_or_else(|| format!("CHAT_PANEL_MARK {}", detail.agent_runtime))
                    })
                })
                .child(if let Some(role) = role {
                    RoleAvatar::new(role.handle.clone(), 40.).into_any_element()
                } else {
                    runtime_mark(&detail.agent_runtime, 40.)
                }),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(rems(3. / 16.))
                .child(
                    column_text(
                        role.map(|role| role.display_name.clone())
                            .unwrap_or_else(|| runtime_display_name(&detail.agent_runtime)),
                        identity_width,
                    )
                    .text_size(theme::text_title())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text()),
                )
                .children(role.map(|role| {
                    column_text(format!("@{}", role.handle), identity_width)
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_ui())
                        .text_color(theme::muted())
                })),
        );
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(14. / 16.))
        .child(identity)
        .children(if let Some((name, email, _)) = account {
            Some(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        setup_row(
                            crate::i18n::text("heading.runtime"),
                            div()
                                .min_w(px(0.))
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .child(runtime_mark(&detail.agent_runtime, 12.))
                                .child(setup_value(
                                    runtime_display_name(&detail.agent_runtime),
                                    half_column - 18.,
                                    false,
                                    false,
                                ))
                                .into_any_element(),
                        )
                        .flex_1(),
                    )
                    .child(
                        setup_row(
                            crate::i18n::text("accounts.account"),
                            div()
                                .debug_selector(|| "CHAT_ACCOUNT_DETAIL".into())
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(setup_value(name.clone(), half_column, false, false))
                                .children(email.as_ref().map(|email| {
                                    column_text(email.clone(), half_column)
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                }))
                                .into_any_element(),
                        )
                        .flex_1(),
                    ),
            )
        } else {
            role.map(|_| {
                setup_row(
                    crate::i18n::text("heading.runtime"),
                    div()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .child(runtime_mark(&detail.agent_runtime, 12.))
                        .child(setup_value(
                            runtime_display_name(&detail.agent_runtime),
                            column - 18.,
                            false,
                            false,
                        ))
                        .into_any_element(),
                )
            })
        })
        .child(
            div()
                .flex()
                .gap_4()
                .child(
                    setup_row(
                        crate::i18n::text("heading.model"),
                        setup_value(model, half_column, !model_default, model_default),
                    )
                    .flex_1(),
                )
                .child(
                    setup_row(
                        crate::i18n::text("heading.effort"),
                        setup_value(effort, half_column, !effort_default, effort_default),
                    )
                    .flex_1(),
                ),
        )
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

pub(super) fn side_panel_label(label: &'static str) -> AnyElement {
    div()
        .when(cfg!(test), |label_row| {
            label_row.debug_selector(|| format!("CHAT_PANEL_LABEL {label}"))
        })
        .flex_none()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text_caption())
        .text_color(theme::faint())
        .child(label.to_uppercase())
        .into_any_element()
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

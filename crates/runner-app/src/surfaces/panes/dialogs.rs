use super::*;
use crate::surfaces::sidebar::{LinkedMissionArchiveItem, LinkedMissionIdleState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArchiveConfirmCopy {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) detail: Option<String>,
    pub(crate) confirm_label: String,
    pub(crate) icon: &'static str,
}

pub(crate) fn single_chat_with_missions_confirm_copy(
    label: &str,
    chat_running: bool,
    linked_missions: &[LinkedMissionArchiveItem],
) -> ArchiveConfirmCopy {
    let mission_count = linked_missions.len();
    let running_missions = linked_missions.iter().filter(|m| m.running).count();
    let any_running = chat_running || running_missions > 0;

    let title = if mission_count == 1 {
        crate::i18n::text("ui.archive_chat_and_mission").to_owned()
    } else {
        crate::i18n::tf(
            "format.archive_chat_and_missions",
            &[("count", mission_count.to_string())],
        )
    };

    let body = match (chat_running, running_missions) {
        (false, 0) if mission_count == 1 => crate::i18n::tf(
            "format.archiving_chat_also_archives_linked_mission",
            &[("label", label.to_owned())],
        ),
        (false, 0) => crate::i18n::tf(
            "format.archiving_chat_also_archives_linked_missions",
            &[("label", label.to_owned())],
        ),
        (true, 0) if mission_count == 1 => crate::i18n::tf(
            "format.chat_running_also_archives_linked_mission",
            &[("label", label.to_owned())],
        ),
        (true, 0) => crate::i18n::tf(
            "format.chat_running_also_archives_linked_missions",
            &[("label", label.to_owned())],
        ),
        (true, running) => crate::i18n::tf(
            "format.chat_and_linked_missions_running_stops_first",
            &[
                ("label", label.to_owned()),
                (
                    "missions",
                    crate::i18n::tn("count.linked_missions", running),
                ),
            ],
        ),
        (false, 1) if mission_count == 1 => crate::i18n::tf(
            "format.linked_mission_running_stops_first_both",
            &[("label", label.to_owned())],
        ),
        (false, 1) => crate::i18n::tf(
            "format.linked_mission_running_stops_first_all",
            &[("label", label.to_owned())],
        ),
        (false, running) => crate::i18n::tf(
            "format.linked_missions_running_stops_first",
            &[
                (
                    "missions",
                    crate::i18n::tn("count.linked_missions", running),
                ),
                ("label", label.to_owned()),
            ],
        ),
    };

    let detail = if mission_count == 1 {
        crate::i18n::text("ui.you_can_restore_both_from_settings_archived").to_owned()
    } else {
        crate::i18n::text("ui.you_can_restore_chats_and_missions_from_settings_archived").to_owned()
    };

    let confirm_label = if any_running {
        crate::i18n::text("ui.stop_and_archive_all").to_owned()
    } else if mission_count == 1 {
        crate::i18n::text("ui.archive_chat_and_mission_action").to_owned()
    } else {
        crate::i18n::tf(
            "format.archive_chat_and_missions_action",
            &[("count", mission_count.to_string())],
        )
    };

    ArchiveConfirmCopy {
        title,
        body,
        detail: Some(detail),
        confirm_label,
        icon: "archive.svg",
    }
}

pub(crate) fn tab_with_missions_confirm_copy(
    chats: usize,
    terminals: usize,
    running_chats: usize,
    linked_missions: &[LinkedMissionArchiveItem],
) -> ArchiveConfirmCopy {
    let chats_label = crate::i18n::tn("count.chats", chats);
    let missions_label = crate::i18n::tn("count.linked_missions", linked_missions.len());
    let base = if terminals > 0 {
        let terminal_action = crate::i18n::tn("format.permanently_closes_terminal", terminals);
        crate::i18n::tf(
            "format.this_archives_chats_and_linked_missions_and_terminals",
            &[
                ("chats", chats_label),
                ("missions", missions_label),
                ("terminal_action", terminal_action),
            ],
        )
    } else {
        crate::i18n::tf(
            "format.this_archives_chats_and_linked_missions",
            &[("chats", chats_label), ("missions", missions_label)],
        )
    };

    let running_missions = linked_missions.iter().filter(|m| m.running).count();
    let running_clause = match (running_chats > 0, running_missions > 0) {
        (true, true) => Some(crate::i18n::text(
            "ui.running_chats_and_missions_stopped_first",
        )),
        (true, false) => Some(crate::i18n::text("ui.running_chats_stopped_first")),
        (false, true) => Some(crate::i18n::text(
            "ui.running_linked_missions_stopped_first",
        )),
        (false, false) => None,
    };
    let body = if let Some(clause) = running_clause {
        let separator = if crate::i18n::active_locale() == crate::i18n::Locale::SimplifiedChinese {
            ""
        } else {
            " "
        };
        format!("{base}{separator}{clause}")
    } else {
        base
    };

    let detail = if terminals > 0 {
        crate::i18n::text("ui.archived_chats_and_missions_can_be_restored_closed_terminals_cannot")
            .to_owned()
    } else {
        crate::i18n::text("ui.you_can_restore_chats_and_missions_from_settings_archived").to_owned()
    };

    ArchiveConfirmCopy {
        title: crate::i18n::text("ui.archive_tab").to_owned(),
        body,
        detail: Some(detail),
        confirm_label: crate::i18n::text("ui.archive_tab_action").to_owned(),
        icon: if terminals > 0 {
            "trash.svg"
        } else {
            "archive.svg"
        },
    }
}

fn render_linked_missions_note(missions: &[LinkedMissionArchiveItem]) -> AnyElement {
    div()
        .id("confirm-linked-missions")
        .w(rems((420. - 2. * 22.) / 16.))
        .max_h(rems(192. / 16.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(rems(2. / 16.))
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .debug_selector(|| "CONFIRM_DIALOG_LINKED_MISSIONS".into())
        .children(missions.iter().map(|mission| {
            let trailing = if mission.running {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(rems(5. / 16.))
                    .child(
                        div()
                            .size(rems(6. / 16.))
                            .rounded_full()
                            .bg(theme::warning()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::warning())
                            .child(crate::i18n::text("ui.running_stops_crew")),
                    )
                    .into_any_element()
            } else {
                let state_label = match mission.idle_state {
                    LinkedMissionIdleState::Settled => crate::i18n::text("ui.settled"),
                    LinkedMissionIdleState::Ended => crate::i18n::text("ui.ended"),
                };
                div()
                    .flex_none()
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::faint())
                    .child(state_label)
                    .into_any_element()
            };
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(rems(8. / 16.))
                .px(rems(2. / 16.))
                .py(rems(5. / 16.))
                .debug_selector({
                    let id = mission.id.clone();
                    move || format!("CONFIRM_LINKED_MISSION {id}")
                })
                .child(
                    svg()
                        .path("flag.svg")
                        .size(rems(13. / 16.))
                        .flex_none()
                        .text_color(if mission.running {
                            theme::warning()
                        } else {
                            theme::faint()
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text())
                        .child(mission.title.clone()),
                )
                .child(trailing)
        }))
        .into_any_element()
}

impl NativeRoot {
    pub(crate) fn render_terminal_close_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .terminal_close_confirm
            .as_ref()
            .expect("archive confirm is open");
        let (copy, linked_missions) = match &confirm.target {
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                confirmation_body,
                ..
            } => {
                let linked_missions = self.linked_missions_for_chats(session_ids, cx);
                if linked_missions.is_empty() {
                    (
                        ArchiveConfirmCopy {
                            title: crate::i18n::text("ui.archive_tab").to_owned(),
                            body: confirmation_body.resolve().to_string(),
                            detail: None,
                            confirm_label: crate::i18n::text("ui.archive").to_owned(),
                            icon: "trash.svg",
                        },
                        linked_missions,
                    )
                } else {
                    let mut chats = Vec::new();
                    let mut terminals = 0usize;
                    let mut running_chats = 0usize;
                    for id in session_ids {
                        let entry = self.front_page_session(id, cx);
                        let is_shell = entry.as_ref().is_some_and(|session| {
                            Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell)
                        });
                        if is_shell {
                            terminals += 1;
                        } else {
                            if entry
                                .as_ref()
                                .is_some_and(|session| session.status == SessionStatus::Running)
                            {
                                running_chats += 1;
                            }
                            chats.push((id, entry));
                        }
                    }
                    let copy = if chats.len() == 1 && terminals == 0 {
                        let (_, entry) = &chats[0];
                        let label = entry
                            .as_ref()
                            .map(|entry| entry.label(None))
                            .unwrap_or_else(|| crate::i18n::text("ui.this_chat").into());
                        single_chat_with_missions_confirm_copy(
                            &label,
                            running_chats > 0,
                            &linked_missions,
                        )
                    } else {
                        tab_with_missions_confirm_copy(
                            chats.len(),
                            terminals,
                            running_chats,
                            &linked_missions,
                        )
                    };
                    (copy, linked_missions)
                }
            }
            TerminalCloseTarget::ArchiveChatPane { session_id, .. } => {
                let entry = self.front_page_session(session_id, cx);
                let label = entry
                    .as_ref()
                    .map(|entry| entry.label(None))
                    .unwrap_or_else(|| crate::i18n::text("ui.this_chat").into());
                let running = entry
                    .as_ref()
                    .is_some_and(|entry| entry.status == SessionStatus::Running);
                let linked_missions =
                    self.linked_missions_for_chats(std::slice::from_ref(session_id), cx);
                let copy = if linked_missions.is_empty() {
                    ArchiveConfirmCopy {
                        title: crate::i18n::text("ui.archive_chat").to_owned(),
                        body: archive_chat_confirm_body(&label, running),
                        detail: None,
                        confirm_label: crate::i18n::text("ui.archive_chat_action").to_owned(),
                        icon: "archive.svg",
                    }
                } else {
                    single_chat_with_missions_confirm_copy(&label, running, &linked_missions)
                };
                (copy, linked_missions)
            }
        };
        let root = cx.entity();
        let confirm_root = root.clone();
        let mut dialog = ConfirmDialog::new(
            copy.title,
            copy.body,
            copy.confirm_label,
            crate::i18n::text("ui.archiving"),
            false,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_terminal_close(window, cx));
            }),
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.cancel_terminal_close(window, cx));
            }),
        )
        .focus_handle(confirm.focus.clone())
        .icon(copy.icon);
        if !linked_missions.is_empty() {
            dialog = dialog.note(render_linked_missions_note(&linked_missions));
        }
        if let Some(detail) = copy.detail {
            dialog = dialog.detail(detail);
        }
        dialog.into_any_element()
    }

    pub(crate) fn render_fork_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self.fork_confirm.as_ref().expect("fork confirm is open");
        let label = self
            .front_page_session(&confirm.session_id, cx)
            .map(|entry| entry.label(None))
            .unwrap_or_else(|| crate::i18n::text("ui.this_chat").into());
        let body = crate::i18n::tf(
            "format.start_a_new_chat_from_with_its_full_conversation_history_the_fork_opens_in_a_n",
            &[("label", (label).to_string())],
        );
        let busy = confirm.pending;
        let root = cx.entity();
        let confirm_root = root.clone();
        ConfirmDialog::new(
            crate::i18n::text("ui.fork_chat"),
            body,
            crate::i18n::text("ui.fork"),
            crate::i18n::text("ui.forking"),
            busy,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_fork_chat(window, cx));
            }),
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| this.cancel_fork_chat(cx));
            }),
        )
        .icon("git-fork.svg")
        .variant(ButtonVariant::Primary)
        .into_any_element()
    }
}

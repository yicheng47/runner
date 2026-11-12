use crate::surfaces::chat::PaneCloseFocus;
use crate::surfaces::front_page::actions::WorkspaceAction;
use runner_core::protocol::model::Runtime;

use super::*;
use crate::*;

pub(super) fn archive_session_plan(
    session_ids: &[String],
    sessions: &[DirectSessionEntry],
    active: Option<&str>,
) -> Vec<(String, ArchiveSessionOperation)> {
    let mut plan = session_ids
        .iter()
        .map(|id| {
            let entry = sessions.iter().find(|session| session.session_id == *id);
            let operation = if entry.is_some_and(|session| {
                Runtime::parse(&session.agent_runtime).is_some_and(Runtime::is_shell)
            }) {
                ArchiveSessionOperation::CloseTerminal
            } else {
                ArchiveSessionOperation::ArchiveChat {
                    running: entry.is_some_and(|session| session.status == SessionStatus::Running),
                }
            };
            (id.clone(), operation)
        })
        .collect::<Vec<_>>();
    plan.sort_by_key(|(id, operation)| {
        (
            *operation == ArchiveSessionOperation::CloseTerminal,
            active == Some(id.as_str()),
        )
    });
    plan
}

pub(crate) fn archive_targets_for_chats(
    mut session_ids: Vec<String>,
    layouts: &[PaneLayout],
) -> Vec<String> {
    let requested = session_ids.iter().cloned().collect::<HashSet<_>>();
    for layout in layouts {
        let pane_sessions = layout.session_ids();
        if pane_sessions.is_empty()
            || !pane_sessions
                .iter()
                .all(|session_id| requested.contains(session_id))
        {
            continue;
        }
        for shell_id in layout.drawer_shells() {
            if !session_ids.contains(shell_id) {
                session_ids.push(shell_id.clone());
            }
        }
    }
    session_ids
}

pub(super) fn take_pending_pane_closes(
    pending: &mut HashMap<String, PendingPaneClose>,
    attempted: &[String],
    removed: &[String],
) -> Vec<PendingPaneClose> {
    attempted
        .iter()
        .filter_map(|session_id| {
            let close = pending.remove(session_id)?;
            removed.contains(session_id).then_some(close)
        })
        .collect()
}

pub(super) fn pane_close_after_archive<'a>(
    tabs: &'a [PaneLayout],
    pending: &PendingPaneClose,
) -> Option<&'a PaneLayout> {
    let layout = tabs.iter().find(|layout| layout.id == pending.tab_id)?;
    let leaves = layout.root.leaves();
    let emptied = leaves.len() > 1
        && leaves
            .iter()
            .any(|leaf| leaf.id == pending.pane_id && leaf.session_id.is_none());
    emptied.then_some(layout)
}

pub(crate) fn archive_all_confirmation_body(
    session_ids: &[String],
    sessions: &[DirectSessionEntry],
) -> Option<crate::i18n::Text> {
    let (chats, terminals) = archive_session_plan(session_ids, sessions, None)
        .into_iter()
        .fold(
            (0, 0),
            |(chats, terminals), (_, operation)| match operation {
                ArchiveSessionOperation::ArchiveChat { .. } => (chats + 1, terminals),
                ArchiveSessionOperation::CloseTerminal => (chats, terminals + 1),
            },
        );
    archive_confirmation_counts(chats, terminals)
}

pub(crate) fn archive_confirmation_counts(
    chats: usize,
    terminals: usize,
) -> Option<crate::i18n::Text> {
    if terminals == 0 {
        return None;
    }
    let terminal_action =
        crate::i18n::message_count("format.permanently_closes_terminal", terminals, &[]);
    if chats == 0 {
        return Some(crate::i18n::message_with(
            "format.this_closed_terminals_cannot_be_restored",
            &[("terminal_action", terminal_action.clone())],
        ));
    }
    Some(crate::i18n::Text::Catalog {
        key:
            "format.this_archives_chat_and_archived_chats_can_be_restored_from_settings_archived_c",
        count: Some(chats),
        arguments: vec![("terminal_action", terminal_action)],
    })
}

impl NativeRoot {
    pub(crate) fn archive_chat_sessions(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            let session_ids = self.remote_client.archive_targets(session_ids);
            self.request_archive_all(None, session_ids, ArchiveAllSource::Chat, window, cx);
            return;
        }

        let chat_count = session_ids.len();
        let session_ids = archive_targets_for_chats(session_ids, self.tabs.tabs());
        if session_ids.len() > chat_count {
            self.request_archive_all(None, session_ids, ArchiveAllSource::Chat, window, cx);
        } else {
            self.archive_all_sessions(session_ids, ArchiveAllSource::Chat, window, cx);
        }
    }

    pub(crate) fn archive_all_sessions(
        &mut self,
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.archive_workspace_sessions(session_ids, source, None, window, cx);
    }

    pub(crate) fn archive_workspace_sessions(
        &mut self,
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        pane_close: Option<PendingPaneClose>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session_ids
            .iter()
            .any(|id| self.launcher_session_pending(id, cx))
        {
            return;
        }
        let remote = self.front_page_client(cx).is_remote();
        let focus = self
            .front_page_layout()
            .and_then(PaneLayout::focused_session_id)
            .filter(|id| session_ids.iter().any(|session| session == id))
            .and_then(|_| PaneCloseFocus::capture(self, window, cx));
        let error_target = match source {
            ArchiveAllSource::Chat => ArchiveErrorTarget::Chat,
            ArchiveAllSource::Sidebar => ArchiveErrorTarget::App,
        };
        let local_plan = if !remote {
            if let (Some(id), Some(close)) = (session_ids.first(), pane_close.clone()) {
                self.pending_pane_closes.insert(id.clone(), close);
            }
            let active = self.active_focused_session_id();
            self.prepare_tab_close(&session_ids, cx);
            let Some(plan) = self.sidebar.update(cx, |sidebar, cx| {
                sidebar.prepare_archive_sessions(&session_ids, active.as_deref(), cx)
            }) else {
                return;
            };
            Some(plan)
        } else {
            None
        };
        let pending_ids = local_plan
            .as_ref()
            .map(|plan| plan.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let Some((action, operation)) =
            WorkspaceAction::archive(self, session_ids, local_plan, pane_close, cx)
        else {
            return;
        };
        let task = cx.background_spawn(async move { operation() });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                let Some(result) = WorkspaceAction::complete_observed(
                    root,
                    action,
                    result,
                    |done| done.completion,
                    |_, client, succeeded, cx| {
                        if client.is_remote() && succeeded {
                            if let Some(focus) = focus.clone() {
                                focus.defer_restore(window, cx);
                            }
                        }
                    },
                    cx,
                ) else {
                    return;
                };
                let (removed, error) = match result {
                    Ok(done) => (done.removed, done.error),
                    Err(error) => (Vec::new(), Some(error)),
                };
                let root = cx.entity();
                window.defer(cx, move |window, cx| {
                    root.update(cx, |root, cx| {
                        let restore_focus = focus
                            .as_ref()
                            .is_some_and(|focus| focus.is_current(root, window, cx));
                        let refreshed = root.finish_sidebar_archive(
                            pending_ids,
                            removed,
                            error,
                            error_target,
                            window,
                            cx,
                        );
                        if restore_focus && refreshed {
                            root.settle_active_tab(window, cx);
                        }
                    });
                });
            });
        })
        .detach();
    }
}

impl Sidebar {
    pub(crate) fn session_archiving(&self, session_id: &str) -> bool {
        self.archiving_sessions.contains(session_id)
    }

    fn prepare_archive_sessions(
        &mut self,
        session_ids: &[String],
        active: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<Vec<(String, ArchiveSessionOperation)>> {
        if session_ids.is_empty()
            || session_ids
                .iter()
                .any(|id| self.archiving_sessions.contains(id))
        {
            return None;
        }
        let plan = archive_session_plan(session_ids, &self.app_store.read(cx).sessions, active);
        self.archiving_sessions
            .extend(plan.iter().map(|(id, _)| id.clone()));
        self.schedule_shell_notify(cx);
        cx.notify();
        Some(plan)
    }
}

impl NativeRoot {
    fn finish_sidebar_archive(
        &mut self,
        attempted: Vec<String>,
        removed: Vec<String>,
        archive_error: Option<String>,
        error_target: ArchiveErrorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sidebar.update(cx, |sidebar, cx| {
            for id in &attempted {
                sidebar.archiving_sessions.remove(id);
            }
            cx.notify();
        });
        for session_id in &removed {
            self.attached.remove(session_id);
            self.session_exit_codes.remove(session_id);
            self.chat_transitions.remove(session_id);
        }
        let pane_closes =
            take_pending_pane_closes(&mut self.pending_pane_closes, &attempted, &removed);
        if self.remote_client.is_active() {
            self.clear_pending_tab_close(&attempted);
            cx.notify();
            return false;
        }
        let refresh_result = (|| -> Result<()> {
            self.refresh_sessions(cx);
            self.reload_tabs(cx)?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(())
        })();
        let refresh_ok = refresh_result.is_ok();
        self.clear_pending_tab_close(&attempted);
        let error = archive_error.or_else(|| {
            refresh_result
                .err()
                .map(|refresh_error| refresh_error.to_string())
        });
        match error_target {
            ArchiveErrorTarget::App => self.error = error.map(Into::into),
            ArchiveErrorTarget::Chat => self.chat_error = error.map(Into::into),
        }
        if refresh_ok {
            for pending in pane_closes {
                self.close_archived_pane(&pending, cx);
            }
        }
        cx.notify();
        refresh_ok
    }

    // The archived chat's tab may no longer be the active one, so the
    // collapse targets the layout by id and persists it directly.
    fn close_archived_pane(&mut self, pending: &PendingPaneClose, cx: &mut Context<Self>) {
        let Some(mut layout) = pane_close_after_archive(self.tabs.tabs(), pending).cloned() else {
            return;
        };
        if !layout.close_pane(&pending.pane_id) {
            return;
        }
        let result = self
            .front_page_client(cx)
            .layout(&layout, None)
            .map_err(anyhow::Error::from)
            .and_then(|()| self.reload_tabs(cx));
        if let Err(error) = result {
            self.chat_error = Some(error.to_string().into());
        }
        cx.notify();
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};

    #[test]
    fn archived_pane_launcher_cannot_steal_focus_before_the_close_completion() {
        for navigate in [false, true] {
            let mut harness =
                modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            let mut focus = None;
            let mut input_focus = None;
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                let core = root.app_store.read(cx).test_core.clone();
                let conn = core.db.get().unwrap();
                conn.execute("INSERT INTO sessions(id,status,agent_runtime,agent_command) VALUES ('closing','running','codex','codex'),('surviving','running','shell','/bin/sh')", []).unwrap();
                drop(conn);
                let events: Arc<dyn runner_daemon::session::manager::SessionEvents> = Arc::new(core.session_events());
                for id in ["closing", "surviving"] {
                    core.sessions.prepare_unlisted_terminal(id, (80, 24), &core.db, &events).unwrap();
                    root.app_store.update(cx, |store, _| store.bridge.attach(id).unwrap());
                }
                let mut layout = PaneLayout::single(Some("closing"), &[]);
                layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
                let survivor = layout.split("p1", SplitOrientation::Row).unwrap();
                layout.assign_session(&survivor, "surviving").unwrap();
                layout.focus_pane("p1");
                root.core(cx).node_tab_upsert(layout.upsert_input().unwrap()).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate(&layout.id);
                root.route = AppRoute::Chat;
                root.ensure_active_tab_attached(window, cx).unwrap();
                root.focus_active_terminal(window, cx);
                focus = PaneCloseFocus::capture(root, window, cx);
                root.pending_pane_closes.insert("closing".into(), PendingPaneClose { tab_id: layout.id, pane_id: "p1".into() });
                if navigate {
                    let find = root.attached["surviving"].terminal_find.clone();
                    find.update(cx, |find, cx| find.open(window, cx));
                }
                input_focus = window.focused(cx);
                core.db.get().unwrap().execute("UPDATE sessions SET status = 'stopped' WHERE id = 'closing'", []).unwrap();
                runner_daemon::ops::session::session_archive(&core, "closing").unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
            });
            harness.act(|root, window, cx| {
                assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
                assert_eq!(root.tabs.active().unwrap().focused_session_id(), None);
                assert!(
                    !root.launchers.is_empty(),
                    "the intermediate empty pane must render"
                );
                assert_eq!(
                    window.focused(cx),
                    input_focus,
                    "intermediate launcher stole focus"
                );
                let restore_focus = focus.as_ref().unwrap().is_current(root, window, cx);
                assert_eq!(restore_focus, !navigate);
                let refreshed = root.finish_sidebar_archive(
                    vec!["closing".into()],
                    vec!["closing".into()],
                    None,
                    ArchiveErrorTarget::Chat,
                    window,
                    cx,
                );
                assert!(refreshed);
                if restore_focus {
                    root.settle_active_tab(window, cx);
                }
                assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 1);
            });
            harness.act(|root, window, cx| {
                if navigate {
                    assert_eq!(window.focused(cx), input_focus);
                } else {
                    assert!(root.attached["surviving"].terminal_focus.is_focused(window));
                }
            });
        }
    }
}

use runner_backend::model::Runtime;

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
                Runtime::parse(&session.agent_runtime) == Some(Runtime::Shell)
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
) -> Option<String> {
    let (chats, terminals) = archive_session_plan(session_ids, sessions, None)
        .into_iter()
        .fold(
            (0, 0),
            |(chats, terminals), (_, operation)| match operation {
                ArchiveSessionOperation::ArchiveChat { .. } => (chats + 1, terminals),
                ArchiveSessionOperation::CloseTerminal => (chats, terminals + 1),
            },
        );
    if terminals == 0 {
        return None;
    }
    let terminal_action = format!(
        "permanently closes {terminals} terminal{}",
        if terminals == 1 { "" } else { "s" }
    );
    if chats == 0 {
        return Some(format!(
            "This {terminal_action}. Closed terminals cannot be restored."
        ));
    }
    Some(format!(
        "This archives {chats} chat{} and {terminal_action}. Archived chats can be restored from Settings → Archived; closed terminals cannot.",
        if chats == 1 { "" } else { "s" },
    ))
}

impl NativeRoot {
    pub(crate) fn archive_chat_sessions(
        &mut self,
        session_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let active = self.active_focused_session_id();
        let error_target = match source {
            ArchiveAllSource::Chat => ArchiveErrorTarget::Chat,
            ArchiveAllSource::Sidebar => ArchiveErrorTarget::App,
        };
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.archive_sessions(session_ids, error_target, active, window, sidebar_cx)
        });
    }
}

impl Sidebar {
    fn archive_sessions(
        &mut self,
        session_ids: Vec<String>,
        error_target: ArchiveErrorTarget,
        active: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session_ids.is_empty() {
            return;
        }
        if session_ids
            .iter()
            .any(|id| self.archiving_sessions.contains(id))
        {
            return;
        }
        let plan = archive_session_plan(
            &session_ids,
            &self.app_store.read(cx).sessions,
            active.as_deref(),
        );
        let pending_ids = plan.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        self.archiving_sessions.extend(pending_ids.iter().cloned());
        self.schedule_shell_notify(cx);
        cx.notify();

        let core = self.core(cx).clone();
        let archive_task = cx.background_spawn(async move {
            let mut removed = Vec::new();
            for (session_id, operation) in plan {
                let result = match operation {
                    ArchiveSessionOperation::CloseTerminal => {
                        runner_backend::ops::session::session_close(&core, &session_id)
                    }
                    ArchiveSessionOperation::ArchiveChat { running } => {
                        if running {
                            let _ = runner_backend::ops::session::session_kill(&core, &session_id);
                        }
                        runner_backend::ops::session::session_archive(&core, &session_id)
                    }
                };
                if let Err(error) = result {
                    return (removed, Some(error.to_string()));
                }
                removed.push(session_id);
            }
            (removed, None)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (removed, archive_error) = archive_task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                for session_id in &pending_ids {
                    this.archiving_sessions.remove(session_id);
                }
                cx.notify();
                if let Some(shell) = this.shell.upgrade() {
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, shell_cx| {
                            shell.finish_sidebar_archive(
                                pending_ids,
                                removed,
                                archive_error,
                                error_target,
                                window,
                                shell_cx,
                            )
                        });
                    });
                }
            });
        })
        .detach();
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
    ) {
        for session_id in &removed {
            self.attached.remove(session_id);
            self.session_exit_codes.remove(session_id);
            self.chat_transitions.remove(session_id);
        }
        let pane_closes =
            take_pending_pane_closes(&mut self.pending_pane_closes, &attempted, &removed);
        let refresh_result = (|| -> Result<()> {
            self.refresh_sessions(cx);
            self.reload_tabs(cx)?;
            self.ensure_active_tab_attached(window, cx)?;
            Ok(())
        })();
        let refresh_ok = refresh_result.is_ok();
        if refresh_ok {
            self.mark_active_tab_viewed(window, cx);
            self.focus_active_terminal(window, cx);
        }
        let error = archive_error.or_else(|| {
            refresh_result
                .err()
                .map(|refresh_error| refresh_error.to_string())
        });
        match error_target {
            ArchiveErrorTarget::App => self.error = error,
            ArchiveErrorTarget::Chat => self.chat_error = error,
        }
        if refresh_ok {
            for pending in pane_closes {
                self.close_archived_pane(&pending, window, cx);
            }
        }
        cx.notify();
    }

    // The archived chat's tab may no longer be the active one, so the
    // collapse targets the layout by id and persists it directly.
    fn close_archived_pane(
        &mut self,
        pending: &PendingPaneClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut layout) = pane_close_after_archive(self.tabs.tabs(), pending).cloned() else {
            return;
        };
        if self.tabs.active_tab_id() == Some(layout.id.as_str()) {
            self.close_pane(&pending.pane_id, window, cx);
            return;
        }
        if !layout.close_pane(&pending.pane_id) {
            return;
        }
        let result = layout.upsert_input().and_then(|input| {
            runner_backend::ops::node::node_tab_upsert(self.core(cx), input)?;
            self.reload_tabs(cx)
        });
        if let Err(error) = result {
            self.chat_error = Some(error.to_string());
        }
        cx.notify();
    }
}

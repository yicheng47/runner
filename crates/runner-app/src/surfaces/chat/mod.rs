//! Chat lifecycle and input: attaching sessions to terminals, tab
//! activation/focus, key/scroll/paste routing, chat start/resume, and
//! split resizing.
mod actions;
mod attachment;
mod close;
mod layout;

pub(crate) use close::PaneCloseFocus;

use super::*;
use runner_core::protocol::project::ProjectScope;

use crate::surfaces::front_page::actions::WorkspaceAction;

use crate::*;

use runner_core::protocol::model::Runtime;

#[derive(Clone)]
pub(crate) struct ForkRequest {
    pub(crate) client: crate::surfaces::front_page::FrontPageClient,
    pub(crate) source: String,
    title: Option<String>,
    size: (u16, u16),
}

pub(super) fn tab_is_terminal(
    layout: &PaneLayout,
    runtime: impl Fn(&str) -> Option<Runtime>,
) -> bool {
    let sessions = layout.session_ids();
    !sessions.is_empty()
        && sessions
            .iter()
            .all(|session_id| runtime(session_id).is_some_and(Runtime::is_shell))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PaneRenameChange {
    Unchanged,
    Persist(Option<String>),
}

pub(super) fn pane_rename_change(original: &str, value: &str) -> PaneRenameChange {
    let value = value.trim();
    if value == original.trim() {
        PaneRenameChange::Unchanged
    } else if value.is_empty() {
        PaneRenameChange::Persist(None)
    } else {
        PaneRenameChange::Persist(Some(value.to_owned()))
    }
}

fn archive_chat_pane_target(
    layout: &PaneLayout,
    pane_id: &str,
    session_id: &str,
) -> TerminalCloseTarget {
    TerminalCloseTarget::ArchiveChatPane {
        session_id: session_id.to_owned(),
        close: PendingPaneClose {
            tab_id: layout.id.clone(),
            pane_id: pane_id.to_owned(),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabCloseBehavior {
    ArchiveChat,
    CloseTerminal,
    ArchiveDrawerShells,
    DeleteEmptyTab,
}

fn tab_close_behavior(runtime: Option<&str>, has_drawer_shells: bool) -> TabCloseBehavior {
    match pane_close_behavior(runtime) {
        PaneCloseBehavior::ArchiveChat => TabCloseBehavior::ArchiveChat,
        PaneCloseBehavior::CloseTerminal => TabCloseBehavior::CloseTerminal,
        PaneCloseBehavior::LayoutOnly if has_drawer_shells => TabCloseBehavior::ArchiveDrawerShells,
        PaneCloseBehavior::LayoutOnly => TabCloseBehavior::DeleteEmptyTab,
    }
}

fn fork_confirmation(entry: Option<&DirectSessionEntry>) -> Option<ForkConfirm> {
    entry
        .filter(|entry| {
            !Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell) && entry.forkable
        })
        .map(|entry| ForkConfirm {
            session_id: entry.session_id.clone(),
            pending: false,
        })
}

fn begin_fork_submission(confirm: &mut Option<ForkConfirm>) -> Option<String> {
    let confirm = confirm.as_mut()?;
    if confirm.pending {
        return None;
    }
    confirm.pending = true;
    Some(confirm.session_id.clone())
}

fn cancel_fork_confirmation(confirm: &mut Option<ForkConfirm>) {
    if confirm.as_ref().is_some_and(|confirm| !confirm.pending) {
        *confirm = None;
    }
}

pub(crate) fn fork_in_progress(forks: &HashMap<String, String>, session_id: &str) -> bool {
    forks.contains_key(session_id) || forks.values().any(|fork_id| fork_id == session_id)
}

pub(crate) fn fork_materializing(forks: &HashMap<String, String>, session_id: &str) -> bool {
    forks.values().any(|fork_id| fork_id == session_id)
}

fn accept_fork_started(
    confirm: &mut Option<ForkConfirm>,
    forks: &mut HashMap<String, String>,
    source_session_id: &str,
    fork_session_id: &str,
) -> bool {
    if !confirm
        .as_ref()
        .is_some_and(|confirm| confirm.pending && confirm.session_id == source_session_id)
    {
        return false;
    }
    *confirm = None;
    forks.insert(source_session_id.to_owned(), fork_session_id.to_owned());
    true
}

fn finish_fork_tracking(
    forks: &mut HashMap<String, String>,
    fork_session_id: &str,
) -> Option<String> {
    let source_session_id = forks
        .iter()
        .find_map(|(source_id, fork_id)| (fork_id == fork_session_id).then(|| source_id.clone()))?;
    forks.remove(&source_session_id);
    Some(source_session_id)
}

impl NativeRoot {
    pub(crate) fn open_archived_chat(
        &mut self,
        chat: DirectSessionEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }

        self.dismiss_sidebar_transients(cx);
        let session_key = chat.agent_session_key.clone();
        self.archived_chat_detail = Some(chat);
        self.archived_session_key_copy
            .update(cx, |copy, copy_cx| copy.set_value(session_key, copy_cx));
        self.set_route(AppRoute::ArchivedChat, cx);
        self.chat_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn fork_chat(
        &mut self,
        session_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            if self.front_page_client(cx).supports(&["fork"])
                && self
                    .front_page_session(session_id, cx)
                    .is_some_and(|entry| entry.native_fork)
            {
                self.fork_confirm = Some(ForkConfirm {
                    session_id: session_id.into(),
                    pending: false,
                });
                cx.notify();
            }
            return;
        }

        if self
            .fork_confirm
            .as_ref()
            .is_some_and(|confirm| confirm.pending)
        {
            return;
        }
        if fork_in_progress(&self.forking_sessions, session_id) {
            return;
        }
        self.fork_confirm = fork_confirmation(self.session_entry(session_id, cx));
        cx.notify();
    }

    pub(crate) fn retry_agent_fork(
        &mut self,
        request: ForkRequest,
        launcher: Entity<crate::surfaces::start_chat::launcher::ChatLauncher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_agent_fork(request, Some(launcher), window, cx);
    }

    pub(crate) fn confirm_fork_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote = self.front_page_client(cx).is_remote();
        let (source_session_id, title, size) = if remote {
            let Some(confirm) = self.fork_confirm.take() else {
                return;
            };
            let size = self
                .remote_client
                .remote_terminal_size(&confirm.session_id)
                .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
            (confirm.session_id, None, (size.0.max(20), size.1.max(8)))
        } else {
            let Some(id) = begin_fork_submission(&mut self.fork_confirm) else {
                return;
            };
            let Some(entry) = self.session_entry(&id, cx) else {
                self.fork_confirm = None;
                cx.notify();
                return;
            };
            let title = Some(crate::i18n::tf(
                "format.fork",
                &[("chat", session_label(entry).to_string())],
            ));
            self.chat_error = None;
            (id, title, (INITIAL_COLS, INITIAL_ROWS))
        };
        let request = ForkRequest {
            client: self.front_page_client(cx),
            source: source_session_id,
            title,
            size,
        };
        self.submit_agent_fork(request, None, window, cx);
    }

    fn submit_agent_fork(
        &mut self,
        request: ForkRequest,
        launcher: Option<Entity<crate::surfaces::start_chat::launcher::ChatLauncher>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_session_id = request.source.clone();
        let retry_request = request.clone();
        let reveal = launcher
            .as_ref()
            .is_none_or(|launcher| self.update_launcher_visible(launcher, cx));
        cx.notify();
        let Some(operation) = self.remote_client.prepare_action(
            request.client.clone(),
            format!("session:{source_session_id}"),
            &["fork"],
            cx,
        ) else {
            return;
        };
        cx.notify();
        let core = operation.client.clone();
        let backend_source_session_id = source_session_id.clone();
        let fork = cx.background_spawn(async move {
            let result = core.fork(&backend_source_session_id, request.title, request.size);
            let gate = result
                .as_ref()
                .err()
                .and_then(|error| error.version_gate.clone());
            (result.map_err(|error| error.to_string()), gate)
        });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let (result, gate) = fork.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = WorkspaceAction::complete_observed(
                    this,
                    operation,
                    result,
                    |session| crate::surfaces::front_page::actions::Completion {
                        session: reveal.then_some(session),
                        ..Default::default()
                    },
                    |root, client, succeeded, cx| {
                        root.record_session_gate(
                            &client.target,
                            &source_session_id,
                            None,
                            crate::surfaces::agent_recovery::RecoveryAction::Resume,
                            succeeded,
                        );
                        if client.is_remote() {
                            if let Some(gate) = gate.clone() {
                                root.show_fork_update(
                                    gate,
                                    retry_request.clone(),
                                    launcher.clone(),
                                    window,
                                    cx,
                                );
                            } else if succeeded {
                                if let Some(launcher) = &launcher {
                                    root.launchers.retain(|_, cached| cached != launcher);
                                }
                            }
                        }
                    },
                    cx,
                ) else {
                    return;
                };
                if launcher.is_none() && this.remote_client.ui.epoch != origin_epoch {
                    this.forking_sessions.remove(&source_session_id);
                    cx.notify();
                    return;
                }
                if launcher.is_none() {
                    this.fork_confirm = None;
                }
                match result {
                    Ok(spawned) => {
                        let session_id = spawned;
                        finish_fork_tracking(&mut this.forking_sessions, &session_id);
                        let should_reveal = launcher.as_ref().map_or(reveal, |launcher| {
                            this.update_launcher_visible(launcher, cx)
                        });
                        if let Some(launcher) = &launcher {
                            this.launchers.retain(|_, cached| cached != launcher);
                        }
                        let placement = (|| -> Result<()> {
                            this.refresh_sessions(cx);
                            if !should_reveal {
                                this.reload_tabs(cx)?;
                                return Ok(());
                            }
                            this.reload_tabs(cx)?;
                            this.tabs.activate_session(&session_id);
                            this.sync_active_project_from_active_tab(cx);
                            this.set_route(AppRoute::Chat, cx);
                            this.ensure_active_tab_attached(window, cx)?;
                            if !this.chat_transitions.contains_key(&session_id) {
                                this.begin_chat_transition(
                                    &session_id,
                                    chat_lifecycle::TransitionKind::Starting,
                                    Some(0),
                                    window,
                                    cx,
                                );
                            }
                            this.remember_active_role(cx);
                            this.mark_active_tab_viewed(window, cx);
                            this.sync_active_chat_detail(cx);
                            Ok(())
                        })();
                        if let Err(error) = placement {
                            this.chat_error = Some(error.to_string().into());
                        }
                    }
                    Err(error) => {
                        let fork_session_id = this.forking_sessions.remove(&source_session_id);
                        if let Some(fork_session_id) = fork_session_id {
                            this.chat_transitions.remove(&fork_session_id);
                            this.attached.remove(&fork_session_id);
                        }
                        this.refresh_sessions(cx);
                        let _ = this.reload_tabs(cx);
                        if launcher.is_none()
                            || launcher
                                .as_ref()
                                .is_some_and(|launcher| this.update_launcher_visible(launcher, cx))
                        {
                            this.tabs.activate_session(&source_session_id);
                            this.sync_active_project_from_active_tab(cx);
                            let _ = this.ensure_active_tab_attached(window, cx);
                            this.sync_active_chat_detail(cx);
                        }
                        if let Some(gate) = gate {
                            this.show_fork_update(gate, retry_request, launcher, window, cx);
                        } else {
                            this.chat_error = Some(error.into());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn cancel_fork_chat(&mut self, cx: &mut Context<Self>) {
        cancel_fork_confirmation(&mut self.fork_confirm);
        cx.notify();
    }

    pub(crate) fn handle_chat_lifecycle_event(
        &mut self,
        event: runner_core::protocol::ClientEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The sidebar also refreshes exit/updated rows; repeat it here so chat-local
        // lifecycle state and row data land together regardless of subscriber order.
        let session_id = event
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if event.name == "session/input-error" {
            if let Some(address) = session_id
                .as_deref()
                .filter(|id| runner_core::remote::terminal_target(id).is_some())
            {
                if let Some(message) = event
                    .payload
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                {
                    self.remote_client
                        .report_remote_input_error(address, message.to_owned(), cx);
                }
                return;
            }
        }
        if event.name == "session/status"
            && event
                .payload
                .get("status")
                .and_then(|status| status.get("failed_since"))
                .is_some_and(|failed_since| !failed_since.is_null())
            && session_id.as_deref() == self.active_focused_session_id().as_deref()
        {
            self.mark_active_tab_viewed(window, cx);
        }
        match event.name.as_str() {
            "session/input-error" => {
                if self.route == AppRoute::Chat
                    && session_id.as_deref().is_some_and(|id| {
                        self.tabs.active().is_some_and(|layout| {
                            layout.all_session_ids().iter().any(|session| session == id)
                        })
                    })
                {
                    self.chat_error = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(|message| message.to_owned().into());
                }
            }
            "session/exit" => {
                if let Some(session_id) = session_id {
                    let exit_code = event
                        .payload
                        .get("exit_code")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|code| i32::try_from(code).ok());
                    self.session_exit_codes
                        .insert(session_id.clone(), exit_code);
                    self.chat_transitions.remove(&session_id);
                    let focus_on_completion = self.stopping_sessions.remove(&session_id)
                        || self.pending_pane_closes.contains_key(&session_id)
                        || self.sidebar.read(cx).session_archiving(&session_id);
                    self.refresh_sessions(cx);
                    self.sync_active_chat_detail(cx);
                    // A pending stop, close or archive settles focus at completion.
                    if self.route == AppRoute::Chat && !focus_on_completion {
                        if self
                            .tabs
                            .active()
                            .filter(|layout| layout.drawer_open())
                            .and_then(PaneLayout::active_drawer_shell)
                            == Some(session_id.as_str())
                        {
                            self.drawer_focus.focus(window, cx);
                        } else if self.active_focused_session_id().as_deref()
                            == Some(session_id.as_str())
                        {
                            self.chat_focus.focus(window, cx);
                            self.mark_active_tab_viewed(window, cx);
                        }
                    }
                }
            }
            "session/archived" => {
                if let Some(session_id) = session_id {
                    self.attached.remove(&session_id);
                    self.session_exit_codes.remove(&session_id);
                    self.chat_transitions.remove(&session_id);
                    self.stopping_sessions.remove(&session_id);
                    self.refresh_sessions(cx);
                    self.sync_active_chat_detail(cx);
                }
            }
            "session/spawned" => {
                if let Some(session_id) = session_id {
                    self.attached.remove(&session_id);
                    if finish_fork_tracking(&mut self.forking_sessions, &session_id).is_some() {
                        self.begin_chat_transition(
                            &session_id,
                            chat_lifecycle::TransitionKind::Starting,
                            Some(0),
                            window,
                            cx,
                        );
                    }
                    if let Some(transition) = self.chat_transitions.get_mut(&session_id) {
                        transition.baseline_seq = 0;
                    }
                    self.refresh_sessions(cx);
                    if let Some(layout) = self.tabs.active().cloned().filter(|layout| {
                        layout.all_session_ids().iter().any(|id| id == &session_id)
                    }) {
                        if let Err(error) = self.ensure_attached(&layout, &session_id, window, cx) {
                            self.chat_error = Some(error.to_string().into());
                        }
                    }
                    self.sync_active_chat_detail(cx);
                }
            }
            "session/fork-started" => {
                let source_session_id = event
                    .payload
                    .get("source_session_id")
                    .and_then(serde_json::Value::as_str);
                if let (Some(source_session_id), Some(session_id)) =
                    (source_session_id, session_id.as_deref())
                {
                    if accept_fork_started(
                        &mut self.fork_confirm,
                        &mut self.forking_sessions,
                        source_session_id,
                        session_id,
                    ) {
                        let placement = (|| -> Result<()> {
                            self.refresh_sessions(cx);
                            self.reload_tabs(cx)?;
                            self.tabs.activate_session(session_id);
                            self.sync_active_project_from_active_tab(cx);
                            self.set_route(AppRoute::Chat, cx);
                            self.remember_active_role(cx);
                            self.mark_active_tab_viewed(window, cx);
                            self.sync_active_chat_detail(cx);
                            Ok(())
                        })();
                        if let Err(error) = placement {
                            self.chat_error = Some(error.to_string().into());
                        }
                    }
                }
            }
            "session/warning" => {
                let direct = event
                    .payload
                    .get("mission_id")
                    .is_some_and(serde_json::Value::is_null);
                let visible = session_id.as_deref().is_some_and(|session_id| {
                    self.tabs.active().is_some_and(|layout| {
                        layout.session_ids().iter().any(|id| id == session_id)
                    })
                });
                if direct && visible {
                    self.chat_warning = event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
            }
            "session/updated" => {
                self.refresh_sessions(cx);
                self.sync_active_chat_detail(cx);
            }
            _ => {}
        }
    }

    fn panel_session_id(&mut self, cx: &App) -> Option<String> {
        let layout = self.front_page_layout()?;
        let tab_id = layout.id.clone();
        let focused = layout.focused_session_id().map(str::to_owned);
        let (session, chat) = self.panel_session_for(focused.as_deref(), cx)?;
        if chat {
            self.panel_chat_by_tab.insert(tab_id, session.clone());
        }
        Some(session)
    }

    /// The session the side panel follows once `focused` has focus, and
    /// whether it is that chat: a chat itself, or for a terminal the tab's
    /// last focused chat, else its first, else the terminal.
    pub(crate) fn panel_session_for(
        &self,
        focused: Option<&str>,
        cx: &App,
    ) -> Option<(String, bool)> {
        let layout = self.front_page_layout()?;
        let chats = layout
            .session_ids()
            .into_iter()
            .filter(|id| {
                !self
                    .front_page_session(id, cx)
                    .and_then(|entry| Runtime::parse(&entry.agent_runtime))
                    .is_some_and(Runtime::is_shell)
            })
            .collect::<Vec<_>>();
        if let Some(focused) = focused.filter(|id| chats.iter().any(|chat| chat.as_str() == *id)) {
            return Some((focused.to_owned(), true));
        }
        self.panel_chat_by_tab
            .get(&layout.id)
            .filter(|id| chats.contains(id))
            .or_else(|| chats.first())
            .cloned()
            .or_else(|| focused.map(str::to_owned))
            .map(|session| (session, false))
    }

    pub(crate) fn sync_active_chat_detail(&mut self, cx: &mut Context<Self>) {
        self.sync_chat_panel(cx);
        if self.remote_client.is_active() {
            return;
        }
        let selected = self.panel_session_id(cx);
        let Some(session_id) = selected else {
            return;
        };
        self.active_chat_detail = self.core(cx).session_get(&session_id).ok().flatten();
        let session_key = self
            .active_chat_detail
            .as_ref()
            .and_then(|entry| entry.agent_session_key.clone());
        self.session_key_copy
            .update(cx, |copy, copy_cx| copy.set_value(session_key, copy_cx));
    }

    pub(crate) fn ensure_active_chat_detail(&mut self, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        let active = self.panel_session_id(cx);
        let loaded = self
            .active_chat_detail
            .as_ref()
            .map(|entry| entry.session_id.as_str());
        if active.as_deref() != loaded {
            self.active_chat_detail = active
                .as_ref()
                .and_then(|id| self.app_store.read(cx).session_details.get(id))
                .cloned();
            let key = self
                .active_chat_detail
                .as_ref()
                .and_then(|detail| detail.agent_session_key.clone());
            self.session_key_copy
                .update(cx, |copy, cx| copy.set_value(key, cx));
        }
    }

    pub(crate) fn begin_chat_transition(
        &mut self,
        session_id: &str,
        kind: chat_lifecycle::TransitionKind,
        baseline_seq: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_chat_transition_generation += 1;
        let generation = self.next_chat_transition_generation;
        let baseline_seq = baseline_seq.unwrap_or_else(|| {
            self.attached
                .get(session_id)
                .map(|chat| chat.terminal.output_activity().last_seq)
                .unwrap_or(0)
        });
        self.chat_transitions.insert(
            session_id.to_owned(),
            ChatTransition {
                kind,
                started_at: Instant::now(),
                baseline_seq,
                generation,
            },
        );
        if self.route == AppRoute::Chat
            && self.active_focused_session_id().as_deref() == Some(session_id)
        {
            self.chat_focus.focus(window, cx);
        }
        let tracked_id = session_id.to_owned();
        cx.spawn_in(window, async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let done = weak
                .update_in(cx, |this, window, cx| {
                    let Some(transition) = this.chat_transitions.get(&tracked_id).copied() else {
                        return true;
                    };
                    if transition.generation != generation {
                        return true;
                    }
                    let now = Instant::now();
                    let activity = this
                        .attached
                        .get(&tracked_id)
                        .map(|chat| chat.terminal.output_activity());
                    let output_seen = activity
                        .is_some_and(|activity| activity.last_seq > transition.baseline_seq);
                    let first_paint_seen = activity
                        .is_some_and(|activity| activity.first_paint_seq > transition.baseline_seq);
                    let tui_ready_seen = activity
                        .is_some_and(|activity| activity.tui_ready_seq > transition.baseline_seq);
                    let output_idle_for = activity
                        .and_then(|activity| activity.last_output_at)
                        .map(|last| now.saturating_duration_since(last));
                    let settled = chat_lifecycle::transition_should_settle(
                        transition.kind,
                        now.saturating_duration_since(transition.started_at),
                        first_paint_seen,
                        tui_ready_seen,
                        output_seen,
                        output_idle_for,
                    );
                    if settled {
                        this.chat_transitions.remove(&tracked_id);
                        if this.remote_client.is_local() && this.route == AppRoute::Chat {
                            let active_drawer = this
                                .tabs
                                .active()
                                .filter(|layout| layout.drawer_open())
                                .and_then(PaneLayout::active_drawer_shell)
                                == Some(tracked_id.as_str());
                            if active_drawer {
                                this.focus_drawer_terminal(&tracked_id, window, cx);
                            } else if this.active_focused_session_id().as_deref()
                                == Some(tracked_id.as_str())
                                && !this.chat_surfaces.list_focus().is_focused(window)
                            {
                                this.focus_active_terminal(window, cx);
                            }
                        }
                        cx.notify();
                    }
                    settled
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        accept_fork_started, archive_chat_pane_target, begin_fork_submission,
        cancel_fork_confirmation, finish_fork_tracking, fork_confirmation, fork_in_progress,
        fork_materializing, pane_rename_change, tab_close_behavior, tab_is_terminal,
        PaneRenameChange, TabCloseBehavior,
    };
    use crate::{PendingPaneClose, TerminalCloseTarget};
    use runner_app::pane_layout::{PaneLayout, SplitOrientation};
    use runner_core::protocol::model::{Runtime, SessionStatus};
    use runner_core::protocol::session::DirectSessionEntry;
    use std::collections::HashMap;

    #[test]
    fn queued_input_errors_reach_only_the_active_chat_tab() {
        use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
        use crate::AppRoute;
        use runner_app::pane_layout::TabSet;
        use runner_core::protocol::NodeTabUpsertInput;
        use runner_terminal::terminal::TerminalMirror;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut core = None;
        harness.act(|root, _, cx| {
            let store = root.app_store.read(cx);
            let layout = PaneLayout::single(Some("missing-session"), &[]);
            let row = store
                .client
                .node_tab_upsert(NodeTabUpsertInput {
                    id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                    parent_id: None,
                    name: "Error tab".into(),
                    layout: layout.serialize().unwrap(),
                })
                .unwrap();
            core = Some(store.test_core.clone());
            root.tabs = TabSet::from_rows(&[row]);
            root.route = AppRoute::Chat;
            root.start_chat_modal = None;
        });
        let core = core.unwrap();
        let client = runner_daemon::daemon::InProcessTransport::client(core.clone());
        let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        for id in ["other-session", "missing-session"] {
            core.sessions
                .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                .unwrap();
            let mut received = client.subscribe();
            let mirror =
                TerminalMirror::attach(client.clone(), id.into(), Arc::new(|| {})).unwrap();
            mirror.write_user_bytes(b"x").unwrap();
            let event = futures::executor::block_on(received.recv()).unwrap();
            assert_eq!(event.name, "session/input-error");
            assert_eq!(event.payload["session_id"], id);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut visible = false;
                harness.act(|root, _, _| {
                    visible = root
                        .chat_error
                        .as_ref()
                        .is_some_and(|error| error.resolve().contains("session not found"));
                });
                if id == "other-session" {
                    assert!(!visible);
                    break;
                }
                if visible {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "queued error did not reach the chat surface"
                );
                std::thread::yield_now();
            }
        }
    }

    #[test]
    fn lifecycle_shortcuts_ignore_the_focused_shell() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.refresh_nodes(cx).unwrap();
        });
        let host = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "shell-shortcuts".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("busy-shell"));
            root.route = crate::AppRoute::Chat;
            root.stop_focused_session(&crate::StopFocusedSession, window, cx);
            root.resume_focused_session(&crate::ResumeFocusedSession, window, cx);
            assert!(root.stopping_sessions.is_empty());
            assert!(root.chat_transitions.is_empty());
            assert_eq!(
                root.front_page_session("busy-shell", cx).unwrap().status,
                SessionStatus::Running
            );
            root
        });
        host.read_with(&cx, |root, _| assert!(root.stopping_sessions.is_empty()))
            .unwrap();
    }

    #[test]
    fn single_pane_tab_close_dispatches_chat_terminal_and_empty_panes() {
        for (runtime, drawers, expected) in [
            (Some("codex"), false, TabCloseBehavior::ArchiveChat),
            (Some("claude"), true, TabCloseBehavior::ArchiveChat),
            (Some("shell"), false, TabCloseBehavior::CloseTerminal),
            (None, true, TabCloseBehavior::ArchiveDrawerShells),
            (None, false, TabCloseBehavior::DeleteEmptyTab),
        ] {
            assert_eq!(tab_close_behavior(runtime, drawers), expected);
        }
    }

    #[cfg(unix)]
    #[test]
    fn exited_terminal_close_lands_on_the_other_tab_unless_another_pane_session_remains() {
        use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
        use std::time::{Duration, Instant};

        for variant in ["single", "empty-sibling", "filled-sibling"] {
            let mut harness =
                modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            let mut tab_id = String::new();
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.root_focus.focus(window, cx);
                let conn = root.app_store.read(cx).test_core.db.get().unwrap();
                for id in ["exited-shell", "other-tab-shell"]
                    .into_iter()
                    .chain((variant == "filled-sibling").then_some("kept-shell"))
                {
                    conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', 'shell', '/bin/sh')", [id]).unwrap();
                }
                drop(conn);
                let mut layout = PaneLayout::single(Some("exited-shell"), &[]);
                layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
                if variant != "single" {
                    let pane = layout.split("p1", SplitOrientation::Row).unwrap();
                    if variant == "filled-sibling" {
                        layout.assign_session(&pane, "kept-shell").unwrap();
                    }
                }
                tab_id = layout.id.clone();
                root.core(cx).node_tab_upsert(layout.upsert_input().unwrap()).unwrap();
                let mut other = PaneLayout::single(Some("other-tab-shell"), &[]);
                other.id = "01ARZ3NDEKTSV4RRFFQ69G5FAX".into();
                root.core(cx).node_tab_upsert(other.upsert_input().unwrap()).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate(&tab_id);
                root.close_terminal_pane("p1", "exited-shell", window, cx);
                assert_eq!(root.pending_tab_close.is_some(), variant != "filled-sibling");
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut finished = false;
                harness.act(|root, _, cx| {
                    finished = !root.stopping_sessions.contains("exited-shell");
                    if finished {
                        assert!(root.chat_error.is_none(), "{:?}", root.chat_error);
                        root.reload_tabs(cx).unwrap();
                        assert!(root.session_entry("exited-shell", cx).is_none());
                        assert!(root.session_entry("other-tab-shell", cx).is_some());
                        if variant == "filled-sibling" {
                            let layout = root.tabs.active().unwrap();
                            assert_eq!(layout.id, tab_id);
                            assert_eq!(layout.root.leaves().len(), 1);
                            assert_eq!(layout.focused_session_id(), Some("kept-shell"));
                        } else {
                            let other = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
                            assert_eq!(root.tabs.active_tab_id(), Some(other), "{variant}");
                            assert_eq!(root.tabs.tabs().len(), 1);
                            root.reload_tabs(cx).unwrap();
                            assert_eq!(root.tabs.active_tab_id(), Some(other), "{variant}");
                            assert_eq!(
                                root.runtime_navigation_history.last(),
                                Some(&crate::RuntimeLocation::Chat("other-tab-shell".into()))
                            );
                            assert!(root.tab_close_guarded());
                        }
                    }
                });
                if finished {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "terminal close did not finish: {variant}"
                );
            }
        }
    }

    #[test]
    fn terminal_tabs_require_sessions_and_ignore_empty_panes() {
        for (sessions, expected) in [
            (vec![Some("shell")], true),
            (vec![Some("shell"), Some("shell-2")], true),
            (vec![Some("shell"), None], true),
            (vec![Some("codex")], false),
            (vec![Some("codex"), Some("shell")], false),
            (vec![None], false),
            (vec![None, None], false),
            (vec![Some("unknown")], false),
        ] {
            let mut layout = PaneLayout::single(sessions[0], &[]);
            for session in &sessions[1..] {
                let pane = layout.split("p1", SplitOrientation::Row).unwrap();
                if let Some(session) = session {
                    layout.assign_session(&pane, session).unwrap();
                }
            }
            assert_eq!(
                tab_is_terminal(&layout, |session| match session {
                    "shell-2" => Some(Runtime::Shell),
                    runtime => Runtime::parse(runtime),
                }),
                expected,
                "{sessions:?}"
            );
        }
    }

    fn direct_session(runtime: &str, forkable: bool) -> DirectSessionEntry {
        DirectSessionEntry {
            claude_account: None,
            session_id: "chat-1".into(),
            project_id: None,
            role_id: None,
            handle: Some("coder".into()),
            agent_runtime: runtime.into(),
            agent_command: runtime.into(),
            agent_model: None,
            agent_effort: None,
            display_name: runtime.into(),
            status: SessionStatus::Running,
            title: None,
            live_title: None,
            cwd: None,
            started_at: None,
            stopped_at: None,
            resumable: forkable,
            native_fork: runtime != "shell",
            forkable,
            agent_session_key: forkable.then(|| "key".into()),
            pinned: false,
            archived_at: None,
        }
    }

    #[test]
    fn archive_chat_pane_target_pins_the_requesting_tab_before_confirm() {
        let mut requesting = PaneLayout::single(Some("chat"), &["chat".into()]);
        let other = requesting.split("p1", SplitOrientation::Row).unwrap();
        requesting.assign_session(&other, "other").unwrap();
        requesting.id = "tab-a".into();
        let TerminalCloseTarget::ArchiveChatPane { session_id, close } =
            archive_chat_pane_target(&requesting, "p1", "chat")
        else {
            panic!("× on a chat pane requests an archive");
        };
        assert_eq!(session_id, "chat");
        assert_eq!(
            close,
            PendingPaneClose {
                tab_id: "tab-a".into(),
                pane_id: "p1".into(),
            }
        );
    }

    #[test]
    fn pane_rename_enter_persists_titles_and_empty_restores_the_default() {
        assert_eq!(
            pane_rename_change("@coder", "  feature 64  "),
            PaneRenameChange::Persist(Some("feature 64".into()))
        );
        assert_eq!(
            pane_rename_change("feature 64", "   "),
            PaneRenameChange::Persist(None)
        );
        assert_eq!(
            pane_rename_change("feature 64", " feature 64 "),
            PaneRenameChange::Unchanged
        );
    }

    #[test]
    fn fork_confirmation_opens_cancels_and_rejects_ineligible_sessions() {
        let entry = direct_session("codex", true);
        let mut confirm = fork_confirmation(Some(&entry));
        assert_eq!(confirm.as_ref().map(|confirm| confirm.pending), Some(false));
        cancel_fork_confirmation(&mut confirm);
        assert!(confirm.is_none());

        assert!(fork_confirmation(Some(&direct_session("codex", false))).is_none());
        assert!(fork_confirmation(Some(&direct_session("shell", true))).is_none());
        assert!(fork_confirmation(None).is_none());
    }

    #[test]
    fn fork_confirmation_allows_only_one_backend_submission() {
        let mut confirm = fork_confirmation(Some(&direct_session("claude-code", true)));
        let mut calls = 0;
        for _ in 0..2 {
            if begin_fork_submission(&mut confirm).is_some() {
                calls += 1;
            }
        }
        assert_eq!(calls, 1);
        assert!(confirm.as_ref().is_some_and(|confirm| confirm.pending));
        cancel_fork_confirmation(&mut confirm);
        assert!(confirm.is_some());
    }

    #[test]
    fn fork_started_moves_busy_state_from_confirmation_to_both_sessions() {
        let mut confirm = fork_confirmation(Some(&direct_session("codex", true)));
        assert_eq!(
            begin_fork_submission(&mut confirm).as_deref(),
            Some("chat-1")
        );
        let mut forks = HashMap::new();

        assert!(!accept_fork_started(
            &mut confirm,
            &mut forks,
            "another-chat",
            "fork-1",
        ));
        assert!(confirm.as_ref().is_some_and(|confirm| confirm.pending));
        assert!(accept_fork_started(
            &mut confirm,
            &mut forks,
            "chat-1",
            "fork-1",
        ));
        assert!(confirm.is_none());
        assert!(fork_in_progress(&forks, "chat-1"));
        assert!(fork_in_progress(&forks, "fork-1"));
        assert!(!fork_in_progress(&forks, "another-chat"));
        assert!(!fork_materializing(&forks, "chat-1"));
        assert!(fork_materializing(&forks, "fork-1"));

        assert_eq!(
            finish_fork_tracking(&mut forks, "fork-1").as_deref(),
            Some("chat-1")
        );
        assert!(forks.is_empty());
    }
}

#[cfg(test)]
mod terminal_close_tests {
    use super::*;
    use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};
    use std::sync::Arc;

    struct NoForegroundProbe(runner_daemon::daemon::InProcessTransport);
    impl Transport for NoForegroundProbe {
        fn call(&self, request: Request) -> Result<Response, ClientError> {
            assert!(
                !matches!(
                    request,
                    Request::session_shell_has_foreground_process { .. }
                ),
                "terminal close must not query foreground work or ask for confirmation"
            );
            self.0.call(request)
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            self.0.subscribe()
        }
    }

    #[test]
    fn terminal_pane_and_sidebar_tab_close_without_a_probe_or_dialog_while_chat_archive_still_asks()
    {
        for entry in ["pane", "shortcut", "sidebar"] {
            let mut harness =
                modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            harness.act(|root, _, cx| {
                root.start_chat_modal = None;
                let core = root.app_store.read(cx).test_core.clone();
                let conn = core.db.get().unwrap();
                conn.execute("INSERT INTO sessions(id,status,agent_runtime,agent_command) VALUES ('close-shell','stopped','shell','/bin/sh'),('kept-chat','running','codex','codex')", []).unwrap();
                drop(conn);
                root.app_store.update(cx, |store, _| store.client = runner_core::protocol::DaemonClient::new(Arc::new(NoForegroundProbe(runner_daemon::daemon::InProcessTransport(core)))));
                let mut layout = PaneLayout::single(Some("close-shell"), &[]);
                layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
                if entry == "pane" {
                    let second = layout.split("p1", SplitOrientation::Row).unwrap();
                    layout.assign_session(&second, "kept-chat").unwrap();
                    layout.focus_pane("p1");
                }
                root.core(cx).node_tab_upsert(layout.upsert_input().unwrap()).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate(&layout.id);
                root.set_route(AppRoute::Chat, cx);
            });
            harness.act(|root, window, cx| {
                let tab = root.tabs.active().unwrap().id.clone();
                if entry == "pane" {
                    root.request_close_terminal_pane("p1", "close-shell", window, cx);
                } else if entry == "shortcut" {
                    crate::surfaces::windowing::close_tab(root, window, cx);
                } else {
                    root.request_close_terminal_tab(&tab, "close-shell", window, cx);
                }
                assert!(root.terminal_close_confirm.is_none());
            });
            harness.act(|root, window, cx| {
                assert!(root.session_entry("close-shell", cx).is_none());
                assert!(root.terminal_close_confirm.is_none());
                if entry == "pane" {
                    let pane = root.tabs.active().unwrap().focused_pane_id.clone();
                    root.request_close_chat_pane(&pane, "kept-chat", window, cx);
                    assert!(matches!(
                        root.terminal_close_confirm
                            .as_ref()
                            .map(|confirm| &confirm.target),
                        Some(TerminalCloseTarget::ArchiveChatPane { .. })
                    ));
                }
            });
        }
    }
}

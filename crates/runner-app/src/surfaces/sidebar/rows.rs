use super::*;
use crate::surfaces::sidebar_logic::AttentionState;
use crate::*;
use runner_app::ui::TextField;
use runner_backend::model::Runtime;
use runner_backend::repo::node::NodeType;

impl Sidebar {
    pub(super) fn dismiss_transients(&mut self, cx: &mut Context<Self>) {
        self.create_menu
            .update(cx, |menu, menu_cx| menu.close(menu_cx));
        let had_context_menu = self.context_menu.take().is_some();
        self.rename = None;
        self._rename_focus_subscription = None;
        self.clear_sidebar_drag("dismiss", cx);
        if had_context_menu {
            self.schedule_shell_notify(cx);
        }
        cx.notify();
    }

    pub(super) fn set_active_project(
        &mut self,
        project_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.active_project_id = project_id;
        cx.notify();
    }

    pub(super) fn set_mission_archiving(
        &mut self,
        mission_id: String,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = if archiving {
            self.archiving_missions.insert(mission_id)
        } else {
            self.archiving_missions.remove(&mission_id)
        };
        if changed {
            cx.notify();
        }
    }

    pub(super) fn open_project_modal(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.open_project_modal(window, shell_cx)
            });
        });
    }

    pub(super) fn open_mission(
        &self,
        mission_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.open_mission(mission_id, window, shell_cx)
            });
        });
    }

    pub(super) fn resolved_sidebar_rows(&self, cx: &App) -> Vec<SidebarRow> {
        let Some(shell) = self.shell.upgrade() else {
            return Vec::new();
        };
        let shell = shell.read(cx);
        self.resolved_sidebar_rows_for_tabs(&shell.tabs, cx)
    }

    pub(super) fn resolved_sidebar_rows_for_tabs(
        &self,
        tabs: &TabSet,
        cx: &App,
    ) -> Vec<SidebarRow> {
        let layouts = tabs
            .tabs()
            .iter()
            .map(|layout| (layout.id.as_str(), layout))
            .collect::<HashMap<_, _>>();
        let sessions = self
            .app_store
            .read(cx)
            .sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();
        let missions = self
            .app_store
            .read(cx)
            .missions
            .iter()
            .map(|summary| (summary.mission.id.as_str(), summary))
            .collect::<HashMap<_, _>>();
        self.app_store
            .read(cx)
            .nodes
            .iter()
            .filter_map(|node| match node.node_type {
                NodeType::Tab => {
                    let layout = layouts.get(node.id.as_str())?;
                    let members = layout
                        .session_ids()
                        .into_iter()
                        .filter_map(|id| sessions.get(id.as_str()).map(|row| (*row).clone()))
                        .collect::<Vec<_>>();
                    if members.is_empty() {
                        return None;
                    }
                    let rollup = self.tab_status_rollup(&members, cx);
                    let attention = status_attention(rollup.priority_value()).max(
                        if members
                            .iter()
                            .any(|member| self.archiving_sessions.contains(&member.session_id))
                        {
                            AttentionState::Working
                        } else {
                            AttentionState::None
                        },
                    );
                    Some(SidebarRow::Tab {
                        node: node.clone(),
                        layout: (*layout).clone(),
                        members,
                        attention,
                    })
                }
                NodeType::Mission => {
                    let summary = missions.get(node.ref_id.as_deref()?)?;
                    let rollup = self.mission_status_rollup(summary);
                    let attention = status_attention(rollup.priority_value()).max(
                        if self.archiving_missions.contains(&summary.mission.id) {
                            AttentionState::Working
                        } else {
                            AttentionState::None
                        },
                    );
                    Some(SidebarRow::Mission {
                        node: node.clone(),
                        summary: (*summary).clone(),
                        attention,
                    })
                }
                NodeType::Project => None,
            })
            .collect()
    }

    pub(super) fn tab_status_rollup(
        &self,
        members: &[DirectSessionEntry],
        cx: &App,
    ) -> runner_app::ui::agent_status::StatusRollup {
        use runner_backend::session::status::{AgentStatus, Lifecycle};
        let store = self.app_store.read(cx);
        let entries = members
            .iter()
            .filter(|member| Runtime::parse(&member.agent_runtime) != Some(Runtime::Shell))
            .map(|member| {
                let mut status = store
                    .session_statuses
                    .get(&member.session_id)
                    .cloned()
                    .unwrap_or(AgentStatus {
                        lifecycle: Lifecycle::Running,
                        ..Default::default()
                    });
                match member.status {
                    SessionStatus::Running => {}
                    SessionStatus::Stopped => status.lifecycle = Lifecycle::Stopped,
                    SessionStatus::Crashed => status.lifecycle = Lifecycle::Error,
                }
                if self.archiving_sessions.contains(&member.session_id) {
                    mark_archiving_status(&mut status);
                }
                (member.session_id.clone(), status)
            })
            .collect();
        runner_app::ui::agent_status::StatusRollup { entries }
    }

    pub(super) fn mission_status_rollup(
        &self,
        summary: &runner_backend::ops::mission::MissionSummary,
    ) -> runner_app::ui::agent_status::StatusRollup {
        let mut entries = summary.session_statuses.clone();
        if self.archiving_missions.contains(&summary.mission.id) {
            for (_, status) in &mut entries {
                mark_archiving_status(status);
            }
        }
        runner_app::ui::agent_status::StatusRollup { entries }
    }

    pub(super) fn focus_status_target(
        &mut self,
        node_id: &str,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.resolved_sidebar_rows(cx);
        let Some(row) = rows.iter().find(|row| row.node().id == node_id) else {
            return;
        };
        let project = row
            .node()
            .parent_id
            .as_deref()
            .and_then(|id| {
                self.app_store
                    .read(cx)
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
            })
            .and_then(|node| node.ref_id.clone());
        self.update_app_settings(cx, true, move |settings| {
            settings.sidebar_projects_open = true;
            settings.sidebar_chats_open = true;
            if let Some(project) = &project {
                settings.sidebar_collapsed_projects.remove(project);
            }
            true
        });
        match row {
            SidebarRow::Tab { node, .. } => {
                self.activate_sidebar_session(&node.id, &target, window, cx)
            }
            SidebarRow::Mission { summary, .. } => {
                let mission_id = summary.mission.id.clone();
                let session_id = target.clone();
                self.update_app_settings(cx, true, move |settings| {
                    settings
                        .last_mission_terminal_ids
                        .insert(mission_id.clone(), session_id.clone());
                    true
                });
                if let Some(shell) = self.shell.upgrade() {
                    let mission_id = summary.mission.id.clone();
                    window.defer(cx, move |window, cx| {
                        shell.update(cx, |shell, cx| {
                            shell.open_mission(mission_id.clone(), window, cx);
                            shell.mission_workspace.update(cx, |workspace, cx| {
                                workspace.focus_status_session(&mission_id, &target, window, cx);
                            });
                        });
                    });
                }
            }
        }
    }

    pub(super) fn render_status_attention<'a>(
        &self,
        rows: impl IntoIterator<Item = &'a SidebarRow>,
        id: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<_> = rows.into_iter().collect();
        let rollup = runner_app::ui::agent_status::StatusRollup {
            entries: rows
                .iter()
                .flat_map(|row| match row {
                    SidebarRow::Tab { members, .. } => self.tab_status_rollup(members, cx).entries,
                    SidebarRow::Mission { summary, .. } => {
                        self.mission_status_rollup(summary).entries
                    }
                })
                .collect(),
        };
        let node_id = rollup
            .target()
            .and_then(|target| {
                rows.iter().find(|row| match row {
                    SidebarRow::Tab { members, .. } => {
                        members.iter().any(|member| member.session_id == target)
                    }
                    SidebarRow::Mission { summary, .. } => {
                        summary.session_statuses.iter().any(|(id, _)| id == target)
                    }
                })
            })
            .map(|row| row.node().id.clone());
        let archiving = rows.iter().any(|row| match row {
            SidebarRow::Tab { members, .. } => members
                .iter()
                .any(|member| self.archiving_sessions.contains(&member.session_id)),
            SidebarRow::Mission { summary, .. } => {
                self.archiving_missions.contains(&summary.mission.id)
            }
        });
        self.render_rollup_attention(rollup, node_id, archiving, false, id, cx)
    }

    pub(super) fn render_rollup_attention(
        &self,
        rollup: runner_app::ui::agent_status::StatusRollup,
        node_id: Option<String>,
        archiving: bool,
        pane_tooltip: bool,
        id: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = node_id.zip(rollup.target().map(str::to_owned));
        let indicator = if archiving && rollup.priority_value() < 3 {
            runner_app::ui::Tooltip::new(
                SharedString::from(format!("{id}-archiving")),
                "Archiving",
                super::elements::attention_indicator(AttentionState::Working),
            )
            .into_any_element()
        } else {
            if pane_tooltip {
                rollup.render_pane(id.clone())
            } else {
                rollup.render(id.clone())
            }
        };
        div()
            .id(id.clone())
            .flex_none()
            .cursor_pointer()
            .child(indicator)
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if let Some((node_id, session_id)) = &target {
                    this.focus_status_target(node_id, session_id.clone(), window, cx);
                }
            }))
            .into_any_element()
    }

    pub(super) fn scope_rows(
        &self,
        rows: &[SidebarRow],
        parent_id: Option<&str>,
    ) -> Vec<SidebarRow> {
        rows.iter()
            .filter(|row| {
                row.node().pinned_position.is_none() && row.node().parent_id.as_deref() == parent_id
            })
            .cloned()
            .collect()
    }

    pub(super) fn toggle_project(&mut self, project_id: &str, cx: &mut Context<Self>) {
        self.active_project_id = Some(project_id.to_owned());
        let project_id = project_id.to_owned();
        self.update_app_settings(cx, true, move |settings| {
            if !settings.sidebar_collapsed_projects.remove(&project_id) {
                settings.sidebar_collapsed_projects.insert(project_id);
            }
            true
        });
        cx.notify();
    }

    pub(super) fn begin_sidebar_rename(
        &mut self,
        target: SidebarRenameTarget,
        value: String,
        placeholder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), value, placeholder, false)
                .text_size(theme::text_body())
        });
        input.update(cx, |field, input_cx| {
            field.set_bare(true, input_cx);
            field.set_right_padding(0., input_cx);
            field.select_all(input_cx);
        });
        let focus = input.read(cx).focus_handle();
        self._rename_focus_subscription =
            Some(cx.on_focus_out(&focus, window, |this, _, window, cx| {
                this.submit_sidebar_rename(window, cx);
            }));
        self.rename = Some(SidebarRename { target, input });
        focus.focus(window);
        cx.notify();
    }

    pub(super) fn submit_sidebar_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        self._rename_focus_subscription = None;
        let next = rename.input.read(cx).text().trim().to_owned();
        let result = match rename.target {
            SidebarRenameTarget::Tab {
                node_id,
                original,
                session_id,
            } => {
                if next == original.trim() {
                    Ok(())
                } else if let Some(session_id) = session_id {
                    runner_backend::ops::session::session_rename(
                        self.core(cx),
                        &session_id,
                        Some(next),
                    )
                } else {
                    runner_backend::ops::node::node_rename(self.core(cx), node_id, next).map(drop)
                }
            }
            SidebarRenameTarget::Project {
                project_id,
                original,
            } => {
                if next.is_empty() || next == original.trim() {
                    Ok(())
                } else {
                    runner_backend::ops::project::project_rename(self.core(cx), project_id, next)
                        .map(drop)
                }
            }
            SidebarRenameTarget::Mission {
                mission_id,
                original,
            } => {
                if next.is_empty() || next == original.trim() {
                    Ok(())
                } else {
                    futures::executor::block_on(runner_backend::ops::mission::mission_rename_impl(
                        self.core(cx),
                        mission_id,
                        next,
                    ))
                    .map(drop)
                }
            }
        };
        match result {
            Ok(()) => self.refresh_store(StoreRefreshKind::All, cx),
            Err(error) => self.report_error(error.to_string(), cx),
        }
        self.focus_shell_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_sidebar_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename = None;
        self._rename_focus_subscription = None;
        self.focus_shell_terminal(window, cx);
        cx.notify();
    }
}

fn status_attention(priority: u8) -> AttentionState {
    match priority {
        5 => AttentionState::Error,
        4 => AttentionState::NeedsYou,
        3 => AttentionState::Working,
        2 => AttentionState::Unread,
        1 => AttentionState::Unavailable,
        _ => AttentionState::None,
    }
}

fn mark_archiving_status(status: &mut runner_backend::session::status::AgentStatus) {
    use runner_backend::session::status::{Activity, Lifecycle, ObservationSource};
    if runner_app::ui::agent_status::StatusRollup::priority(status) < 3 {
        status.lifecycle = Lifecycle::Running;
        status.observation.activity = Activity::Working;
        status.observation.source = ObservationSource::Unavailable;
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use runner_app::ui::agent_status::StatusRollup;
    use runner_backend::session::status::{AgentStatus, HumanInteraction, Lifecycle, WaitReason};

    #[test]
    fn archive_feedback_preserves_higher_attention_and_unread() {
        let mut status = AgentStatus {
            lifecycle: Lifecycle::Stopped,
            unread_since: Some(1),
            ..Default::default()
        };
        mark_archiving_status(&mut status);
        assert_eq!(StatusRollup::priority(&status), 3);
        assert_eq!(status.unread_since, Some(1));
        status.observation.interactions.push(HumanInteraction {
            id: "wait".into(),
            reason: WaitReason::Approval,
            owners: vec![],
            since: 1,
        });
        mark_archiving_status(&mut status);
        assert_eq!(StatusRollup::priority(&status), 4);
        status.lifecycle = Lifecycle::Error;
        status.error_since = Some(1);
        mark_archiving_status(&mut status);
        assert_eq!(status.lifecycle, Lifecycle::Error);
        assert_eq!(StatusRollup::priority(&status), 5);
    }
}

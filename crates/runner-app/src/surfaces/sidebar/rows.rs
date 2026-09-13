use super::*;
use crate::surfaces::sidebar_logic::{
    direct_tab_attention_state, mission_attention_state, AttentionState,
};
use crate::*;
use runner_app::ui::TextField;
use runner_backend::ops::mission::MissionActivityState;
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
                    let attention = direct_tab_attention_state(
                        members.iter().map(|member| {
                            let running_busy = self.archiving_sessions.contains(&member.session_id)
                                || (member.status == SessionStatus::Running
                                    && self
                                        .app_store
                                        .read(cx)
                                        .session_activity
                                        .get(&member.session_id)
                                        == Some(&SessionActivityState::Busy));
                            (member.agent_runtime.as_str(), running_busy)
                        }),
                        node.last_completed_at.as_deref(),
                        node.last_viewed_at.as_deref(),
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
                    let idle = summary.activity == Some(MissionActivityState::Idle);
                    let attention = if self.archiving_missions.contains(&summary.mission.id) {
                        AttentionState::Working
                    } else {
                        mission_attention_state(summary.any_session_live, idle)
                    };
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

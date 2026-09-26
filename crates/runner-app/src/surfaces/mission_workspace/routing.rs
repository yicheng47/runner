use gpui::prelude::*;
use gpui::{App, Window};

use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn set_route(&mut self, route: AppRoute, cx: &mut Context<Self>) {
        let route_changed = self.route != route;
        let leaving_mission =
            matches!(self.route, AppRoute::Mission(_)) && !matches!(route, AppRoute::Mission(_));
        if !matches!(route, AppRoute::ArchivedChat | AppRoute::Settings) {
            self.archived_chat_detail = None;
        }
        self.drop_role_edit_for_route(&route);
        self.route = route;
        if route_changed {
            self.dismissed_duplicate_chats.clear();
        }
        if self.route != AppRoute::Chat {
            self.attached.clear();
        }
        self.report_current_subjects(cx);
        self.record_current_runtime_location();
        if route_changed {
            let sidebar = self.sidebar.clone();
            cx.defer(move |cx| {
                sidebar.update(cx, |_, sidebar_cx| sidebar_cx.notify());
            });
        }
        if leaving_mission {
            let workspace = self.mission_workspace.clone();
            cx.defer(move |cx| {
                workspace.update(cx, |workspace, workspace_cx| {
                    let still_active = workspace.shell.upgrade().is_some_and(|shell| {
                        matches!(
                            &shell.read(workspace_cx).route,
                            AppRoute::Mission(active)
                                if Some(active.as_str()) == workspace.mission_id.as_deref()
                        )
                    });
                    if !still_active {
                        workspace.active = false;
                        workspace.attached.clear();
                        workspace_cx.notify();
                    }
                });
            });
        }
    }

    pub(crate) fn record_current_runtime_location(&mut self) {
        let location = match &self.route {
            AppRoute::Chat => self.active_focused_session_id().map(RuntimeLocation::Chat),
            AppRoute::Mission(mission_id) => Some(RuntimeLocation::Mission(mission_id.clone())),
            _ => None,
        };
        let Some(location) = location else {
            return;
        };
        if self
            .runtime_navigation_index
            .and_then(|index| self.runtime_navigation_history.get(index))
            == Some(&location)
        {
            return;
        }
        let keep = self.runtime_navigation_index.map_or(0, |index| index + 1);
        self.runtime_navigation_history.truncate(keep);
        self.runtime_navigation_history.push(location);
        if self.runtime_navigation_history.len() > RUNTIME_NAVIGATION_HISTORY_LIMIT {
            let excess = self.runtime_navigation_history.len() - RUNTIME_NAVIGATION_HISTORY_LIMIT;
            self.runtime_navigation_history.drain(..excess);
        }
        self.runtime_navigation_index = self.runtime_navigation_history.len().checked_sub(1);
    }

    pub(crate) fn navigate_runtime_page(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.route == AppRoute::Settings || self.runtime_navigation_history.len() < 2 {
            return;
        }
        let Some(index) = self.runtime_navigation_index else {
            return;
        };
        let mut candidate = index as isize + direction;
        while candidate >= 0 && (candidate as usize) < self.runtime_navigation_history.len() {
            let next_index = candidate as usize;
            let location = self.runtime_navigation_history[next_index].clone();
            let available = match &location {
                RuntimeLocation::Chat(session_id) => self
                    .app_store
                    .read(cx)
                    .sessions
                    .iter()
                    .any(|session| &session.session_id == session_id),
                RuntimeLocation::Mission(mission_id) => self
                    .app_store
                    .read(cx)
                    .missions
                    .iter()
                    .any(|mission| &mission.mission.id == mission_id),
            };
            if available {
                self.runtime_navigation_index = Some(next_index);
                let navigated = match location {
                    RuntimeLocation::Chat(session_id) => {
                        self.open_chat_session(&session_id, window, cx)
                    }
                    RuntimeLocation::Mission(mission_id) => {
                        self.open_mission(mission_id, window, cx);
                        true
                    }
                };
                if navigated {
                    return;
                }
                self.runtime_navigation_index = Some(index);
            }
            candidate += direction;
        }
    }

    pub(crate) fn open_mission(
        &mut self,
        mission_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&self.route, AppRoute::Mission(active) if active == &mission_id) {
            let workspace = self.mission_workspace.read(cx);
            if workspace.loading || workspace.mission.is_some() {
                return;
            }
        }
        self.dismiss_sidebar_transients(cx);
        self.set_route(AppRoute::Mission(mission_id.clone()), cx);
        let workspace = self.mission_workspace.clone();
        workspace.update(cx, |workspace, workspace_cx| {
            workspace.open_mission(mission_id, window, workspace_cx)
        });
        cx.notify();
    }

    pub(crate) fn estimated_mission_terminal_size(&self, window: &Window, cx: &App) -> (u16, u16) {
        self.mission_workspace
            .read(cx)
            .estimated_mission_terminal_size(window, cx)
    }

    pub(crate) fn sync_mission_subject_ownership(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = matches!(self.route, AppRoute::Mission(_));
        let workspace = self.mission_workspace.clone();
        workspace.update(cx, |workspace, workspace_cx| {
            workspace.sync_mission_subject_ownership(active, window, workspace_cx)
        });
    }
}

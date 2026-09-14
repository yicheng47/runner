use super::*;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn tab_label(&self, layout: &PaneLayout, cx: &App) -> String {
        if let Some(name) = &layout.name {
            return name.clone();
        }
        let labels = layout
            .session_ids()
            .into_iter()
            .filter_map(|session_id| self.session_entry(&session_id, cx))
            .map(session_label)
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "Empty tab".into()
        } else {
            labels.join(" + ")
        }
    }

    pub(crate) fn prune_sidebar_collapse_state(&mut self, cx: &mut Context<Self>) {
        let project_ids = self
            .app_store
            .read(cx)
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(
                |settings| {
                    let previous = settings.sidebar_collapsed_projects.len();
                    settings
                        .sidebar_collapsed_projects
                        .retain(|id| project_ids.contains(id));
                    previous != settings.sidebar_collapsed_projects.len()
                },
                true,
                store_cx,
            );
        });
    }

    pub(crate) fn prune_store_dependent_window_state(&mut self, cx: &mut Context<Self>) {
        let sessions = &self.app_store.read(cx).sessions;
        let visible = sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.attached.retain(|id, _| visible.contains(id));
        let direct_visible = sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.pane_action_menus
            .retain(|id, _| direct_visible.contains(id.as_str()));
        self.session_exit_codes
            .retain(|id, _| direct_visible.contains(id.as_str()));
    }

    pub(crate) fn dismiss_sidebar_transients(&mut self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.dismiss_transients(sidebar_cx)
        });
        self.project_modal = None;
        self._project_cwd_subscription = None;
        self.project_delete_confirm = None;
        self.project_delete_busy = false;
        cx.notify();
    }

    pub(crate) fn sync_sidebar_window_activation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() {
            self.sidebar.update(cx, |sidebar, sidebar_cx| {
                sidebar.clear_shortcut_pills(sidebar_cx)
            });
        }
        self.sync_window_activation(window, cx);
    }

    pub(crate) fn sync_sidebar_shortcut_rows(&mut self, cx: &mut Context<Self>) {
        let tabs = &self.tabs;
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.refresh_shortcut_rows(tabs, sidebar_cx)
        });
    }

    pub(crate) fn mark_active_tab_viewed(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Chat {
            self.report_current_subjects(cx);
            return;
        }
        let Some(layout) = self.tabs.active() else {
            self.report_current_subjects(cx);
            return;
        };
        let tab_id = layout.id.clone();
        let member_ids = layout.session_ids();
        let viewed_session_id = layout.focused_session_id().map(str::to_owned);
        if !window.is_window_active() {
            self.report_current_subjects(cx);
            runner_backend::ops::window::mark_blurred(self.core(cx), &self.window_label);
            return;
        }
        match runner_backend::ops::node::node_mark_viewed(
            self.core(cx),
            &self.window_label,
            &tab_id,
            member_ids,
            viewed_session_id.as_deref(),
        ) {
            Ok(updated) => {
                self.app_store
                    .update(cx, |store, store_cx| store.replace_node(updated, store_cx));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(crate) fn sync_active_project_from_active_tab(&mut self, cx: &mut Context<Self>) {
        let active_project_id = self.tabs.active_tab_id().and_then(|tab_id| {
            let node = self
                .app_store
                .read(cx)
                .nodes
                .iter()
                .find(|node| node.id == tab_id)?;
            node_project_id(&self.app_store.read(cx).nodes, node)
        });
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.set_active_project(active_project_id, sidebar_cx)
        });
    }

    pub(crate) fn active_project_id(&self, cx: &App) -> Option<String> {
        self.sidebar.read(cx).active_project_id.clone()
    }

    pub(crate) fn sidebar_archiving_session(&self, session_id: &str, cx: &App) -> bool {
        self.sidebar
            .read(cx)
            .archiving_sessions
            .contains(session_id)
    }

    pub(crate) fn set_sidebar_mission_archiving(
        &mut self,
        mission_id: String,
        archiving: bool,
        cx: &mut Context<Self>,
    ) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.set_mission_archiving(mission_id, archiving, sidebar_cx)
        });
    }

    pub(crate) fn clear_sidebar_drag(&mut self, path: &'static str, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.clear_sidebar_drag(path, sidebar_cx)
        });
    }

    pub(super) fn activate_sidebar_session(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.activate(tab_id) {
            return;
        }
        if let Some(layout) = self.tabs.active_mut() {
            layout.focus_session(session_id);
        }
        self.set_route(AppRoute::Chat, cx);
        self.activate_tab(tab_id, window, cx);
    }

    pub(crate) fn open_chat_session(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let tab_id = self
            .tabs
            .tabs()
            .iter()
            .find(|layout| layout.session_ids().iter().any(|id| id == session_id))
            .map(|layout| layout.id.clone());
        let Some(tab_id) = tab_id else {
            let drawer_tab_id = self
                .tabs
                .tabs()
                .iter()
                .find(|layout| layout.drawer_shells().iter().any(|id| id == session_id))
                .map(|layout| layout.id.clone());
            let Some(tab_id) = drawer_tab_id else {
                return false;
            };
            if !self.tabs.activate(&tab_id) {
                return false;
            }
            if let Some(layout) = self.tabs.active_mut() {
                layout.activate_drawer_shell(session_id);
            }
            self.sync_active_project_from_active_tab(cx);
            self.set_route(AppRoute::Chat, cx);
            if let Err(error) = self
                .persist_active_tab(cx)
                .and_then(|_| self.ensure_active_tab_attached(window, cx))
            {
                self.chat_error = Some(error.to_string());
            } else {
                self.focus_drawer_terminal(session_id, window, cx);
            }
            cx.notify();
            return true;
        };
        self.activate_sidebar_session(&tab_id, session_id, window, cx);
        true
    }
}

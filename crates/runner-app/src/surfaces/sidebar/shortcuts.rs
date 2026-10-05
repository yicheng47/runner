use super::elements::command_held_alone;
use super::elements::other_modifiers_held;

use super::*;
use crate::surfaces::sidebar_logic::{
    activation_target_for_index, numbered_sidebar_rows, should_show_shortcut_pills,
    visible_sidebar_walk, SidebarActivationTarget, SidebarShortcutProject,
    SHORTCUT_PILL_REVEAL_DELAY,
};
use crate::*;
use runner_core::protocol::node::{NodeRow, NodeType};

impl Sidebar {
    fn shortcut_row_walk(
        &self,
        rows: &[(NodeRow, SidebarShortcutRow)],
        include_hidden: bool,
        cx: &App,
    ) -> Vec<crate::surfaces::sidebar_logic::VisibleSidebarRow> {
        let store = self.app_store.read(cx);
        let mut pinned = rows
            .iter()
            .filter(|(node, _)| node.pinned_position.is_some())
            .collect::<Vec<_>>();
        pinned.sort_by_key(|(node, _)| node.pinned_position);
        let project_ids = store
            .projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<HashSet<_>>();
        let projects = store
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id.is_none()
                    && node.pinned_position.is_none()
                    && node.node_type == NodeType::Project
            })
            .filter_map(|node| {
                let project_id = node
                    .ref_id
                    .as_deref()
                    .filter(|project_id| project_ids.contains(project_id))?;
                Some(SidebarShortcutProject {
                    node_id: node.id.clone(),
                    expanded: include_hidden
                        || !store
                            .settings
                            .sidebar_collapsed_projects
                            .contains(project_id),
                    children: rows
                        .iter()
                        .filter(|(child, _)| {
                            child.pinned_position.is_none()
                                && child.parent_id.as_deref() == Some(node.id.as_str())
                        })
                        .map(|(_, row)| row.clone())
                        .collect(),
                })
            })
            .collect();
        let recent = rows
            .iter()
            .filter(|(node, _)| node.pinned_position.is_none() && node.parent_id.is_none())
            .map(|(_, row)| row.clone())
            .collect();
        visible_sidebar_walk(
            pinned.into_iter().map(|(_, row)| row.clone()).collect(),
            include_hidden || store.settings.sidebar_projects_open,
            projects,
            include_hidden || store.settings.sidebar_chats_open,
            recent,
        )
    }

    pub(crate) fn tab_close_neighbour(
        &self,
        tabs: &TabSet,
        tab_id: &str,
        cx: &App,
    ) -> Option<String> {
        let resolved = self.resolved_sidebar_rows_for_tabs(tabs, cx);
        let store = self.app_store.read(cx);
        let rows = store
            .nodes
            .iter()
            .filter_map(|node| {
                let row = if let Some(row) = resolved.iter().find(|row| row.node().id == node.id) {
                    row.shortcut_row(&store.nodes)
                } else if node.id == tab_id {
                    SidebarShortcutRow {
                        node_id: node.id.clone(),
                        target: SidebarActivationTarget::Tab {
                            tab_id: node.id.clone(),
                            session_id: String::new(),
                        },
                    }
                } else {
                    return None;
                };
                Some((node.clone(), row))
            })
            .collect::<Vec<_>>();
        let mut walk = self.shortcut_row_walk(&rows, false, cx);
        if !walk.iter().any(|row| {
            matches!(row,
                crate::surfaces::sidebar_logic::VisibleSidebarRow::Activatable(row)
                    if row.node_id == tab_id
            )
        }) {
            walk = self.shortcut_row_walk(&rows, true, cx);
        }
        crate::surfaces::sidebar_logic::neighbour_tab_id(walk, tab_id)
    }

    pub(crate) fn refresh_shortcut_rows(&mut self, tabs: &TabSet, cx: &App) {
        let rows = self
            .resolved_sidebar_rows_for_tabs(tabs, cx)
            .iter()
            .map(|row| {
                (
                    row.node().clone(),
                    row.shortcut_row(&self.app_store.read(cx).nodes),
                )
            })
            .collect::<Vec<_>>();
        let numbered_shortcut_rows =
            numbered_sidebar_rows(self.shortcut_row_walk(&rows, false, cx));
        self.tab_index_by_node = numbered_shortcut_rows
            .iter()
            .map(|row| (row.node_id.clone(), row.index))
            .collect();
        self.numbered_shortcut_rows = numbered_shortcut_rows;
    }

    pub(crate) fn select_shortcut_row(
        &mut self,
        index: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = activation_target_for_index(&self.numbered_shortcut_rows, index) else {
            return;
        };
        match target {
            SidebarActivationTarget::Tab { tab_id, session_id } => {
                self.activate_sidebar_session(&tab_id, &session_id, window, cx)
            }
            SidebarActivationTarget::Mission {
                mission_id,
                project_id,
            } => {
                self.active_project_id = project_id;
                self.open_mission(mission_id, window, cx);
            }
        }
    }

    pub(crate) fn handle_shortcut_modifiers_changed(
        &mut self,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() || !command_held_alone(modifiers) {
            self.clear_shortcut_pills(cx);
            return;
        }
        if self.cmd_held_since.is_some() {
            return;
        }
        let started_at = Instant::now();
        self.cmd_held_since = Some(started_at);
        self.shortcut_key_pressed = false;
        if std::mem::take(&mut self.show_shortcut_pills) {
            cx.notify();
        }
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(SHORTCUT_PILL_REVEAL_DELAY)
                .await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let modifiers = window.modifiers();
                let held_since = (crate::platform_ui::primary_modifier_held(modifiers)
                    && this.cmd_held_since == Some(started_at))
                .then_some(started_at);
                if should_show_shortcut_pills(
                    held_since,
                    Instant::now(),
                    other_modifiers_held(modifiers),
                    this.shortcut_key_pressed,
                    window.is_window_active(),
                ) && !this.show_shortcut_pills
                {
                    this.show_shortcut_pills = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn handle_shortcut_key_pressed(&mut self, cx: &mut Context<Self>) {
        self.cmd_held_since = None;
        self.shortcut_key_pressed = true;
        self.hide_shortcut_pills(cx);
    }

    pub(crate) fn clear_shortcut_pills(&mut self, cx: &mut Context<Self>) {
        self.cmd_held_since = None;
        self.shortcut_key_pressed = false;
        self.hide_shortcut_pills(cx);
    }

    fn hide_shortcut_pills(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.show_shortcut_pills) {
            cx.notify();
        }
    }

    pub(super) fn activate_sidebar_session(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let tab_id = tab_id.to_owned();
        let session_id = session_id.to_owned();
        window.defer(cx, move |window, cx| {
            shell.update(cx, |shell, shell_cx| {
                shell.activate_sidebar_session(&tab_id, &session_id, window, shell_cx)
            });
        });
    }
}

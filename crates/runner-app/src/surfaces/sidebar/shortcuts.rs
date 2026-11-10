use super::elements::command_held_alone;
use super::elements::other_modifiers_held;

use super::*;
use crate::surfaces::sidebar::logic::{
    activation_target_for_index, numbered_sidebar_rows, should_show_shortcut_pills,
    visible_sidebar_walk, SidebarActivationTarget, SidebarShortcutProject,
    SHORTCUT_PILL_REVEAL_DELAY,
};
use crate::*;
use runner_core::protocol::node::NodeType;

impl Sidebar {
    fn shortcut_row_walk(
        &self,
        rows: &[(NodeRow, SidebarShortcutRow)],
        groups: &super::link::LinkGroups,
        cx: &App,
    ) -> Vec<crate::surfaces::sidebar::logic::VisibleSidebarRow> {
        // Each scope in the order it draws: a linked mission right after its
        // chat's row, unless the group is collapsed (#748).
        let collapsed = &self.settings(cx).sidebar_collapsed_link_groups;
        let expand = |scope: Vec<&(NodeRow, SidebarShortcutRow)>| {
            scope
                .into_iter()
                .filter(|(node, _)| groups.head(&node.id).is_none())
                .flat_map(|(node, row)| {
                    let children = if collapsed.contains(&node.id) {
                        &[][..]
                    } else {
                        groups.missions(&node.id)
                    };
                    std::iter::once(row.clone()).chain(children.iter().filter_map(|id| {
                        rows.iter()
                            .find(|(child, _)| &child.id == id)
                            .map(|(_, row)| row.clone())
                    }))
                })
                .collect::<Vec<_>>()
        };
        let mut pinned = rows
            .iter()
            .filter(|(node, _)| node.pinned_position.is_some())
            .collect::<Vec<_>>();
        pinned.sort_by_key(|(node, _)| node.pinned_position);
        let source_projects = self.source_projects(cx);
        let source_nodes = self.source_nodes(cx);
        let project_ids = source_projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<HashSet<_>>();
        let projects = source_nodes
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
                    expanded: !self
                        .settings(cx)
                        .sidebar_collapsed_projects
                        .contains(project_id),
                    children: expand(
                        rows.iter()
                            .filter(|(child, _)| {
                                child.pinned_position.is_none()
                                    && child.parent_id.as_deref() == Some(node.id.as_str())
                            })
                            .collect(),
                    ),
                })
            })
            .collect();
        let recent = expand(
            rows.iter()
                .filter(|(node, _)| node.pinned_position.is_none() && node.parent_id.is_none())
                .collect(),
        );
        visible_sidebar_walk(
            expand(pinned),
            self.settings(cx).sidebar_projects_open,
            projects,
            self.settings(cx).sidebar_chats_open,
            recent,
        )
    }

    pub(crate) fn refresh_shortcut_rows(&mut self, tabs: &TabSet, cx: &App) {
        let walk = if self.settings(cx).sidebar_active_only {
            let collapsed = &self.settings(cx).sidebar_collapsed_active_groups;
            let collapsed_links = &self.settings(cx).sidebar_collapsed_link_groups;
            let rows = self.resolved_active_rows_for_tabs(tabs, cx);
            let sources = rows
                .iter()
                .map(|row| row.source.clone())
                .collect::<Vec<_>>();
            let groups = super::link::LinkGroups::of(&sources);
            let nodes = self.source_nodes(cx);
            let activatable = |row: &super::active::ActiveSidebarRow| {
                crate::surfaces::sidebar::logic::VisibleSidebarRow::Activatable(
                    row.shortcut_row(&nodes),
                )
            };
            // Each heading in the order it draws, a group's missions under its chat.
            let mut walk = Vec::new();
            for group in [
                super::active::ActiveGroup::Agents,
                super::active::ActiveGroup::Terminals,
            ] {
                if collapsed.contains(group.key()) {
                    continue;
                }
                for row in rows.iter().filter(|row| row.group() == group) {
                    if groups.head(&row.source.node().id).is_some() {
                        continue;
                    }
                    walk.push(activatable(row));
                    if collapsed_links.contains(&row.source.node().id) {
                        continue;
                    }
                    for id in groups.missions(&row.source.node().id) {
                        if let Some(child) = rows.iter().find(|row| &row.source.node().id == id) {
                            walk.push(activatable(child));
                        }
                    }
                }
            }
            walk
        } else {
            let resolved = self.resolved_sidebar_rows_for_tabs(tabs, cx);
            let groups = super::link::LinkGroups::of(&resolved);
            let rows = resolved
                .iter()
                .map(|row| (row.node().clone(), row.shortcut_row(&self.source_nodes(cx))))
                .collect::<Vec<_>>();
            self.shortcut_row_walk(&rows, &groups, cx)
        };
        let numbered_shortcut_rows = numbered_sidebar_rows(walk);
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

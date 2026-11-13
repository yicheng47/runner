use super::*;
use crate::surfaces::sidebar::logic::{
    complete_unpinned_scope_order, container_drop_target, list_drop_target,
    ordered_pinned_node_ids_after_drop, ordered_root_node_ids_after_project_drop,
    ordered_visible_node_ids_after_drop, take_drag_state, DropKind,
};
use crate::*;

impl Sidebar {
    /// The nodes a drop orders. A grouped mission has no pin of its own but may
    /// be dropped among the pinned rows, leaving its group pinned there (#748).
    fn drop_nodes(&self, dragged_id: &str, kind: DropKind, cx: &App) -> Vec<NodeRow> {
        let mut nodes = self.source_nodes(cx);
        if kind == DropKind::Pinned
            && super::link::LinkGroups::of(&self.resolved_sidebar_rows(cx))
                .head(dragged_id)
                .is_some()
        {
            if let Some(node) = nodes.iter_mut().find(|node| node.id == dragged_id) {
                node.pinned_position.get_or_insert(i64::MAX);
            }
        }
        nodes
    }

    /// A remote window offers link/unlink drops only when the host advertises
    /// the corresponding companion actions (#1297).
    fn remote_keeps_link(&self, dragged_id: &str, target: &DropTarget, cx: &App) -> bool {
        if !self.front_page.is_remote() || target.kind == DropKind::Project {
            return true;
        }
        let rows = self.resolved_sidebar_rows(cx);
        let mission = rows
            .iter()
            .any(|row| matches!(row, SidebarRow::Mission { node, .. } if node.id == dragged_id));
        if !mission {
            return true;
        }
        match super::link::link_change(
            target.group.as_deref(),
            super::link::LinkGroups::of(&rows).head(dragged_id),
        ) {
            None => true,
            Some(super::link::LinkChange::Link(_)) => self.front_page.supports(&["linkMission"]),
            Some(super::link::LinkChange::Unlink) => {
                self.front_page
                    .supports(if target.kind == DropKind::Pinned {
                        &["unlinkMission", "pin"]
                    } else {
                        &["unlinkMission"]
                    })
            }
        }
    }

    /// The agent chats in a tab, by session.
    fn tab_agent_chats(&self, tab_node_id: &str, rows: &[SidebarRow]) -> Vec<String> {
        rows.iter()
            .find_map(|row| match row {
                SidebarRow::Tab { node, members, .. } if node.id == tab_node_id => Some(
                    members
                        .iter()
                        .filter(|member| super::link::is_agent_chat(member))
                        .map(|member| member.session_id.clone())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub(crate) fn clear_sidebar_drag(&mut self, path: &'static str, cx: &mut Context<Self>) {
        let dragged_id = self.dragged_id.clone();
        let drop_marker = self.drop_marker.clone();
        let changed = take_drag_state(
            &mut self.dragged_id,
            &mut self.drop_target,
            &mut self.drop_marker,
        ) | self.link_drop.take().is_some();
        if changed {
            tracing::debug!(
                target: "sidebar::drag",
                path,
                window_id = self.window_id,
                dragged_id = ?dragged_id,
                marker = ?drop_marker,
                remaining_dragged_id = ?self.dragged_id,
                remaining_drop_target = ?self.drop_target,
                remaining_marker = ?self.drop_marker,
                "clear sidebar drag"
            );
            cx.notify();
        }
    }

    pub(super) fn start_sidebar_drag(&mut self, dragged_id: String, cx: &mut Context<Self>) {
        self.dragged_id = Some(dragged_id);
        self.drop_target = None;
        self.drop_marker = None;
        self.link_drop = None;
        tracing::debug!(
            target: "sidebar::drag",
            path = "drag-start",
            window_id = self.window_id,
            dragged_id = ?self.dragged_id,
            marker = ?self.drop_marker,
            "start sidebar drag"
        );
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_row_drop_target(
        &mut self,
        dragged_id: &str,
        kind: DropKind,
        parent_id: Option<&str>,
        visible_ids: &[String],
        hovered_id: &str,
        after: bool,
        marker: String,
        group: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.front_page.supports(&[if kind == DropKind::Pinned {
            "reorderPinned"
        } else {
            "move"
        }]) {
            return;
        }
        let drop_target = list_drop_target(
            &self.drop_nodes(dragged_id, kind, cx),
            kind,
            parent_id,
            visible_ids,
            dragged_id,
            hovered_id,
            after,
        )
        .map(|target| DropTarget { group, ..target })
        .filter(|target| self.remote_keeps_link(dragged_id, target, cx));
        let drop_marker = drop_target.as_ref().map(|_| marker);
        let changed = self.dragged_id.as_deref() != Some(dragged_id)
            || self.drop_target != drop_target
            || self.drop_marker != drop_marker
            || self.link_drop.is_some();
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = drop_target;
        self.drop_marker = drop_marker;
        self.link_drop = None;
        if changed {
            tracing::debug!(
                target: "sidebar::drag",
                path = "row-target",
                window_id = self.window_id,
                dragged_id = ?self.dragged_id,
                marker = ?self.drop_marker,
                drop_target = ?self.drop_target,
                "set sidebar drop target"
            );
        }
        cx.notify();
    }

    pub(super) fn update_project_container_drop_target(
        &mut self,
        dragged_id: &str,
        project_node_id: &str,
        visible_ids: &[String],
        cx: &mut Context<Self>,
    ) {
        if !self.front_page.supports(&["move"]) {
            return;
        }
        let drop_target = container_drop_target(
            &self.source_nodes(cx),
            visible_ids,
            dragged_id,
            project_node_id,
        )
        .filter(|target| self.remote_keeps_link(dragged_id, target, cx));
        let drop_marker = drop_target
            .as_ref()
            .map(|_| format!("container:{project_node_id}"));
        let changed = self.dragged_id.as_deref() != Some(dragged_id)
            || self.drop_target != drop_target
            || self.drop_marker != drop_marker
            || self.link_drop.is_some();
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = drop_target;
        self.drop_marker = drop_marker;
        self.link_drop = None;
        if changed {
            tracing::debug!(
                target: "sidebar::drag",
                path = "container-target",
                window_id = self.window_id,
                dragged_id = ?self.dragged_id,
                marker = ?self.drop_marker,
                drop_target = ?self.drop_target,
                "set sidebar drop target"
            );
        }
        cx.notify();
    }

    /// Whether a dragged mission may link to one of a tab's chats: a mission in
    /// the tab's own place (a project, or none) that is not already linked there.
    pub(super) fn can_link_drop(
        &self,
        dragged_id: &str,
        tab_node_id: &str,
        chats: &[(String, String)],
        cx: &App,
    ) -> bool {
        if !self.front_page.supports(&["linkMission"]) {
            return false;
        }
        let nodes = self.source_nodes(cx);
        let node = |id: &str| nodes.iter().find(|node| node.id == id);
        let (Some(mission_node), Some(tab_node)) = (node(dragged_id), node(tab_node_id)) else {
            return false;
        };
        if mission_node.node_type != NodeType::Mission
            || mission_node.parent_id != tab_node.parent_id
        {
            return false;
        }
        let already_linked =
            |session_id: &str| chats.iter().any(|(session, _)| session == session_id);
        if self.front_page.is_remote() {
            return self.remote_overview(cx).is_some_and(|overview| {
                overview
                    .missions
                    .iter()
                    .find(|mission| mission_node.ref_id.as_deref() == Some(mission.id.as_str()))
                    .is_some_and(|mission| {
                        !mission
                            .linked_chat
                            .as_ref()
                            .is_some_and(|linked| already_linked(&linked.session_id))
                    })
            });
        }
        self.app_store
            .read(cx)
            .missions
            .iter()
            .find(|summary| mission_node.ref_id.as_deref() == Some(summary.mission.id.as_str()))
            .is_some_and(|summary| {
                summary.mission.archived_at.is_none()
                    && !summary
                        .mission
                        .linked_chat
                        .as_ref()
                        .is_some_and(|linked| already_linked(&linked.session_id))
            })
    }

    pub(super) fn set_link_drop(&mut self, drop: super::link::LinkDrop, cx: &mut Context<Self>) {
        let marker = Some(format!("link:{}", drop.tab_node));
        if self.link_drop.as_ref() == Some(&drop) && self.drop_marker == marker {
            return;
        }
        self.dragged_id = Some(drop.mission_node.clone());
        self.drop_target = None;
        self.drop_marker = marker;
        self.link_drop = Some(drop);
        cx.notify();
    }

    /// Link the dropped mission to the tab's chat, or ask which when the tab holds several.
    fn commit_link_drop(
        &mut self,
        drop: super::link::LinkDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mission_id = self
            .source_nodes(cx)
            .into_iter()
            .find(|node| node.id == drop.mission_node)
            .and_then(|node| node.ref_id);
        let Some(mission_id) = mission_id else {
            return;
        };
        match drop.chats.as_slice() {
            [(session, _)] => {
                self.link_mission_to(mission_id, session.clone(), cx);
            }
            chats => {
                let only = chats.iter().map(|(session, _)| session.clone()).collect();
                self.open_link_picker(
                    mission_id,
                    Some(only),
                    None,
                    super::link::PickerAnchor::Pointer,
                    window,
                    cx,
                );
            }
        }
    }

    pub(super) fn commit_sidebar_drop(
        &mut self,
        dragged_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drop) = self
            .link_drop
            .take()
            .filter(|drop| drop.mission_node == dragged_id)
        {
            self.commit_link_drop(drop, window, cx);
            self.clear_sidebar_drag("link-drop", cx);
            return;
        }
        if self.dragged_id.as_deref() != Some(dragged_id) {
            tracing::debug!(
                target: "sidebar::drag",
                path = "commit-early-return",
                reason = "dragged-id-mismatch",
                window_id = self.window_id,
                dragged_id = ?self.dragged_id,
                marker = ?self.drop_marker,
                event_dragged_id = dragged_id,
                "skip sidebar drop"
            );
            self.clear_sidebar_drag("commit-early-return", cx);
            return;
        }
        let Some(target) = self.drop_target.clone() else {
            tracing::debug!(
                target: "sidebar::drag",
                path = "commit-early-return",
                reason = "missing-drop-target",
                window_id = self.window_id,
                dragged_id = ?self.dragged_id,
                marker = ?self.drop_marker,
                event_dragged_id = dragged_id,
                "skip sidebar drop"
            );
            self.clear_sidebar_drag("commit-early-return", cx);
            return;
        };
        tracing::debug!(
            target: "sidebar::drag",
            path = "drop",
            window_id = self.window_id,
            dragged_id = ?self.dragged_id,
            marker = ?self.drop_marker,
            drop_target = ?target,
            "commit sidebar drop"
        );
        let rows = self.resolved_sidebar_rows(cx);
        let nodes = self.source_nodes(cx);
        let Some(node) = nodes.iter().find(|node| node.id == dragged_id) else {
            self.clear_sidebar_drag("drop", cx);
            return;
        };
        if self
            .shell
            .upgrade()
            .is_some_and(|root| root.read(cx).launcher_tab_pending(&node.id, cx))
        {
            self.clear_sidebar_drag("start-pending", cx);
            return;
        }
        let ordered = |ids: Vec<String>| {
            ids.into_iter()
                .filter_map(|id| nodes.iter().find(|node| node.id == id).cloned())
                .collect::<Vec<_>>()
        };
        let order = match target.kind {
            DropKind::Pinned => {
                let mut visible = rows
                    .iter()
                    .filter(|row| row.node().pinned_position.is_some())
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                // A grouped mission has no pinned row yet; it joins at the drop.
                if !visible.iter().any(|id| id == dragged_id) {
                    visible.push(dragged_id.to_owned());
                }
                ordered_pinned_node_ids_after_drop(
                    &self.drop_nodes(dragged_id, DropKind::Pinned, cx),
                    &visible,
                    dragged_id,
                    target.index,
                )
            }
            DropKind::Project => ordered_root_node_ids_after_project_drop(
                &self.source_nodes(cx),
                dragged_id,
                target.index,
            ),
            DropKind::Leaf => {
                let groups = super::link::LinkGroups::of(&rows);
                let visible = match &target.group {
                    Some(tab) => groups.missions(tab).to_vec(),
                    None => self
                        .scope_rows(&rows, target.parent_id.as_deref())
                        .into_iter()
                        .map(|row| row.node().id.clone())
                        .filter(|id| groups.head(id).is_none())
                        .collect(),
                };
                let visible =
                    ordered_visible_node_ids_after_drop(&visible, dragged_id, target.index);
                complete_unpinned_scope_order(
                    &self.source_nodes(cx),
                    target.parent_id.as_deref(),
                    dragged_id,
                    &visible,
                )
            }
        };
        let order = ordered(order);
        let project = target
            .parent_id
            .as_ref()
            .and_then(|id| nodes.iter().find(|node| &node.id == id));
        let pinned = target.kind == DropKind::Pinned;
        let feature = if pinned { "reorderPinned" } else { "move" };
        if !self.front_page.supports(&[feature]) {
            self.clear_sidebar_drag("drop", cx);
            return;
        }
        // Into a group links the mission to its chat; a grouped mission dropped among
        // the scope's own rows or the pinned rows leaves its chat and takes that
        // place's pin (#748). The link changes before the move, so an unlink's own
        // placement yields to the drop.
        enum DropLinkStep {
            Link { mission_id: String, session: String },
            Unlink { mission_id: String },
        }
        let mut link_step = None;
        if target.kind != DropKind::Project {
            let groups = super::link::LinkGroups::of(&rows);
            let mission_id = node
                .ref_id
                .clone()
                .filter(|_| node.node_type == NodeType::Mission);
            let head = groups.head(&node.id).map(str::to_owned);
            if let Some(mission_id) = mission_id {
                match super::link::link_change(target.group.as_deref(), head.as_deref()) {
                    Some(super::link::LinkChange::Link(tab)) => {
                        let chats = self.tab_agent_chats(&tab, &rows);
                        let [session] = chats.as_slice() else {
                            self.clear_sidebar_drag("drop", cx);
                            let pending = super::link::PendingMove {
                                node: node.clone(),
                                project: project.cloned(),
                                order,
                            };
                            self.open_link_picker(
                                mission_id,
                                Some(chats),
                                Some(pending),
                                super::link::PickerAnchor::Pointer,
                                window,
                                cx,
                            );
                            return;
                        };
                        link_step = Some(DropLinkStep::Link {
                            mission_id,
                            session: session.clone(),
                        });
                    }
                    Some(super::link::LinkChange::Unlink) => {
                        link_step = Some(DropLinkStep::Unlink { mission_id });
                    }
                    None => {}
                }
            }
        }
        let pin_node = node.clone();
        let move_node = node.clone();
        let project = project.cloned();
        let operation = move |client: crate::surfaces::front_page::FrontPageClient| {
            if pinned {
                client.reorder_pinned(&order)
            } else {
                client.move_item(&move_node, project.as_ref(), &order)
            }
        };
        if self.front_page.is_remote() {
            if let Some(shell) = self.shell.upgrade() {
                shell.update(cx, |root, cx| match link_step {
                    Some(DropLinkStep::Link {
                        mission_id,
                        session,
                    }) => root.remote_client.remote_action(
                        "sidebar-order".into(),
                        &["linkMission", feature],
                        move |client| {
                            client.link_mission(mission_id, session)?;
                            operation(client)?;
                            Ok(Default::default())
                        },
                        cx,
                    ),
                    Some(DropLinkStep::Unlink { mission_id }) if pinned => {
                        root.remote_client.remote_action(
                            "sidebar-order".into(),
                            &["unlinkMission", "pin", feature],
                            move |client| {
                                client.unlink_mission(mission_id)?;
                                client.pin(&pin_node, true)?;
                                operation(client)?;
                                Ok(Default::default())
                            },
                            cx,
                        )
                    }
                    Some(DropLinkStep::Unlink { mission_id }) => root.remote_client.remote_action(
                        "sidebar-order".into(),
                        &["unlinkMission", feature],
                        move |client| {
                            client.unlink_mission(mission_id)?;
                            operation(client)?;
                            Ok(Default::default())
                        },
                        cx,
                    ),
                    None => root.remote_client.remote_action(
                        "sidebar-order".into(),
                        &[feature],
                        move |client| {
                            operation(client)?;
                            Ok(Default::default())
                        },
                        cx,
                    ),
                });
            }
            self.clear_sidebar_drag("drop", cx);
            return;
        }
        match link_step {
            Some(DropLinkStep::Link {
                mission_id,
                session,
            }) => {
                if let Err(error) = self.front_page.link_mission(mission_id, session) {
                    self.report_error(error.to_string(), cx);
                    self.clear_sidebar_drag("drop", cx);
                    return;
                }
            }
            Some(DropLinkStep::Unlink { mission_id }) => {
                if let Err(error) = self.front_page.unlink_mission(mission_id).and_then(|_| {
                    if pinned {
                        self.front_page.pin(&pin_node, true)
                    } else {
                        Ok(())
                    }
                }) {
                    self.report_error(error.to_string(), cx);
                    self.clear_sidebar_drag("drop", cx);
                    return;
                }
            }
            None => {}
        }
        let result = operation(self.front_page.clone());
        match result {
            Ok(nodes) => {
                let nodes = nodes.expect("local sidebar rows");
                tracing::debug!(
                    target: "sidebar::drag",
                    path = "drop-result",
                    outcome = "ok",
                    window_id = self.window_id,
                    dragged_id = ?self.dragged_id,
                    marker = ?self.drop_marker,
                    node_count = nodes.len(),
                    "sidebar drop committed"
                );
                if let Some(shell) = self.shell.upgrade() {
                    shell.update(cx, |shell, cx| shell.replace_tab_rows(&nodes, cx));
                }
                self.app_store
                    .update(cx, |store, store_cx| store.replace_nodes(nodes, store_cx));
                self.refresh_store(StoreRefreshKind::All, cx);
            }
            Err(error) => {
                tracing::debug!(
                    target: "sidebar::drag",
                    path = "drop-result",
                    outcome = "error",
                    window_id = self.window_id,
                    dragged_id = ?self.dragged_id,
                    marker = ?self.drop_marker,
                    error = %error,
                    "sidebar drop failed"
                );
                self.report_error(error.to_string(), cx);
            }
        }
        self.clear_sidebar_drag("drop", cx);
    }
}

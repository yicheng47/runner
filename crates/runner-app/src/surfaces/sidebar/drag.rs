use super::*;
use crate::surfaces::sidebar_logic::{
    complete_unpinned_scope_order, container_drop_target, list_drop_target,
    ordered_pinned_node_ids_after_drop, ordered_root_node_ids_after_project_drop,
    ordered_visible_node_ids_after_drop, take_drag_state, DropKind,
};
use crate::*;

impl Sidebar {
    pub(crate) fn clear_sidebar_drag(&mut self, path: &'static str, cx: &mut Context<Self>) {
        let dragged_id = self.dragged_id.clone();
        let drop_marker = self.drop_marker.clone();
        let changed = take_drag_state(
            &mut self.dragged_id,
            &mut self.drop_target,
            &mut self.drop_marker,
        );
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
        cx: &mut Context<Self>,
    ) {
        let drop_target = list_drop_target(
            &self.app_store.read(cx).nodes,
            kind,
            parent_id,
            visible_ids,
            dragged_id,
            hovered_id,
            after,
        );
        let drop_marker = drop_target.as_ref().map(|_| marker);
        let changed = self.dragged_id.as_deref() != Some(dragged_id)
            || self.drop_target != drop_target
            || self.drop_marker != drop_marker;
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = drop_target;
        self.drop_marker = drop_marker;
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
        let drop_target = container_drop_target(
            &self.app_store.read(cx).nodes,
            visible_ids,
            dragged_id,
            project_node_id,
        );
        let drop_marker = drop_target
            .as_ref()
            .map(|_| format!("container:{project_node_id}"));
        let changed = self.dragged_id.as_deref() != Some(dragged_id)
            || self.drop_target != drop_target
            || self.drop_marker != drop_marker;
        self.dragged_id = Some(dragged_id.to_owned());
        self.drop_target = drop_target;
        self.drop_marker = drop_marker;
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

    pub(super) fn commit_sidebar_drop(&mut self, dragged_id: &str, cx: &mut Context<Self>) {
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
        let result = match target.kind {
            DropKind::Pinned => {
                let visible = rows
                    .iter()
                    .filter(|row| row.node().pinned_position.is_some())
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let order = ordered_pinned_node_ids_after_drop(
                    &self.app_store.read(cx).nodes,
                    &visible,
                    dragged_id,
                    target.index,
                );
                runner_backend::ops::node::node_reorder_pinned(self.core(cx), order)
            }
            DropKind::Project => {
                let order = ordered_root_node_ids_after_project_drop(
                    &self.app_store.read(cx).nodes,
                    dragged_id,
                    target.index,
                );
                runner_backend::ops::node::node_move(
                    self.core(cx),
                    dragged_id.to_owned(),
                    None,
                    order,
                )
            }
            DropKind::Leaf => {
                let visible = self
                    .scope_rows(&rows, target.parent_id.as_deref())
                    .into_iter()
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let visible =
                    ordered_visible_node_ids_after_drop(&visible, dragged_id, target.index);
                let order = complete_unpinned_scope_order(
                    &self.app_store.read(cx).nodes,
                    target.parent_id.as_deref(),
                    dragged_id,
                    &visible,
                );
                runner_backend::ops::node::node_move(
                    self.core(cx),
                    dragged_id.to_owned(),
                    target.parent_id,
                    order,
                )
            }
        };
        match result {
            Ok(nodes) => {
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
                    shell.update(cx, |shell, _| shell.tabs.replace_rows(&nodes));
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

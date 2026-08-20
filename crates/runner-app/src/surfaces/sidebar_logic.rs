use std::collections::BTreeMap;

use chrono::DateTime;
use runner_backend::repo::node::{NodeRow, NodeType};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum AttentionState {
    #[default]
    None,
    Unread,
    Working,
}

pub(crate) fn tab_attention_state(
    any_running_busy: bool,
    last_completed_at: Option<&str>,
    last_viewed_at: Option<&str>,
) -> AttentionState {
    if any_running_busy {
        return AttentionState::Working;
    }
    if tab_has_unread_completion(last_completed_at, last_viewed_at) {
        AttentionState::Unread
    } else {
        AttentionState::None
    }
}

fn tab_has_unread_completion(
    last_completed_at: Option<&str>,
    last_viewed_at: Option<&str>,
) -> bool {
    let Some(completed) = last_completed_at.and_then(parse_timestamp) else {
        return false;
    };
    let Some(viewed) = last_viewed_at.and_then(parse_timestamp) else {
        return true;
    };
    completed > viewed
}

fn parse_timestamp(value: &str) -> Option<DateTime<chrono::FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

pub(crate) fn mission_attention_state(any_session_live: bool, idle: bool) -> AttentionState {
    if any_session_live && !idle {
        AttentionState::Working
    } else {
        AttentionState::None
    }
}

pub(crate) fn rollup_attention_state(
    states: impl IntoIterator<Item = AttentionState>,
) -> AttentionState {
    states.into_iter().max().unwrap_or_default()
}

pub(crate) fn attention_rollups(
    rows: impl IntoIterator<Item = (Option<String>, AttentionState)>,
) -> BTreeMap<Option<String>, AttentionState> {
    let mut rollups: BTreeMap<Option<String>, AttentionState> = BTreeMap::new();
    for (parent_id, attention) in rows {
        let current = rollups.entry(parent_id).or_default();
        *current = (*current).max(attention);
    }
    rollups
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropKind {
    Pinned,
    Project,
    Leaf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DropTarget {
    pub kind: DropKind,
    pub parent_id: Option<String>,
    pub index: usize,
}

pub(crate) fn can_drop_in_scope(
    nodes: &[NodeRow],
    dragged_id: &str,
    parent_id: Option<&str>,
) -> bool {
    let Some(dragged) = node(nodes, dragged_id) else {
        return false;
    };
    if dragged.pinned_position.is_some() || dragged.node_type == NodeType::Project {
        return false;
    }
    let leaving_project = dragged
        .parent_id
        .as_deref()
        .and_then(|id| node(nodes, id))
        .is_some_and(|parent| parent.node_type == NodeType::Project);
    let Some(parent_id) = parent_id else {
        return !leaving_project;
    };
    node(nodes, parent_id).is_some_and(|parent| {
        parent.node_type == NodeType::Project
            && matches!(dragged.node_type, NodeType::Tab | NodeType::Mission)
    })
}

pub(crate) fn list_drop_target(
    nodes: &[NodeRow],
    kind: DropKind,
    parent_id: Option<&str>,
    visible_ids: &[String],
    dragged_id: &str,
    hovered_id: &str,
    after: bool,
) -> Option<DropTarget> {
    let dragged = node(nodes, dragged_id)?;
    match kind {
        DropKind::Pinned if dragged.pinned_position.is_none() => return None,
        DropKind::Project if dragged.node_type != NodeType::Project => return None,
        DropKind::Leaf if !can_drop_in_scope(nodes, dragged_id, parent_id) => return None,
        _ => {}
    }
    let hovered = visible_ids.iter().position(|id| id == hovered_id)?;
    let original_index = hovered + usize::from(after);
    let index = visible_ids[..original_index]
        .iter()
        .filter(|id| id.as_str() != dragged_id)
        .count();
    Some(DropTarget {
        kind,
        parent_id: parent_id.map(str::to_owned),
        index,
    })
}

pub(crate) fn container_drop_target(
    nodes: &[NodeRow],
    visible_child_ids: &[String],
    dragged_id: &str,
    container_id: &str,
) -> Option<DropTarget> {
    if !can_drop_in_scope(nodes, dragged_id, Some(container_id)) {
        return None;
    }
    Some(DropTarget {
        kind: DropKind::Leaf,
        parent_id: Some(container_id.to_owned()),
        index: visible_child_ids
            .iter()
            .filter(|id| id.as_str() != dragged_id)
            .count(),
    })
}

pub(crate) fn ordered_visible_node_ids_after_drop(
    target_ids: &[String],
    dragged_id: &str,
    requested_index: usize,
) -> Vec<String> {
    let mut remaining = target_ids
        .iter()
        .filter(|id| id.as_str() != dragged_id)
        .cloned()
        .collect::<Vec<_>>();
    let index = requested_index.min(remaining.len());
    remaining.insert(index, dragged_id.to_owned());
    remaining
}

pub(crate) fn complete_unpinned_scope_order(
    nodes: &[NodeRow],
    parent_id: Option<&str>,
    dragged_id: &str,
    ordered_visible_ids: &[String],
) -> Vec<String> {
    let pinned = nodes
        .iter()
        .filter(|node| node.pinned_position.is_some())
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let visible = ordered_visible_ids
        .iter()
        .filter(|id| !pinned.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let visible_set = visible
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let siblings = nodes.iter().filter(|node| {
        node.parent_id.as_deref() == parent_id
            && node.id != dragged_id
            && node.pinned_position.is_none()
    });
    let projects_first = if parent_id.is_none() {
        siblings
            .clone()
            .filter(|node| node.node_type == NodeType::Project)
            .map(|node| node.id.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let hidden = siblings
        .filter(|node| !(parent_id.is_none() && node.node_type == NodeType::Project))
        .map(|node| node.id.clone())
        .filter(|id| !visible_set.contains(id.as_str()))
        .collect::<Vec<_>>();
    projects_first
        .into_iter()
        .chain(visible)
        .chain(hidden)
        .collect()
}

pub(crate) fn ordered_pinned_node_ids_after_drop(
    nodes: &[NodeRow],
    visible_pinned_ids: &[String],
    dragged_id: &str,
    requested_index: usize,
) -> Vec<String> {
    let mut pinned = nodes
        .iter()
        .filter_map(|node| {
            node.pinned_position
                .map(|position| (position, node.id.clone()))
        })
        .collect::<Vec<_>>();
    pinned.sort_by_key(|(position, _)| *position);
    let all_ids = pinned.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
    let pinned_set = all_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut seen = std::collections::HashSet::new();
    let visible = visible_pinned_ids
        .iter()
        .filter(|id| pinned_set.contains(id.as_str()) && seen.insert(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !visible.iter().any(|id| id == dragged_id) {
        return all_ids;
    }
    let reordered = ordered_visible_node_ids_after_drop(&visible, dragged_id, requested_index);
    let visible_set = visible
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut reordered = reordered.into_iter();
    all_ids
        .into_iter()
        .map(|id| {
            if visible_set.contains(id.as_str()) {
                reordered.next().expect("visible pinned replacement")
            } else {
                id
            }
        })
        .collect()
}

pub(crate) fn ordered_root_node_ids_after_project_drop(
    nodes: &[NodeRow],
    dragged_id: &str,
    requested_index: usize,
) -> Vec<String> {
    let root = nodes
        .iter()
        .filter(|node| node.parent_id.is_none() && node.pinned_position.is_none())
        .collect::<Vec<_>>();
    let mut projects = root
        .iter()
        .filter(|node| node.node_type == NodeType::Project && node.id != dragged_id)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let index = requested_index.min(projects.len());
    projects.insert(index, dragged_id.to_owned());
    let mut projects = projects.into_iter();
    root.into_iter()
        .map(|node| {
            if node.node_type == NodeType::Project {
                projects.next().expect("project replacement")
            } else {
                node.id.clone()
            }
        })
        .collect()
}

fn node<'a>(nodes: &'a [NodeRow], id: &str) -> Option<&'a NodeRow> {
    nodes.iter().find(|node| node.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: &str,
        node_type: NodeType,
        parent_id: Option<&str>,
        position: i64,
        pinned_position: Option<i64>,
    ) -> NodeRow {
        NodeRow {
            id: id.into(),
            parent_id: parent_id.map(str::to_owned),
            position,
            node_type,
            name: None,
            ref_id: None,
            layout: None,
            pinned_position,
            last_completed_at: None,
            last_viewed_at: None,
            created_at: format!("2026-08-19T00:00:0{position}Z"),
        }
    }

    #[test]
    fn attention_priority_is_working_then_unread_then_clear() {
        assert_eq!(
            tab_attention_state(
                true,
                Some("2026-08-19T01:00:00Z"),
                Some("2026-08-19T02:00:00Z")
            ),
            AttentionState::Working
        );
        assert_eq!(
            tab_attention_state(
                false,
                Some("2026-08-19T02:00:00Z"),
                Some("2026-08-19T01:00:00Z")
            ),
            AttentionState::Unread
        );
        assert_eq!(
            tab_attention_state(
                false,
                Some("2026-08-19T01:00:00Z"),
                Some("2026-08-19T02:00:00Z")
            ),
            AttentionState::None
        );
        assert_eq!(
            mission_attention_state(true, false),
            AttentionState::Working
        );
        assert_eq!(mission_attention_state(true, true), AttentionState::None);
    }

    #[test]
    fn collapsed_container_rollups_keep_the_highest_child_priority() {
        let rollups = attention_rollups([
            (Some("project-a".into()), AttentionState::Unread),
            (Some("project-a".into()), AttentionState::Working),
            (Some("project-b".into()), AttentionState::Unread),
            (None, AttentionState::None),
        ]);
        assert_eq!(rollups[&Some("project-a".into())], AttentionState::Working);
        assert_eq!(rollups[&Some("project-b".into())], AttentionState::Unread);
        assert_eq!(rollups[&None], AttentionState::None);
        assert_eq!(
            rollup_attention_state(rollups.values().copied()),
            AttentionState::Working
        );
    }

    #[test]
    fn drag_targets_respect_project_boundaries_and_compute_reparent_index() {
        let nodes = vec![
            row("project-a", NodeType::Project, None, 0, None),
            row("project-b", NodeType::Project, None, 1, None),
            row("root-tab", NodeType::Tab, None, 2, None),
            row("inside-a", NodeType::Tab, Some("project-a"), 0, None),
            row("inside-b", NodeType::Mission, Some("project-b"), 0, None),
        ];
        assert_eq!(
            container_drop_target(&nodes, &["inside-b".into()], "root-tab", "project-b"),
            Some(DropTarget {
                kind: DropKind::Leaf,
                parent_id: Some("project-b".into()),
                index: 1,
            })
        );
        assert!(list_drop_target(
            &nodes,
            DropKind::Leaf,
            None,
            &["root-tab".into()],
            "inside-a",
            "root-tab",
            false,
        )
        .is_none());
        assert_eq!(
            list_drop_target(
                &nodes,
                DropKind::Leaf,
                Some("project-b"),
                &["inside-b".into()],
                "inside-a",
                "inside-b",
                false,
            )
            .unwrap()
            .index,
            0
        );
    }

    #[test]
    fn tree_ordering_is_scoped_and_preserves_hidden_and_interleaved_rows() {
        let nodes = vec![
            row("project-a", NodeType::Project, None, 0, None),
            row("root-a", NodeType::Tab, None, 1, None),
            row("project-b", NodeType::Project, None, 2, None),
            row("hidden", NodeType::Mission, None, 3, None),
            row("root-b", NodeType::Tab, None, 4, None),
            row("pin-hidden", NodeType::Mission, None, 5, Some(0)),
            row("pin-a", NodeType::Tab, None, 6, Some(1)),
            row("pin-b", NodeType::Tab, None, 7, Some(2)),
            row("child-a", NodeType::Tab, Some("project-a"), 0, None),
            row(
                "child-hidden",
                NodeType::Mission,
                Some("project-a"),
                1,
                None,
            ),
            row("child-b", NodeType::Tab, Some("project-a"), 2, None),
        ];
        let visible =
            ordered_visible_node_ids_after_drop(&["root-a".into(), "root-b".into()], "root-b", 0);
        assert_eq!(visible, ["root-b", "root-a"]);
        assert_eq!(
            complete_unpinned_scope_order(&nodes, None, "root-b", &visible),
            ["project-a", "project-b", "root-b", "root-a", "hidden"]
        );
        assert_eq!(
            ordered_root_node_ids_after_project_drop(&nodes, "project-b", 0),
            ["project-b", "root-a", "project-a", "hidden", "root-b"]
        );
        assert_eq!(
            ordered_pinned_node_ids_after_drop(
                &nodes,
                &["pin-a".into(), "pin-b".into()],
                "pin-b",
                0,
            ),
            ["pin-hidden", "pin-b", "pin-a"]
        );
        let nested = ordered_visible_node_ids_after_drop(
            &["child-a".into(), "child-b".into()],
            "child-b",
            0,
        );
        assert_eq!(
            complete_unpinned_scope_order(&nodes, Some("project-a"), "child-b", &nested),
            ["child-b", "child-a", "child-hidden"]
        );
    }
}

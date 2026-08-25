use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::DateTime;
use runner_backend::repo::node::{NodeRow, NodeType};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum AttentionState {
    #[default]
    None,
    Unread,
    Working,
}

pub(crate) const SHORTCUT_PILL_REVEAL_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SidebarActivationTarget {
    Tab {
        tab_id: String,
        session_id: String,
    },
    Mission {
        mission_id: String,
        project_id: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SidebarShortcutRow {
    pub(crate) node_id: String,
    pub(crate) target: SidebarActivationTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SidebarShortcutProject {
    pub(crate) node_id: String,
    pub(crate) expanded: bool,
    pub(crate) children: Vec<SidebarShortcutRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VisibleSidebarRow {
    Header(String),
    Activatable(SidebarShortcutRow),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NumberedSidebarRow {
    pub(crate) index: u8,
    pub(crate) node_id: String,
    pub(crate) target: SidebarActivationTarget,
}

pub(crate) fn visible_sidebar_walk(
    pinned: Vec<SidebarShortcutRow>,
    projects_open: bool,
    projects: Vec<SidebarShortcutProject>,
    recent_open: bool,
    recent: Vec<SidebarShortcutRow>,
) -> Vec<VisibleSidebarRow> {
    let mut rows = Vec::new();
    if !pinned.is_empty() {
        rows.push(VisibleSidebarRow::Header("pinned".into()));
        rows.extend(pinned.into_iter().map(VisibleSidebarRow::Activatable));
    }
    rows.push(VisibleSidebarRow::Header("projects".into()));
    if projects_open {
        for project in projects {
            rows.push(VisibleSidebarRow::Header(project.node_id));
            if project.expanded {
                rows.extend(
                    project
                        .children
                        .into_iter()
                        .map(VisibleSidebarRow::Activatable),
                );
            }
        }
    }
    rows.push(VisibleSidebarRow::Header("recent".into()));
    if recent_open {
        rows.extend(recent.into_iter().map(VisibleSidebarRow::Activatable));
    }
    rows.push(VisibleSidebarRow::Header("workspace".into()));
    rows
}

pub(crate) fn numbered_sidebar_rows(
    rows: impl IntoIterator<Item = VisibleSidebarRow>,
) -> Vec<NumberedSidebarRow> {
    rows.into_iter()
        .filter_map(|row| match row {
            VisibleSidebarRow::Header(_) => None,
            VisibleSidebarRow::Activatable(row) => Some(row),
        })
        .take(9)
        .enumerate()
        .map(|(index, row)| NumberedSidebarRow {
            index: index as u8 + 1,
            node_id: row.node_id,
            target: row.target,
        })
        .collect()
}

pub(crate) fn activation_target_for_index(
    rows: &[NumberedSidebarRow],
    index: u8,
) -> Option<SidebarActivationTarget> {
    rows.iter()
        .find(|row| row.index == index)
        .map(|row| row.target.clone())
}

pub(crate) fn should_show_shortcut_pills(
    command_held_alone_since: Option<Instant>,
    now: Instant,
    other_modifiers: bool,
    key_pressed: bool,
    window_active: bool,
) -> bool {
    window_active
        && !other_modifiers
        && !key_pressed
        && command_held_alone_since.is_some_and(|started_at| {
            now.saturating_duration_since(started_at) >= SHORTCUT_PILL_REVEAL_DELAY
        })
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

pub(crate) fn direct_tab_attention_state<'a>(
    members: impl IntoIterator<Item = (&'a str, bool)>,
    last_completed_at: Option<&str>,
    last_viewed_at: Option<&str>,
) -> AttentionState {
    let mut has_chat = false;
    let mut any_chat_running_busy = false;
    for (runtime, running_busy) in members {
        if runtime != "shell" {
            has_chat = true;
            any_chat_running_busy |= running_busy;
        }
    }
    if has_chat {
        tab_attention_state(any_chat_running_busy, last_completed_at, last_viewed_at)
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

pub(crate) fn indicator_visible(drop_marker: Option<&str>, has_active_drag: bool) -> bool {
    drop_marker.is_some() && has_active_drag
}

pub(crate) fn take_drag_state(
    dragged_id: &mut Option<String>,
    drop_target: &mut Option<DropTarget>,
    drop_marker: &mut Option<String>,
) -> bool {
    let had_id = dragged_id.take().is_some();
    let had_target = drop_target.take().is_some();
    let had_marker = drop_marker.take().is_some();
    had_id || had_target || had_marker
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
    let Some(parent_id) = parent_id else {
        return true;
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

    fn shortcut_tab(id: &str) -> SidebarShortcutRow {
        SidebarShortcutRow {
            node_id: id.into(),
            target: SidebarActivationTarget::Tab {
                tab_id: id.into(),
                session_id: format!("{id}-session"),
            },
        }
    }

    fn shortcut_mission(id: &str, project_id: Option<&str>) -> SidebarShortcutRow {
        SidebarShortcutRow {
            node_id: id.into(),
            target: SidebarActivationTarget::Mission {
                mission_id: id.into(),
                project_id: project_id.map(str::to_owned),
            },
        }
    }

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
    fn direct_tab_attention_ignores_shell_members() {
        assert_eq!(
            direct_tab_attention_state(
                [("shell", true)],
                Some("2026-08-19T02:00:00Z"),
                Some("2026-08-19T01:00:00Z"),
            ),
            AttentionState::None,
        );
        assert_eq!(
            direct_tab_attention_state([("shell", true), ("codex", false)], None, None,),
            AttentionState::None,
        );
        assert_eq!(
            direct_tab_attention_state([("shell", false), ("codex", true)], None, None,),
            AttentionState::Working,
        );
    }

    #[test]
    fn visible_row_numbering_skips_headers_and_collapsed_projects_then_caps_at_nine() {
        let pinned = vec![
            shortcut_tab("pin-tab"),
            shortcut_mission("pin-mission", None),
        ];
        let projects = vec![
            SidebarShortcutProject {
                node_id: "expanded-project".into(),
                expanded: true,
                children: vec![
                    shortcut_tab("project-tab"),
                    shortcut_mission("project-mission", Some("project-a")),
                ],
            },
            SidebarShortcutProject {
                node_id: "collapsed-project".into(),
                expanded: false,
                children: vec![shortcut_tab("hidden-tab")],
            },
        ];
        let recent = (1..=8)
            .map(|index| shortcut_tab(&format!("recent-{index}")))
            .collect();
        let walk = visible_sidebar_walk(pinned, true, projects, true, recent);
        assert!(walk
            .iter()
            .any(|row| matches!(row, VisibleSidebarRow::Header(id) if id == "expanded-project")));
        assert!(walk
            .iter()
            .any(|row| matches!(row, VisibleSidebarRow::Header(id) if id == "collapsed-project")));
        assert!(!walk.iter().any(|row| matches!(
            row,
            VisibleSidebarRow::Activatable(row) if row.node_id == "hidden-tab"
        )));

        let numbered = numbered_sidebar_rows(walk);
        assert_eq!(numbered.len(), 9);
        assert_eq!(
            numbered
                .iter()
                .map(|row| (row.index, row.node_id.as_str()))
                .collect::<Vec<_>>(),
            [
                (1, "pin-tab"),
                (2, "pin-mission"),
                (3, "project-tab"),
                (4, "project-mission"),
                (5, "recent-1"),
                (6, "recent-2"),
                (7, "recent-3"),
                (8, "recent-4"),
                (9, "recent-5"),
            ]
        );
    }

    #[test]
    fn shortcut_reveal_requires_command_alone_for_the_full_delay() {
        let started_at = Instant::now();
        assert!(!should_show_shortcut_pills(
            Some(started_at),
            started_at + Duration::from_millis(149),
            false,
            false,
            true,
        ));
        assert!(should_show_shortcut_pills(
            Some(started_at),
            started_at + SHORTCUT_PILL_REVEAL_DELAY,
            false,
            false,
            true,
        ));
        assert!(!should_show_shortcut_pills(
            None,
            started_at + SHORTCUT_PILL_REVEAL_DELAY,
            false,
            false,
            true,
        ));
        assert!(!should_show_shortcut_pills(
            Some(started_at),
            started_at + SHORTCUT_PILL_REVEAL_DELAY,
            true,
            false,
            true,
        ));
        assert!(!should_show_shortcut_pills(
            Some(started_at),
            started_at + SHORTCUT_PILL_REVEAL_DELAY,
            false,
            true,
            true,
        ));
        assert!(!should_show_shortcut_pills(
            Some(started_at),
            started_at + SHORTCUT_PILL_REVEAL_DELAY,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn shortcut_index_resolves_the_numbered_tab_or_mission_target() {
        let numbered = numbered_sidebar_rows(visible_sidebar_walk(
            vec![shortcut_tab("tab-a")],
            true,
            vec![SidebarShortcutProject {
                node_id: "project".into(),
                expanded: true,
                children: vec![shortcut_mission("mission-a", Some("project-a"))],
            }],
            true,
            vec![],
        ));
        assert_eq!(
            activation_target_for_index(&numbered, 1),
            Some(SidebarActivationTarget::Tab {
                tab_id: "tab-a".into(),
                session_id: "tab-a-session".into(),
            })
        );
        assert_eq!(
            activation_target_for_index(&numbered, 2),
            Some(SidebarActivationTarget::Mission {
                mission_id: "mission-a".into(),
                project_id: Some("project-a".into()),
            })
        );
        assert_eq!(activation_target_for_index(&numbered, 3), None);
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
    fn drag_targets_cross_project_boundaries_and_compute_reparent_index() {
        let nodes = vec![
            row("project-a", NodeType::Project, None, 0, None),
            row("project-b", NodeType::Project, None, 1, None),
            row("root-tab", NodeType::Tab, None, 2, None),
            row("inside-a", NodeType::Tab, Some("project-a"), 0, None),
            row("inside-b", NodeType::Mission, Some("project-b"), 0, None),
            row("pinned", NodeType::Tab, None, 3, Some(0)),
        ];
        assert_eq!(
            container_drop_target(&nodes, &["inside-b".into()], "root-tab", "project-b"),
            Some(DropTarget {
                kind: DropKind::Leaf,
                parent_id: Some("project-b".into()),
                index: 1,
            })
        );
        let root_target = list_drop_target(
            &nodes,
            DropKind::Leaf,
            None,
            &["root-tab".into()],
            "inside-a",
            "root-tab",
            false,
        )
        .unwrap();
        assert_eq!(
            root_target,
            DropTarget {
                kind: DropKind::Leaf,
                parent_id: None,
                index: 0,
            }
        );
        let visible = ordered_visible_node_ids_after_drop(
            &["root-tab".into()],
            "inside-a",
            root_target.index,
        );
        assert_eq!(
            complete_unpinned_scope_order(&nodes, None, "inside-a", &visible),
            ["project-a", "project-b", "inside-a", "root-tab"]
        );
        assert!(list_drop_target(
            &nodes,
            DropKind::Leaf,
            None,
            &["root-tab".into()],
            "pinned",
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
    fn drop_indicator_requires_an_active_drag() {
        assert!(indicator_visible(Some("leaf:tab:true"), true));
        assert!(!indicator_visible(Some("leaf:tab:true"), false));
        assert!(!indicator_visible(None, true));
        assert!(!indicator_visible(None, false));
    }

    #[test]
    fn taking_drag_state_clears_every_field() {
        let mut dragged_id = Some("tab-a".into());
        let mut drop_target = Some(DropTarget {
            kind: DropKind::Leaf,
            parent_id: None,
            index: 1,
        });
        let mut drop_marker = Some("leaf:tab-b:true".into());

        assert!(take_drag_state(
            &mut dragged_id,
            &mut drop_target,
            &mut drop_marker
        ));
        assert_eq!(dragged_id, None);
        assert_eq!(drop_target, None);
        assert_eq!(drop_marker, None);
        assert!(!take_drag_state(
            &mut dragged_id,
            &mut drop_target,
            &mut drop_marker
        ));
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

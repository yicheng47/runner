use runner_app::pane_layout::{
    MissionLayout, PaneLayout, PaneLeaf, PaneNode, PaneSplit, SplitOrientation, TabSet,
    DEFAULT_DRAWER_HEIGHT, MAX_DRAWER_HEIGHT, MIN_DRAWER_HEIGHT,
};
use runner_backend::repo::node::{NodeRow, NodeType};

fn row(id: &str, position: i64, layout: &PaneLayout) -> NodeRow {
    NodeRow {
        id: id.to_owned(),
        parent_id: None,
        position,
        node_type: NodeType::Tab,
        name: None,
        ref_id: None,
        layout: Some(layout.serialize().unwrap()),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: "2026-07-19T00:00:00Z".into(),
    }
}

fn mission_row(id: &str, layout: Option<&str>) -> NodeRow {
    NodeRow {
        id: id.to_owned(),
        parent_id: None,
        position: 0,
        node_type: NodeType::Mission,
        name: None,
        ref_id: Some("mission-1".into()),
        layout: layout.map(str::to_owned),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: "2026-07-19T00:00:00Z".into(),
    }
}

fn leaf(id: &str, session_id: Option<&str>) -> PaneNode {
    PaneNode::Leaf(PaneLeaf {
        id: id.to_owned(),
        session_id: session_id.map(str::to_owned),
    })
}

fn split(
    id: &str,
    orientation: SplitOrientation,
    sizes: [f32; 2],
    a: PaneNode,
    b: PaneNode,
) -> PaneNode {
    PaneNode::Split(PaneSplit {
        id: id.to_owned(),
        orientation,
        sizes,
        a: Box::new(a),
        b: Box::new(b),
    })
}

/// `single` plus one split of the trailing pane per extra session, which is
/// how a tab grows now that the picker is gone.
fn columns(sessions: &[&str]) -> PaneLayout {
    let mut layout = PaneLayout::single(sessions.first().copied(), &[]);
    let mut target = layout.focused_pane_id.clone();
    for session_id in sessions.iter().skip(1) {
        target = layout.split(&target, SplitOrientation::Row).unwrap();
        layout.assign_session(&target, session_id).unwrap();
    }
    layout
}

#[test]
fn splitting_a_tab_leaves_the_new_pane_empty_focused_and_last_in_leaf_order() {
    let mut layout = PaneLayout::single(Some("A"), &["A".into()]);

    let second = layout.split("p1", SplitOrientation::Row).unwrap();

    assert_eq!(layout.session_ids(), ["A"]);
    assert_eq!(layout.focused_pane_id, second);
    assert!(layout.focused_session_id().is_none());
    assert_eq!(layout.root.leaves().last().unwrap().id, second);
}

#[test]
fn pane_assignment_is_move_not_copy_across_tabs() {
    let mut tab_a = columns(&["A"]);
    let second = tab_a.split("p1", SplitOrientation::Row).unwrap();
    let tab_b = PaneLayout::single(Some("B"), &["B".into()]);
    let rows = [
        row("01K00000000000000000000000", 0, &tab_a),
        row("01K00000000000000000000001", 1, &tab_b),
    ];
    let mut tabs = TabSet::from_rows(&rows);
    tabs.assign_to_active(&second, "B").unwrap();

    assert_eq!(tabs.tabs()[0].session_ids(), ["A", "B"]);
    assert!(tabs.tabs()[1].session_ids().is_empty());
}

#[test]
fn grouped_duplicate_session_has_one_resize_owner() {
    let mut layout = columns(&["A"]);
    let second = layout.split("p1", SplitOrientation::Row).unwrap();
    // `assign_session` moves rather than copies, so the duplicate is staged
    // straight onto the leaf the way a second window would show it.
    let PaneNode::Split(split) = &mut layout.root else {
        panic!("a split tab must have a root split");
    };
    let PaneNode::Leaf(leaf) = split.b.as_mut() else {
        panic!("the new pane is a leaf");
    };
    leaf.session_id = Some("A".into());

    assert!(layout.is_resize_owner("p1", "A"));
    assert!(!layout.is_resize_owner(&second, "A"));
}

#[test]
fn a_five_pane_tree_round_trips_with_its_orientations_sizes_and_slot_order() {
    let mut layout = columns(&["A", "B"]);
    let stacked = layout.split("p1", SplitOrientation::Column).unwrap();
    layout.assign_session(&stacked, "C").unwrap();
    let nested = layout.split(&stacked, SplitOrientation::Row).unwrap();
    layout.assign_session(&nested, "D").unwrap();
    let last = layout.split(&nested, SplitOrientation::Column).unwrap();
    layout.assign_session(&last, "E").unwrap();
    let split_ids = ["s2", "s4", "s6", "s8"];
    for (index, split_id) in split_ids.iter().enumerate() {
        assert!(layout.set_split_sizes(split_id, [60. + index as f32, 40. - index as f32]));
    }
    layout.add_drawer_shell("shell".into());
    layout.set_drawer_height(321.);

    let serialized = layout.serialize().unwrap();
    let restored =
        PaneLayout::from_node_row(&row("01K00000000000000000000000", 0, &layout)).unwrap();

    assert_eq!(restored.root, layout.root);
    assert_eq!(restored.root.leaves().len(), 5);
    assert_eq!(restored.session_ids(), ["A", "C", "D", "E", "B"]);
    assert_eq!(restored.drawer_shells(), ["shell"]);
    assert_eq!(restored.drawer_height(), 321.);
    assert!(restored.drawer_open());

    // `slots` and `drawer` are the backend's reconciler contract.
    let raw: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        raw["slots"],
        serde_json::json!(["A", "C", "D", "E", "B"]),
        "{serialized}"
    );
    assert_eq!(raw["drawer"]["shells"], serde_json::json!(["shell"]));
    assert!(raw.get("tree").is_some());
    assert!(raw.get("preset").is_none(), "{serialized}");
    assert!(raw.get("sizes").is_none(), "{serialized}");
    assert_eq!(
        runner_backend::repo::node::session_ids_from_layout(&serialized),
        ["A", "C", "D", "E", "B", "shell"]
    );
}

#[test]
fn empty_panes_persist_as_null_slots_so_the_backend_still_sees_every_session() {
    let mut layout = columns(&["A"]);
    layout.split("p1", SplitOrientation::Row).unwrap();

    let serialized = layout.serialize().unwrap();
    let raw: serde_json::Value = serde_json::from_str(&serialized).unwrap();

    assert_eq!(raw["slots"], serde_json::json!(["A", null]));
    assert_eq!(
        runner_backend::repo::node::session_ids_from_layout(&serialized),
        ["A"]
    );
}

#[test]
fn rows_saved_before_the_tree_rebuild_the_trees_their_presets_drew() {
    let cases = [
        ("single", leaf("p1", Some("A"))),
        (
            "cols-2",
            split(
                "cols-2:outer",
                SplitOrientation::Row,
                [50., 50.],
                leaf("p1", Some("A")),
                leaf("p2", Some("B")),
            ),
        ),
        (
            "rows-2",
            split(
                "rows-2:outer",
                SplitOrientation::Column,
                [50., 50.],
                leaf("p1", Some("A")),
                leaf("p2", Some("B")),
            ),
        ),
        (
            "main-2",
            split(
                "main-2:outer",
                SplitOrientation::Row,
                [60., 40.],
                leaf("p1", Some("A")),
                split(
                    "main-2:inner",
                    SplitOrientation::Column,
                    [50., 50.],
                    leaf("p2", Some("B")),
                    leaf("p3", Some("C")),
                ),
            ),
        ),
        (
            "cols-3",
            split(
                "cols-3:outer",
                SplitOrientation::Row,
                [33.33, 66.67],
                leaf("p1", Some("A")),
                split(
                    "cols-3:inner",
                    SplitOrientation::Row,
                    [50., 50.],
                    leaf("p2", Some("B")),
                    leaf("p3", Some("C")),
                ),
            ),
        ),
        (
            "rows-3",
            split(
                "rows-3:outer",
                SplitOrientation::Column,
                [33.33, 66.67],
                leaf("p1", Some("A")),
                split(
                    "rows-3:inner",
                    SplitOrientation::Column,
                    [50., 50.],
                    leaf("p2", Some("B")),
                    leaf("p3", Some("C")),
                ),
            ),
        ),
    ];
    for (preset, expected) in cases {
        let restored = PaneLayout::from_node_row(&raw_row(
            "01K00000000000000000000000",
            &format!(r#"{{"preset":"{preset}","slots":["A","B","C"],"sizes":{{}}}}"#),
        ))
        .unwrap();
        assert_eq!(restored.root, expected, "{preset}");
        assert_eq!(restored.focused_pane_id, "p1", "{preset}");
    }

    // 0.6.0/0.6.1 spellings and stored gutter sizes survive the same path.
    let restored = PaneLayout::from_node_row(&raw_row(
        "01K00000000000000000000001",
        r#"{"preset":"cols2","slots":["A","B"],"sizes":{"cols-2:outer":[65,35]}}"#,
    ))
    .unwrap();
    let PaneNode::Split(outer) = &restored.root else {
        panic!("cols-2 must have an outer split");
    };
    assert_eq!(outer.sizes, [65., 35.]);
}

#[test]
fn slots_win_over_a_stale_tree_because_the_backend_only_edits_slots() {
    let mut layout = columns(&["A", "B"]);
    let third = layout.split("p3", SplitOrientation::Row).unwrap();
    layout.assign_session(&third, "C").unwrap();
    let serialized = layout.serialize().unwrap();

    // What `repo::node::remove_session_except` does to a row when "B" is
    // archived: the slot is nulled in place, the tree is left untouched.
    let mut stored: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    stored["slots"] = serde_json::json!(["A", null, "C"]);
    let restored =
        PaneLayout::from_node_row(&raw_row("01K00000000000000000000000", &stored.to_string()))
            .unwrap();

    assert_eq!(restored.session_ids(), ["A", "C"]);
    assert_eq!(restored.root.leaves()[1].session_id, None);
    assert_eq!(restored.root.leaves().len(), 3);
    assert!(!restored.contains_session("B"));

    // And the next save no longer carries the archived session anywhere.
    let rewritten = restored.serialize().unwrap();
    assert!(!rewritten.contains("\"B\""), "{rewritten}");
    assert_eq!(
        runner_backend::repo::node::session_ids_from_layout(&rewritten),
        ["A", "C"]
    );

    // A session moved to another tab is dropped from this one the same way.
    let mut moved: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    moved["slots"] = serde_json::json!([null, null, null]);
    let emptied =
        PaneLayout::from_node_row(&raw_row("01K00000000000000000000001", &moved.to_string()))
            .unwrap();
    assert!(emptied.session_ids().is_empty());
    assert_eq!(emptied.root.leaves().len(), 3);
}

#[test]
fn a_legacy_row_is_rewritten_in_the_tree_shape_on_its_next_save() {
    let legacy = PaneLayout::from_node_row(&raw_row(
        "01K00000000000000000000000",
        r#"{"preset":"main-2","slots":["A","B","C"],"sizes":{"main-2:outer":[70,30]}}"#,
    ))
    .unwrap();

    let rewritten = legacy.serialize().unwrap();
    let raw: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert!(raw.get("preset").is_none(), "{rewritten}");
    assert!(raw.get("sizes").is_none(), "{rewritten}");

    let reread =
        PaneLayout::from_node_row(&raw_row("01K00000000000000000000000", &rewritten)).unwrap();
    assert_eq!(reread.root, legacy.root);
}

#[test]
fn a_layout_with_neither_a_tree_nor_a_preset_is_unreadable() {
    assert!(PaneLayout::from_node_row(&raw_row(
        "01K00000000000000000000000",
        r#"{"slots":["A"]}"#
    ))
    .is_err());
}

#[test]
fn persisted_layout_round_trips_drawer_state_and_old_rows_use_defaults() {
    let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);
    layout.add_drawer_shell("shell-a".into());
    layout.add_drawer_shell("shell-b".into());
    assert!(layout.activate_drawer_shell("shell-a"));
    layout.set_drawer_height(412.);
    let restored =
        PaneLayout::from_node_row(&row("01K00000000000000000000000", 0, &layout)).unwrap();

    assert!(restored.drawer_open());
    assert_eq!(restored.drawer_height(), 412.);
    assert_eq!(restored.drawer_shells(), ["shell-a", "shell-b"]);
    assert_eq!(restored.active_drawer_shell(), Some("shell-a"));
    assert_eq!(restored.session_ids(), ["chat"]);
    assert_eq!(restored.all_session_ids(), ["chat", "shell-a", "shell-b"]);

    let old = PaneLayout::from_node_row(&raw_row(
        "01K00000000000000000000001",
        r#"{"preset":"single","slots":["chat"],"sizes":{}}"#,
    ))
    .unwrap();
    assert!(!old.drawer_open());
    assert_eq!(old.drawer_height(), DEFAULT_DRAWER_HEIGHT);
    assert!(old.drawer_shells().is_empty());
    assert_eq!(old.active_drawer_shell(), None);
}

#[test]
fn mission_layout_round_trips_and_old_rows_use_drawer_defaults() {
    let mut layout = MissionLayout::default();
    layout.drawer.add("shell-a".into());
    layout.drawer.add("shell-b".into());
    assert!(layout.drawer.activate("shell-a"));
    layout.drawer.set_height(412.);
    let serialized = layout.serialize().unwrap();
    let restored = MissionLayout::from_node_row(&mission_row(
        "01K00000000000000000000000",
        Some(&serialized),
    ))
    .unwrap();

    assert!(restored.drawer.open());
    assert_eq!(restored.drawer.height(), 412.);
    assert_eq!(restored.drawer.shells(), ["shell-a", "shell-b"]);
    assert_eq!(restored.drawer.active_shell(), Some("shell-a"));

    let old = MissionLayout::from_node_row(&mission_row("01K00000000000000000000001", Some("{}")))
        .unwrap();
    assert!(!old.drawer.open());
    assert_eq!(old.drawer.height(), DEFAULT_DRAWER_HEIGHT);
    assert!(old.drawer.shells().is_empty());
    assert_eq!(old.drawer.active_shell(), None);

    let absent =
        MissionLayout::from_node_row(&mission_row("01K00000000000000000000002", None)).unwrap();
    assert_eq!(absent, MissionLayout::default());
}

#[test]
fn drawer_height_clamps_and_removing_the_active_shell_prefers_its_left_neighbour() {
    let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);
    layout.set_drawer_height(10.);
    assert_eq!(layout.drawer_height(), MIN_DRAWER_HEIGHT);
    layout.set_drawer_height(900.);
    assert_eq!(layout.drawer_height(), MAX_DRAWER_HEIGHT);

    layout.add_drawer_shell("one".into());
    layout.add_drawer_shell("two".into());
    layout.add_drawer_shell("three".into());
    assert_eq!(layout.active_drawer_shell(), Some("three"));
    assert!(layout.remove_drawer_shell("three"));
    assert_eq!(layout.active_drawer_shell(), Some("two"));
    assert!(layout.remove_drawer_shell("two"));
    assert_eq!(layout.active_drawer_shell(), Some("one"));
    assert!(layout.remove_drawer_shell("one"));
    assert!(!layout.drawer_open());
    assert_eq!(layout.active_drawer_shell(), None);
}

#[test]
fn generic_session_removal_also_clears_drawer_membership() {
    let mut layout = PaneLayout::single(Some("chat"), &["chat".into()]);
    layout.add_drawer_shell("shell".into());

    layout.remove_session("shell");

    assert!(layout.drawer_shells().is_empty());
    assert!(!layout.drawer_open());
}

#[test]
fn close_pane_collapses_the_tree_and_keeps_sessions_in_the_surviving_order() {
    let mut focused = columns(&["A", "B"]);
    let focused_pane = focused.focused_pane_id.clone();
    assert!(focused.close_pane("p1"));
    assert_eq!(focused.session_ids(), ["B"]);
    assert_eq!(focused.focused_pane_id, focused_pane);
    assert!(matches!(focused.root, PaneNode::Leaf(_)));

    let mut nested = columns(&["A", "B"]);
    let third = nested.split("p3", SplitOrientation::Column).unwrap();
    nested.assign_session(&third, "C").unwrap();
    assert!(nested.set_split_sizes("s2", [70., 30.]));
    assert!(nested.close_pane("p1"));

    assert_eq!(nested.session_ids(), ["B", "C"]);
    let PaneNode::Split(split) = &nested.root else {
        panic!("the inner split must survive as the new root");
    };
    assert_eq!(split.id, "s4");
    assert_eq!(split.orientation, SplitOrientation::Column);

    let restored =
        PaneLayout::from_node_row(&row("01K00000000000000000000000", 0, &nested)).unwrap();
    assert_eq!(restored.root, nested.root);
}

#[test]
fn close_pane_preserves_focus_when_another_pane_closes_and_single_is_a_noop() {
    let mut layout = columns(&["A", "B"]);
    let third = layout.split("p3", SplitOrientation::Row).unwrap();
    layout.assign_session(&third, "C").unwrap();
    assert!(layout.focus_session("A"));
    let original_focus = layout.focused_pane_id.clone();

    assert!(layout.close_pane(&third));
    assert_eq!(layout.session_ids(), ["A", "B"]);
    assert_eq!(layout.focused_pane_id, original_focus);

    let mut single = PaneLayout::single(Some("A"), &["A".into()]);
    assert!(!single.close_pane("p1"));
    assert_eq!(single.session_ids(), ["A"]);
    assert!(!layout.close_pane("missing-pane"));
}

#[test]
fn switching_tabs_preserves_each_tabs_sessions_focus_and_geometry() {
    let mut tab_a = columns(&["A", "B"]);
    tab_a.set_split_sizes("s2", [65., 35.]);
    tab_a.focus_session("B");
    let tab_b = columns(&["C", "D"]);
    let rows = [
        row("01K00000000000000000000000", 0, &tab_a),
        row("01K00000000000000000000001", 1, &tab_b),
    ];
    let mut tabs = TabSet::from_rows(&rows);
    tabs.active_mut().unwrap().focus_session("B");

    assert!(tabs.activate("01K00000000000000000000001"));
    assert!(tabs.activate("01K00000000000000000000000"));
    let active = tabs.active().unwrap();
    assert_eq!(active.session_ids(), ["A", "B"]);
    assert_eq!(active.focused_session_id(), Some("B"));
    let PaneNode::Split(split) = &active.root else {
        panic!("a two-pane tab must stay split");
    };
    assert_eq!(split.sizes, [65., 35.]);
}

#[test]
fn rehydration_keeps_the_active_tab_by_stable_id() {
    let a = PaneLayout::single(Some("A"), &["A".into()]);
    let b = PaneLayout::single(Some("B"), &["B".into()]);
    let c = PaneLayout::single(Some("C"), &["C".into()]);
    let rows = [
        row("01K00000000000000000000000", 0, &a),
        row("01K00000000000000000000001", 1, &b),
        row("01K00000000000000000000002", 2, &c),
    ];
    let mut tabs = TabSet::from_rows(&rows);
    tabs.activate("01K00000000000000000000001");
    tabs.replace_rows(&rows[1..]);

    assert_eq!(tabs.active_tab_id(), Some("01K00000000000000000000001"));
}

#[test]
fn structural_writes_preserve_parent_scope() {
    // Node model: a layout/name upsert carries the parent scope but no
    // position — placement changes go exclusively through `node_move`,
    // so a structural write can never scramble sibling ordering.
    let filed_tab = PaneLayout::single(Some("A"), &["A".into()]);
    let loose_tab = columns(&["B"]);
    let mut filed_row = row("01K00000000000000000000000", 7, &filed_tab);
    filed_row.parent_id = Some("folder-1".into());
    let loose_row = row("01K00000000000000000000001", 2, &loose_tab);
    let tabs = TabSet::from_rows(&[filed_row, loose_row]);

    let filed = &tabs.tabs()[0];
    assert_eq!(filed.parent_id.as_deref(), Some("folder-1"));
    assert_eq!(
        filed.upsert_input().unwrap().parent_id.as_deref(),
        Some("folder-1")
    );
    let loose = &tabs.tabs()[1];
    assert_eq!(loose.position, 2);
}

fn raw_row(id: &str, layout: &str) -> NodeRow {
    NodeRow {
        id: id.to_owned(),
        parent_id: None,
        position: 0,
        node_type: NodeType::Tab,
        name: None,
        ref_id: None,
        layout: Some(layout.to_owned()),
        pinned_position: None,
        last_completed_at: None,
        last_viewed_at: None,
        created_at: "2026-07-19T00:00:00Z".into(),
    }
}

#[test]
fn tauri_era_null_sizes_fall_back_to_preset_defaults() {
    let row = raw_row(
        "01K00000000000000000000000",
        r#"{"preset":"cols-2","slots":["A","B"],"sizes":{"cols-2:outer":[null,null],"stale":[70,30,0],"other":"x"}}"#,
    );
    let restored = PaneLayout::from_node_row(&row).unwrap();

    assert_eq!(restored.session_ids(), ["A", "B"]);
    let PaneNode::Split(split) = restored.root else {
        panic!("cols-2 must have an outer split");
    };
    assert_eq!(split.sizes, [50., 50.]);
}

#[test]
fn unreadable_tab_row_is_skipped_without_dropping_the_set() {
    let good = PaneLayout::single(Some("A"), &["A".into()]);
    let rows = [
        raw_row(
            "01K00000000000000000000000",
            r#"{"preset":"nope","slots":["B"]}"#,
        ),
        row("01K00000000000000000000001", 1, &good),
        raw_row("01K00000000000000000000002", "not json"),
    ];
    let tabs = TabSet::from_rows(&rows);

    assert_eq!(tabs.tabs().len(), 1);
    assert_eq!(tabs.active_tab_id(), Some("01K00000000000000000000001"));
    assert_eq!(tabs.tabs()[0].session_ids(), ["A"]);
}

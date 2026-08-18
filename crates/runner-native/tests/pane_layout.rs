use runner_app::repo::node::{NodeRow, NodeType};
use runner_native::pane_layout::{PaneLayout, PaneNode, PresetKind, SplitOrientation, TabSet};

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

#[test]
fn builds_the_six_picker_presets_with_tauri_shapes() {
    let cases = [
        (PresetKind::Single, 1),
        (PresetKind::Cols2, 2),
        (PresetKind::Rows2, 2),
        (PresetKind::Main2, 3),
        (PresetKind::Cols3, 3),
        (PresetKind::Rows3, 3),
    ];
    for (preset, count) in cases {
        assert_eq!(
            PaneLayout::fresh(preset, None, &[]).root.leaves().len(),
            count
        );
    }

    let main = PaneLayout::fresh(PresetKind::Main2, None, &[]);
    let PaneNode::Split(outer) = main.root else {
        panic!("main-2 must have an outer split");
    };
    assert_eq!(outer.orientation, SplitOrientation::Row);
    assert_eq!(outer.sizes, [60., 40.]);
    let PaneNode::Split(inner) = *outer.b else {
        panic!("main-2 must stack its secondary panes");
    };
    assert_eq!(inner.orientation, SplitOrientation::Column);
}

#[test]
fn preset_keeps_the_focused_session_in_the_first_slot_and_focuses_empty() {
    let layout = PaneLayout::fresh(
        PresetKind::Main2,
        Some("B"),
        &["A".into(), "B".into(), "C".into()],
    );
    assert_eq!(layout.session_ids(), ["B", "A", "C"]);

    let layout = PaneLayout::fresh(PresetKind::Cols3, Some("A"), &["A".into()]);
    assert_eq!(layout.focused_pane_id, "p2");
}

#[test]
fn pane_assignment_is_move_not_copy_across_tabs() {
    let tab_a = PaneLayout::fresh(PresetKind::Cols2, Some("A"), &["A".into()]);
    let tab_b = PaneLayout::fresh(PresetKind::Single, Some("B"), &["B".into()]);
    let rows = [
        row("01K00000000000000000000000", 0, &tab_a),
        row("01K00000000000000000000001", 1, &tab_b),
    ];
    let mut tabs = TabSet::from_rows(&rows).unwrap();
    tabs.assign_to_active("p2", "B").unwrap();

    assert_eq!(tabs.tabs()[0].session_ids(), ["A", "B"]);
    assert!(tabs.tabs()[1].session_ids().is_empty());
}

#[test]
fn grouped_duplicate_session_has_one_resize_owner() {
    let mut layout = PaneLayout::fresh(PresetKind::Cols2, Some("A"), &["A".into()]);
    let PaneNode::Split(split) = &mut layout.root else {
        panic!("cols-2 must have an outer split");
    };
    let PaneNode::Leaf(second) = split.b.as_mut() else {
        panic!("cols-2 second child must be a leaf");
    };
    second.session_id = Some("A".into());

    assert!(layout.is_resize_owner("p1", "A"));
    assert!(!layout.is_resize_owner("p2", "A"));
}

#[test]
fn persisted_layout_round_trips_slots_and_per_split_sizes() {
    let mut layout = PaneLayout::fresh(PresetKind::Main2, Some("B"), &["A".into(), "B".into()]);
    assert!(layout.set_split_sizes("main-2:outer", [70., 30.]));
    let restored =
        PaneLayout::from_node_row(&row("01K00000000000000000000000", 0, &layout)).unwrap();

    assert_eq!(restored.preset, PresetKind::Main2);
    assert_eq!(restored.session_ids(), ["B", "A"]);
    let PaneNode::Split(outer) = restored.root else {
        panic!("main-2 must have an outer split");
    };
    assert_eq!(outer.sizes, [70., 30.]);
}

#[test]
fn switching_tabs_preserves_each_tabs_sessions_focus_and_geometry() {
    let mut tab_a = PaneLayout::fresh(PresetKind::Cols2, Some("A"), &["A".into(), "B".into()]);
    tab_a.set_split_sizes("cols-2:outer", [65., 35.]);
    tab_a.focus_session("B");
    let tab_b = PaneLayout::fresh(PresetKind::Rows2, Some("C"), &["C".into(), "D".into()]);
    let rows = [
        row("01K00000000000000000000000", 0, &tab_a),
        row("01K00000000000000000000001", 1, &tab_b),
    ];
    let mut tabs = TabSet::from_rows(&rows).unwrap();
    tabs.active_mut().unwrap().focus_session("B");

    assert!(tabs.activate("01K00000000000000000000001"));
    assert!(tabs.activate("01K00000000000000000000000"));
    let active = tabs.active().unwrap();
    assert_eq!(active.session_ids(), ["A", "B"]);
    assert_eq!(active.focused_session_id(), Some("B"));
    let PaneNode::Split(split) = &active.root else {
        panic!("cols-2 must stay split");
    };
    assert_eq!(split.sizes, [65., 35.]);
}

#[test]
fn rehydration_keeps_the_active_tab_by_stable_id() {
    let a = PaneLayout::fresh(PresetKind::Single, Some("A"), &["A".into()]);
    let b = PaneLayout::fresh(PresetKind::Single, Some("B"), &["B".into()]);
    let c = PaneLayout::fresh(PresetKind::Single, Some("C"), &["C".into()]);
    let rows = [
        row("01K00000000000000000000000", 0, &a),
        row("01K00000000000000000000001", 1, &b),
        row("01K00000000000000000000002", 2, &c),
    ];
    let mut tabs = TabSet::from_rows(&rows).unwrap();
    tabs.activate("01K00000000000000000000001");
    tabs.replace_rows(&rows[1..]).unwrap();

    assert_eq!(tabs.active_tab_id(), Some("01K00000000000000000000001"));
}

#[test]
fn structural_writes_preserve_parent_scope() {
    // Node model: a layout/name upsert carries the parent scope but no
    // position — placement changes go exclusively through `node_move`,
    // so a structural write can never scramble sibling ordering.
    let filed_tab = PaneLayout::fresh(PresetKind::Single, Some("A"), &["A".into()]);
    let loose_tab = PaneLayout::fresh(PresetKind::Cols2, Some("B"), &["B".into()]);
    let mut filed_row = row("01K00000000000000000000000", 7, &filed_tab);
    filed_row.parent_id = Some("folder-1".into());
    let loose_row = row("01K00000000000000000000001", 2, &loose_tab);
    let tabs = TabSet::from_rows(&[filed_row, loose_row]).unwrap();

    let filed = &tabs.tabs()[0];
    assert_eq!(filed.parent_id.as_deref(), Some("folder-1"));
    assert_eq!(
        filed.upsert_input().unwrap().parent_id.as_deref(),
        Some("folder-1")
    );
    let loose = &tabs.tabs()[1];
    assert_eq!(loose.position, 2);
}

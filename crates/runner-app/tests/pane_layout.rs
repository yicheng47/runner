use runner_app::pane_layout::{PaneLayout, PaneNode, PresetKind, SplitOrientation, TabSet};
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
    let mut tabs = TabSet::from_rows(&rows);
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
fn close_pane_collapses_the_tree_and_keeps_sessions_in_the_surviving_order() {
    let mut focused = PaneLayout::fresh(PresetKind::Cols2, Some("A"), &["A".into(), "B".into()]);
    let focused_pane = focused.focused_pane_id.clone();
    assert!(focused.close_pane(&focused_pane));
    assert_eq!(focused.preset, PresetKind::Single);
    assert_eq!(focused.session_ids(), ["B"]);
    assert_eq!(focused.focused_pane_id, focused.root.leaves()[0].id);

    let mut main = PaneLayout::fresh(
        PresetKind::Main2,
        Some("A"),
        &["A".into(), "B".into(), "C".into()],
    );
    assert!(main.set_split_sizes("main-2:inner", [70., 30.]));
    let big_pane = main.root.leaves()[0].id.clone();
    assert!(main.close_pane(&big_pane));
    assert_eq!(main.preset, PresetKind::Rows2);
    assert_eq!(main.session_ids(), ["B", "C"]);
    let PaneNode::Split(split) = &main.root else {
        panic!("rows-2 must stay split");
    };
    assert_eq!(split.id, "rows-2:outer");
    assert_eq!(split.sizes, [70., 30.]);

    let restored = PaneLayout::from_node_row(&row("01K00000000000000000000000", 0, &main)).unwrap();
    assert_eq!(restored.preset, PresetKind::Rows2);
    assert_eq!(restored.session_ids(), ["B", "C"]);
    let PaneNode::Split(split) = restored.root else {
        panic!("rows-2 must stay split");
    };
    assert_eq!(split.sizes, [70., 30.]);
}

#[test]
fn close_pane_preserves_focus_when_another_pane_closes_and_single_is_a_noop() {
    let mut main = PaneLayout::fresh(
        PresetKind::Main2,
        Some("A"),
        &["A".into(), "B".into(), "C".into()],
    );
    let original_focus = main.focused_pane_id.clone();
    let last_pane = main.root.leaves()[2].id.clone();
    assert!(main.close_pane(&last_pane));
    assert_eq!(main.preset, PresetKind::Cols2);
    assert_eq!(main.session_ids(), ["A", "B"]);
    assert_eq!(main.focused_pane_id, original_focus);

    let mut single = PaneLayout::fresh(PresetKind::Single, Some("A"), &["A".into()]);
    let pane_id = single.focused_pane_id.clone();
    assert!(!single.close_pane(&pane_id));
    assert_eq!(single.session_ids(), ["A"]);
}

#[test]
fn close_pane_handles_three_way_rows_and_columns_and_missing_ids() {
    for preset in [PresetKind::Cols3, PresetKind::Rows3] {
        let mut layout =
            PaneLayout::fresh(preset, Some("A"), &["A".into(), "B".into(), "C".into()]);
        let middle = layout.root.leaves()[1].id.clone();
        assert!(layout.close_pane(&middle));
        assert_eq!(layout.session_ids(), ["A", "C"]);
        assert_eq!(
            layout.preset,
            if preset == PresetKind::Cols3 {
                PresetKind::Cols2
            } else {
                PresetKind::Rows2
            }
        );
        assert!(!layout.close_pane("missing-pane"));
    }
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
    let mut tabs = TabSet::from_rows(&rows);
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
    let filed_tab = PaneLayout::fresh(PresetKind::Single, Some("A"), &["A".into()]);
    let loose_tab = PaneLayout::fresh(PresetKind::Cols2, Some("B"), &["B".into()]);
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

    assert_eq!(restored.preset, PresetKind::Cols2);
    assert_eq!(restored.session_ids(), ["A", "B"]);
    let PaneNode::Split(split) = restored.root else {
        panic!("cols-2 must have an outer split");
    };
    assert_eq!(split.sizes, [50., 50.]);
}

#[test]
fn unreadable_tab_row_is_skipped_without_dropping_the_set() {
    let good = PaneLayout::fresh(PresetKind::Single, Some("A"), &["A".into()]);
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

#[test]
fn preset_wire_names_match_tauri_and_accept_the_0_6_0_spelling() {
    let cases = [
        (PresetKind::Single, "single", "single"),
        (PresetKind::Cols2, "cols-2", "cols2"),
        (PresetKind::Rows2, "rows-2", "rows2"),
        (PresetKind::Main2, "main-2", "main2"),
        (PresetKind::Cols3, "cols-3", "cols3"),
        (PresetKind::Rows3, "rows-3", "rows3"),
    ];
    for (preset, canonical, legacy_0_6) in cases {
        let written = PaneLayout::fresh(preset, None, &[]).serialize().unwrap();
        assert!(
            written.contains(&format!("\"preset\":\"{canonical}\"")),
            "{preset:?} wrote {written}"
        );
        for spelling in [canonical, legacy_0_6] {
            let row = raw_row(
                "01K00000000000000000000000",
                &format!("{{\"preset\":\"{spelling}\",\"slots\":[],\"sizes\":{{}}}}"),
            );
            assert_eq!(PaneLayout::from_node_row(&row).unwrap().preset, preset);
        }
    }
}

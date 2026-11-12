use super::*;
use crate::repo::node::{NodeRow, NodeType};
use crate::test_support::test_core;
use rusqlite::params;
fn seed_project_mission(state: &AppCore, id: &str, project_id: Option<&str>) -> NodeRow {
    let conn = state.db.get().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at, cwd, project_id)
             VALUES (?1, 'crew', 'Mission', 'aborted', '2026-09-10T00:00:00Z', '/original', ?2)",
        params![id, project_id],
    )
    .unwrap();
    repo::node::ensure_mission_node(&conn, id, project_id).unwrap()
}

fn result_json(result: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(result).unwrap()
}

#[test]
fn mission_set_project_round_trips_and_appends_in_mixed_scopes() {
    let handler = test_core();
    let moved = seed_project_mission(&handler, "moved", None);
    let project = project::project_create(&handler, "Project".into(), "/project".into()).unwrap();
    seed_project_mission(&handler, "root-sibling", None);
    let project_sibling = seed_project_mission(&handler, "project-sibling", Some(&project.id));
    let pinned_sibling = seed_project_mission(&handler, "pinned-sibling", Some(&project.id));
    {
        let conn = handler.db.get().unwrap();
        repo::node::set_pinned(&conn, &pinned_sibling.id, true).unwrap();
        for parent_id in [None, project_sibling.parent_id.as_deref()] {
            repo::node::create_tab(&conn, parent_id, "Tab", 7, "{}").unwrap();
        }
    }
    for project_id in [Some(project.id.clone()), None] {
        let target_parent = project_id.as_ref().and(project_sibling.parent_id.clone());
        let mut expected: Vec<String> = {
            let conn = handler.db.get().unwrap();
            repo::node::list(&conn)
                .unwrap()
                .into_iter()
                .filter(|row| {
                    row.parent_id == target_parent
                        && row.pinned_position.is_none()
                        && row.id != moved.id
                })
                .map(|row| row.id)
                .collect()
        };
        expected.push(moved.id.clone());
        let mut events = handler.events.subscribe();
        let result =
            result_json(mission_set_project(&handler, "moved".into(), project_id.clone()).unwrap());
        let conn = handler.db.get().unwrap();
        let mission = repo::mission::get(&conn, "moved").unwrap().unwrap();
        assert_eq!(result, serde_json::json!(mission));
        assert_eq!(mission.project_id, project_id);
        assert_eq!(mission.cwd.as_deref(), Some("/original"));
        let node = repo::node::get(&conn, &moved.id).unwrap().unwrap();
        assert_eq!(node.parent_id, target_parent);
        let children: Vec<_> = repo::node::list(&conn)
            .unwrap()
            .into_iter()
            .filter(|row| row.parent_id == target_parent && row.pinned_position.is_none())
            .collect();
        assert_eq!(
            children
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(node.position, children.len() as i64 - 1);
        assert_eq!(events.try_recv().unwrap().name, "chat/layout-changed");
        assert_eq!(events.try_recv().unwrap().name, "mission/changed");
        assert!(events.try_recv().is_err());
    }
}

#[test]
fn mission_set_project_preserves_pinned_order_and_cwd() {
    let handler = test_core();
    let moved = seed_project_mission(&handler, "moved", None);
    let project = project::project_create(&handler, "Project".into(), "/project".into()).unwrap();
    let sibling = seed_project_mission(&handler, "sibling", Some(&project.id));
    let pin = {
        let conn = handler.db.get().unwrap();
        repo::node::set_pinned(&conn, &sibling.id, true).unwrap();
        repo::node::set_pinned(&conn, &moved.id, true).unwrap();
        repo::node::get(&conn, &moved.id)
            .unwrap()
            .unwrap()
            .pinned_position
    };
    let unpinned = seed_project_mission(&handler, "unpinned", Some(&project.id));
    for project_id in [Some(project.id.clone()), None] {
        let target_parent = project_id.as_ref().and(sibling.parent_id.clone());
        let result =
            result_json(mission_set_project(&handler, "moved".into(), project_id.clone()).unwrap());
        assert_eq!(result["project_id"], serde_json::json!(project_id));
        assert_eq!(result["cwd"], "/original");
        let conn = handler.db.get().unwrap();
        let node = repo::node::get(&conn, &moved.id).unwrap().unwrap();
        assert_eq!(node.parent_id, target_parent);
        assert_eq!(node.pinned_position, pin);
        assert_eq!(node.position, moved.position);
        let pinned: Vec<_> = repo::node::list(&conn)
            .unwrap()
            .into_iter()
            .filter(|row| row.pinned_position.is_some())
            .map(|row| row.id)
            .collect();
        assert_eq!(pinned, vec![sibling.id.clone(), moved.id.clone()]);
        let unpinned = repo::node::get(&conn, &unpinned.id).unwrap().unwrap();
        assert_eq!(unpinned.parent_id, sibling.parent_id);
        assert_eq!(unpinned.position, 0);
    }
}

#[test]
fn mission_set_project_same_container_is_a_no_op() {
    let handler = test_core();
    let project = project::project_create(&handler, "Project".into(), "/project".into()).unwrap();
    for (id, project_id) in [("root", None), ("filed", Some(project.id.clone()))] {
        seed_project_mission(&handler, id, project_id.as_deref());
        seed_project_mission(&handler, &format!("{id}-sibling"), project_id.as_deref());
        let (before, nodes) = {
            let conn = handler.db.get().unwrap();
            (
                repo::mission::get(&conn, id).unwrap().unwrap(),
                repo::node::list(&conn).unwrap(),
            )
        };
        let mut events = handler.events.subscribe();
        let result = result_json(mission_set_project(&handler, id.into(), project_id).unwrap());
        assert_eq!(result, serde_json::json!(before));
        let conn = handler.db.get().unwrap();
        assert_eq!(repo::node::list(&conn).unwrap(), nodes);
        assert!(events.try_recv().is_err());
    }
}

#[test]
fn mission_set_project_rejects_missing_mission_node_or_project() {
    let handler = test_core();
    let node = seed_project_mission(&handler, "no-node", None);
    seed_project_mission(&handler, "valid", None);
    {
        let conn = handler.db.get().unwrap();
        repo::node::delete(&conn, &node.id).unwrap();
    }
    let mut events = handler.events.subscribe();
    for (id, project_id, message) in [
        ("missing", None, "mission not found: missing"),
        ("no-node", None, "mission node not found: no-node"),
        (
            "valid",
            Some("missing".into()),
            "project not found: missing",
        ),
    ] {
        let error = mission_set_project(&handler, id.into(), project_id).unwrap_err();

        assert_eq!(error.to_string(), message);
    }
    let conn = handler.db.get().unwrap();
    assert!(repo::node::find_by_ref(&conn, NodeType::Mission, "valid")
        .unwrap()
        .unwrap()
        .parent_id
        .is_none());
    assert!(events.try_recv().is_err());
}

use super::*;
use crate::test_support::*;
use runner_core::protocol::mission::PostMessageInput;
use std::time::{Duration, Instant};

struct Fixture {
    state: AppCore,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = test_core_in(dir.path().into());
        {
            let conn = state.db.get().unwrap();
            conn.execute_batch("INSERT INTO crews (id,name,created_at,updated_at) VALUES ('c','Team','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z'); INSERT INTO missions (id,crew_id,title,status,started_at) VALUES ('m','c','Test mission','running','2026-10-01T00:00:00Z');").unwrap();
            for (id, handle, lead) in [("r0", "lead", true), ("r1", "coder", false)] {
                insert_test_role(&conn, id, handle, "shell", "fixture-shell");
                insert_test_slot(&conn, id, "c", id, handle, i64::from(!lead), lead);
                let mut row = test_session_row(&format!("session-{id}"), SessionStatus::Stopped);
                row.mission_id = Some("m".into());
                row.slot_id = Some(id.into());
                row.role_id = Some(id.into());
                insert_session_row(&conn, &row).unwrap();
            }
            roster::load(
                &conn,
                &state.app_data_dir,
                &super::super::get(&conn, "m").unwrap(),
            )
            .unwrap();
        }
        crate::daemon::block_on(super::super::ensure_mission_router_mounted(&state, "m")).unwrap();
        Self { state, _dir: dir }
    }

    fn add(&self, handle: Option<&str>) -> Result<MissionRoleChange> {
        add_role(
            &self.state,
            MissionAddRoleInput {
                mission_id: "m".into(),
                role_id: "r1".into(),
                handle: handle.map(str::to_owned),
                runtime: None,
                model: Some("test-model".into()),
                effort: Some("high".into()),
                speed: None,
                task: Some("tiny task".into()),
            },
        )
    }

    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.state.sessions.has_pending_mission_spawns("m") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!self.state.sessions.has_pending_mission_spawns("m"));
    }

    fn events(&self) -> Vec<runner_core::Event> {
        super::super::read_events(&self.state.app_data_dir, &self.state.db.get().unwrap(), "m")
            .unwrap()
    }

    fn post(&self, target: Option<&str>) -> Result<runner_core::Event> {
        crate::daemon::block_on(super::super::mission_post_impl(
            &self.state,
            PostMessageInput {
                mission_id: "m".into(),
                from: None,
                to: target.map(str::to_owned).into(),
                text: "message".into(),
            },
        ))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.settle();
        self.state.buses.unmount("m");
        self.state.routers.unregister("m");
    }
}

#[test]
fn add_and_remove_keep_roster_log_router_and_history_across_remount() {
    let fixture = Fixture::new();
    fixture.post(None).unwrap();
    let added = fixture.add(None).unwrap();
    assert_eq!(added.handle, "coder-2");
    fixture.post(Some(&added.handle)).unwrap();
    fixture.settle();
    let events = fixture.events();
    let join = events
        .iter()
        .position(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "role_joined")
        })
        .unwrap();
    assert_eq!(events[join].from, "human");
    assert_eq!(events[join].payload["session_id"], added.session_id);
    assert_eq!(events[join].payload["lead"], false);
    assert_eq!(events[join].payload["runtime"], "shell");
    assert_eq!(events[join].payload["model"], "test-model");
    assert_eq!(events[join].payload["effort"], "high");
    assert_eq!(events[join].payload["speed"], serde_json::Value::Null);
    // The mounted router can append its own events between add_role's writes,
    // so the notice and task are found in order rather than at fixed offsets.
    let join_notice = join
        + events[join..]
            .iter()
            .position(|event| event.from == "runner")
            .unwrap();
    assert!(events[join_notice].payload["text"]
        .as_str()
        .unwrap()
        .contains("@lead (lead), @coder and @coder-2"));
    let task = join_notice
        + events[join_notice..]
            .iter()
            .position(|event| event.payload["text"] == "tiny task")
            .unwrap();
    assert_eq!(events[task].from, "human");
    assert_eq!(
        events[task].recipients().first().map(String::as_str),
        Some("coder-2")
    );
    {
        let conn = fixture.state.db.get().unwrap();
        let row = repo::session::get_row(&conn, &added.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Crashed);
        assert_eq!(
            row.launch_definition.unwrap().model.as_deref(),
            Some("test-model")
        );
    }
    for removing in [false, true] {
        if removing {
            remove_role(&fixture.state, "m", &added.handle).unwrap();
            assert!(fixture
                .post(Some(&added.handle))
                .unwrap_err()
                .to_string()
                .contains("valid handles: @lead, @coder"));
            let sessions = session::session_list(&fixture.state, "m").unwrap();
            assert!(!sessions
                .iter()
                .any(|session| session.session.id == added.session_id));
            assert!(
                repo::session::get_row(&fixture.state.db.get().unwrap(), &added.session_id)
                    .unwrap()
                    .is_some()
            );
            assert!(remove_role(&fixture.state, "m", &added.handle)
                .unwrap_err()
                .to_string()
                .contains("already removed"));
            assert!(fixture
                .add(Some(&added.handle))
                .unwrap_err()
                .to_string()
                .contains("isn't reused"));
        }
        fixture.state.buses.unmount("m");
        fixture.state.routers.unregister("m");
        crate::daemon::block_on(super::super::ensure_mission_router_mounted(
            &fixture.state,
            "m",
        ))
        .unwrap();
        let router = fixture.state.routers.get("m").unwrap();
        assert_eq!(
            router
                .launch()
                .roster()
                .iter()
                .any(|slot| slot.handle() == added.handle),
            !removing
        );
    }
    let events = fixture.events();
    let leave = events
        .iter()
        .position(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "role_removed")
        })
        .unwrap();
    assert_eq!(events[leave].payload["session_id"], added.session_id);
    let leave_notice = leave
        + events[leave..]
            .iter()
            .position(|event| event.from == "runner")
            .unwrap();
    assert!(events[leave_notice].payload["text"]
        .as_str()
        .unwrap()
        .starts_with("@coder-2 left the mission."));
    assert!(fixture.post(None).is_ok());
    let next = fixture.add(None).unwrap();
    assert_eq!(next.handle, "coder-3");
}

#[test]
fn refuse_pending_spawns_lead_and_bad_handles_without_changes() {
    let fixture = Fixture::new();
    let cancel = fixture.state.sessions.register_pending_mission_cancel("m");
    assert!(fixture
        .add(None)
        .unwrap_err()
        .to_string()
        .contains("still starting"));
    assert!(remove_role(&fixture.state, "m", "coder")
        .unwrap_err()
        .to_string()
        .contains("still starting"));
    fixture
        .state
        .sessions
        .drop_pending_mission_cancel("m", &cancel);
    assert!(remove_role(&fixture.state, "m", "lead")
        .unwrap_err()
        .to_string()
        .contains("is the lead"));
    assert!(remove_role(&fixture.state, "m", "unknown")
        .unwrap_err()
        .to_string()
        .contains("not in this mission"));
    assert!(fixture
        .add(Some("coder"))
        .unwrap_err()
        .to_string()
        .contains("already used in this mission"));
    assert!(fixture.add(Some("Bad handle")).is_err());
    for (role_id, runtime) in [("unknown", None), ("r1", Some(runner_core::Runtime::Shell))] {
        assert!(add_role(
            &fixture.state,
            MissionAddRoleInput {
                mission_id: "m".into(),
                role_id: role_id.into(),
                handle: None,
                runtime,
                model: None,
                effort: None,
                speed: None,
                task: None,
            }
        )
        .is_err());
        assert!(!fixture.state.sessions.has_pending_mission_spawns("m"));
    }
    fixture
        .state
        .db
        .get()
        .unwrap()
        .execute("UPDATE missions SET archived_at=started_at", [])
        .unwrap();
    assert!(fixture
        .add(None)
        .unwrap_err()
        .to_string()
        .contains("Test mission” is not running"));
    assert!(!fixture.state.sessions.has_pending_mission_spawns("m"));
    assert!(!fixture.events().iter().any(|event| event
        .signal_type
        .as_ref()
        .is_some_and(|kind| kind.as_str() == "role_joined")));
}

#[cfg(unix)]
#[test]
fn failed_roster_write_rolls_back_join_session_and_router() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let dir = event_log::mission_dir(&fixture.state.app_data_dir, Some("c"), "m");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = fixture.add(None);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(session::session_list(&fixture.state, "m").unwrap().len(), 2);
    let events = fixture.events();
    let joins: Vec<_> = events
        .iter()
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|kind| matches!(kind.as_str(), "role_joined" | "role_removed"))
        })
        .collect();
    assert_eq!(joins.len(), 2);
    assert_eq!(
        joins[0].signal_type.as_ref().unwrap().as_str(),
        "role_joined"
    );
    assert_eq!(
        joins[1].signal_type.as_ref().unwrap().as_str(),
        "role_removed"
    );
    assert!(repo::session::get_row(
        &fixture.state.db.get().unwrap(),
        joins[0].payload["session_id"].as_str().unwrap()
    )
    .unwrap()
    .is_none());
    assert_eq!(
        fixture
            .state
            .routers
            .get("m")
            .unwrap()
            .launch()
            .roster()
            .len(),
        2
    );
}

#[test]
fn cancellation_after_checks_keeps_newcomer_stopped_and_visible() {
    let fixture = Fixture::new();
    let mission = checked_mission(&fixture.state, "m").unwrap();
    let cancel = fixture.state.sessions.register_pending_mission_cancel("m");
    fixture.state.sessions.cancel_pending_mission_spawns("m");
    let change = add_checked(
        &fixture.state,
        &mission,
        MissionAddRoleInput {
            mission_id: "m".into(),
            role_id: "r1".into(),
            handle: None,
            runtime: None,
            model: None,
            effort: None,
            speed: None,
            task: None,
        },
        cancel,
    )
    .unwrap();
    fixture.settle();
    let sessions = session::session_list(&fixture.state, "m").unwrap();
    let newcomer = sessions
        .iter()
        .find(|session| session.session.id == change.session_id)
        .unwrap();
    assert_eq!(newcomer.session.status, SessionStatus::Stopped);
    assert!(fixture.state.sessions.live_session_ids().is_empty());
    assert!(fixture.post(Some(&change.handle)).is_ok());
}

#[test]
fn handles_are_scoped_to_a_mission_and_serialized_adds_stay_unique() {
    let fixture = Fixture::new();
    {
        let conn = fixture.state.db.get().unwrap();
        conn.execute("INSERT INTO missions (id,crew_id,title,status,started_at) VALUES ('m2','c','Second mission','running','2026-10-01T00:00:00Z')", []).unwrap();
    }
    let first = fixture.add(None).unwrap();
    fixture.settle();
    let second = add_role(
        &fixture.state,
        MissionAddRoleInput {
            mission_id: "m2".into(),
            role_id: "r1".into(),
            handle: None,
            runtime: None,
            model: None,
            effort: None,
            speed: None,
            task: None,
        },
    )
    .unwrap();
    assert_eq!(first.handle, second.handle);
    let third = fixture.add(None).unwrap();
    assert_eq!(third.handle, "coder-3");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.state.sessions.has_pending_mission_spawns("m2") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!fixture.state.sessions.has_pending_mission_spawns("m2"));
    fixture.state.buses.unmount("m2");
    fixture.state.routers.unregister("m2");
}

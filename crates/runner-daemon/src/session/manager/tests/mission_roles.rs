use super::*;
use crate::AppCore;
use runner_core::protocol::mission::MissionAddRoleInput;

#[test]
fn added_slot_records_overrides_resumes_restarts_and_removal_stops_it() {
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    let mut core = crate::test_support::test_core_in(data.path().into());
    core.sessions = manager.clone();
    {
        let conn = core.db.get().unwrap();
        conn.execute_batch("INSERT INTO crews (id,name,created_at,updated_at) VALUES ('c','Team','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z'); INSERT INTO missions (id,crew_id,title,status,started_at) VALUES ('m','c','Test mission','running','2026-10-01T00:00:00Z');").unwrap();
        crate::test_support::insert_test_role(&conn, "lead", "lead", "shell", "fixture-shell");
        crate::test_support::insert_test_slot(&conn, "lead", "c", "lead", "lead", 0, true);
        let mut lead_row = crate::test_support::test_session_row(
            "lead-session",
            crate::model::SessionStatus::Stopped,
        );
        lead_row.mission_id = Some("m".into());
        lead_row.slot_id = Some("lead".into());
        lead_row.role_id = Some("lead".into());
        crate::test_support::insert_session_row(&conn, &lead_row).unwrap();
        crate::test_support::insert_test_role(&conn, "added", "coder", "codex", "fixture-codex");
        conn.execute("UPDATE roles SET system_prompt='ORIGINAL_BRIEF',model='role-model',effort='high' WHERE id='added'",[]).unwrap();
    }
    let change = crate::ops::mission::roles::add_role(
        &core,
        MissionAddRoleInput {
            mission_id: "m".into(),
            role_id: "added".into(),
            handle: None,
            runtime: Some(Runtime::Codex),
            model: Some("override-model".into()),
            effort: Some("xhigh".into()),
            speed: Some(crate::model::CodexSpeed::Fast),
            task: Some("small task".into()),
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while manager.has_pending_mission_spawns("m") && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!manager.has_pending_mission_spawns("m"));
    assert!(manager.live_session_ids().contains(&change.session_id));
    let initial = crate::repo::session::get_row(&core.db.get().unwrap(), &change.session_id)
        .unwrap()
        .unwrap();
    let definition = initial.launch_definition.unwrap();
    assert_eq!(definition.model.as_deref(), Some("override-model"));
    assert_eq!(definition.effort.as_deref(), Some("xhigh"));
    assert_eq!(definition.codex_speed, Some(crate::model::CodexSpeed::Fast));
    let log = open_mission_event_log(data.path(), Some("c"), "m").unwrap();
    let (entries, _) = log.read_from_lossy(0).unwrap();
    let joined = entries
        .iter()
        .find(|entry| {
            entry
                .event
                .signal_type
                .as_ref()
                .is_some_and(|kind| kind.as_str() == "role_joined")
        })
        .unwrap();
    assert_eq!(
        joined.event.payload["model"].as_str(),
        definition.model.as_deref()
    );
    assert_eq!(
        joined.event.payload["effort"].as_str(),
        definition.effort.as_deref()
    );
    assert_eq!(
        joined.event.payload["speed"],
        serde_json::to_value(definition.codex_speed).unwrap()
    );
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg.contains("ORIGINAL_BRIEF") && arg.contains("team can change")));
    let roster = crate::ops::mission::mission_roster(&core, "m").unwrap();
    let slot = &roster
        .iter()
        .find(|member| member.slot.slot_handle == change.handle)
        .unwrap()
        .slot;
    assert_eq!(slot.model_override.as_deref(), Some("override-model"));
    assert_eq!(slot.runtime_override.as_deref(), Some("codex"));
    core.db.get().unwrap().execute("UPDATE roles SET system_prompt='EDITED_BRIEF',model='edited-model',effort='medium' WHERE id='added'",[]).unwrap();
    manager.kill(&change.session_id).unwrap();
    crate::ops::session::session_resume(&core, &change.session_id, None, None).unwrap();
    let resumed = crate::repo::session::get_row(&core.db.get().unwrap(), &change.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(resumed.launch_definition.unwrap(), definition);
    crate::ops::session::session_restart(&core, &change.session_id, None, None).unwrap();
    let restarted = crate::repo::session::get_row(&core.db.get().unwrap(), &change.session_id)
        .unwrap()
        .unwrap();
    let definition = restarted.launch_definition.unwrap();
    assert_eq!(definition.system_prompt.as_deref(), Some("EDITED_BRIEF"));
    assert_eq!(definition.model.as_deref(), Some("override-model"));
    assert_eq!(definition.effort.as_deref(), Some("xhigh"));
    assert_eq!(definition.codex_speed, Some(crate::model::CodexSpeed::Fast));
    let count = fake.spawn_count();
    crate::ops::mission::roles::remove_role(&core, "m", &change.handle).unwrap();
    assert!(manager.live_session_ids().is_empty());
    let removed = crate::repo::session::get_row(&core.db.get().unwrap(), &change.session_id)
        .unwrap()
        .unwrap();
    assert!(removed.archived_at.is_none());
    assert_eq!(removed.status, crate::model::SessionStatus::Stopped);
    let error = crate::ops::session::session_resume(&core, &change.session_id, None, None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("@coder was removed from this mission"),
        "{error}"
    );
    assert_eq!(fake.spawn_count(), count);
    core.buses.unmount("m");
    core.routers.unregister("m");
    crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
        &core, "m",
    ))
    .unwrap();
    let listed = crate::ops::session::session_list(&core, "m").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session.id, "lead-session");
    let resumed = crate::ops::mission::mission_resume(&core, "m").unwrap();
    assert_eq!(resumed.resumed_session_ids, vec!["lead-session"]);
    assert_eq!(fake.spawn_count(), count + 1);
    manager.kill("lead-session").unwrap();
    let error = crate::ops::role::delete(&mut core.db.get().unwrap(), "added")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unarchived missions"), "{error}");
    core.buses.unmount("m");
    core.routers.unregister("m");
}

fn removal_race_fixture() -> (tempfile::TempDir, AppCore, Arc<FakeRuntime>) {
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mut core = crate::test_support::test_core_in(data.path().into());
    core.sessions = mgr_with_fake(None, fake.clone());
    {
        let conn = core.db.get().unwrap();
        conn.execute_batch("INSERT INTO crews (id,name,created_at,updated_at) VALUES ('c','Team','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z'); INSERT INTO missions (id,crew_id,title,status,started_at) VALUES ('m','c','Test mission','running','2026-10-01T00:00:00Z');").unwrap();
        for (handle, lead) in [("lead", true), ("worker", false)] {
            crate::test_support::insert_test_role(&conn, handle, handle, "shell", "fixture-shell");
            crate::test_support::insert_test_slot(
                &conn,
                handle,
                "c",
                handle,
                handle,
                i64::from(!lead),
                lead,
            );
            let mut row =
                crate::test_support::test_session_row(handle, crate::model::SessionStatus::Stopped);
            row.mission_id = Some("m".into());
            row.slot_id = Some(handle.into());
            row.role_id = Some(handle.into());
            crate::test_support::insert_session_row(&conn, &row).unwrap();
        }
    }
    crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
        &core, "m",
    ))
    .unwrap();
    (data, core, fake)
}

fn run_worker_lifecycle(core: &AppCore, restart: bool) -> Result<SpawnedSession> {
    if restart {
        crate::ops::session::session_restart(core, "worker", None, None)
    } else {
        crate::ops::session::session_resume(core, "worker", None, None)
    }
}

#[test]
fn removal_refuses_resume_and_restart_during_spawn_then_stops_the_completed_spawn() {
    for restart in [false, true] {
        let (_data, core, fake) = removal_race_fixture();
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
        let removal = thread::scope(|scope| {
            let task = scope.spawn(|| run_worker_lifecycle(&core, restart));
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            let removal = crate::ops::mission::roles::remove_role(&core, "m", "worker");
            release.send(()).unwrap();
            task.join().unwrap().unwrap();
            removal
        });
        let error = removal.unwrap_err().to_string();
        assert!(
            error.contains("@worker is being resumed or restarted"),
            "{error}"
        );
        assert!(core.sessions.live_session_ids().contains(&"worker".into()));
        assert!(crate::ops::mission::mission_roster(&core, "m")
            .unwrap()
            .iter()
            .any(|member| member.slot.slot_handle == "worker"));
        assert!(fake.stops.lock().unwrap().is_empty());
        *fake.spawn_hook.lock().unwrap() = None;
        crate::ops::mission::roles::remove_role(&core, "m", "worker").unwrap();
        assert!(core.sessions.live_session_ids().is_empty());
        for fresh in [false, true] {
            let error = run_worker_lifecycle(&core, fresh).unwrap_err().to_string();
            assert!(
                error.contains("@worker was removed from this mission"),
                "{error}"
            );
        }
        assert_eq!(fake.spawn_count(), 1);
        core.buses.unmount("m");
        core.routers.unregister("m");
    }
}

#[test]
fn removal_claim_excludes_resume_and_restart_from_stop_through_publication() {
    let (_data, core, fake) = removal_race_fixture();
    run_worker_lifecycle(&core, false).unwrap();
    let (entered, release) = fake.arm_stop_gate();
    let errors = thread::scope(|scope| {
        let removal = scope.spawn(|| crate::ops::mission::roles::remove_role(&core, "m", "worker"));
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let results = [
            run_worker_lifecycle(&core, false),
            run_worker_lifecycle(&core, true),
        ];
        release.send(()).unwrap();
        removal.join().unwrap().unwrap();
        results
    });
    for result in errors {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is already being resumed"));
    }
    assert!(core.sessions.live_session_ids().is_empty());
    assert_eq!(
        crate::ops::mission::mission_roster(&core, "m")
            .unwrap()
            .len(),
        1
    );
    for restart in [false, true] {
        let error = run_worker_lifecycle(&core, restart)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("@worker was removed from this mission"),
            "{error}"
        );
    }
    assert_eq!(fake.spawn_count(), 1);
    let stops = fake.stops.lock().unwrap();
    assert!(!stops.is_empty());
    assert!(stops.iter().all(|id| id == "worker"));
    drop(stops);
    core.buses.unmount("m");
    core.routers.unregister("m");
}

#[test]
fn empty_mission_mounts_adds_workers_and_remounts_without_a_crew() {
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mut core = crate::test_support::test_core_in(data.path().into());
    core.sessions = mgr_with_fake(None, fake.clone());
    let out = crate::daemon::block_on(crate::ops::mission::mission_start_impl_with_size(
        &core,
        runner_core::protocol::MissionStart {
            crew_id: None,
            scope: runner_core::protocol::ProjectScope::Root,
            title: "Empty test".into(),
            goal_override: Some("SHARED_GOAL".into()),
            cwd: Some(data.path().to_string_lossy().into_owned()),
        },
        None,
    ))
    .unwrap();
    let id = &out.mission.id;
    assert!(out.mission.crew_id.is_none());
    assert_eq!(fake.spawn_count(), 0);
    assert!(core.routers.get(id).unwrap().lead_handle().is_none());
    assert!(core.buses.get(id).is_some());
    let dir = runner_core::event_log::mission_dir(data.path(), None, id);
    assert_eq!(dir, data.path().join("missions").join(id));
    let roster: runner_core::mission_roster::MissionRoster =
        serde_json::from_slice(&std::fs::read(dir.join("roster.json")).unwrap()).unwrap();
    assert!(roster.slots.is_empty());
    assert!(roster.crew_name.is_empty());
    assert!(roster.crew_addendum.is_none());
    let log = runner_core::EventLog::open(&dir).unwrap();
    let events = log.read_from(0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].event.signal_type.as_ref().unwrap().as_str(),
        "mission_start"
    );
    assert_eq!(
        events[1].event.signal_type.as_ref().unwrap().as_str(),
        "mission_goal"
    );
    assert!(events.iter().all(|entry| serde_json::to_value(&entry.event)
        .unwrap()
        .get("crew_id")
        .is_none()));
    core.buses.unmount(id);
    core.routers.unregister(id);
    crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
        &core, id,
    ))
    .unwrap();
    assert!(core.routers.get(id).unwrap().lead_handle().is_none());
    for role in ["one", "two"] {
        crate::test_support::insert_test_role(
            &core.db.get().unwrap(),
            role,
            role,
            "shell",
            "fixture-shell",
        );
        let change = crate::ops::mission::roles::add_role(
            &core,
            MissionAddRoleInput {
                mission_id: id.clone(),
                role_id: role.into(),
                handle: None,
                runtime: None,
                model: None,
                effort: None,
                speed: None,
                task: Some(format!("TASK_{role}")),
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while core.sessions.has_pending_mission_spawns(id) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!core.sessions.has_pending_mission_spawns(id));
        assert!(core
            .sessions
            .live_session_ids()
            .contains(&change.session_id));
        assert!(!fake
            .last_spawn_spec()
            .unwrap()
            .env
            .contains_key("RUNNER_CREW_ID"));
        let shim =
            std::fs::read_to_string(dir.join("shims").join(role).join("bin/runner")).unwrap();
        assert!(shim.contains("unset RUNNER_CREW_ID"));
        assert!(!shim.contains("export RUNNER_CREW_ID"));
        assert!(log.read_from(0).unwrap().iter().any(|entry| entry
            .event
            .recipients()
            .first()
            .map(String::as_str)
            == Some(role)
            && entry.event.payload["text"] == format!("TASK_{role}")));
    }
    let status = crate::ops::mission::feed::mission_status(&core, id).unwrap();
    assert!(status.crew.is_none());
    assert_eq!(status.sessions.len(), 2);
    for (from, to) in [("one", Some("two")), ("two", Some("one")), ("one", None)] {
        crate::daemon::block_on(crate::ops::mission::mission_post_impl(
            &core,
            runner_core::protocol::PostMessageInput {
                mission_id: id.clone(),
                from: Some(from.into()),
                to: to.map(str::to_owned).into(),
                text: "Hello team".into(),
            },
        ))
        .unwrap();
    }
    core.buses.unmount(id);
    core.routers.unregister(id);
    crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
        &core, id,
    ))
    .unwrap();
    assert!(core.routers.get(id).unwrap().lead_handle().is_none());
    assert_eq!(
        crate::ops::mission::mission_roster(&core, id)
            .unwrap()
            .len(),
        2
    );
    for session in status.sessions {
        core.sessions.kill(&session.session.id).unwrap();
    }
    core.buses.unmount(id);
    core.routers.unregister(id);
}

#[test]
fn deleting_a_crew_keeps_its_mission_roster_status_and_add_role() {
    let (_data, core, fake) = removal_race_fixture();
    crate::ops::mission::mission_resume(&core, "m").unwrap();
    let before = core.sessions.live_session_ids();
    crate::ops::crew::delete(&mut core.db.get().unwrap(), "c").unwrap();
    assert_eq!(core.sessions.live_session_ids(), before);
    let status = crate::ops::mission::feed::mission_status(&core, "m").unwrap();
    assert!(status.crew.is_none());
    assert_eq!(status.mission.crew_id.as_deref(), Some("c"));
    assert_eq!(status.sessions.len(), 2);
    crate::daemon::block_on(crate::ops::mission::mission_post_impl(
        &core,
        runner_core::protocol::PostMessageInput {
            mission_id: "m".into(),
            from: Some("worker".into()),
            to: Some("lead".into()).into(),
            text: "still here".into(),
        },
    ))
    .unwrap();
    let added = crate::ops::mission::roles::add_role(
        &core,
        MissionAddRoleInput {
            mission_id: "m".into(),
            role_id: "worker".into(),
            handle: None,
            runtime: None,
            model: None,
            effort: None,
            speed: None,
            task: None,
        },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while core.sessions.has_pending_mission_spawns("m") && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!core.sessions.has_pending_mission_spawns("m"));
    assert_eq!(fake.spawn_count(), 3);
    assert!(core.sessions.live_session_ids().contains(&added.session_id));
    core.buses.unmount("m");
    core.routers.unregister("m");
    crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
        &core, "m",
    ))
    .unwrap();
    assert_eq!(core.routers.get("m").unwrap().lead_handle(), Some("lead"));
    for id in core.sessions.live_session_ids() {
        core.sessions.kill(&id).unwrap();
    }
    core.buses.unmount("m");
    core.routers.unregister("m");
}

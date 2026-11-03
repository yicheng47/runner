use super::*;

#[test]
fn removed_mission_slot_resumes_with_its_frozen_identity() {
    for lead in [false, true] {
        let (pool, data, id) = slot_respawn_fixture("shell", lead);
        let mission = {
            let mut conn = pool.get().unwrap();
            let row = crate::repo::session::get_row(&conn, &id).unwrap().unwrap();
            let mission =
                crate::ops::mission::get(&conn, row.mission_id.as_deref().unwrap()).unwrap();
            crate::ops::mission::roster::load(&conn, data.path(), &mission).unwrap();
            let path = runner_core::event_log::mission_dir(
                data.path(),
                mission.crew_id.as_deref(),
                &mission.id,
            )
            .join("roster.json");
            let mut legacy: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            legacy["slots"][0]["lead"] = lead.into();
            std::fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
            crate::ops::slot::delete(&mut conn, row.slot_id.as_deref().unwrap()).unwrap();
            conn.execute(
                "UPDATE crews SET name='Edited', system_prompt_addendum='EDITED_RULES'",
                [],
            )
            .unwrap();
            mission
        };
        let fake = fake_runtime();
        let mgr = mgr_with_fake(Some("/bin/sh".into()), fake.clone());
        let out = mgr
            .resume(&id, None, None, data.path(), pool.clone(), capture())
            .unwrap();
        assert_eq!(out.id, id);
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(
            spec.env.get("RUNNER_HANDLE").map(String::as_str),
            Some("slot")
        );
        assert_eq!(spec.env.get("RUNNER_MISSION_ID"), Some(&mission.id));
        let loaded =
            crate::ops::mission::roster::load(&pool.get().unwrap(), data.path(), &mission).unwrap();
        assert!(!loaded.slots[0].slot.lead);
        assert_eq!(loaded.file.crew_addendum.as_deref(), Some("TEAM_RULES"));
        mgr.kill(&id).unwrap();
    }
}

#[tokio::test]
async fn mission_start_spawns_the_snapshot_even_when_the_crew_changes() {
    let data = tempfile::tempdir().unwrap();
    let mut state = crate::test_support::test_core_in(data.path().into());
    let fake = fake_runtime();
    state.sessions = mgr_with_fake(Some("/bin/sh".into()), fake.clone());
    let crew_id = {
        let mut conn = state.db.get().unwrap();
        let crew = crate::ops::crew::create(
            &conn,
            crate::ops::crew::CreateCrewInput {
                name: "Original".into(),
                system_prompt_addendum: Some("ORIGINAL_RULES".into()),
            },
        )
        .unwrap();
        for handle in ["lead", "worker"] {
            let mut role = role("/bin/sh", &[]);
            role.handle = handle.into();
            insert_role_row(&conn, &role);
            crate::ops::slot::create(&mut conn, &crew.id, &role.id, handle, None, None).unwrap();
        }
        crew.id
    };
    let pool_for_hook = state.db.clone();
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        pool_for_hook.get().unwrap().execute("UPDATE slots SET slot_handle = 'edited-' || slot_handle WHERE slot_handle NOT LIKE 'edited-%'", []).unwrap();
    }));
    let out = crate::ops::mission::mission_start_impl(
        &state,
        runner_core::protocol::mission::StartMissionInput {
            crew_id: Some(crew_id),
            project_id: None,
            title: "Snapshot start".into(),
            goal_override: None,
            cwd: Some(data.path().to_string_lossy().into_owned()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .await
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.spawn_count() != 2 {
        assert!(Instant::now() < deadline, "snapshot slots did not spawn");
        std::thread::sleep(Duration::from_millis(10));
    }
    let spawns = fake.spawns.lock().unwrap();
    let handles = spawns
        .iter()
        .map(|s| s.spec.env["RUNNER_HANDLE"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(handles, vec!["lead", "worker"]);
    drop(spawns);
    assert_eq!(
        crate::ops::mission::mission_roster(&state, &out.mission.id)
            .unwrap()
            .iter()
            .map(|m| m.slot.slot_handle.as_str())
            .collect::<Vec<_>>(),
        vec!["lead", "worker"]
    );
    state
        .sessions
        .kill_all_for_mission(&out.mission.id)
        .unwrap();
    state.routers.unregister(&out.mission.id);
    state.buses.unmount(&out.mission.id);
}

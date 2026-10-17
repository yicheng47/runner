use std::{collections::HashSet, io::Write, path::Path};

use runner_core::{
    event_log,
    mission_roster::{MissionRoster, RosterSlot},
};
use rusqlite::Connection;

use crate::{
    error::{Error, Result},
    model::{Crew, Mission, Slot, SlotWithRole},
    ops::{crew, slot},
    repo,
};

pub struct LoadedRoster {
    pub file: MissionRoster,
    pub slots: Vec<SlotWithRole>,
}

pub fn snapshot(crew: Option<&Crew>, slots: &[SlotWithRole]) -> MissionRoster {
    MissionRoster {
        version: 2,
        crew_name: crew.map(|crew| crew.name.clone()).unwrap_or_default(),
        crew_addendum: crew.and_then(|crew| crew.system_prompt_addendum.clone()),
        slots: slots
            .iter()
            .map(|member| RosterSlot::from(&member.slot))
            .collect(),
    }
}

pub fn write(mission_dir: &Path, roster: &MissionRoster) -> Result<()> {
    std::fs::create_dir_all(mission_dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(mission_dir)?;
    tmp.write_all(&serde_json::to_vec(roster)?)?;
    tmp.flush()?;
    let destination = mission_dir.join("roster.json");
    let mut attempts = 0;
    loop {
        match tmp.persist(&destination) {
            Ok(_) => break,
            Err(error) => {
                if cfg!(windows) && attempts < 4 {
                    tmp = error.file;
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                } else {
                    return Err(Error::Io(error.error));
                }
            }
        }
    }
    Ok(())
}

pub fn load(conn: &Connection, app_data_dir: &Path, mission: &Mission) -> Result<LoadedRoster> {
    let dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let loaded = std::fs::read(dir.join("roster.json"))
        .map_err(Error::from)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).map_err(Error::from));
    let file: MissionRoster = match loaded {
        Ok(value) => serde_json::from_value(value)?,
        Err(error) => {
            if matches!(&error, Error::Io(io) if io.kind() != std::io::ErrorKind::NotFound) {
                return Err(error);
            }
            let Some(crew) = mission
                .crew_id
                .as_deref()
                .map(|id| repo::crew::get(conn, id))
                .transpose()?
                .flatten()
            else {
                return Err(error);
            };
            log::warn!(
                "mission {} roster unavailable ({error}); falling back to current crew slots",
                mission.id
            );
            let slots = slot::list(conn, &crew.id)?;
            let file = snapshot(Some(&crew), &slots);
            write(&dir, &file)?;
            file
        }
    };
    let mut slots = Vec::with_capacity(file.slots.len());
    for entry in &file.slots {
        let Some(role) = repo::role::get(conn, &entry.role_id)? else {
            log::warn!(
                "mission {} roster slot @{} references missing role {}; skipping",
                mission.id,
                entry.handle,
                entry.role_id
            );
            continue;
        };
        slots.push(SlotWithRole {
            slot: Slot {
                id: entry.id.clone(),
                crew_id: mission.crew_id.clone().unwrap_or_default(),
                role_id: entry.role_id.clone(),
                slot_handle: entry.handle.clone(),
                position: entry.position,
                lead: entry.lead,
                runtime_override: entry.runtime_override.clone(),
                model_override: entry.model_override.clone(),
                effort_override: entry.effort_override.clone(),
                codex_speed_override: entry.codex_speed_override,
                added_at: mission.started_at,
            },
            role,
        });
    }
    Ok(LoadedRoster { file, slots })
}

pub fn convert_all(conn: &Connection, app_data_dir: &Path) -> Result<()> {
    fn convert_one(conn: &Connection, app_data_dir: &Path, mission: &Mission) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct LegacyEntry {
            handle: String,
            lead: bool,
        }

        let dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), &mission.id);
        let bytes = match std::fs::read(dir.join("roster.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                load(conn, app_data_dir, mission)?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        if serde_json::from_slice::<MissionRoster>(&bytes).is_ok() {
            return Ok(());
        }
        let entries = match serde_json::from_slice::<Vec<LegacyEntry>>(&bytes) {
            Ok(entries) => entries,
            Err(_) => {
                load(conn, app_data_dir, mission)?;
                return Ok(());
            }
        };
        let crew_id = mission
            .crew_id
            .as_deref()
            .ok_or_else(|| Error::msg("legacy mission roster has no crew"))?;
        let crew = crew::get(conn, crew_id)?;
        let current = slot::list(conn, crew_id)?;
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for session in repo::session::roster_rows_for_mission(conn, &mission.id)? {
            let Some(id) = session.slot_id.as_deref() else {
                continue;
            };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let entry = if let Some(slot) = repo::slot::get(conn, id)? {
                RosterSlot::from(&slot)
            } else {
                let Some(role_id) = session.role_id else {
                    continue;
                };
                RosterSlot {
                    id: id.to_owned(),
                    handle: String::new(),
                    lead: false,
                    position: 0,
                    role_id,
                    runtime_override: session.agent_runtime,
                    model_override: session
                        .launch_definition
                        .as_ref()
                        .and_then(|definition| definition.model.clone()),
                    effort_override: session
                        .launch_definition
                        .as_ref()
                        .and_then(|definition| definition.effort.clone()),
                    codex_speed_override: session
                        .launch_definition
                        .as_ref()
                        .and_then(|definition| definition.codex_speed),
                }
            };
            candidates.push(entry);
        }
        // Reserve exact matches before assigning renamed/deleted slots by registration order.
        let mut used = HashSet::new();
        let matches: Vec<_> = entries
            .iter()
            .map(|entry| {
                let index = candidates
                    .iter()
                    .position(|slot| slot.handle == entry.handle);
                if let Some(index) = index {
                    used.insert(index);
                }
                index
            })
            .collect();
        let mut slots = Vec::new();
        for (position, (entry, matched)) in entries.into_iter().zip(matches).enumerate() {
            let current_slot = current.iter().find(|m| m.slot.slot_handle == entry.handle);
            // A failed spawn's original slot must not consume a renamed/deleted session.
            let unspawned = current_slot
                .filter(|m| !seen.contains(&m.slot.id) && m.slot.added_at <= mission.started_at);
            let mut slot = if let Some(index) = matched {
                used.insert(index);
                candidates[index].clone()
            } else if let Some(member) = unspawned {
                RosterSlot::from(&member.slot)
            } else if let Some(index) = (0..candidates.len()).find(|index| !used.contains(index)) {
                used.insert(index);
                candidates[index].clone()
            } else if let Some(member) = current_slot {
                RosterSlot::from(&member.slot)
            } else {
                log::warn!(
                    "mission {} legacy roster @{} has neither session nor crew slot; dropping",
                    mission.id,
                    entry.handle
                );
                continue;
            };
            slot.handle = entry.handle;
            slot.lead = entry.lead;
            slot.position = position as i64;
            slots.push(slot);
        }
        write(
            &dir,
            &MissionRoster {
                version: 2,
                crew_name: crew.name,
                crew_addendum: crew.system_prompt_addendum,
                slots,
            },
        )
    }

    for mission in repo::mission::list_all(conn)? {
        if let Err(error) = convert_one(conn, app_data_dir, &mission) {
            log::warn!("mission {} roster conversion failed: {error}", mission.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::CodexSpeed, ops::mission, test_support::*};
    use runner_core::protocol::mission::{PostMessageInput, PostSignalInput, StartMissionInput};

    fn seed(conn: &Connection, archived: bool) -> Mission {
        conn.execute_batch(
            "INSERT INTO crews (id, name, system_prompt_addendum, created_at, updated_at)
             VALUES ('c', 'Original crew', 'ORIGINAL_RULES', '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m', 'c', 'Original mission', 'running', '2026-10-01T00:00:00Z');",
        ).unwrap();
        if archived {
            conn.execute(
                "UPDATE missions SET status = 'completed', archived_at = started_at",
                [],
            )
            .unwrap();
        }
        for (id, handle, position) in [("s0", "lead", 0), ("s1", "worker", 1), ("s2", "other", 2)] {
            insert_test_role(conn, id, handle, "shell", "fixture-shell");
            insert_test_slot(conn, id, "c", id, handle, position, position == 0);
        }
        conn.execute("UPDATE slots SET added_at='2026-09-30T00:00:00Z'", [])
            .unwrap();
        mission::get(conn, "m").unwrap()
    }

    fn session(conn: &Connection, slot_id: &str, position: u32) {
        let mut row = test_session_row(
            &format!("session-{slot_id}"),
            crate::model::SessionStatus::Stopped,
        );
        row.mission_id = Some("m".into());
        row.slot_id = Some(slot_id.into());
        row.role_id = Some(slot_id.into());
        row.started_at = Some(
            chrono::DateTime::parse_from_rfc3339(&format!("2026-10-01T00:00:{position:02}Z"))
                .unwrap()
                .into(),
        );
        row.agent_runtime = Some("codex".into());
        row.launch_definition = Some(crate::test_support::session_definition("codex", "codex"));
        row.launch_definition.as_mut().unwrap().model = Some("session-model".into());
        row.launch_definition.as_mut().unwrap().effort = Some("high".into());
        row.launch_definition.as_mut().unwrap().codex_speed = Some(CodexSpeed::Fast);
        // Archived session rows must participate in the upgrade as well.
        row.archived_at = mission::get(conn, "m").unwrap().archived_at;
        repo::session::insert(conn, &row).unwrap();
    }

    #[test]
    fn start_snapshots_the_crew_and_all_slot_overrides() {
        let data = tempfile::tempdir().unwrap();
        let state = test_core_in(data.path().into());
        let mut conn = state.db.get().unwrap();
        let old = seed(&conn, false);
        repo::slot::set_runtime_override(&conn, "s1", Some("codex")).unwrap();
        repo::slot::set_model_override(&conn, "s1", Some("chosen-model")).unwrap();
        repo::slot::set_effort_override(&conn, "s1", Some("high")).unwrap();
        repo::slot::set_codex_speed_override(&conn, "s1", Some(CodexSpeed::Fast)).unwrap();
        let out = mission::start(
            &mut conn,
            data.path(),
            StartMissionInput {
                crew_id: old.crew_id,
                project_id: None,
                title: "New mission".into(),
                goal_override: None,
                cwd: None,
            },
            crate::router::runtime::MissionPermissionMode::Bypass,
        )
        .unwrap();
        let loaded = load(&conn, data.path(), &out.mission).unwrap();
        assert_eq!(loaded.file.version, 2);
        assert_eq!(loaded.file.crew_name, "Original crew");
        assert_eq!(loaded.file.crew_addendum.as_deref(), Some("ORIGINAL_RULES"));
        let expected = slot::list(&conn, "c").unwrap();
        assert_eq!(
            loaded.file.slots,
            expected
                .iter()
                .map(|m| RosterSlot::from(&m.slot))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            loaded.file.slots[1].codex_speed_override,
            Some(CodexSpeed::Fast)
        );
        drop(conn);
        let client = crate::daemon::InProcessTransport::client(state);
        assert_eq!(client.mission_roster(&out.mission.id).unwrap().len(), 3);
    }

    #[test]
    fn conversion_preserves_legacy_handles_for_running_and_archived_missions() {
        for archived in [false, true] {
            for case in [
                "untouched",
                "renamed",
                "deleted",
                "reused",
                "failed",
                "failed-renamed",
                "failed-deleted",
                "resumed-deleted",
                "reordered",
                "dropped",
            ] {
                let data = tempfile::tempdir().unwrap();
                let pool = crate::db::open_in_memory().unwrap();
                let conn = pool.get().unwrap();
                let mission = seed(&conn, archived);
                session(&conn, "s0", 0);
                if !matches!(case, "failed" | "failed-renamed" | "failed-deleted") {
                    session(&conn, "s1", 1);
                }
                session(&conn, "s2", 2);
                let mut legacy = serde_json::json!([
                    {"handle":"lead", "lead":true}, {"handle":"worker", "lead":false}, {"handle":"other", "lead":false}
                ]);
                if case == "renamed" {
                    conn.execute(
                        "UPDATE slots SET slot_handle='renamed', lead=1 WHERE id='s1'",
                        [],
                    )
                    .unwrap();
                    conn.execute("UPDATE slots SET lead=0 WHERE id='s0'", [])
                        .unwrap();
                }
                if matches!(case, "deleted" | "reused") {
                    repo::slot::delete(&conn, "s1").unwrap();
                    if case == "reused" {
                        insert_test_slot(&conn, "replacement", "c", "s2", "worker", 1, false);
                    }
                }
                if case == "failed-renamed" {
                    conn.execute("UPDATE slots SET slot_handle='new-other' WHERE id='s2'", [])
                        .unwrap();
                }
                if matches!(case, "failed-deleted" | "resumed-deleted") {
                    repo::slot::delete(&conn, "s2").unwrap();
                }
                if case == "resumed-deleted" {
                    repo::session::resume_in_place(
                        &conn,
                        "session-s1",
                        chrono::DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
                            .unwrap()
                            .into(),
                        None,
                        false,
                        (80, 24),
                        None,
                    )
                    .unwrap();
                    repo::slot::delete(&conn, "s1").unwrap();
                }
                if case == "reordered" {
                    // A renamed first entry must not steal the exact match of a later entry.
                    conn.execute("UPDATE slots SET slot_handle='new-other' WHERE id='s2'", [])
                        .unwrap();
                    legacy = serde_json::json!([
                        {"handle":"other", "lead":false}, {"handle":"lead", "lead":true}, {"handle":"worker", "lead":false}
                    ]);
                }
                if case == "dropped" {
                    legacy
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({"handle":"never-ran", "lead":false}));
                }
                let dir = event_log::mission_dir(data.path(), Some("c"), "m");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join("roster.json"),
                    serde_json::to_vec(&legacy).unwrap(),
                )
                .unwrap();
                convert_all(&conn, data.path()).unwrap();
                let result = load(&conn, data.path(), &mission).unwrap().file;
                assert_eq!(result.crew_name, "Original crew", "{case}");
                assert_eq!(result.crew_addendum.as_deref(), Some("ORIGINAL_RULES"));
                let expected = if case == "reordered" {
                    vec!["s2", "s0", "s1"]
                } else {
                    vec!["s0", "s1", "s2"]
                };
                assert_eq!(
                    result
                        .slots
                        .iter()
                        .map(|s| s.id.as_str())
                        .collect::<Vec<_>>(),
                    expected,
                    "{case}, archived={archived}"
                );
                assert_eq!(
                    result
                        .slots
                        .iter()
                        .map(|s| s.handle.as_str())
                        .collect::<Vec<_>>(),
                    if case == "reordered" {
                        vec!["other", "lead", "worker"]
                    } else {
                        vec!["lead", "worker", "other"]
                    }
                );
                assert_eq!(
                    result.slots.iter().map(|s| s.lead).collect::<Vec<_>>(),
                    if case == "reordered" {
                        vec![false, true, false]
                    } else {
                        vec![true, false, false]
                    }
                );
                assert_eq!(
                    result
                        .slots
                        .iter()
                        .map(|s| s.role_id.as_str())
                        .collect::<Vec<_>>(),
                    expected,
                    "roles: {case}, archived={archived}"
                );
                if matches!(case, "deleted" | "reused" | "resumed-deleted") {
                    assert_eq!(result.slots[1].role_id, "s1");
                    assert_eq!(result.slots[1].runtime_override.as_deref(), Some("codex"));
                    assert_eq!(
                        result.slots[1].model_override.as_deref(),
                        Some("session-model")
                    );
                    assert_eq!(result.slots[1].effort_override.as_deref(), Some("high"));
                    assert_eq!(result.slots[1].codex_speed_override, Some(CodexSpeed::Fast));
                }
                let before = std::fs::read(dir.join("roster.json")).unwrap();
                conn.execute("UPDATE crews SET name='Edited'", []).unwrap();
                convert_all(&conn, data.path()).unwrap();
                assert_eq!(std::fs::read(dir.join("roster.json")).unwrap(), before);
            }
        }
    }

    #[test]
    fn missing_and_invalid_json_files_fall_back_and_missing_roles_are_skipped() {
        for bytes in [None, Some(b"broken".as_slice())] {
            let data = tempfile::tempdir().unwrap();
            let state = test_core_in(data.path().into());
            let conn = state.db.get().unwrap();
            let mission = seed(&conn, false);
            if let Some(bytes) = bytes {
                let dir = event_log::mission_dir(data.path(), Some("c"), "m");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("roster.json"), bytes).unwrap();
            }
            convert_all(&conn, data.path()).unwrap();
            assert_eq!(load(&conn, data.path(), &mission).unwrap().slots.len(), 3);
            conn.execute(
                "UPDATE roles SET system_prompt='EDITED_ROLE' WHERE id='s0'",
                [],
            )
            .unwrap();
            assert_eq!(
                load(&conn, data.path(), &mission).unwrap().slots[0]
                    .role
                    .system_prompt
                    .as_deref(),
                Some("EDITED_ROLE")
            );
            repo::slot::delete(&conn, "s1").unwrap();
            repo::role::delete(&conn, "s1").unwrap();
            drop(conn);
            let roster = mission::mission_roster(&state, "m").unwrap();
            assert_eq!(
                roster
                    .iter()
                    .map(|m| m.slot.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["s0", "s2"]
            );
        }
        // Exercise the load fallback independently of the startup conversion.
        let data = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let mission = seed(&conn, false);
        load(&conn, data.path(), &mission).unwrap();
        let dir = event_log::mission_dir(data.path(), Some("c"), "m");
        std::fs::write(dir.join("roster.json"), b"bad-json").unwrap();
        conn.execute("UPDATE slots SET slot_handle='current' WHERE id='s1'", [])
            .unwrap();
        assert_eq!(
            load(&conn, data.path(), &mission).unwrap().file.slots[1].handle,
            "current"
        );
    }

    #[test]
    fn missing_and_invalid_json_cannot_fall_back_without_an_existing_crew() {
        for crew_id in [None, Some("deleted-crew".to_owned())] {
            for bytes in [None, Some(b"broken".as_slice())] {
                let data = tempfile::tempdir().unwrap();
                let pool = crate::db::open_in_memory().unwrap();
                let conn = pool.get().unwrap();
                let mut mission = seed(&conn, false);
                mission.crew_id = crew_id.clone();
                let dir =
                    event_log::mission_dir(data.path(), mission.crew_id.as_deref(), &mission.id);
                if let Some(bytes) = bytes {
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("roster.json"), bytes).unwrap();
                }
                let error = load(&conn, data.path(), &mission)
                    .err()
                    .expect("must not replace the missing roster");
                if let Some(bytes) = bytes {
                    assert!(matches!(error, Error::Json(_)));
                    assert_eq!(std::fs::read(dir.join("roster.json")).unwrap(), bytes);
                } else {
                    assert!(
                        matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound)
                    );
                    assert!(!dir.join("roster.json").exists());
                }
            }
        }
    }

    #[test]
    fn valid_json_with_an_unsupported_version_or_invalid_shape_is_preserved() {
        let data = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let mission = seed(&conn, false);
        let mut newer = snapshot(
            Some(&crew::get(&conn, "c").unwrap()),
            &slot::list(&conn, "c").unwrap(),
        );
        newer.version = 3;
        let dir = event_log::mission_dir(data.path(), Some("c"), "m");
        std::fs::create_dir_all(&dir).unwrap();
        for bytes in [
            serde_json::to_vec(&newer).unwrap(),
            br#"{"version":2}"#.to_vec(),
        ] {
            std::fs::write(dir.join("roster.json"), &bytes).unwrap();
            assert!(matches!(
                load(&conn, data.path(), &mission),
                Err(Error::Json(_))
            ));
            assert_eq!(std::fs::read(dir.join("roster.json")).unwrap(), bytes);
            convert_all(&conn, data.path()).unwrap();
            assert_eq!(std::fs::read(dir.join("roster.json")).unwrap(), bytes);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_not_found_read_errors_preserve_the_roster_path() {
        for startup in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let pool = crate::db::open_in_memory().unwrap();
            let conn = pool.get().unwrap();
            let mission = seed(&conn, false);
            let dir = event_log::mission_dir(data.path(), Some("c"), "m");
            let target = dir.join("unreadable-roster");
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("original"), b"preserve me").unwrap();
            let path = dir.join("roster.json");
            std::os::unix::fs::symlink(&target, &path).unwrap();
            if startup {
                convert_all(&conn, data.path()).unwrap();
            }
            assert!(
                matches!(load(&conn, data.path(), &mission), Err(Error::Io(error)) if error.kind() != std::io::ErrorKind::NotFound)
            );
            assert_eq!(std::fs::read_link(&path).unwrap(), target);
            assert_eq!(
                std::fs::read(target.join("original")).unwrap(),
                b"preserve me"
            );
        }
    }

    #[test]
    fn start_waits_for_a_wal_writer_before_reading_the_crew() {
        use std::{sync::mpsc, time::Duration};

        let data = tempfile::tempdir().unwrap();
        let pool = crate::db::open_pool(&data.path().join("runner.db")).unwrap();
        let mut conn = pool.get().unwrap();
        seed(&conn, false);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let start_pool = pool.clone();
        let start_dir = data.path().to_owned();
        let starter = std::thread::spawn(move || {
            let mut conn = start_pool.get().unwrap();
            ready_tx.send(()).unwrap();
            go_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = mission::start(
                &mut conn,
                &start_dir,
                StartMissionInput {
                    crew_id: Some("c".into()),
                    project_id: None,
                    title: "Concurrent start".into(),
                    goal_override: None,
                    cwd: None,
                },
                crate::router::runtime::MissionPermissionMode::Bypass,
            )
            .map_err(|error| error.to_string());
            done_tx.send(result).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("UPDATE crews SET name='Committed crew' WHERE id='c'", [])
            .unwrap();
        tx.execute(
            "UPDATE slots SET slot_handle='committed-worker' WHERE id='s1'",
            [],
        )
        .unwrap();
        go_tx.send(()).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(250)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        tx.commit().unwrap();
        let started = done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        starter.join().unwrap();
        let roster = load(&conn, data.path(), &started.mission).unwrap();
        assert_eq!(roster.file.crew_name, "Committed crew");
        assert_eq!(roster.file.slots[1].handle, "committed-worker");
    }

    #[test]
    fn one_failed_roster_does_not_stop_conversion_or_summary_listing() {
        let data = tempfile::tempdir().unwrap();
        let state = test_core_in(data.path().into());
        let conn = state.db.get().unwrap();
        seed(&conn, false);
        conn.execute("INSERT INTO missions (id, crew_id, title, status, started_at) VALUES ('bad', 'c', 'Bad', 'running', '2026-09-01T00:00:00Z')", []).unwrap();
        let dir = event_log::mission_dir(data.path(), Some("c"), "bad");
        std::fs::create_dir_all(dir.join("roster.json")).unwrap();
        convert_all(&conn, data.path()).unwrap();
        assert!(event_log::mission_dir(data.path(), Some("c"), "m")
            .join("roster.json")
            .is_file());
        drop(conn);
        let client = crate::daemon::InProcessTransport::client(state);
        for crew_id in [None, Some("c".into())] {
            let summaries = client.mission_list_summary_impl(crew_id).unwrap();
            assert_eq!(summaries.len(), 1);
            assert_eq!(summaries[0].mission.id, "m");
            assert_eq!(summaries[0].crew_name.as_deref(), Some("Original crew"));
        }
    }

    #[test]
    fn boot_converts_running_and_archived_rosters_before_mounting_routers() {
        let data = tempfile::tempdir().unwrap();
        {
            let pool = crate::db::open_pool(&data.path().join("runner.db")).unwrap();
            let conn = pool.get().unwrap();
            seed(&conn, false);
            session(&conn, "s0", 0);
            session(&conn, "s1", 1);
            session(&conn, "s2", 2);
            conn.execute("INSERT INTO missions (id, crew_id, title, status, started_at, archived_at) VALUES ('archived', 'c', 'Archived', 'completed', '2026-10-01T00:00:00Z', '2026-10-01T00:00:01Z')", []).unwrap();
            conn.execute("INSERT INTO sessions (id, mission_id, slot_id, role_id, status, started_at) VALUES ('archived-session', 'archived', 's0', 's0', 'stopped', '2026-10-01T00:00:00Z')", []).unwrap();
            for id in ["m", "archived"] {
                let dir = event_log::mission_dir(data.path(), Some("c"), id);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join("roster.json"),
                    br#"[{"handle":"lead","lead":true}]"#,
                )
                .unwrap();
            }
            conn.execute(
                "UPDATE slots SET slot_handle='edited-lead' WHERE id='s0'",
                [],
            )
            .unwrap();
        }
        let paths = runner_core::daemon_process::NativePaths::new(
            data.path().into(),
            data.path().join("logs"),
        );
        let core = crate::daemon::boot::boot(&paths, Vec::new(), false).unwrap();
        assert_eq!(core.routers.get("m").unwrap().lead_handle(), Some("lead"));
        assert!(core.routers.get("archived").is_none());
        for id in ["m", "archived"] {
            let file: MissionRoster = serde_json::from_slice(
                &std::fs::read(
                    event_log::mission_dir(data.path(), Some("c"), id).join("roster.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(file.slots[0].handle, "lead");
            assert_eq!(file.slots[0].id, "s0");
        }
        core.routers.unregister("m");
        core.buses.unmount("m");
    }

    #[tokio::test]
    async fn every_crew_edit_keeps_remounted_roster_and_message_validation() {
        let data = tempfile::tempdir().unwrap();
        let state = test_core_in(data.path().into());
        let mission = {
            let conn = state.db.get().unwrap();
            let mission = seed(&conn, false);
            session(&conn, "s0", 0);
            session(&conn, "s1", 1);
            session(&conn, "s2", 2);
            convert_all(&conn, data.path()).unwrap();
            mission
        };
        for edit in ["remove", "add", "rename", "lead", "override", "addendum"] {
            let mut conn = state.db.get().unwrap();
            match edit {
                "remove" => {
                    slot::delete(&mut conn, "s1").unwrap();
                }
                "add" => {
                    slot::create(&mut conn, "c", "s1", "new-worker", None, None).unwrap();
                }
                "rename" => {
                    conn.execute("UPDATE slots SET slot_handle='new-other' WHERE id='s2'", [])
                        .unwrap();
                }
                "lead" => {
                    slot::set_lead(&mut conn, "s2").unwrap();
                }
                "override" => {
                    conn.execute("UPDATE slots SET runtime_override='codex', model_override='different' WHERE id='s0'", []).unwrap();
                }
                "addendum" => {
                    conn.execute("UPDATE crews SET name='Edited crew', system_prompt_addendum='EDITED_RULES', updated_at='2026-10-02T00:00:00Z'", []).unwrap();
                }
                _ => unreachable!(),
            }
            drop(conn);
            state.routers.unregister("m");
            state.buses.unmount("m");
            mission::ensure_mission_router_mounted(&state, "m")
                .await
                .unwrap();
            let router = state.routers.get("m").unwrap();
            let mut events = state.events.subscribe();
            assert_eq!(router.lead_handle(), Some("lead"), "{edit}");
            assert_eq!(
                router
                    .launch()
                    .roster()
                    .iter()
                    .map(|s| s.handle().to_owned())
                    .collect::<Vec<_>>(),
                vec!["lead", "worker", "other"]
            );
            let log =
                event_log::EventLog::open(&event_log::mission_dir(data.path(), Some("c"), "m"))
                    .unwrap();
            let loaded = load(&state.db.get().unwrap(), data.path(), &mission).unwrap();
            let (_, body) = crate::router::prompt::member_prompt_channels(
                crate::runtimes::for_key("claude-code"),
                &loaded.slots[0],
                &loaded.slots,
                &loaded.file,
                &log,
            );
            let body = body.unwrap();
            assert!(body.contains("Original crew") && body.contains("ORIGINAL_RULES"));
            assert!(!body.contains("new-worker") && !body.contains("EDITED_RULES"));
            assert_eq!(
                mission::mission_roster(&state, "m").unwrap()[0]
                    .slot
                    .runtime_override,
                None
            );
            for handle in ["lead", "worker", "other"] {
                mission::mission_post_impl(
                    &state,
                    PostMessageInput {
                        mission_id: "m".into(),
                        text: "hello".into(),
                        from: Some(handle.into()),
                        to: Some("lead".into()).into(),
                    },
                )
                .await
                .unwrap();
                mission::mission_signal_impl(
                    &state,
                    PostSignalInput {
                        mission_id: "m".into(),
                        from: Some(handle.into()),
                        signal_type: "session_status".into(),
                        payload: serde_json::json!({"state":"idle"}),
                    },
                )
                .await
                .unwrap();
            }
            for handle in ["new-worker", "new-other"] {
                for from in [true, false] {
                    assert!(mission::mission_post_impl(
                        &state,
                        PostMessageInput {
                            mission_id: "m".into(),
                            text: "hello".into(),
                            from: from.then(|| handle.into()),
                            to: (!from).then(|| handle.into()).into()
                        }
                    )
                    .await
                    .is_err());
                }
            }
            let broadcast = mission::mission_post_impl(
                &state,
                PostMessageInput {
                    mission_id: "m".into(),
                    text: "broadcast".into(),
                    from: Some("lead".into()),
                    to: Default::default(),
                },
            )
            .await
            .unwrap();
            let barrier = mission::mission_signal_impl(
                &state,
                PostSignalInput {
                    mission_id: "m".into(),
                    from: Some("lead".into()),
                    signal_type: "session_status".into(),
                    payload: serde_json::json!({"state":"idle"}),
                },
            )
            .await
            .unwrap();
            let recipients = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut recipients = Vec::new();
                loop {
                    let event = events.recv().await.unwrap();
                    if event.name == "inbox/updated" && event.payload["last_id"] == broadcast.id {
                        recipients.push(event.payload["role_handle"].as_str().unwrap().to_owned());
                    }
                    if event.name == "event/appended" && event.payload["event"]["id"] == barrier.id
                    {
                        break;
                    }
                }
                recipients.sort();
                recipients
            })
            .await
            .unwrap();
            assert_eq!(
                recipients,
                vec!["other", "worker"],
                "bus roster after {edit}"
            );
            // A mapped stopped recipient queues delivery instead of reporting a missing handle.
            let warnings =
                mission::read_events(data.path(), &state.db.get().unwrap(), "m").unwrap();
            assert!(!warnings.iter().any(|event| event.payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("no live session for handle"))));
            let sessions = crate::ops::session::session_list(&state, "m").unwrap();
            assert_eq!(
                sessions
                    .iter()
                    .map(|s| s.handle.as_str())
                    .collect::<Vec<_>>(),
                vec!["lead", "worker", "other"]
            );
            let status = mission::feed::mission_status(&state, "m").unwrap();
            let current_name = if edit == "addendum" {
                "Edited crew"
            } else {
                "Original crew"
            };
            assert_eq!(status.crew.as_ref().unwrap().name, current_name);
            assert_eq!(
                status
                    .crew
                    .as_ref()
                    .unwrap()
                    .system_prompt_addendum
                    .as_deref(),
                Some(if edit == "addendum" {
                    "EDITED_RULES"
                } else {
                    "ORIGINAL_RULES"
                })
            );
            let summaries = mission::mission_list_summary_impl(&state, None)
                .await
                .unwrap();
            assert_eq!(summaries[0].crew_name.as_deref(), Some(current_name));
            if edit == "addendum" {
                assert!(
                    status.crew.as_ref().unwrap().updated_at
                        > status.crew.as_ref().unwrap().created_at
                );
            }
            let conn = state.db.get().unwrap();
            assert_eq!(
                load(&conn, data.path(), &mission).unwrap().file.slots[1].id,
                "s1"
            );
        }
        state.routers.unregister("m");
        state.buses.unmount("m");
    }

    #[test]
    fn role_delete_requires_archiving_the_mission_after_slot_removal() {
        let pool = crate::db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let mission = seed(&conn, false);
        session(&conn, "s1", 1);
        slot::delete(&mut conn, "s1").unwrap();
        // Even an archived session in an unarchived mission still protects its role.
        conn.execute("UPDATE sessions SET archived_at=started_at", [])
            .unwrap();
        let error = crate::ops::role::delete(&mut conn, "s1")
            .unwrap_err()
            .to_string();
        assert!(error.contains("role @worker is used by unarchived missions"));
        assert!(error.contains("Original mission (m)"));
        assert!(repo::session::get_row(&conn, "session-s1")
            .unwrap()
            .is_some());
        crew::delete(&mut conn, "c").unwrap();
        repo::mission::complete_and_archive_if_running(&conn, &mission.id, chrono::Utc::now())
            .unwrap();
        crate::ops::role::delete(&mut conn, "s1").unwrap();
        assert!(repo::role::get(&conn, "s1").unwrap().is_none());
    }
}

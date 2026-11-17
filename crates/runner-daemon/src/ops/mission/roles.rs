use std::collections::HashSet;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

use chrono::Utc;
use runner_core::{
    event_log,
    mission_roster::RosterSlot,
    protocol::mission::{MissionAddRoleInput, MissionRoleChange},
    slot_handle::{slot_handle_error, suggest_slot_handle},
    EventDraft, EventKind, EventLog, SignalType,
};

use crate::{
    error::{Error, Result},
    model::{Mission, MissionStatus, SessionStatus, Slot, SlotWithRole},
    ops::{role, session, slot},
    repo,
    session::{manager::SessionEvents, CompleteSpawnOutcome, PendingMissionSpawn},
    AppCore,
};

use super::roster;

fn mission_lock(state: &AppCore, mission_id: &str) -> Arc<Mutex<()>> {
    state
        .mission_roster_locks
        .lock()
        .unwrap()
        .entry(mission_id.to_owned())
        .or_default()
        .clone()
}

fn checked_mission(state: &AppCore, mission_id: &str) -> Result<Mission> {
    let mission = super::get(&*state.db.get()?, mission_id)?;
    if mission.status != MissionStatus::Running || mission.archived_at.is_some() {
        return Err(Error::msg(format!(
            "mission “{}” is not running",
            mission.title
        )));
    }
    if state.sessions.has_pending_mission_spawns(mission_id) {
        return Err(Error::msg(format!(
            "a session of mission “{}” is still starting; try again once it is up",
            mission.title
        )));
    }
    Ok(mission)
}

pub fn removed_handles(log: &EventLog) -> Result<HashSet<String>> {
    let (entries, _) = log.read_from_lossy(0)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let event = entry.event;
            (event.kind == EventKind::Signal
                && event.from == "human"
                && event
                    .signal_type
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == "role_removed"))
            .then(|| event.payload["handle"].as_str().map(str::to_owned))
            .flatten()
        })
        .collect())
}

pub(crate) fn join_handles(mut handles: Vec<String>) -> String {
    match handles.len() {
        0 => String::new(),
        1 => handles.pop().unwrap(),
        2 => format!("{} and {}", handles[0], handles[1]),
        _ => {
            let last = handles.pop().unwrap();
            format!("{} and {last}", handles.join(", "))
        }
    }
}

fn team(slots: &[RosterSlot]) -> String {
    join_handles(
        slots
            .iter()
            .map(|slot| format!("@{}", slot.handle))
            .collect(),
    )
}

fn leave(
    log: &EventLog,
    change: &MissionRoleChange,
    slot_id: &str,
    crew_id: Option<&str>,
) -> Result<()> {
    log.append(EventDraft::signal(crew_id, &change.mission_id, "human", SignalType::new("role_removed"),
        serde_json::json!({"handle": change.handle, "slot_id": slot_id, "session_id": change.session_id})))?;
    Ok(())
}

pub fn add_role(state: &AppCore, input: MissionAddRoleInput) -> Result<MissionRoleChange> {
    let lock = mission_lock(state, &input.mission_id);
    let _guard = lock.lock().unwrap();
    let mission = checked_mission(state, &input.mission_id)?;
    let cancel = state.sessions.register_pending_mission_cancel(&mission.id);
    let result = add_checked(state, &mission, input, Arc::clone(&cancel));
    if result.is_err() {
        state
            .sessions
            .drop_pending_mission_cancel(&mission.id, &cancel);
    }
    result
}

fn add_checked(
    state: &AppCore,
    mission: &Mission,
    input: MissionAddRoleInput,
    cancel: Arc<AtomicBool>,
) -> Result<MissionRoleChange> {
    let (mut loaded, role) = {
        let conn = state.db.get()?;
        (
            roster::load(&conn, &state.app_data_dir, mission)?,
            role::get(&conn, &input.role_id)?,
        )
    };
    let dir = event_log::mission_dir(&state.app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let log = EventLog::open(&dir)?;
    let removed = removed_handles(&log)?;
    let existing: HashSet<_> = loaded
        .file
        .slots
        .iter()
        .map(|slot| slot.handle.clone())
        .collect();
    let handle = input.handle.unwrap_or_else(|| {
        suggest_slot_handle(&role.handle, &existing.union(&removed).cloned().collect())
    });
    if handle.is_empty() {
        return Err(Error::msg("handle must not be empty"));
    }
    if let Some(error) = slot_handle_error(&handle, &existing, &removed, "mission") {
        return Err(Error::msg(error));
    }
    let runtime = slot::validate_runtime_override(input.runtime.map(|runtime| runtime.key()))?;
    let member = SlotWithRole {
        slot: Slot {
            id: ulid::Ulid::new().to_string(),
            crew_id: mission.crew_id.clone().unwrap_or_default(),
            role_id: role.id.clone(),
            slot_handle: handle.clone(),
            lead: false,
            position: loaded
                .file
                .slots
                .iter()
                .map(|slot| slot.position)
                .max()
                .unwrap_or(-1)
                + 1,
            runtime_override: runtime.map(|runtime| runtime.key().to_owned()),
            model_override: slot::normalize_override_value(input.model.as_deref()),
            effort_override: slot::normalize_override_value(input.effort.as_deref()),
            codex_speed_override: input.speed,
            added_at: Utc::now(),
        },
        role,
    };
    let adapter = crate::runtimes::for_key(
        member
            .slot
            .runtime_override
            .as_deref()
            .unwrap_or(&member.role.runtime),
    );
    let (system_prompt, first_turn) =
        crate::router::prompt::member_prompt_channels(adapter, &member, &loaded.file, &log, false);
    if let Some(body) = &first_turn {
        super::ensure_first_turn_fits(&handle, body)?;
    }
    crate::ops::session::restore_missing_worktree_for_cwd(
        state,
        mission.cwd.as_deref(),
        mission.project_id.as_deref(),
    )?;
    crate::daemon::block_on(super::ensure_mission_router_mounted(state, &mission.id))?;
    let router = state
        .routers
        .get(&mission.id)
        .ok_or_else(|| Error::msg("mission router is unavailable"))?;
    let pending = state.sessions.register_mission_session(
        mission,
        &member.role,
        &member.slot,
        &state.app_data_dir,
        log.path().to_path_buf(),
        state.db.clone(),
        system_prompt,
        first_turn,
        *state.mission_grid_hint.lock().unwrap(),
        "mission-add-role",
    )?;
    let change = MissionRoleChange {
        mission_id: mission.id.clone(),
        handle,
        session_id: pending.session_id.clone(),
    };
    let definition = &pending.launch_definition;
    let joined = log.append(EventDraft::signal(mission.crew_id.as_deref(), &mission.id, "human", SignalType::new("role_joined"),
        serde_json::json!({"handle": change.handle, "slot_id": member.slot.id, "session_id": change.session_id,
            "role_id": member.role.id, "lead": false, "runtime": member.slot.runtime_override.as_deref().unwrap_or(&member.role.runtime),
            "model": definition.model, "effort": definition.effort, "speed": definition.codex_speed})));
    if let Err(error) = joined {
        repo::session::delete_many(&*state.db.get()?, std::slice::from_ref(&change.session_id))?;
        crate::session::system_prompt::remove(&state.app_data_dir, &change.session_id);
        return Err(error.into());
    }
    let previous = loaded.file.clone();
    loaded.file.slots.push(RosterSlot::from(&member.slot));
    if let Err(error) = roster::write(&dir, &loaded.file) {
        let leave_result = leave(&log, &change, &member.slot.id, mission.crew_id.as_deref());
        let restore_result = roster::write(&dir, &previous);
        router.remove_roster_member(&change.handle);
        let delete_result =
            repo::session::delete_many(&*state.db.get()?, std::slice::from_ref(&change.session_id));
        crate::session::system_prompt::remove(&state.app_data_dir, &change.session_id);
        leave_result?;
        delete_result?;
        restore_result?;
        return Err(error);
    }
    router.add_roster_member(&member);
    router.register_pending_sessions(&[(change.handle.clone(), change.session_id.clone())]);
    let runtime_name = runner_core::protocol::runtime_metadata::runtime_list()
        .into_iter()
        .find(|entry| {
            entry.name.key()
                == member
                    .slot
                    .runtime_override
                    .as_deref()
                    .unwrap_or(&member.role.runtime)
        })
        .map(|entry| entry.display_name)
        .unwrap_or_else(|| member.role.runtime.clone());
    let text = format!("@{} joined the mission as a worker: {}, on {}. The team is now {}. `runner mission show` lists the current team.", change.handle, member.role.display_name, runtime_name, team(&loaded.file.slots));
    let notices = (|| -> Result<()> {
        log.append(EventDraft::message(
            mission.crew_id.as_deref(),
            &mission.id,
            "runner",
            None,
            text,
        ))?;
        if let Some(task) = input.task.filter(|task| !task.trim().is_empty()) {
            log.append(EventDraft::message(
                mission.crew_id.as_deref(),
                &mission.id,
                "human",
                Some(change.handle.clone()),
                task,
            ))?;
        }
        Ok(())
    })();
    if let Err(error) = notices {
        let leave_result = leave(&log, &change, &member.slot.id, mission.crew_id.as_deref());
        let restore_result = roster::write(&dir, &previous);
        router.remove_roster_member(&change.handle);
        let delete_result =
            repo::session::delete_many(&*state.db.get()?, std::slice::from_ref(&change.session_id));
        crate::session::system_prompt::remove(&state.app_data_dir, &change.session_id);
        leave_result?;
        delete_result?;
        restore_result?;
        return Err(error);
    }
    spawn_added(state, pending, cancel, mission.id.clone());
    Ok(change)
}

fn spawn_added(
    state: &AppCore,
    pending: PendingMissionSpawn,
    cancel: Arc<AtomicBool>,
    mission_id: String,
) {
    let state = state.clone();
    std::thread::spawn(move || {
        let session_id = pending.session_id.clone();
        let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
        let result = state.sessions.complete_mission_session_spawn(
            pending,
            Arc::clone(&emitter),
            Arc::clone(&cancel),
        );
        let status = match result {
            Ok(CompleteSpawnOutcome::Spawned) => None,
            Ok(CompleteSpawnOutcome::Cancelled) => Some(SessionStatus::Stopped),
            Err(error) => {
                log::error!(
                    "added role spawn failed: mission={mission_id} session={session_id}: {error}"
                );
                Some(SessionStatus::Crashed)
            }
        };
        if let Some(status) = status {
            if let Ok(conn) = state.db.get() {
                let _ = repo::session::set_exit_status(&conn, &session_id, status, Utc::now());
            }
            emitter.exit(&crate::session::manager::ExitEvent {
                session_id,
                mission_id: Some(mission_id.clone()),
                exit_code: None,
                success: false,
            });
        }
        state
            .sessions
            .drop_pending_mission_cancel(&mission_id, &cancel);
    });
}

pub fn remove_role(state: &AppCore, mission_id: &str, handle: &str) -> Result<MissionRoleChange> {
    let lock = mission_lock(state, mission_id);
    let _guard = lock.lock().unwrap();
    let mission = checked_mission(state, mission_id)?;
    let mut loaded = roster::load(&*state.db.get()?, &state.app_data_dir, &mission)?;
    let dir = event_log::mission_dir(&state.app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let log = EventLog::open(&dir)?;
    let slot = loaded
        .file
        .slots
        .iter()
        .find(|slot| slot.handle == handle)
        .ok_or_else(|| {
            if removed_handles(&log).is_ok_and(|removed| removed.contains(handle)) {
                Error::msg(format!("@{handle} was already removed from this mission"))
            } else {
                Error::msg(format!("@{handle} is not in this mission"))
            }
        })?
        .clone();
    let session_id =
        repo::session::latest_unarchived_id_for_slot(&*state.db.get()?, mission_id, &slot.id)?
            .ok_or_else(|| Error::msg(format!("@{handle} has no session in this mission")))?;
    let _claim = state
        .sessions
        .try_claim_resume(&session_id)
        .ok_or_else(|| {
            Error::msg(format!(
                "@{handle} is being resumed or restarted; try again once it is up"
            ))
        })?;
    crate::daemon::block_on(super::ensure_mission_router_mounted(state, mission_id))?;
    if state.sessions.live_session_ids().contains(&session_id) {
        session::session_kill(state, &session_id)?;
    }
    let previous = loaded.file.clone();
    loaded.file.slots.retain(|entry| entry.id != slot.id);
    roster::write(&dir, &loaded.file)?;
    if let Some(router) = state.routers.get(mission_id) {
        router.remove_roster_member(handle);
    }
    let change = MissionRoleChange {
        mission_id: mission_id.into(),
        handle: handle.into(),
        session_id,
    };
    if let Err(error) = leave(&log, &change, &slot.id, mission.crew_id.as_deref()) {
        roster::write(&dir, &previous)?;
        if let Some(router) = state.routers.get(mission_id) {
            if let Some(member) = loaded.slots.iter().find(|member| member.slot.id == slot.id) {
                router.add_roster_member(member);
                router.register_sessions(&[(handle.to_owned(), change.session_id.clone())]);
            }
        }
        return Err(error);
    }
    log.append(EventDraft::message(mission.crew_id.as_deref(), mission_id, "runner", None,
        format!("@{handle} left the mission. Messages to @{handle} are refused from now on. The team is now {}. `runner mission show` lists the current team.", team(&loaded.file.slots))))?;
    Ok(change)
}

#[cfg(test)]
mod tests;

// Interrupted missions (#1187). runnerd stamps a running mission's sessions
// for launch resume and kills them when it stops; the resumed agents come
// back idle, and a turn in flight is gone. Before launch resume, boot marks
// each member that was mid-turn with a `session_interrupted` signal in the
// mission's log, so the mark survives another restart and needs no migration.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use runner_core::event_log::{self, EventLog};
use runner_core::model::{EventDraft, EventKind, SignalType};

use super::report::{Restart, SETTLE_WINDOW};
use crate::model::{Mission, MissionStatus, Runtime};
use crate::router::{Interruptions, SESSION_INTERRUPTED};
use crate::{repo, AppCore};

/// Mark the members a stopped runnerd interrupted, and say what the restart
/// left each mission launch resume brings back.
pub(crate) fn mark_on_boot(core: &AppCore) -> crate::error::Result<HashMap<String, Restart>> {
    let conn = core.db.get()?;
    let stamped = repo::session::resume_on_launch_ids(&conn)?;
    let now = Utc::now();
    let mut restarts = HashMap::new();
    for mission in repo::mission::list(&conn, None)? {
        if mission.status != MissionStatus::Running || mission.archived_at.is_some() {
            continue;
        }
        let sessions =
            match crate::ops::session::list_for_mission(&conn, &core.app_data_dir, &mission.id) {
                Ok(sessions) => sessions,
                Err(error) => {
                    log::warn!("mission {} interruptions unavailable: {error}", mission.id);
                    continue;
                }
            };
        let resumed: Vec<_> = sessions
            .iter()
            .filter(|row| stamped.contains(&row.session.id))
            .filter(|row| !Runtime::parse(&row.runtime).is_some_and(Runtime::is_shell))
            .collect();
        if resumed.is_empty() {
            continue;
        }
        let dir =
            event_log::mission_dir(&core.app_data_dir, mission.crew_id.as_deref(), &mission.id);
        let log = EventLog::open(&dir)?;
        let (entries, skipped) = log.read_from_lossy(0)?;
        let offset = entries
            .iter()
            .map(|entry| entry.next_offset)
            .chain(skipped.iter().map(|skip| skip.next_offset))
            .max()
            .unwrap_or(0);
        let interrupted = Interruptions::from_entries(&entries);
        let latest = latest_statuses(&entries);
        let stopped_at = |row: &&runner_core::protocol::session::SessionRow| {
            row.session.stopped_at.unwrap_or(now)
        };
        let mut marked = false;
        for row in &resumed {
            let working = latest.get(&row.handle).is_some_and(|last| last.working);
            if !working || interrupted.get(&row.handle).is_some() {
                continue;
            }
            log.append(EventDraft::signal(
                mission.crew_id.as_deref(),
                &mission.id,
                &row.handle,
                SignalType::new(SESSION_INTERRUPTED),
                serde_json::json!({
                    "at": stopped_at(row).to_rfc3339(),
                    "session_id": row.session.id,
                }),
            ))?;
            log::info!(
                "mission {}: @{} was mid-turn when runnerd stopped",
                mission.id,
                row.handle
            );
            marked = true;
        }
        let stopped = resumed.iter().map(stopped_at).max().unwrap_or(now);
        let settled = resumed.iter().all(|row| {
            latest.get(&row.handle).is_some_and(|last| {
                !last.busy
                    && (stopped - last.at)
                        .to_std()
                        .is_ok_and(|idle| idle >= SETTLE_WINDOW)
            })
        });
        let restart = if marked {
            Some(Restart::Interrupted)
        } else if resumed
            .iter()
            .any(|row| interrupted.get(&row.handle).is_some())
        {
            Some(Restart::StillInterrupted)
        } else {
            settled.then_some(Restart::Settled(offset))
        };
        if let Some(restart) = restart {
            restarts.insert(mission.id, restart);
        }
    }
    Ok(restarts)
}

/// Whether a member started a turn in `mission`'s log after `offset`.
pub(crate) fn turn_since(core: &AppCore, mission: &Mission, offset: u64) -> bool {
    let dir = event_log::mission_dir(&core.app_data_dir, mission.crew_id.as_deref(), &mission.id);
    EventLog::open(&dir)
        .and_then(|log| log.read_from_lossy(offset))
        .map(|(entries, _)| {
            entries
                .iter()
                .any(|entry| crate::router::starts_turn(&entry.event))
        })
        .unwrap_or(true)
}

struct LastStatus {
    busy: bool,
    /// Busy because a turn ran, not because the member waited for input.
    working: bool,
    at: DateTime<Utc>,
}

/// Each handle's last `session_status`.
fn latest_statuses(entries: &[runner_core::event_log::LogEntry]) -> HashMap<String, LastStatus> {
    let mut latest = HashMap::new();
    for entry in entries {
        let event = &entry.event;
        if event.kind != EventKind::Signal
            || !event
                .signal_type
                .as_ref()
                .is_some_and(SignalType::is_session_status)
        {
            continue;
        }
        let busy = match event.payload["state"].as_str() {
            Some("busy") => true,
            Some("idle") => false,
            _ => continue,
        };
        latest.insert(
            event.from.clone(),
            LastStatus {
                busy,
                working: crate::router::working(&event.payload),
                at: event.ts,
            },
        );
    }
    latest
}

// Mission lifecycle — start, stop, list, get.
//
// A mission is the runtime container: it owns a directory, an NDJSON event
// log, and a set of sessions (spawned in C6). This module only does the
// bookkeeping layer — no PTYs yet.
//
// `mission_start` is the point where config crystallizes into runtime:
// validate the crew has ≥1 runner and exactly one lead, create the mission
// row, create the mission directory, export the crew's `signal_types`
// allowlist to a sidecar file for the CLI to read (arch §5.3 Layer 2), and
// emit the two opening events — `mission_start` (system announces the run)
// and `mission_goal` (the human's intent, which the orchestrator routes to
// the lead via the built-in rule in C8).

use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use runner_core::event_log::{self, EventLog};
use runner_core::model::{EventDraft, EventKind, SignalType};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use tauri::State;
use ulid::Ulid as UlidGen;

use crate::{
    commands::{crew, slot},
    error::{Error, Result},
    model::{Mission, MissionStatus, Timestamp},
    AppState,
};

#[derive(Debug, Clone, Deserialize)]
pub struct StartMissionInput {
    pub crew_id: String,
    pub title: String,
    /// Optional override of the crew's default goal. When `None`, the crew's
    /// `goal` column is used; if that is also unset the mission starts with
    /// an empty-goal event (valid — the human may post a `human_said` signal
    /// later instead of setting a goal up front).
    #[serde(default)]
    pub goal_override: Option<String>,
    /// Working directory exposed to every session as `$MISSION_CWD`.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMissionOutput {
    pub mission: Mission,
    /// Effective goal (override if present, else crew default, else empty).
    /// The frontend uses this to render the first event in the workspace
    /// without making a second round-trip.
    pub goal: String,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

fn row_to_mission(row: &Row<'_>) -> rusqlite::Result<Mission> {
    let status: String = row.get("status")?;
    let started_at: String = row.get("started_at")?;
    let stopped_at: Option<String> = row.get("stopped_at")?;
    let pinned_at: Option<String> = row.get("pinned_at")?;
    let archived_at: Option<String> = row.get("archived_at")?;

    let status = match status.as_str() {
        "running" => MissionStatus::Running,
        "completed" => MissionStatus::Completed,
        "aborted" => MissionStatus::Aborted,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown mission status {other:?}").into(),
            ))
        }
    };
    let parse_ts = |s: String| -> rusqlite::Result<Timestamp> {
        s.parse().map_err(|e: chrono::ParseError| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    };

    Ok(Mission {
        id: row.get("id")?,
        crew_id: row.get("crew_id")?,
        title: row.get("title")?,
        status,
        goal_override: row.get("goal_override")?,
        cwd: row.get("cwd")?,
        started_at: parse_ts(started_at)?,
        stopped_at: stopped_at.map(parse_ts).transpose()?,
        pinned_at: pinned_at.map(parse_ts).transpose()?,
        archived_at: archived_at.map(parse_ts).transpose()?,
    })
}

pub fn list(conn: &Connection, crew_id: Option<&str>) -> Result<Vec<Mission>> {
    // Pinned missions float to the top, then most-recently-started.
    // Sort key: NULL pinned_at sorts last (DESC), older pinned_at
    // sorts after newer (last-pinned first feels right for testing).
    //
    // `archived_at IS NULL` is the single chokepoint that hides
    // archived missions from every surface that lists missions: the
    // ⌘K palette, the sidebar tray, the Missions page summary. New
    // surfaces inherit the filter by going through this helper. To
    // open an archived mission by direct URL, use `get()` instead —
    // it intentionally does NOT filter.
    let sql = "SELECT id, crew_id, title, status, goal_override, cwd,
                      started_at, stopped_at, pinned_at, archived_at
                 FROM missions
                 WHERE (?1 IS NULL OR crew_id = ?1)
                   AND archived_at IS NULL
                 ORDER BY pinned_at IS NULL, pinned_at DESC, started_at DESC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![crew_id], row_to_mission)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// One row in the Missions page list — the mission's own fields denormalized
/// with the crew name and the pending-ask count. The count comes from the
/// live `RouterRegistry` when the mission is mounted; otherwise it's
/// reconstructed from the event log (unmatched `human_question` /
/// `human_response` pairs) so post-restart and terminal-status missions
/// still surface unanswered cards.
#[derive(Debug, Clone, Serialize)]
pub struct MissionSummary {
    #[serde(flatten)]
    pub mission: Mission,
    pub crew_name: String,
    pub pending_ask_count: usize,
}

pub fn get(conn: &Connection, id: &str) -> Result<Mission> {
    // Intentionally no `archived_at` filter — opening an archived
    // mission by direct URL has to still resolve so the workspace can
    // render it read-only.
    conn.query_row(
        "SELECT id, crew_id, title, status, goal_override, cwd,
                started_at, stopped_at, pinned_at, archived_at
           FROM missions WHERE id = ?1",
        params![id],
        row_to_mission,
    )
    .optional()?
    .ok_or_else(|| Error::msg(format!("mission not found: {id}")))
}

/// Cap on the effective mission goal byte length. The launch prompt
/// composer pastes this into the lead's first-user-turn body
/// alongside `system_prompt` (≤ `MAX_SYSTEM_PROMPT_BYTES`) and the
/// roster + coordination block; together they have to fit under
/// `router::runtime::FIRST_TURN_ARGV_MAX_BYTES`. 8 KB is roomy for
/// a real mission goal (typical goals are a few sentences) while
/// leaving generous headroom for the rest of the composed body.
pub const MAX_MISSION_GOAL_BYTES: usize = 8 * 1024;

fn validate_mission_goal(goal: &str) -> Result<()> {
    if goal.len() > MAX_MISSION_GOAL_BYTES {
        return Err(Error::msg(format!(
            "mission goal is {} bytes; max {} ({} KB). Trim the goal text or move \
             long-form context into the runner brief / per-task messages.",
            goal.len(),
            MAX_MISSION_GOAL_BYTES,
            MAX_MISSION_GOAL_BYTES / 1024,
        )));
    }
    Ok(())
}

pub fn start(
    conn: &mut Connection,
    app_data_dir: &Path,
    input: StartMissionInput,
) -> Result<StartMissionOutput> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(Error::msg("mission title must not be empty"));
    }
    if let Some(g) = input.goal_override.as_deref() {
        validate_mission_goal(g)?;
    }

    // Validate crew exists and is launchable.
    let crew = crew::get(conn, &input.crew_id)?;
    let roster = slot::list(conn, &input.crew_id)?;
    if roster.is_empty() {
        return Err(Error::msg(format!(
            "crew {} has no slots; cannot start mission",
            crew.name
        )));
    }
    // The slot commands enforce one-lead-per-crew (clear-others-then-set
    // inside a transaction). Defense in depth: still check at least one
    // slot carries the flag in case a path leaves a crew leaderless.
    if !roster.iter().any(|m| m.slot.lead) {
        return Err(Error::msg(format!(
            "crew {} has no lead slot; cannot start mission",
            crew.name
        )));
    }

    // Everything below is done under a DB transaction so that if any of the
    // filesystem or event-log writes fail, the mission row is rolled back
    // and the operator doesn't see a phantom `running` mission (review
    // finding #1). The sole piece of state that can linger on failure is
    // an empty mission directory — harmless because the ULID is never
    // reused, and the next `mission_start` gets a fresh ID + dir.
    //
    // Per #55 the crew-level "at most one live mission" guard was lifted
    // — the constraint was conservative, not load-bearing. Per-mission
    // state is fully namespaced: `sessions.mission_id` is a foreign key,
    // `kill_all_for_mission` is mission-scoped, the runner-CLI shim path
    // includes mission_id (`$APPDATA/missions/<mission_id>/shims/...`),
    // the roster sidecar is per-mission, and the router boots fresh per
    // mission. The shared crew row's `signal_types` allowlist is
    // immutable mid-mission and safe to reuse.
    let tx = conn.transaction()?;

    let id = new_id();
    let started_at = now();
    tx.execute(
        "INSERT INTO missions
            (id, crew_id, title, status, goal_override, cwd, started_at)
         VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6)",
        params![
            id,
            crew.id,
            title,
            input.goal_override,
            input.cwd,
            started_at.to_rfc3339(),
        ],
    )?;

    // Create the mission directory and export the signal-types allowlist
    // sidecar. The CLI (C9) reads this file to validate signal types.
    let mission_dir = event_log::mission_dir(app_data_dir, &crew.id, &id);
    std::fs::create_dir_all(&mission_dir)?;
    write_signal_types_sidecar(app_data_dir, &crew.id, &crew.signal_types)?;

    // Snapshot the roster into a per-mission sidecar so the CLI can
    // validate `runner msg post --to <handle>` without DB access. The
    // roster is frozen here at mission_start: later changes to crew
    // membership do not retroactively invalidate `--to` lookups in this
    // mission's log (per PR #19 reviewer guidance).
    let roster_for_sidecar = slot::list(&tx, &crew.id)?;
    write_roster_sidecar(&mission_dir, &roster_for_sidecar)?;

    // Effective goal = override || crew default || "".
    let goal_text = input
        .goal_override
        .as_deref()
        .or(crew.goal.as_deref())
        .unwrap_or("")
        .to_string();

    // Open the event log and emit the two opening events.
    let log = EventLog::open(&mission_dir)?;
    log.append(EventDraft {
        crew_id: crew.id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: None,
        signal_type: Some(SignalType::new("mission_start")),
        payload: serde_json::json!({
            "title": title,
            "cwd": input.cwd,
        }),
    })?;
    log.append(EventDraft {
        crew_id: crew.id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "human".into(),
        to: None,
        signal_type: Some(SignalType::new("mission_goal")),
        payload: serde_json::json!({ "text": goal_text }),
    })?;

    // All log writes succeeded — commit the DB row so the mission becomes
    // visible to list/get only after its startup events are durable.
    let mission = get(&tx, &id)?;
    tx.commit()?;
    Ok(StartMissionOutput {
        mission,
        goal: goal_text,
    })
}

pub fn stop(conn: &mut Connection, app_data_dir: &Path, id: &str) -> Result<Mission> {
    // Mirror `start`: flip status inside a tx and only commit once the
    // terminal `mission_stopped` event has been appended. If the log write
    // fails, the mission stays `running` and the operator can retry.
    let tx = conn.transaction()?;

    // Conditional UPDATE binds the status check and the transition into one
    // atomic SQL statement. Without this, two racing `mission_stop` calls
    // could each observe `running`, both commit `completed`, and both append
    // a `mission_stopped` event (duplicate terminal). With `WHERE status =
    // 'running'`, the slower of the two updates 0 rows and is rejected
    // below, so only one writer ever reaches the log append.
    //
    // `archived_at` is set in the same UPDATE: `stop()` is reached only
    // via `mission_archive`, so a terminal stop is by definition an
    // archive. Atomic with the status flip means a row never observes
    // `status='completed' AND archived_at IS NULL` (other than
    // pre-existing rows the migration backfilled).
    let stopped_at = now();
    let affected = tx.execute(
        "UPDATE missions
            SET status = 'completed', stopped_at = ?1, archived_at = ?1
          WHERE id = ?2 AND status = 'running'",
        params![stopped_at.to_rfc3339(), id],
    )?;
    if affected == 0 {
        // Either the id doesn't exist or the mission isn't running anymore
        // (a concurrent stop won the race). Fetch for a precise error.
        let mission = get(&tx, id)?;
        return Err(Error::msg(format!(
            "mission {id} is not running; status = {:?}",
            mission.status
        )));
    }

    // Fetch crew_id now that we know the row exists and we own the
    // transition; used for the mission-dir path below.
    let mission = get(&tx, id)?;

    let mission_dir = event_log::mission_dir(app_data_dir, &mission.crew_id, id);
    let log = EventLog::open(&mission_dir)?;
    log.append(EventDraft {
        crew_id: mission.crew_id.clone(),
        mission_id: id.to_string(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: None,
        signal_type: Some(SignalType::new("mission_stopped")),
        payload: serde_json::json!({}),
    })?;

    tx.commit()?;
    Ok(mission)
}

/// Write the crew's signal-type allowlist to
/// `$APPDATA/runner/crews/{crew_id}/signal_types.json` atomically so a
/// crash during write never leaves a half-written file that the CLI would
/// read and reject valid types on.
///
/// Uses `tempfile::NamedTempFile::persist` for the replace — plain
/// `std::fs::rename` fails on Windows when the destination exists, which
/// would break every mission start after the first for a given crew.
fn write_signal_types_sidecar(
    app_data_dir: &Path,
    crew_id: &str,
    allowlist: &[SignalType],
) -> Result<()> {
    use std::io::Write;

    let target = event_log::signal_types_path(app_data_dir, crew_id);
    let parent = target
        .parent()
        .ok_or_else(|| Error::msg("signal_types.json path has no parent"))?;
    std::fs::create_dir_all(parent)?;

    // tempfile places the temp file in the same directory so the rename is
    // intra-filesystem (required for atomicity on Unix) and uses
    // `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` under the hood on Windows.
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let json = serde_json::to_vec(allowlist)?;
    tmp.write_all(&json)?;
    tmp.flush()?;
    tmp.persist(&target).map_err(|e| Error::Io(e.error))?;
    Ok(())
}

/// Write the per-mission roster snapshot to `roster.json` next to
/// `events.ndjson`. The CLI (`runner msg post --to`) reads this to
/// validate handles without DB access. Frozen at mission_start: if the
/// crew's membership changes mid-mission, the running mission still
/// validates against this snapshot.
///
/// Atomic write via `tempfile::NamedTempFile::persist` — same dance as
/// `write_signal_types_sidecar` — so a crash mid-write can't leave a
/// half-formed file the CLI would parse-fail on.
fn write_roster_sidecar(mission_dir: &Path, roster: &[crate::model::SlotWithRunner]) -> Result<()> {
    use std::io::Write;

    #[derive(serde::Serialize)]
    struct RosterEntry<'a> {
        // `handle` is the slot's in-crew identity (slot_handle). The
        // CLI's `runner msg post --to <handle>` looks up against this
        // sidecar — the runner template's globally-unique `handle`
        // is irrelevant in mission contexts, where two slots could
        // share the same template.
        handle: &'a str,
        lead: bool,
    }
    let entries: Vec<RosterEntry> = roster
        .iter()
        .map(|m| RosterEntry {
            handle: &m.slot.slot_handle,
            lead: m.slot.lead,
        })
        .collect();

    std::fs::create_dir_all(mission_dir)?;
    let target = mission_dir.join("roster.json");
    let mut tmp = tempfile::NamedTempFile::new_in(mission_dir)?;
    let json = serde_json::to_vec(&entries)?;
    tmp.write_all(&json)?;
    tmp.flush()?;
    tmp.persist(&target).map_err(|e| Error::Io(e.error))?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostHumanSignalInput {
    pub mission_id: String,
    /// Signal type — restricted to the human-originated ones the workspace
    /// UI is allowed to emit. Anything else is rejected.
    pub signal_type: String,
    pub payload: serde_json::Value,
}

/// Replay the full event log for a mission. Used by the workspace UI when
/// it first mounts (or remounts after navigation): it folds the historical
/// envelopes into its feed before subscribing to `event/appended` for live
/// tailing. Returns lossy-decoded events so a single corrupt line can't
/// freeze the UI; the bus already tolerates the same.
pub fn read_events(
    app_data_dir: &Path,
    conn: &Connection,
    mission_id: &str,
) -> Result<Vec<runner_core::model::Event>> {
    let mission = get(conn, mission_id)?;
    let mission_dir = event_log::mission_dir(app_data_dir, &mission.crew_id, mission_id);
    let log = EventLog::open(&mission_dir)?;
    let (entries, _skipped) = log.read_from_lossy(0)?;
    Ok(entries.into_iter().map(|e| e.event).collect())
}

#[tauri::command]
pub async fn mission_events_replay(
    state: State<'_, AppState>,
    mission_id: String,
) -> Result<Vec<runner_core::model::Event>> {
    let conn = state.db.get()?;
    read_events(&state.app_data_dir, &conn, &mission_id)
}

#[tauri::command]
pub async fn mission_post_human_signal(
    state: State<'_, AppState>,
    input: PostHumanSignalInput,
) -> Result<runner_core::model::Event> {
    // Whitelist: only the two signal types the workspace UI is supposed
    // to emit. The router treats `from = "human"` as authoritative for
    // these, so a buggy frontend that posted `mission_goal` or `ask_lead`
    // could trigger handler side-effects from the wrong identity.
    let allowed = matches!(input.signal_type.as_str(), "human_said" | "human_response");
    if !allowed {
        return Err(Error::msg(format!(
            "signal_type {:?} is not allowed from the workspace UI",
            input.signal_type
        )));
    }

    let mission = {
        let conn = state.db.get()?;
        get(&conn, &input.mission_id)?
    };
    if !matches!(mission.status, MissionStatus::Running) {
        return Err(Error::msg(format!(
            "mission {} is not running (status = {:?}); cannot post signals",
            mission.id, mission.status
        )));
    }

    let mission_dir = event_log::mission_dir(&state.app_data_dir, &mission.crew_id, &mission.id);
    let log = EventLog::open(&mission_dir)?;
    let event = log.append(EventDraft {
        crew_id: mission.crew_id.clone(),
        mission_id: mission.id.clone(),
        kind: EventKind::Signal,
        from: "human".into(),
        to: None,
        signal_type: Some(SignalType::new(input.signal_type)),
        payload: input.payload,
    })?;
    Ok(event)
}

#[tauri::command]
pub async fn mission_start(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    input: StartMissionInput,
) -> Result<StartMissionOutput> {
    use crate::event_bus::{BusEmitter, TauriBusEvents};
    use crate::router::{
        open_log_for_mission, CompositeBusEmitter, Router, RouterSubscriber, StdinInjector,
    };
    use crate::session::manager::{SessionEvents, TauriSessionEvents};
    use std::sync::Arc;

    let out = {
        let mut conn = state.db.get()?;
        start(&mut conn, &state.app_data_dir, input)?
    };
    log::info!(
        "mission starting: id={} crew={} title={:?}",
        out.mission.id,
        out.mission.crew_id,
        out.mission.title,
    );

    // Mission row + opening events are durable. Now spawn one PTY per
    // slot. This loop is **all-or-nothing**: if any spawn fails we kill
    // the sessions we already created, flip the mission to `aborted`,
    // and return the error. Without this the caller could see "err"
    // while the crew still has half a live mission that blocks future
    // starts via the one-live-mission-per-crew invariant.
    //
    // The roster lives in `slots`, joined with the runner template
    // each slot references. Mission spawn iterates per slot — two
    // slots referencing the same runner template both produce
    // distinct PTYs identifying as their respective slot_handles.
    let (crew_name, allowed_signals, crew_default_goal) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &out.mission.crew_id)?;
        (crew.name, crew.signal_types, crew.goal)
    };
    let roster = {
        let conn = state.db.get()?;
        slot::list(&conn, &out.mission.crew_id)?
    };
    let events_log_path =
        event_log::events_path(&state.app_data_dir, &out.mission.crew_id, &out.mission.id);

    // Effective mission goal — same precedence as the `mission_goal`
    // event opened by `start()` above (override > crew default > "").
    // Used here to compose the lead's launch prompt before the spawn
    // loop, so the body can land via the positional `[PROMPT]` argv
    // at process boot rather than racing the post-spawn paste path.
    // See `docs/impls/0007-spawn-time-prompt-delivery.md`.
    let goal_text: String = out
        .mission
        .goal_override
        .as_deref()
        .or(crew_default_goal.as_deref())
        .unwrap_or("")
        .to_string();

    // Pre-compose each slot's first-user-turn body. Lead gets the
    // full launch prompt (preamble + brief + goal + roster +
    // coordination). Non-leads get the worker preamble + brief.
    // Both delivery paths (spawn-time argv vs post-spawn paste
    // fallback) read from the same composer in `router::prompt`,
    // so the body is byte-identical regardless of route.
    let first_turns: Vec<Option<String>> = {
        let roster_entries: Vec<crate::router::prompt::RosterEntry> = roster
            .iter()
            .map(|m| crate::router::prompt::RosterEntry {
                handle: m.slot.slot_handle.as_str(),
                display_name: m.runner.display_name.as_str(),
                lead: m.slot.lead,
            })
            .collect();
        let lead_member = roster.iter().find(|m| m.slot.lead);
        roster
            .iter()
            .map(|m| {
                if m.slot.lead {
                    lead_member.map(|lm| {
                        crate::router::prompt::compose_launch_prompt(
                            &crate::router::prompt::LaunchPromptInput {
                                lead: crate::router::prompt::LeadView {
                                    handle: lm.slot.slot_handle.as_str(),
                                    display_name: lm.runner.display_name.as_str(),
                                    system_prompt: lm.runner.system_prompt.as_deref(),
                                },
                                crew_name: crew_name.as_str(),
                                mission_goal: goal_text.as_str(),
                                roster: &roster_entries,
                                allowed_signals: &allowed_signals,
                            },
                        )
                    })
                } else {
                    Some(crate::router::prompt::compose_worker_first_turn(
                        m.runner.system_prompt.as_deref(),
                    ))
                }
            })
            .collect()
    };

    // Build the router up front (opens the log, validates the lead, holds
    // empty state). It does NOT subscribe to the bus yet — see ordering
    // below.
    let mission_dir =
        event_log::mission_dir(&state.app_data_dir, &out.mission.crew_id, &out.mission.id);
    let log_arc = match open_log_for_mission(&mission_dir) {
        Ok(l) => l,
        Err(e) => {
            // Couldn't open the log — roll the mission row back. Bus isn't
            // mounted yet, no sessions were spawned, nothing to clean up.
            if let Ok(conn) = state.db.get() {
                let _ = conn.execute(
                    "UPDATE missions
                        SET status = 'aborted', stopped_at = ?1
                      WHERE id = ?2",
                    rusqlite::params![Utc::now().to_rfc3339(), out.mission.id],
                );
            }
            return Err(e);
        }
    };
    let injector: Arc<dyn StdinInjector> = Arc::clone(&state.sessions) as Arc<dyn StdinInjector>;
    let router = match Router::new(
        out.mission.id.clone(),
        out.mission.crew_id.clone(),
        crew_name,
        &roster,
        allowed_signals,
        Arc::clone(&log_arc),
        injector,
    ) {
        Ok(r) => r,
        Err(e) => {
            if let Ok(conn) = state.db.get() {
                let _ = conn.execute(
                    "UPDATE missions
                        SET status = 'aborted', stopped_at = ?1
                      WHERE id = ?2",
                    rusqlite::params![Utc::now().to_rfc3339(), out.mission.id],
                );
            }
            return Err(e);
        }
    };

    // Spawn sessions BEFORE the bus mounts so `register_sessions` can
    // populate the handle→session_id map up front. The bus's consumer
    // thread starts its initial replay asynchronously inside `mount`; if
    // we mounted first, the `mission_goal` injection could race the
    // session registration and silently no-op (the lead would never get
    // its launch prompt — review finding P1).
    //
    // The bus's initial replay reads from offset 0, so the opening
    // `mission_start` / `mission_goal` events still surface even though
    // the watcher attaches after the writes. Spawning runners before
    // mount is safe: their PTYs come up here, but `runner` CLI invocations
    // can't run before they receive their first stdin (which only comes
    // after the bus delivers `mission_goal` post-mount), so no log writes
    // can race the watcher attachment.
    let emitter: Arc<dyn SessionEvents> = Arc::new(TauriSessionEvents(app.clone()));
    let mut spawned_pairs: Vec<(String, String)> = Vec::with_capacity(roster.len());
    for (idx, member) in roster.iter().enumerate() {
        let first_turn = first_turns.get(idx).cloned().flatten();
        let spawn_res = state.sessions.spawn(
            &out.mission,
            &member.runner,
            &member.slot,
            &state.app_data_dir,
            events_log_path.clone(),
            state.db.clone(),
            Arc::clone(&emitter),
            first_turn,
        );
        match spawn_res {
            Ok(spawned) => {
                // Register by slot_handle (the in-mission identity)
                // — the router routes signals/messages by slot_handle,
                // not by template handle.
                spawned_pairs.push((member.slot.slot_handle.clone(), spawned.id));
            }
            Err(e) => {
                // Rollback: kill the sessions that did start, mark the
                // mission aborted, surface the original error. Bus and
                // router aren't mounted yet so no event-side cleanup.
                let _ = state.sessions.kill_all_for_mission(&out.mission.id);
                if let Ok(conn) = state.db.get() {
                    let _ = conn.execute(
                        "UPDATE missions
                            SET status = 'aborted', stopped_at = ?1
                          WHERE id = ?2",
                        rusqlite::params![Utc::now().to_rfc3339(), out.mission.id],
                    );
                }
                return Err(e);
            }
        }
    }
    // Register the full session map BEFORE the bus mount. From this point
    // any event the bus's initial replay delivers to the router will land
    // on a fully-wired handle map.
    router.register_sessions(&spawned_pairs);

    // Now mount the bus. Initial replay from offset 0 picks up the opening
    // events (durable since `start()` committed them under the DB tx),
    // fans them to the Tauri emitter (UI) and the RouterSubscriber (which
    // dispatches `mission_goal` → launch prompt to the lead). Fresh
    // mission: NO `reconstruct_from_log()` call — setting a watermark
    // over the just-written `mission_goal` would suppress the bootstrap
    // (reviewer's caveat).
    let roster_handles: Vec<String> = roster.iter().map(|m| m.slot.slot_handle.clone()).collect();
    let tauri_emitter: Arc<dyn BusEmitter> = Arc::new(TauriBusEvents(app.clone()));
    let router_emitter: Arc<dyn BusEmitter> = Arc::new(RouterSubscriber(Arc::clone(&router)));
    let composite: Arc<dyn BusEmitter> = Arc::new(CompositeBusEmitter::new(vec![
        tauri_emitter,
        router_emitter,
    ]));
    if let Err(e) = state.buses.mount(
        out.mission.id.clone(),
        &mission_dir,
        &roster_handles,
        composite,
    ) {
        // Bus didn't attach — kill the sessions we spawned, abort the row.
        let _ = state.sessions.kill_all_for_mission(&out.mission.id);
        if let Ok(conn) = state.db.get() {
            let _ = conn.execute(
                "UPDATE missions
                    SET status = 'aborted', stopped_at = ?1
                  WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), out.mission.id],
            );
        }
        return Err(e);
    }

    state.routers.register(out.mission.id.clone(), router);
    log::info!(
        "mission started: id={} sessions={}",
        out.mission.id,
        spawned_pairs.len(),
    );
    Ok(out)
}

/// Re-attach a mission's router + bus after app restart. The mission row
/// stays `running` across restarts but the in-memory Router/Bus die with
/// the old process. The frontend calls this on workspace mount; if the
/// router is already registered (just navigating around in the same
/// process), it returns the existing mission unchanged. After a real
/// restart, this rebuilds the Router from the slot roster, registers
/// the existing slot_handle → session_id mapping (sessions are stopped
/// but the row ids survive — resume preserves them), reconstructs router
/// state from the log, and mounts the bus.
///
/// PTY children are NOT respawned here — that's an explicit per-slot
/// `session_resume` call from the frontend, mirroring direct-chat
/// resume UX.
///
/// Idempotent: when the app's startup reconciler (or a prior workspace
/// mount) has already mounted the bus, this is a no-op that just
/// returns the loaded mission row.
#[tauri::command]
pub async fn mission_attach(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    mission_id: String,
) -> Result<Mission> {
    log::info!("mission attach: id={mission_id}");
    ensure_mission_router_mounted(&state, &app, &mission_id).await?;
    let conn = state.db.get()?;
    get(&conn, &mission_id)
}

/// Mount the in-memory Router + EventBus for `mission_id`, idempotently.
///
/// Called from two places:
///  * `mission_attach` — workspace UI mount path; runs on every
///    workspace navigation. After the startup reconciler runs, this is
///    almost always a no-op.
///  * `reattach_all_running_missions` — app startup path; runs once
///    per running mission before session reattach so forwarder threads
///    don't emit `mission_*` events into a non-existent subscriber.
///
/// Returns `Ok(())` (no-op) when the mission's router is already
/// registered, or when the mission isn't in the `running` state.
pub(crate) async fn ensure_mission_router_mounted(
    state: &AppState,
    app: &tauri::AppHandle,
    mission_id: &str,
) -> Result<()> {
    use crate::event_bus::{BusEmitter, TauriBusEvents};
    use crate::router::{
        open_log_for_mission, CompositeBusEmitter, Router, RouterSubscriber, StdinInjector,
    };
    use std::sync::Arc;

    // Idempotent: if the router is already mounted for this mission,
    // just return. The frontend calls attach on every workspace mount
    // (including back-and-forth navigation), so this happens often.
    // Startup-side: the second mount path (workspace) finds the bus
    // already mounted from the startup reconciler and returns here.
    if state.routers.get(mission_id).is_some() {
        return Ok(());
    }

    let mission = {
        let conn = state.db.get()?;
        get(&conn, mission_id)?
    };

    // Only running missions get rehydrated. Completed/aborted missions
    // are read-only — the workspace shows their feed via
    // mission_events_replay; no live router needed.
    if !matches!(mission.status, MissionStatus::Running) {
        return Ok(());
    }

    let (crew_name, allowed_signals) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &mission.crew_id)?;
        (crew.name, crew.signal_types)
    };
    let roster = {
        let conn = state.db.get()?;
        slot::list(&conn, &mission.crew_id)?
    };

    // Pull the latest session_id per slot for this mission so the
    // rebuilt Router can resolve `slot_handle → session_id` for stdin
    // injection. Independent of whether the panes are alive: the
    // mapping is what the Router holds in memory, not anything about
    // pane state. Filter to non-archived rows so a deleted-and-
    // re-added slot doesn't pull a stale row.
    let session_pairs: Vec<(String, String)> = {
        let conn = state.db.get()?;
        let mut out = Vec::with_capacity(roster.len());
        for member in &roster {
            let session_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM sessions
                       WHERE mission_id = ?1 AND slot_id = ?2 AND archived_at IS NULL
                       ORDER BY started_at DESC
                       LIMIT 1",
                    rusqlite::params![mission.id, member.slot.id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(sid) = session_id {
                out.push((member.slot.slot_handle.clone(), sid));
            }
        }
        out
    };

    let mission_dir = event_log::mission_dir(&state.app_data_dir, &mission.crew_id, &mission.id);
    let log_arc = open_log_for_mission(&mission_dir)?;
    let injector: Arc<dyn StdinInjector> = Arc::clone(&state.sessions) as Arc<dyn StdinInjector>;
    let router = Router::new(
        mission.id.clone(),
        mission.crew_id.clone(),
        crew_name,
        &roster,
        allowed_signals,
        Arc::clone(&log_arc),
        injector,
    )?;
    router.register_sessions(&session_pairs);

    // Set the replay watermark so the bus's initial replay doesn't
    // re-fire `mission_goal` / `human_said` / `ask_lead` — handlers
    // would re-inject historical stdin into the (about-to-be-resumed)
    // PTYs. Pending_asks / runner_status also rehydrate here.
    router.reconstruct_from_log()?;

    let roster_handles: Vec<String> = roster.iter().map(|m| m.slot.slot_handle.clone()).collect();
    let tauri_emitter: Arc<dyn BusEmitter> = Arc::new(TauriBusEvents(app.clone()));
    let router_emitter: Arc<dyn BusEmitter> = Arc::new(RouterSubscriber(Arc::clone(&router)));
    let composite: Arc<dyn BusEmitter> = Arc::new(CompositeBusEmitter::new(vec![
        tauri_emitter,
        router_emitter,
    ]));
    state
        .buses
        .mount(mission.id.clone(), &mission_dir, &roster_handles, composite)?;

    state.routers.register(mission.id.clone(), router);
    Ok(())
}

/// Walk every `running` mission and mount its Router + EventBus.
///
/// Runs once at app startup, before `SessionManager::reattach_running_sessions`,
/// so the bus is in place when forwarder threads start emitting
/// `mission_*` events. The NDJSON log is the source of truth; this
/// just re-wires the in-memory fanout layer that died with the old
/// process.
///
/// Per-mission isolation: if one mission's mount fails (corrupt log,
/// missing crew row, etc.), it gets logged and the loop continues.
/// The returned set lists every mission whose mount failed —
/// callers pass it to `SessionManager::reattach_running_sessions`
/// so those missions' alive panes are stopped instead of reattached
/// (preserving the pre-eager-mount safety property that mission
/// bytes never stream into a non-existent bus).
pub(crate) async fn reattach_all_running_missions(
    state: &AppState,
    app: &tauri::AppHandle,
) -> HashSet<String> {
    let mission_ids = match state.db.get() {
        Ok(conn) => list_running_mission_ids(&conn).unwrap_or_else(|e| {
            log::error!("reattach_all_running_missions query failed: {e}");
            Vec::new()
        }),
        Err(e) => {
            log::error!("reattach_all_running_missions db pool unavailable: {e}");
            Vec::new()
        }
    };

    let mut failed = HashSet::new();
    for mission_id in mission_ids {
        match ensure_mission_router_mounted(state, app, &mission_id).await {
            Ok(()) => {
                log::info!("mission reattach: id={mission_id} action=mounted");
            }
            Err(e) => {
                log::info!("mission reattach: id={mission_id} action=mount_failed → stop");
                log::warn!("mission {mission_id} mount-failed reattach: {e}");
                failed.insert(mission_id);
            }
        }
    }
    failed
}

/// Return the ids of every mission that's currently `running` and not
/// archived. Factored out of `reattach_all_running_missions` so the
/// filter logic is unit-testable without an `AppHandle`.
fn list_running_mission_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM missions
           WHERE status = 'running' AND archived_at IS NULL
           ORDER BY started_at ASC",
    )?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Pause a mission by killing every live PTY but leaving the mission
/// row in `running` state, the router mounted, and the bus mounted.
/// Pairs with `session_resume` per slot for "Resume all" — the
/// frontend iterates the session list. No `mission_stopped` event is
/// written; the audit trail of per-PTY exits already lives in the log
/// via `session/exit`.
///
/// Use this for "I want to stop the agents but might come back later."
/// For end-of-mission, see `mission_archive`.
#[tauri::command]
pub async fn mission_stop(state: State<'_, AppState>, id: String) -> Result<Mission> {
    log::info!("mission stop: id={id}");
    state.sessions.kill_all_for_mission(&id)?;
    let conn = state.db.get()?;
    get(&conn, &id)
}

/// Toggle a mission's pin. Pinned missions float to the top of the
/// sidebar's MISSION list (sort key: `pinned_at IS NULL, pinned_at
/// DESC, started_at DESC`). Setting `pinned = false` clears the
/// timestamp.
#[tauri::command]
pub async fn mission_pin(state: State<'_, AppState>, id: String, pinned: bool) -> Result<Mission> {
    let conn = state.db.get()?;
    let pinned_at: Option<String> = if pinned {
        Some(now().to_rfc3339())
    } else {
        None
    };
    let n = conn.execute(
        "UPDATE missions SET pinned_at = ?1 WHERE id = ?2",
        params![pinned_at, id],
    )?;
    if n != 1 {
        return Err(Error::msg(format!("mission not found: {id}")));
    }
    get(&conn, &id)
}

/// Rename a mission. Title is trimmed; empty values are rejected so
/// the sidebar never renders a blank row. The mission's event log is
/// untouched — the title only ever lived on the row.
#[tauri::command]
pub async fn mission_rename(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<Mission> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(Error::msg("mission title must not be empty"));
    }
    let conn = state.db.get()?;
    let n = conn.execute(
        "UPDATE missions SET title = ?1 WHERE id = ?2",
        params![trimmed, id],
    )?;
    if n != 1 {
        return Err(Error::msg(format!("mission not found: {id}")));
    }
    get(&conn, &id)
}

/// Reset a mission: wipe the run context (event log, agent session
/// keys, router state) and respawn every slot fresh against the same
/// mission row. Mostly for testing — gives you a clean slate without
/// having to rebuild the crew + start a new mission. Preserves the
/// mission's id, title, crew, cwd, and goal so links/bookmarks survive.
#[tauri::command]
pub async fn mission_reset(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<Mission> {
    use crate::event_bus::{BusEmitter, TauriBusEvents};
    use crate::router::{
        open_log_for_mission, CompositeBusEmitter, Router, RouterSubscriber, StdinInjector,
    };
    use crate::session::manager::{SessionEvents, TauriSessionEvents};
    use std::sync::Arc;

    // 1. Snapshot the mission + crew + roster up front.
    let mission_snap = {
        let conn = state.db.get()?;
        get(&conn, &id)?
    };
    let (crew_name, crew_signal_types, crew_goal) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &mission_snap.crew_id)?;
        (crew.name, crew.signal_types, crew.goal)
    };
    let roster = {
        let conn = state.db.get()?;
        slot::list(&conn, &mission_snap.crew_id)?
    };
    if roster.is_empty() {
        return Err(Error::msg(format!(
            "crew {crew_name} has no slots; cannot reset mission",
        )));
    }
    if !roster.iter().any(|m| m.slot.lead) {
        return Err(Error::msg(format!(
            "crew {crew_name} has no lead slot; cannot reset mission",
        )));
    }

    // 2. Tear down the live state. Kill PTYs first (blocks until
    // reader threads join), then unmount bus + router. Same order as
    // mission_archive so any final events the bus is draining still
    // reach a live router.
    state.sessions.kill_all_for_mission(&id)?;
    state.buses.unmount(&id);
    state.routers.unregister(&id);

    // 3. Archive existing session rows for this mission so the sidebar
    // / list queries don't show ghost rows pointing at PTYs that no
    // longer exist. Fresh sessions get inserted by the spawn loop
    // below.
    {
        let conn = state.db.get()?;
        conn.execute(
            "UPDATE sessions
                SET archived_at = ?1
              WHERE mission_id = ?2 AND archived_at IS NULL",
            params![now().to_rfc3339(), id],
        )?;
    }

    // 4. Wipe the event log + per-mission shim dir so the next spawn
    // starts from a clean slate. signal_types + roster sidecars get
    // rewritten below from the current crew / roster state.
    let mission_dir = event_log::mission_dir(&state.app_data_dir, &mission_snap.crew_id, &id);
    let events_file = event_log::events_path(&state.app_data_dir, &mission_snap.crew_id, &id);
    if events_file.exists() {
        std::fs::remove_file(&events_file)?;
    }
    // Drop per-(mission,handle) runner shim dirs — they'll be regenerated
    // by SessionManager::spawn with the freshened env block.
    let shims_root = state.app_data_dir.join("missions").join(&id).join("shims");
    if shims_root.exists() {
        let _ = std::fs::remove_dir_all(&shims_root);
    }
    std::fs::create_dir_all(&mission_dir)?;
    write_signal_types_sidecar(
        &state.app_data_dir,
        &mission_snap.crew_id,
        &crew_signal_types,
    )?;
    write_roster_sidecar(&mission_dir, &roster)?;

    // 5. Update mission row: status back to running, started_at
    // refreshed (this IS a fresh run), stopped_at + archived_at
    // cleared. Title / goal_override / cwd / pinned_at preserved.
    //
    // archived_at must be cleared in lockstep with the status flip:
    // reset is a return-to-running, and `list()` keys on `archived_at
    // IS NULL`, so leaving the stamp would make a freshly-reset live
    // mission vanish from the sidebar / palette. The UI gates reset
    // on `status === 'running'` today so this can't happen via the
    // workspace, but `mission_reset` is a public Tauri command and
    // the backend invariant has to hold regardless of caller.
    let started_at_dt = now();
    {
        let conn = state.db.get()?;
        let n = conn.execute(
            "UPDATE missions
                SET status = 'running',
                    started_at = ?1,
                    stopped_at = NULL,
                    archived_at = NULL
              WHERE id = ?2",
            params![started_at_dt.to_rfc3339(), id],
        )?;
        if n != 1 {
            return Err(Error::msg(format!("mission not found: {id}")));
        }
    }

    // 6. Re-emit the opening events so router can replay the launch
    // prompt to the lead. Same shape mission_start writes.
    let goal_text = mission_snap
        .goal_override
        .as_deref()
        .or(crew_goal.as_deref())
        .unwrap_or("")
        .to_string();
    let log = EventLog::open(&mission_dir)?;
    log.append(EventDraft {
        crew_id: mission_snap.crew_id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: None,
        signal_type: Some(SignalType::new("mission_start")),
        payload: serde_json::json!({
            "title": mission_snap.title,
            "cwd": mission_snap.cwd,
        }),
    })?;
    log.append(EventDraft {
        crew_id: mission_snap.crew_id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "human".into(),
        to: None,
        signal_type: Some(SignalType::new("mission_goal")),
        payload: serde_json::json!({ "text": goal_text }),
    })?;

    // Pre-compose each slot's first-user-turn body so the spawn loop
    // can deliver it via the positional `[PROMPT]` argv at process
    // boot — same contract as `mission_start`. Borrows of
    // `crew_name` / `crew_signal_types` end here; both are moved
    // into `Router::new` below.
    let first_turns: Vec<Option<String>> = {
        let roster_entries: Vec<crate::router::prompt::RosterEntry> = roster
            .iter()
            .map(|m| crate::router::prompt::RosterEntry {
                handle: m.slot.slot_handle.as_str(),
                display_name: m.runner.display_name.as_str(),
                lead: m.slot.lead,
            })
            .collect();
        let lead_member = roster.iter().find(|m| m.slot.lead);
        roster
            .iter()
            .map(|m| {
                if m.slot.lead {
                    lead_member.map(|lm| {
                        crate::router::prompt::compose_launch_prompt(
                            &crate::router::prompt::LaunchPromptInput {
                                lead: crate::router::prompt::LeadView {
                                    handle: lm.slot.slot_handle.as_str(),
                                    display_name: lm.runner.display_name.as_str(),
                                    system_prompt: lm.runner.system_prompt.as_deref(),
                                },
                                crew_name: crew_name.as_str(),
                                mission_goal: goal_text.as_str(),
                                roster: &roster_entries,
                                allowed_signals: &crew_signal_types,
                            },
                        )
                    })
                } else {
                    Some(crate::router::prompt::compose_worker_first_turn(
                        m.runner.system_prompt.as_deref(),
                    ))
                }
            })
            .collect()
    };

    // 7. Build router + spawn fresh PTYs + mount bus. Same ordering
    // contract as mission_start: spawn first so register_sessions has
    // the full handle map before the bus's initial replay fires the
    // router's mission_goal handler.
    let events_log_path = event_log::events_path(&state.app_data_dir, &mission_snap.crew_id, &id);
    let log_arc = open_log_for_mission(&mission_dir)?;
    let injector: Arc<dyn StdinInjector> = Arc::clone(&state.sessions) as Arc<dyn StdinInjector>;
    let router = Router::new(
        id.clone(),
        mission_snap.crew_id.clone(),
        crew_name,
        &roster,
        crew_signal_types,
        Arc::clone(&log_arc),
        injector,
    )?;

    let emitter: Arc<dyn SessionEvents> = Arc::new(TauriSessionEvents(app.clone()));
    let mut spawned_pairs: Vec<(String, String)> = Vec::with_capacity(roster.len());
    let mission_for_spawn = {
        let conn = state.db.get()?;
        get(&conn, &id)?
    };
    // All-or-nothing: same contract as `mission_start`. If any slot
    // fails to spawn, kill the PTYs that did come up, archive the
    // freshly-inserted session rows, flip the mission back to
    // `aborted`, and surface the original error. Without rollback the
    // mission would sit half-reset — old PTYs gone, some new ones
    // alive, no bus / router mounted, and the mission row stuck in
    // `running`.
    for (idx, member) in roster.iter().enumerate() {
        let first_turn = first_turns.get(idx).cloned().flatten();
        let spawn_res = state.sessions.spawn(
            &mission_for_spawn,
            &member.runner,
            &member.slot,
            &state.app_data_dir,
            events_log_path.clone(),
            state.db.clone(),
            Arc::clone(&emitter),
            first_turn,
        );
        match spawn_res {
            Ok(spawned) => {
                spawned_pairs.push((member.slot.slot_handle.clone(), spawned.id));
            }
            Err(e) => {
                let _ = state.sessions.kill_all_for_mission(&id);
                if let Ok(conn) = state.db.get() {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET archived_at = ?1
                          WHERE mission_id = ?2 AND archived_at IS NULL",
                        params![now().to_rfc3339(), id],
                    );
                    let _ = conn.execute(
                        "UPDATE missions
                            SET status = 'aborted', stopped_at = ?1
                          WHERE id = ?2",
                        params![now().to_rfc3339(), id],
                    );
                }
                return Err(e);
            }
        }
    }
    router.register_sessions(&spawned_pairs);

    let roster_handles: Vec<String> = roster.iter().map(|m| m.slot.slot_handle.clone()).collect();
    let tauri_emitter: Arc<dyn BusEmitter> = Arc::new(TauriBusEvents(app.clone()));
    let router_emitter: Arc<dyn BusEmitter> = Arc::new(RouterSubscriber(Arc::clone(&router)));
    let composite: Arc<dyn BusEmitter> = Arc::new(CompositeBusEmitter::new(vec![
        tauri_emitter,
        router_emitter,
    ]));
    if let Err(e) = state
        .buses
        .mount(id.clone(), &mission_dir, &roster_handles, composite)
    {
        // Bus didn't attach — kill the sessions we spawned, archive
        // their rows, abort the mission. Same shape as the spawn-loop
        // rollback above; the bus is the last gate before commit so a
        // failure here would otherwise leave live PTYs with no router
        // listening.
        let _ = state.sessions.kill_all_for_mission(&id);
        if let Ok(conn) = state.db.get() {
            let _ = conn.execute(
                "UPDATE sessions
                    SET archived_at = ?1
                  WHERE mission_id = ?2 AND archived_at IS NULL",
                params![now().to_rfc3339(), id],
            );
            let _ = conn.execute(
                "UPDATE missions
                    SET status = 'aborted', stopped_at = ?1
                  WHERE id = ?2",
                params![now().to_rfc3339(), id],
            );
        }
        return Err(e);
    }
    state.routers.register(id.clone(), router);

    Ok(mission_for_spawn)
}

/// Terminal end-of-mission. Kills every live PTY, writes the
/// `mission_stopped` event, flips the mission row to `completed`, and
/// drops the router + bus. Mirrors what `mission_stop` used to do
/// before the lifecycle split — preserved as a separate command so
/// the workspace UI can guard it behind an explicit confirm.
#[tauri::command]
pub async fn mission_archive(state: State<'_, AppState>, id: String) -> Result<Mission> {
    state.sessions.kill_all_for_mission(&id)?;
    let mut conn = state.db.get()?;
    let mission = stop(&mut conn, &state.app_data_dir, &id)?;
    state.buses.unmount(&id);
    state.routers.unregister(&id);
    Ok(mission)
}

#[tauri::command]
pub async fn mission_list(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<Mission>> {
    let conn = state.db.get()?;
    list(&conn, crew_id.as_deref())
}

#[tauri::command]
pub async fn mission_get(state: State<'_, AppState>, id: String) -> Result<Mission> {
    let conn = state.db.get()?;
    get(&conn, &id)
}

/// Count `human_question` events in `mission_dir`'s log that have not yet
/// been answered by a `human_response` referencing them. The router's
/// in-memory `pending_asks` map is the live source of truth for mounted
/// missions; this helper covers the cold path: a mission whose router
/// isn't registered (post-restart, before the user reopens the workspace,
/// or any mission still flagged `running` in the DB without a live
/// process behind it). Without it, `mission_list_summary` would silently
/// drop the pending-ask flag for orphaned running rows.
fn count_pending_asks_from_log(mission_dir: &Path) -> usize {
    let log = match EventLog::open(mission_dir) {
        Ok(l) => l,
        Err(_) => return 0,
    };
    let entries = match log.read_from_lossy(0) {
        Ok((entries, _skipped)) => entries,
        Err(_) => return 0,
    };
    // pending: human_question.id → still unanswered. Removed when a
    // matching human_response lands. Mirrors Router::reconstruct_from_log
    // so a reopen sees the same state as the live registry would have.
    let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in &entries {
        let event = &entry.event;
        if !matches!(event.kind, runner_core::model::EventKind::Signal) {
            continue;
        }
        let Some(t) = event.signal_type.as_ref() else {
            continue;
        };
        match t.as_str() {
            "human_question" => {
                pending.insert(event.id.clone());
            }
            "human_response" => {
                if let Some(qid) = event.payload.get("question_id").and_then(|v| v.as_str()) {
                    pending.remove(qid);
                }
            }
            _ => {}
        }
    }
    pending.len()
}

#[tauri::command]
pub async fn mission_list_summary(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<MissionSummary>> {
    let conn = state.db.get()?;
    let missions = list(&conn, crew_id.as_deref())?;

    // Crew name lookup. We could JOIN in SQL, but the row count for
    // missions is small in v0 and `crew::get` is a single indexed lookup;
    // doing it in Rust keeps the SQL identical to `list()` and avoids a
    // second row-mapper for the joined shape.
    let mut summaries = Vec::with_capacity(missions.len());
    for m in missions {
        let crew_name = match crew::get(&conn, &m.crew_id) {
            Ok(c) => c.name,
            Err(_) => String::new(), // crew was deleted; row still surfaces
        };
        // Live router is authoritative when mounted. For unmounted
        // missions — terminal status, or orphan `running` rows after a
        // restart — fall back to a log scan so the badge stays accurate
        // until the workspace remounts the router. Past missions
        // typically resolve to 0 too (every ask was answered or the
        // human walked away) but the scan still surfaces genuinely
        // abandoned cards from a prior run.
        let pending_ask_count = match state.routers.get(&m.id) {
            Some(router) => router.pending_ask_count(),
            None => {
                let mission_dir = event_log::mission_dir(&state.app_data_dir, &m.crew_id, &m.id);
                count_pending_asks_from_log(&mission_dir)
            }
        };
        summaries.push(MissionSummary {
            mission: m,
            crew_name,
            pending_ask_count,
        });
    }
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::crew::CreateCrewInput;
    use crate::commands::runner::{self as runner_cmd, CreateRunnerInput};
    use crate::db;

    fn pool() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    fn seed_crew(conn: &Connection, name: &str, goal: Option<&str>) -> String {
        let crew = crew::create(
            conn,
            CreateCrewInput {
                name: name.into(),
                purpose: None,
                goal: goal.map(String::from),
            },
        )
        .unwrap();
        crew.id
    }

    fn add_runner(conn: &mut Connection, crew_id: &str, handle: &str) {
        // Runners are config templates; in-mission identity is on
        // the slot. Test fixtures use the runner handle as both the
        // template name and the slot_handle for simplicity.
        let r = runner_cmd::create(
            conn,
            CreateRunnerInput {
                handle: handle.into(),
                display_name: handle.into(),
                runtime: "shell".into(),
                command: "/bin/sh".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: std::collections::HashMap::new(),
                model: None,
                effort: None,
                permission_mode: crate::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap();
        slot::create(conn, crew_id, &r.id, handle).unwrap();
    }

    #[test]
    fn start_rejects_goal_override_over_cap() {
        // Plan 0007: validation at persist time keeps the composed
        // launch prompt under the runtime argv ceiling. mission_start
        // refuses a goal_override over MAX_MISSION_GOAL_BYTES.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "C", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let oversized = "Z".repeat(MAX_MISSION_GOAL_BYTES + 1);
        let err = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "Try".into(),
                goal_override: Some(oversized),
                cwd: None,
            },
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("goal"), "expected goal-size error, got {msg}");
    }

    #[test]
    fn start_rejects_crew_with_no_runners() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "Empty", None);
        let tmp = tempfile::tempdir().unwrap();

        let err = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "Try".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no slots"),
            "expected 'no slots' error, got {msg}"
        );
    }

    #[test]
    fn start_rejects_empty_title() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "coder");
        let tmp = tempfile::tempdir().unwrap();

        let err = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "   ".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap_err();
        assert!(format!("{err}").contains("title must not be empty"));
    }

    #[test]
    fn start_writes_two_opening_events_and_sidecar() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "Alpha", Some("Ship v0"));
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "first mission".into(),
                goal_override: None,
                cwd: Some("/tmp/work".into()),
            },
        )
        .unwrap();

        assert_eq!(out.mission.title, "first mission");
        assert_eq!(out.mission.status, MissionStatus::Running);
        assert_eq!(out.goal, "Ship v0");

        // Event log has mission_start + mission_goal.
        let mission_dir = event_log::mission_dir(tmp.path(), &crew_id, &out.mission.id);
        let log = EventLog::open(&mission_dir).unwrap();
        let entries = log.read_from(0).unwrap();
        assert_eq!(entries.len(), 2, "expected two opening events");

        let first = &entries[0].event;
        assert_eq!(first.kind, EventKind::Signal);
        assert_eq!(first.from, "system");
        assert_eq!(
            first.signal_type.as_ref().unwrap().as_str(),
            "mission_start"
        );
        assert_eq!(first.payload["title"], "first mission");
        assert_eq!(first.payload["cwd"], "/tmp/work");

        let second = &entries[1].event;
        assert_eq!(second.kind, EventKind::Signal);
        assert_eq!(second.from, "human");
        assert_eq!(
            second.signal_type.as_ref().unwrap().as_str(),
            "mission_goal"
        );
        assert_eq!(second.payload["text"], "Ship v0");
        // mission_goal must sort strictly after mission_start.
        assert!(second.id > first.id);

        // Signal-types sidecar exists with the crew's allowlist.
        let sidecar = event_log::signal_types_path(tmp.path(), &crew_id);
        assert!(sidecar.exists());
        let raw = std::fs::read_to_string(&sidecar).unwrap();
        let types: Vec<String> = serde_json::from_str(&raw).unwrap();
        assert!(types.contains(&"mission_goal".to_string()));
        assert!(types.contains(&"ask_lead".to_string()));
    }

    #[test]
    fn start_override_beats_crew_default_goal() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", Some("default goal"));
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "m".into(),
                goal_override: Some("override goal".into()),
                cwd: None,
            },
        )
        .unwrap();

        assert_eq!(out.goal, "override goal");
    }

    #[test]
    fn stop_marks_completed_and_appends_event() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();

        let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
        assert_eq!(stopped.status, MissionStatus::Completed);
        assert!(stopped.stopped_at.is_some());

        let log = EventLog::open(&event_log::mission_dir(
            tmp.path(),
            &crew_id,
            &out.mission.id,
        ))
        .unwrap();
        let entries = log.read_from(0).unwrap();
        assert_eq!(entries.len(), 3, "start + goal + stopped");
        let last = &entries[2].event;
        assert_eq!(
            last.signal_type.as_ref().unwrap().as_str(),
            "mission_stopped"
        );
        assert_eq!(last.from, "system");
    }

    #[test]
    fn stop_rejects_already_stopped_mission() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        stop(&mut conn, tmp.path(), &out.mission.id).unwrap();

        let err = stop(&mut conn, tmp.path(), &out.mission.id).unwrap_err();
        assert!(format!("{err}").contains("not running"));
    }

    #[test]
    fn list_filters_by_crew_and_orders_by_started_at_desc() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let a = seed_crew(&conn, "A", None);
        let b = seed_crew(&conn, "B", None);
        // C5.5: handles are globally unique — give each crew a distinct one.
        add_runner(&mut conn, &a, "lead-a");
        add_runner(&mut conn, &b, "lead-b");
        let tmp = tempfile::tempdir().unwrap();

        let m1 = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: a.clone(),
                title: "first".into(),
                goal_override: Some("x".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission;
        // Per #55 concurrent missions on one crew are allowed, so we
        // don't need to stop m1 before starting m2. We also avoid
        // stop() here because it now stamps `archived_at` atomically
        // with the status flip — calling it would hide m1 from
        // list(), which would defeat this test's purpose (verifying
        // the crew filter + ordering, not the archive filter; the
        // latter has its own test).
        std::thread::sleep(std::time::Duration::from_millis(5));
        let m2 = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: a.clone(),
                title: "second".into(),
                goal_override: Some("y".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission;
        start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: b,
                title: "other crew".into(),
                goal_override: Some("z".into()),
                cwd: None,
            },
        )
        .unwrap();

        let for_a = list(&conn, Some(&a)).unwrap();
        assert_eq!(for_a.len(), 2);
        assert_eq!(for_a[0].id, m2.id, "newest first");
        assert_eq!(for_a[1].id, m1.id);

        let all = list(&conn, None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn concurrent_missions_on_same_crew_are_allowed() {
        // Per #55 the "at most one live mission per crew" guard was
        // lifted: per-mission state (sessions, kill_all_for_mission,
        // shim path, roster sidecar, router) is fully namespaced by
        // mission_id, so a second mission_start on the same crew
        // produces a distinct live mission rather than rejecting.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let first = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "first".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();

        let second = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "second".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();

        assert_ne!(
            first.mission.id, second.mission.id,
            "second mission must get a distinct id",
        );

        // Both rows must show status='running' and reference the same
        // crew. Order by started_at so the assertion doesn't depend on
        // ULID stamping order.
        let mut rows: Vec<(String, String, String)> = conn
            .prepare(
                "SELECT id, status, crew_id FROM missions
                  WHERE crew_id = ?1
                  ORDER BY started_at ASC",
            )
            .unwrap()
            .query_map(params![crew_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows.len(), 2, "two mission rows expected: {rows:?}");
        for (_, status, row_crew) in &rows {
            assert_eq!(status, "running");
            assert_eq!(row_crew, &crew_id);
        }
    }

    #[test]
    fn sidecar_is_rewritten_on_second_start_for_same_crew() {
        // Regression for the Windows rename-over-existing issue. On Unix the
        // test passes trivially; on Windows it previously failed because
        // `std::fs::rename` errors when the destination exists.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m1".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        stop(&mut conn, tmp.path(), &out.mission.id).unwrap();

        // Sidecar now exists — starting the next mission must overwrite it.
        start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m2".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();

        let sidecar = event_log::signal_types_path(tmp.path(), &crew_id);
        assert!(sidecar.exists());
        let types: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
        assert!(types.contains(&"mission_goal".to_string()));
    }

    #[test]
    fn concurrent_stop_appends_exactly_one_terminal_event() {
        // Two threads race to stop the same running mission. Without the
        // conditional UPDATE, both would see `running`, both would flip the
        // row, and both would append `mission_stopped`. With it, exactly one
        // UPDATE affects a row and exactly one log append happens.
        use std::sync::Arc;
        use std::thread;

        // The default `pool()` helper caps at 1 connection + :memory: which
        // gives each connection its own isolated DB — unusable for a race.
        // Use a file-backed DB on disk so multiple pool connections share state.
        let db_tmp = tempfile::tempdir().unwrap();
        let db_path = db_tmp.path().join("race.db");
        let pool = db::open_pool(&db_path).unwrap();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = Arc::new(tempfile::tempdir().unwrap());

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        drop(conn); // release our pool handle so both threads can grab one

        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let tmp_a = Arc::clone(&tmp);
        let tmp_b = Arc::clone(&tmp);
        let id = out.mission.id.clone();
        let id_a = id.clone();
        let id_b = id.clone();
        let h1 = thread::spawn(move || {
            let mut conn = pool_a.get().unwrap();
            stop(&mut conn, tmp_a.path(), &id_a)
        });
        let h2 = thread::spawn(move || {
            let mut conn = pool_b.get().unwrap();
            stop(&mut conn, tmp_b.path(), &id_b)
        });
        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();

        // Exactly one succeeded and exactly one failed with "not running".
        let (ok_count, err_count) = [&r1, &r2].iter().fold((0, 0), |(o, e), r| match r {
            Ok(_) => (o + 1, e),
            Err(err) => {
                assert!(
                    format!("{err}").contains("not running"),
                    "loser should report not-running, got {err}"
                );
                (o, e + 1)
            }
        });
        assert_eq!((ok_count, err_count), (1, 1));

        // Log has exactly one `mission_stopped` event.
        let log = EventLog::open(&event_log::mission_dir(tmp.path(), &crew_id, &id)).unwrap();
        let stopped_events = log
            .read_from(0)
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.event
                    .signal_type
                    .as_ref()
                    .map(|t| t.as_str() == "mission_stopped")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(stopped_events, 1, "exactly one terminal event must land");
    }

    #[test]
    fn read_events_returns_appended_in_order() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", Some("Ship v0"));
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();

        // mission_start + mission_goal — that's all the opening events
        // plant. Mirrors what the workspace UI sees on first mount.
        let events = read_events(tmp.path(), &conn, &out.mission.id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].signal_type.as_ref().unwrap().as_str(),
            "mission_start"
        );
        assert_eq!(
            events[1].signal_type.as_ref().unwrap().as_str(),
            "mission_goal"
        );
    }

    #[test]
    fn pending_ask_count_from_log_pairs_questions_with_responses() {
        // A mission whose router isn't mounted (post-restart, terminal
        // status, etc.) still needs an accurate pending-ask count for the
        // Missions list flag. Append two human_question events and answer
        // only the second one; the helper must report 1 unanswered.
        use runner_core::model::{EventDraft, EventKind, SignalType};

        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();

        let mission_dir = event_log::mission_dir(tmp.path(), &crew_id, &out.mission.id);
        assert_eq!(
            count_pending_asks_from_log(&mission_dir),
            0,
            "fresh mission has no pending asks"
        );

        let log = EventLog::open(&mission_dir).unwrap();
        let q1 = log
            .append(EventDraft::signal(
                crew_id.clone(),
                out.mission.id.clone(),
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"prompt": "?"}),
            ))
            .unwrap();
        let _q2 = log
            .append(EventDraft::signal(
                crew_id.clone(),
                out.mission.id.clone(),
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"prompt": "??"}),
            ))
            .unwrap();
        assert_eq!(count_pending_asks_from_log(&mission_dir), 2);

        // Resolve only the first question.
        log.append(EventDraft {
            crew_id: crew_id.clone(),
            mission_id: out.mission.id.clone(),
            kind: EventKind::Signal,
            from: "human".into(),
            to: None,
            signal_type: Some(SignalType::new("human_response")),
            payload: serde_json::json!({"question_id": q1.id, "choice": "yes"}),
        })
        .unwrap();
        assert_eq!(count_pending_asks_from_log(&mission_dir), 1);
    }

    #[test]
    fn read_events_unknown_mission_errors() {
        let pool = pool();
        let conn = pool.get().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let err = read_events(tmp.path(), &conn, "01HMISSING").unwrap_err();
        assert!(
            format!("{err}").contains("mission not found"),
            "expected not-found, got {err}"
        );
    }

    #[test]
    fn start_rolls_back_row_when_log_append_fails() {
        // Force `EventLog::open` to fail by giving it an `app_data_dir` that
        // can't be created (we preemptively occupy the path with a regular
        // file so `create_dir_all` bails). The mission row must not survive
        // the failure.
        use std::fs;

        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");

        let tmp = tempfile::tempdir().unwrap();
        // Block the `crews/` subtree by making it a file instead of a dir.
        fs::write(tmp.path().join("crews"), b"blocked").unwrap();

        let err = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::Io(_)),
            "expected IO failure from FS, got {err:?}"
        );

        // No phantom mission.
        let missions = list(&conn, Some(&crew_id)).unwrap();
        assert!(
            missions.is_empty(),
            "mission row must be rolled back; found {missions:?}"
        );
    }

    #[test]
    fn list_excludes_archived_missions() {
        // `archived_at IS NULL` filter at SQL hides archived missions
        // from every list surface (sidebar, ⌘K palette, summary). Open
        // by direct URL still resolves through `get()`.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "live".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();

        // Visible while running.
        assert_eq!(
            list(&conn, Some(&crew_id)).unwrap().len(),
            1,
            "running mission must list"
        );

        // Archive it (stop() flips status AND stamps archived_at atomically).
        let archived = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
        assert!(archived.archived_at.is_some());

        // Hidden from list().
        assert!(
            list(&conn, Some(&crew_id)).unwrap().is_empty(),
            "archived mission must not appear in list"
        );
        // Still resolves by id via get().
        let fetched = get(&conn, &out.mission.id).unwrap();
        assert!(fetched.archived_at.is_some());
        assert_eq!(fetched.status, MissionStatus::Completed);
    }

    #[test]
    fn stop_sets_archived_at_alongside_status() {
        // stop() is the only path to status='completed' (only called
        // from mission_archive); the same UPDATE must stamp
        // archived_at so a future row can never be observed as
        // completed-but-not-archived.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();

        assert_eq!(stopped.status, MissionStatus::Completed);
        let stopped_ts = stopped.stopped_at.expect("stopped_at populated");
        let archived_ts = stopped.archived_at.expect("archived_at populated");
        assert_eq!(stopped_ts, archived_ts, "must share the timestamp");
    }

    #[test]
    fn aborted_missions_stay_visible_in_list() {
        // 0004 backfill is narrow: only `status='completed'` rows get
        // archived_at; aborted rows (spawn-failure rollback) stay in
        // the visible list because they're triage state, not archive.
        // Simulate the production rollback path's UPDATE.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        // Mirror mission_start's rollback path: flip to aborted
        // without setting archived_at.
        conn.execute(
            "UPDATE missions SET status = 'aborted', stopped_at = ?1 WHERE id = ?2",
            params![now().to_rfc3339(), out.mission.id],
        )
        .unwrap();

        let listed = list(&conn, Some(&crew_id)).unwrap();
        assert_eq!(listed.len(), 1, "aborted mission must remain in list");
        assert!(
            listed[0].archived_at.is_none(),
            "aborted mission must not be archived"
        );
        assert_eq!(listed[0].status, MissionStatus::Aborted);
    }

    #[test]
    fn reset_clears_archived_at() {
        // mission_reset's step-5 UPDATE (in production) flips status
        // back to running and must clear archived_at in lockstep —
        // otherwise the freshly-reset live row stays hidden from
        // list() (which filters archived_at IS NULL). The UI today
        // gates reset on status='running', but the backend invariant
        // has to hold regardless of caller. Simulate the reset UPDATE
        // directly here so the test stays focused on the SQL contract.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: Some("go".into()),
                cwd: None,
            },
        )
        .unwrap();
        let archived = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
        assert!(archived.archived_at.is_some(), "stop() must archive");
        assert!(
            list(&conn, Some(&crew_id)).unwrap().is_empty(),
            "archived row hidden before reset"
        );

        // Mirror mission_reset's step-5 SQL (mission.rs:1009-1019).
        conn.execute(
            "UPDATE missions
                SET status = 'running',
                    started_at = ?1,
                    stopped_at = NULL,
                    archived_at = NULL
              WHERE id = ?2",
            params![now().to_rfc3339(), out.mission.id],
        )
        .unwrap();

        let listed = list(&conn, Some(&crew_id)).unwrap();
        assert_eq!(
            listed.len(),
            1,
            "reset must make the mission visible to list() again"
        );
        assert_eq!(listed[0].id, out.mission.id);
        assert!(listed[0].archived_at.is_none(), "archived_at cleared");
        assert_eq!(listed[0].status, MissionStatus::Running);
        assert!(listed[0].stopped_at.is_none(), "stopped_at cleared");
    }

    #[test]
    fn migration_backfills_archived_at_for_completed_rows() {
        // The 0004 migration runs once on first open. Simulate the
        // pre-migration state by inserting a `completed` row with
        // `archived_at` already nulled out (which is how the column
        // would look pre-migration), then re-run the migration body
        // and assert the backfill stamped archived_at = stopped_at.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        // Seed a "pre-migration" completed mission. Use the production
        // stop() then NULL out archived_at to mimic the state of rows
        // that existed before the 0004 column landed.
        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "old".into(),
                goal_override: Some("g".into()),
                cwd: None,
            },
        )
        .unwrap();
        let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
        let original_stopped_at = stopped.stopped_at.unwrap().to_rfc3339();
        conn.execute(
            "UPDATE missions SET archived_at = NULL WHERE id = ?1",
            params![out.mission.id],
        )
        .unwrap();

        // Sanity: row currently looks pre-migration.
        let pre: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM missions WHERE id = ?1",
                params![out.mission.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pre.is_none());

        // Re-run the 0004 backfill statement and assert the post state.
        conn.execute(
            "UPDATE missions
                SET archived_at = stopped_at
              WHERE status = 'completed' AND archived_at IS NULL",
            [],
        )
        .unwrap();

        let backfilled = get(&conn, &out.mission.id).unwrap();
        let backfilled_archived_at = backfilled
            .archived_at
            .expect("backfill must populate archived_at")
            .to_rfc3339();
        assert_eq!(backfilled_archived_at, original_stopped_at);
    }

    #[test]
    fn list_running_mission_ids_filters_status_and_archive() {
        // Startup reconciler should mount router + bus for `running`
        // missions only — archived rows and completed/aborted rows
        // are out of scope. Stable order by started_at so the
        // iteration is reproducible across restarts.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "C", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let m_running_first = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "running-1".into(),
                goal_override: Some("g".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission
        .id;
        let m_running_second = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "running-2".into(),
                goal_override: Some("g".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission
        .id;
        let m_completed = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id: crew_id.clone(),
                title: "completed".into(),
                goal_override: Some("g".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission
        .id;
        stop(&mut conn, tmp.path(), &m_completed).unwrap();

        // A third running row that we manually mark archived to
        // simulate a "soft-deleted while running" case the reconciler
        // must ignore.
        let m_archived = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                crew_id,
                title: "running-archived".into(),
                goal_override: Some("g".into()),
                cwd: None,
            },
        )
        .unwrap()
        .mission
        .id;
        conn.execute(
            "UPDATE missions SET archived_at = ?2 WHERE id = ?1",
            params![m_archived, Utc::now().to_rfc3339()],
        )
        .unwrap();

        let ids = list_running_mission_ids(&conn).unwrap();
        assert_eq!(
            ids,
            vec![m_running_first, m_running_second],
            "only non-archived running rows, ordered by started_at"
        );
    }
}

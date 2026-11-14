pub mod budget;
pub mod control;
pub mod feed;
pub mod interrupt;
pub mod link;
pub mod report;
pub mod review;
pub mod roles;
pub mod roster;

pub fn mission_set_project(
    state: &AppCore,
    mission_id: String,
    project_id: Option<String>,
) -> Result<Mission> {
    let (mission_node, target_id, order) = {
        let conn = state.db.get()?;
        let mission = get(&conn, &mission_id)?;
        let mission_node =
            repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &mission_id)?
                .ok_or_else(|| Error::msg(format!("mission node not found: {mission_id}")))?;
        let target_id = match project_id {
            Some(id) => {
                crate::ops::project::get(&conn, &id)?;
                Some(repo::node::ensure_project_node(&conn, &id)?.id)
            }
            None => None,
        };
        if mission_node.parent_id == target_id {
            return Ok(mission);
        }
        let order = crate::ops::node::append_order(&conn, target_id.as_deref(), &mission_node.id)?;
        (mission_node, target_id, order)
    };
    crate::ops::node::node_move(state, mission_node.id, target_id, order)?;
    let conn = state.db.get()?;
    let mission = get(&conn, &mission_id)?;
    Ok(mission)
}

#[cfg(test)]
use crate::ops::project::ProjectScope;
// Mission lifecycle — start, stop, list, get.
//
// A mission is the runtime container: it owns a directory, an NDJSON event
// log, and a set of sessions (spawned in C6). This module only does the
// bookkeeping layer — no PTYs yet.
//
// `mission_start` is the point where config crystallizes into runtime:
// validate the crew has ≥1 slot, create the mission
// row, create the mission directory, and emit the two opening events —
// `mission_start` (system announces the run) and `mission_goal` (the
// human's intent, shared in every member's first turn).

use std::path::Path;

use chrono::Utc;
use runner_core::event_log::{self, EventLog};
use runner_core::model::{EventDraft, EventKind, KnownSignalType, SignalType};
use rusqlite::Connection;
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Mission, MissionStatus, Runtime, SessionStatus, Timestamp},
    ops::{
        crew,
        project::{self},
        slot,
    },
    repo, AppCore,
};

pub use runner_core::protocol::mission::StartMissionInput;

pub use runner_core::protocol::mission::MissionStart;

pub use runner_core::protocol::mission::StartMissionOutput;

pub use runner_core::protocol::mission::MissionActivityState;

pub use runner_core::protocol::mission::MissionLinkInput;

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

/// Full set of known signal types as `Vec<SignalType>`, the shape the
/// router + launch-prompt composer take.
pub(crate) fn all_known_signals() -> Vec<SignalType> {
    KnownSignalType::ALL
        .iter()
        .map(|k| SignalType::new(k.as_str()))
        .collect()
}

pub fn list(conn: &Connection, crew_id: Option<&str>) -> Result<Vec<Mission>> {
    // Pinned missions float to the top, then most-recently-started.
    //
    // `archived_at IS NULL` (inside the repo query) is the single
    // chokepoint that hides archived missions from every surface that
    // lists missions: the ⌘K palette, the sidebar tray, the Missions
    // page summary. New surfaces inherit the filter by going through
    // this helper. To open an archived mission by direct URL, use
    // `get()` instead — it intentionally does NOT filter.
    repo::mission::list(conn, crew_id).map_err(Into::into)
}

pub use runner_core::protocol::mission::MissionSummary;

pub use runner_core::protocol::mission::ResumeMissionOutput;

pub fn get(conn: &Connection, id: &str) -> Result<Mission> {
    // Intentionally no `archived_at` filter — opening an archived
    // mission by direct URL has to still resolve so the workspace can
    // render it read-only.
    repo::mission::get(conn, id)?.ok_or_else(|| Error::msg(format!("mission not found: {id}")))
}

/// Cap on the effective mission goal byte length. The launch prompt
/// composer pastes this into every member's first-user-turn body
/// alongside `system_prompt` (≤ `MAX_SYSTEM_PROMPT_BYTES`) and the
/// conventions + coordination block; together they have to fit under
/// `router::runtime::FIRST_TURN_ARGV_MAX_BYTES`. 8 KB is roomy for
/// a real mission goal (typical goals are a few sentences) while
/// leaving generous headroom for the rest of the composed body.
pub const MAX_MISSION_GOAL_BYTES: usize = 8 * 1024;

fn validate_mission_goal(goal: &str) -> Result<()> {
    if goal.len() > MAX_MISSION_GOAL_BYTES {
        return Err(Error::msg(format!(
            "mission goal is {} bytes; max {} ({} KB). Trim the goal text or move \
             long-form context into the role brief / per-task messages.",
            goal.len(),
            MAX_MISSION_GOAL_BYTES,
            MAX_MISSION_GOAL_BYTES / 1024,
        )));
    }
    Ok(())
}

/// Guard one composed first-turn body against the positional `[PROMPT]`
/// argv ceiling (`router::runtime::FIRST_TURN_ARGV_MAX_BYTES`) before a
/// PTY is spawned. The individual persist-time caps
/// (`system_prompt` 16 KB, `mission_goal` 8 KB) do NOT
/// compose to this bound — brief + goal alone can reach 24 KB, and
/// `crew.system_prompt_addendum` (team conventions) is uncapped on top —
/// so the real invariant has to be enforced on the assembled body, at
/// the spawn boundary. Otherwise `first_turn_argv` silently drops the
/// entire first turn in release builds (empty argv) and trips a
/// `debug_assert!` in debug. See #247.
pub(crate) fn ensure_first_turn_fits(slot_handle: &str, body: &str) -> Result<()> {
    let max = crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES;
    if body.len() > max {
        return Err(Error::msg(format!(
            "composed first-turn prompt for slot `{slot_handle}` is {} bytes; exceeds the \
             {} KB runtime argv ceiling. Trim this crew's role brief, mission goal, or team \
             conventions.",
            body.len(),
            max / 1024,
        )));
    }
    Ok(())
}

pub fn start(
    conn: &mut Connection,
    app_data_dir: &Path,
    input: impl Into<MissionStart>,
) -> Result<StartMissionOutput> {
    let input = input.into();
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(Error::msg("mission title must not be empty"));
    }
    if let Some(g) = input.goal_override.as_deref() {
        validate_mission_goal(g)?;
    }
    let (project_id, cwd) = project::resolve_scope(conn, &input.scope, input.cwd)?;

    // Validate crew exists and is launchable.
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let crew = input
        .crew_id
        .as_deref()
        .map(|id| crew::get(&tx, id))
        .transpose()?;
    let slots = if let Some(crew) = &crew {
        let slots = slot::list(&tx, &crew.id)?;
        if slots.is_empty() {
            return Err(Error::msg(format!(
                "crew {} has no slots; cannot start mission",
                crew.name
            )));
        }
        slots
    } else {
        Vec::new()
    };

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
    // mission.

    // A direct agent chat that runs `mission start` is the mission's starter
    // and, within its own project, its linked chat (#748). Any other caller
    // starts an unlinked mission.
    let starter = input
        .started_by_session_id
        .as_deref()
        .and_then(|session_id| link::direct_agent_chat(&tx, session_id).ok());
    let linked_session_id = starter
        .as_ref()
        .filter(|chat| link::ensure_linkable(chat, project_id.as_deref()).is_ok())
        .map(|chat| chat.row.id.clone());
    let started_by = starter.as_ref().map(|chat| {
        serde_json::json!({
            "session_id": chat.row.id,
            "title": repo::session::direct_title(chat),
        })
    });

    let id = new_id();
    let started_at = now();
    repo::mission::insert(
        &tx,
        &repo::mission::MissionRow {
            id: id.clone(),
            crew_id: input.crew_id.clone(),
            project_id: project_id.clone(),
            title: title.clone(),
            status: MissionStatus::Running,
            goal_override: input.goal_override.clone(),
            cwd: cwd.clone(),
            started_at,
            stopped_at: None,
            pinned_at: None,
            archived_at: None,
            linked_session_id,
            report_to_chat: !input.no_report,
        },
    )?;
    // Sidebar node, under the project's node when bound (feature 44).
    repo::node::ensure_mission_node(&tx, &id, project_id.as_deref())?;

    let mission_dir = event_log::mission_dir(app_data_dir, input.crew_id.as_deref(), &id);
    std::fs::create_dir_all(&mission_dir)?;

    roster::write(&mission_dir, &roster::snapshot(crew.as_ref(), &slots))?;

    // The mission states its own goal.
    let goal_text = input.goal_override.clone().unwrap_or_default();

    // Open the event log and emit the two opening events.
    let log = EventLog::open(&mission_dir)?;
    let mut start_payload = serde_json::json!({
        "title": title,
        "cwd": cwd,
        "permission_mode": "bypass",
    });
    if let Some(started_by) = started_by {
        start_payload["started_by"] = started_by;
    }
    log.append(EventDraft {
        crew_id: input.crew_id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "system".into(),
        to: Default::default(),
        signal_type: Some(SignalType::new("mission_start")),
        payload: start_payload,
    })?;
    log.append(EventDraft {
        crew_id: input.crew_id.clone(),
        mission_id: id.clone(),
        kind: EventKind::Signal,
        from: "human".into(),
        to: Default::default(),
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

    // Two entry states, one guarded UPDATE each (#376). A running
    // mission takes the terminal-stop path: the status flip and the
    // archive stamp bind into one atomic statement so two racing
    // `mission_archive` calls can't both append the `mission_stopped`
    // terminal event — the slower UPDATE hits 0 rows and falls
    // through. A mission already out of `running` (restored from the
    // Archived pane in an older version, or an aborted spawn) archives on the visibility
    // axis alone: `archived_at` stamps, the lifecycle columns stay,
    // and no second `mission_stopped` is appended — its terminal stop
    // already happened.
    let stopped_at = now();
    let was_running = repo::mission::complete_and_archive_if_running(&tx, id, stopped_at)? == 1;
    if !was_running && repo::mission::archive_if_stopped(&tx, id, stopped_at)? == 0 {
        // Unknown ids error out of the `get`; a row that both UPDATEs
        // refused can only be here because it's already archived.
        let _ = get(&tx, id)?;
        return Err(Error::msg(format!("mission {id} is already archived")));
    }

    // Archiving removes the mission from the sidebar tree; unarchive
    // re-creates the node (feature 44).
    repo::node::delete_mission_node(&tx, id)?;

    // Fetch crew_id now that we know the row exists and we own the
    // transition; used for the mission-dir path below.
    let mission = get(&tx, id)?;

    if was_running {
        let mission_dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), id);
        let log = EventLog::open(&mission_dir)?;
        log.append(EventDraft {
            crew_id: mission.crew_id.clone(),
            mission_id: id.to_string(),
            kind: EventKind::Signal,
            from: "system".into(),
            to: Default::default(),
            signal_type: Some(SignalType::new("mission_stopped")),
            payload: serde_json::json!({}),
        })?;
    }

    tx.commit()?;
    Ok(mission)
}

pub use runner_core::protocol::mission::PostSignalInput;

pub use runner_core::protocol::mission::PostMessageInput;

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
    let mission_dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), mission_id);
    let log = EventLog::open(&mission_dir)?;
    let (entries, _skipped) = log.read_from_lossy(0)?;
    Ok(entries.into_iter().map(|e| e.event).collect())
}

pub fn mission_read_cursor(
    state: &AppCore,
    mission_id: &str,
    handle: &str,
) -> Option<runner_core::protocol::MissionReadCursor> {
    state.buses.get(mission_id)?.read_cursor(handle)
}

pub fn mission_events_replay(
    state: &AppCore,
    mission_id: &str,
) -> Result<Vec<runner_core::model::Event>> {
    let conn = state.db.get()?;
    read_events(&state.app_data_dir, &conn, mission_id)
}

fn validate_roster_handle(
    conn: &Connection,
    app_data_dir: &Path,
    mission: &Mission,
    from: Option<&str>,
) -> Result<()> {
    let Some(from) = from else {
        return Ok(());
    };
    let roster = roster::load(conn, app_data_dir, mission)?.slots;
    if roster.iter().any(|member| member.slot.slot_handle == from) {
        Ok(())
    } else {
        Err(Error::msg(format!("@{from} is not in the mission roster")))
    }
}

pub async fn mission_signal_impl(
    state: &AppCore,
    input: PostSignalInput,
) -> Result<runner_core::model::Event> {
    if input.signal_type == "ask_lead" {
        return Err(Error::msg(runner_core::model::ASK_LEAD_REMOVED_HINT));
    }
    let (mission, from) = {
        let conn = state.db.get()?;
        let mission = get(&conn, &input.mission_id)?;
        validate_roster_handle(&conn, &state.app_data_dir, &mission, input.from.as_deref())?;
        let from = if let Some(from) = input.from.as_deref() {
            let Some(signal_type) = KnownSignalType::from_name(&input.signal_type) else {
                return Err(Error::msg(format!(
                    "unknown signal_type {:?}",
                    input.signal_type
                )));
            };
            if matches!(
                signal_type,
                KnownSignalType::HumanSaid
                    | KnownSignalType::HumanResponse
                    | KnownSignalType::HumanQuestion
                    | KnownSignalType::MissionGoal
            ) {
                return Err(Error::msg(format!(
                    "signal_type {:?} is reserved for the person or Runner",
                    input.signal_type
                )));
            }
            from.to_owned()
        } else {
            if !matches!(input.signal_type.as_str(), "human_said" | "human_response") {
                return Err(Error::msg(format!(
                    "signal_type {:?} is not allowed without --as <handle>",
                    input.signal_type
                )));
            }
            "human".to_owned()
        };
        (mission, from)
    };
    if !matches!(mission.status, MissionStatus::Running) {
        return Err(Error::msg(format!(
            "mission {} is not running (status = {:?}); cannot post signals",
            mission.id, mission.status
        )));
    }

    let mission_dir =
        event_log::mission_dir(&state.app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let log = EventLog::open(&mission_dir)?;
    let mut payload = input.payload;
    if input.signal_type == "human_said" {
        let conn = state.db.get()?;
        let roster = roster::load(&conn, &state.app_data_dir, &mission)?.slots;
        let handles = roster
            .iter()
            .map(|member| member.slot.slot_handle.clone())
            .collect::<Vec<_>>();
        let explicit: runner_core::Recipients = serde_json::from_value(payload["target"].clone())?;
        let resolved = runner_core::addressing::resolve(
            payload["text"].as_str().unwrap_or_default(),
            explicit.as_slice(),
            &handles,
        )?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| Error::msg("person signal payload must be an object"))?;
        object.insert("target".into(), serde_json::to_value(resolved.recipients)?);
    }
    if input.signal_type == "human_response" && from == "human" {
        let question_id = payload["question_id"]
            .as_str()
            .ok_or_else(|| Error::msg("human_response requires payload.question_id"))?;
        let question_id = resolve_pending_question(&log, question_id)?;
        payload["question_id"] = question_id.into();
    }
    if matches!(input.signal_type.as_str(), "human_said" | "human_response") {
        let object = payload
            .as_object_mut()
            .ok_or_else(|| Error::msg("person signal payload must be an object"))?;
        object.insert("delivery".into(), "inbox".into());
    }
    let event = log.append(EventDraft {
        crew_id: mission.crew_id.clone(),
        mission_id: mission.id.clone(),
        kind: EventKind::Signal,
        from,
        to: Default::default(),
        signal_type: Some(SignalType::new(input.signal_type)),
        payload,
    })?;
    Ok(event)
}

/// The router delivers an answer only to a pending `human_question` card,
/// and drops any other ID with a feed warning. A member's `ask_human`
/// event precedes its card in the feed, so its ID names that card too.
fn resolve_pending_question(log: &EventLog, question_id: &str) -> Result<String> {
    let (entries, _skipped) = log.read_from_lossy(0)?;
    let pending = crate::router::pending_asks(&entries);
    if pending.contains_key(question_id) {
        return Ok(question_id.to_owned());
    }
    let card = entries.iter().map(|entry| &entry.event).find(|event| {
        event.signal_type.as_ref().map(SignalType::as_str) == Some("human_question")
            && event.payload["triggered_by"].as_str() == Some(question_id)
            && pending.contains_key(&event.id)
    });
    if let Some(card) = card {
        return Ok(card.id.clone());
    }
    let mut ids = pending.into_keys().collect::<Vec<_>>();
    ids.sort();
    Err(Error::msg(if ids.is_empty() {
        format!("{question_id} is not a pending question; the mission has none pending")
    } else {
        format!(
            "{question_id} is not a pending question; pending question IDs: {}",
            ids.join(", ")
        )
    }))
}

fn post_message(
    app_data_dir: &Path,
    conn: &Connection,
    input: PostMessageInput,
) -> Result<runner_core::model::Event> {
    let text = input.text.trim().to_string();
    if text.is_empty() {
        return Err(Error::msg("mission message text must not be empty"));
    }

    let mission = get(conn, &input.mission_id)?;
    if !matches!(mission.status, MissionStatus::Running) {
        return Err(Error::msg(format!(
            "mission {} is not running (status = {:?}); cannot post messages",
            mission.id, mission.status
        )));
    }

    validate_roster_handle(conn, app_data_dir, &mission, input.from.as_deref())?;

    let roster = roster::load(conn, app_data_dir, &mission)?.slots;
    let handles = roster
        .iter()
        .map(|member| member.slot.slot_handle.clone())
        .collect::<Vec<_>>();
    let resolved = runner_core::addressing::resolve(&text, input.to.as_slice(), &handles)?;

    let mission_dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let log = EventLog::open(&mission_dir)?;
    let mut draft = EventDraft::message(
        mission.crew_id.as_deref(),
        mission.id,
        input.from.unwrap_or_else(|| "human".to_owned()),
        None,
        resolved.text,
    );
    draft.to = resolved.recipients;
    let event = log.append(draft)?;
    Ok(event)
}

pub async fn mission_post_impl(
    state: &AppCore,
    input: PostMessageInput,
) -> Result<runner_core::model::Event> {
    let conn = state.db.get()?;
    post_message(&state.app_data_dir, &conn, input)
}

pub async fn mission_start_impl(
    state: &AppCore,
    input: StartMissionInput,
) -> Result<StartMissionOutput> {
    mission_start_impl_with_size(state, input.into(), None).await
}

/// Grid a mission fork will use, plus the source tag the fork log line
/// reports. Caller-measured dims win; with none, the frontend's cached
/// mission pane grid fills in so backend-initiated starts do not fork at
/// 80x24. `None` means no hint was ever recorded and registration falls
/// back to `DEFAULT_PTY_SIZE`.
pub(crate) fn mission_fork_size(
    caller: Option<(u16, u16)>,
    hint: Option<(u16, u16)>,
) -> (Option<(u16, u16)>, &'static str) {
    match (caller, hint) {
        (Some(size), _) => (Some(size), "caller-supplied"),
        (None, Some(size)) => (Some(size), "mission-hint"),
        (None, None) => (None, "DEFAULT_PTY_SIZE"),
    }
}

/// Record the frontend's most recent measured/estimated mission pane grid.
pub fn mission_grid_hint_set(state: &AppCore, cols: u16, rows: u16) -> Result<()> {
    if cols == 0 || rows == 0 {
        return Err(Error::msg(format!(
            "mission grid hint must be positive; got {cols}x{rows}"
        )));
    }
    *state.mission_grid_hint.lock().unwrap() = Some((cols, rows));
    Ok(())
}

pub async fn mission_start_impl_with_size(
    state: &AppCore,
    input: MissionStart,
    initial_size: Option<(u16, u16)>,
) -> Result<StartMissionOutput> {
    use crate::session::manager::SessionEvents;
    use std::sync::Arc;

    let (initial_size, size_source) =
        mission_fork_size(initial_size, *state.mission_grid_hint.lock().unwrap());

    let out = {
        let mut conn = state.db.get()?;
        start(&mut conn, &state.app_data_dir, input)?
    };
    log::info!(
        "mission starting: id={} crew={:?} title={:?}",
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
    let loaded = {
        let conn = state.db.get()?;
        roster::load(&conn, &state.app_data_dir, &out.mission)?
    };
    let roster = loaded.slots;
    let events_log_path = event_log::events_path(
        &state.app_data_dir,
        out.mission.crew_id.as_deref(),
        &out.mission.id,
    );

    let mission_dir = event_log::mission_dir(
        &state.app_data_dir,
        out.mission.crew_id.as_deref(),
        &out.mission.id,
    );
    let log_arc = crate::router::open_log_for_mission(&mission_dir)?;
    let prompt_channels: Vec<_> = roster
        .iter()
        .map(|member| {
            let adapter = crate::runtimes::for_key(
                member
                    .slot
                    .runtime_override
                    .as_deref()
                    .unwrap_or(&member.role.runtime),
            );
            crate::router::prompt::member_prompt_channels(
                adapter,
                member,
                &loaded.file,
                &log_arc,
                false,
            )
        })
        .collect();

    // Enforce the composed-body argv ceiling before spawning any PTY.
    // Per-field persist caps don't compose to this bound (see
    // `ensure_first_turn_fits`); on overflow, roll the half-open mission
    // back to `aborted` and surface an actionable error rather than
    // booting an agent with an empty first turn.
    for (member, (_, first_turn)) in roster.iter().zip(&prompt_channels) {
        if let Some(first_turn) = first_turn {
            if let Err(e) = ensure_first_turn_fits(&member.slot.slot_handle, first_turn) {
                if let Ok(conn) = state.db.get() {
                    let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
                }
                return Err(e);
            }
        }
    }

    let emitter: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    let mut spawned_pairs: Vec<(String, String)> = Vec::with_capacity(roster.len());
    let mut pendings: Vec<crate::session::PendingMissionSpawn> = Vec::with_capacity(roster.len());
    // Two-phase spawn: `register_mission_session` is the synchronous
    // part — insert the DB row, generate the session id, compose the
    // SpawnSpec — and runs in this loop. The slow part (gate +
    // `runtime.spawn` + reader thread) is `complete_mission_session_spawn`
    // and is dispatched in a background task after router + bus mount,
    // so the Start-mission RPC returns in ~milliseconds instead of
    // ~1.5s × (claude_workers). See issue #171.
    //
    for (idx, member) in roster.iter().enumerate() {
        let (system_prompt, first_turn) = prompt_channels.get(idx).cloned().unwrap_or_default();
        let register_res = state.sessions.register_mission_session(
            &out.mission,
            &member.role,
            &member.slot,
            &state.app_data_dir,
            events_log_path.clone(),
            state.db.clone(),
            system_prompt,
            first_turn,
            initial_size,
            size_source,
        );
        match register_res {
            Ok(pending) => {
                // Register by slot_handle (the in-mission identity)
                // — the router routes signals/messages by slot_handle,
                // not by template handle.
                spawned_pairs.push((member.slot.slot_handle.clone(), pending.session_id.clone()));
                pendings.push(pending);
            }
            Err(e) => {
                // Rollback: DELETE the session rows registered so far
                // (no PTYs spawned yet — register is row-insert-only),
                // mark the mission aborted, surface the original
                // error. Bus and router aren't mounted yet so no
                // event-side cleanup.
                if let Ok(mut conn) = state.db.get() {
                    let _ = abort_start(&mut conn, &out.mission.id);
                }
                for (_, session_id) in &spawned_pairs {
                    crate::session::system_prompt::remove(&state.app_data_dir, session_id);
                }
                return Err(e);
            }
        }
    }
    if let Err(e) = mount_mission_router(state, &out.mission, Mount::Fresh(&spawned_pairs)) {
        // Bus didn't attach — drop the pending PTY spawns, DELETE the
        // session rows (no PTYs ever forked for them at this point),
        // abort the mission. `pendings` going out of scope releases
        // any held resources without ever firing
        // `complete_mission_session_spawn`.
        drop(pendings);
        if let Ok(mut conn) = state.db.get() {
            let _ = abort_start(&mut conn, &out.mission.id);
        }
        for (_, session_id) in &spawned_pairs {
            crate::session::system_prompt::remove(&state.app_data_dir, session_id);
        }
        return Err(e);
    }

    // Dispatch the gate-blocked PTY-spawn phase to a background task
    // so this RPC returns now. The frontend already has every
    // `sessions` row from `register_mission_session`, so the
    // workspace mounts immediately and renders starting pills; each
    // pill clears when its PTY emits the TUI-ready signal (see
    // `chunkIndicatesTuiReady`). Spawns run sequentially in roster order.
    //
    // Per-mission cancel flag lets Stop / Archive abort
    // queued slots before their PTYs fork; without it, a slot
    // sleeping in the gate would spawn into a stopped mission. See
    // `SessionManager::cancel_pending_mission_spawns`.
    //
    // Each slot is independent: a spawn failure marks only that row
    // crashed and emits `session/exit`; the remaining slots still spawn.
    let manager = Arc::clone(&state.sessions);
    let pool_for_task = state.db.clone();
    let mission_id_for_task = out.mission.id.clone();
    let emitter_for_task = Arc::clone(&emitter);
    let cancel = state
        .sessions
        .register_pending_mission_cancel(&out.mission.id);
    let cancel_for_drop = Arc::clone(&cancel);
    std::thread::spawn(move || {
        for pending in pendings {
            let session_id = pending.session_id.clone();
            match manager.complete_mission_session_spawn(
                pending,
                Arc::clone(&emitter_for_task),
                Arc::clone(&cancel),
            ) {
                Ok(crate::session::CompleteSpawnOutcome::Spawned) => {}
                Ok(crate::session::CompleteSpawnOutcome::Cancelled) => {
                    if let Ok(conn) = pool_for_task.get() {
                        let _ = repo::session::set_exit_status(
                            &conn,
                            &session_id,
                            SessionStatus::Stopped,
                            Utc::now(),
                        );
                    }
                    emitter_for_task.exit(&crate::session::manager::ExitEvent {
                        session_id: session_id.clone(),
                        mission_id: Some(mission_id_for_task.clone()),
                        exit_code: None,
                        success: false,
                    });
                }
                Err(e) => {
                    log::error!(
                        "mission session spawn failed in background task: \
                         mission={mission_id_for_task} session={session_id} error={e}"
                    );
                    if let Ok(conn) = pool_for_task.get() {
                        let _ = repo::session::set_exit_status(
                            &conn,
                            &session_id,
                            SessionStatus::Crashed,
                            Utc::now(),
                        );
                    }
                    // Tell the workspace the slot died so the pane
                    // flips out of "starting" without waiting for a
                    // manual refresh.
                    emitter_for_task.exit(&crate::session::manager::ExitEvent {
                        session_id: session_id.clone(),
                        mission_id: Some(mission_id_for_task.clone()),
                        exit_code: None,
                        success: false,
                    });
                }
            }
        }
        // Identity-checked drop: only remove the map entry if it's
        // still *this* task's flag. A replacement registration must
        // stay reachable from `cancel_pending_mission_spawns`.
        manager.drop_pending_mission_cancel(&mission_id_for_task, &cancel_for_drop);
    });

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
pub async fn mission_attach(state: &AppCore, mission_id: &str) -> Result<Mission> {
    log::info!("mission attach: id={mission_id}");
    ensure_mission_router_mounted(state, mission_id).await?;
    let conn = state.db.get()?;
    get(&conn, mission_id)
}

/// Mount the in-memory Router + EventBus for `mission_id`, idempotently.
///
/// Called from two places:
///  * `mission_attach` — workspace UI mount path; runs on every
///    workspace navigation. After the startup reconciler runs, this is
///    almost always a no-op.
///  * `mount_all_running_mission_routers` — app startup path; runs once
///    per running mission so router/event-bus fanout is restored for
///    missions that remain marked running in the DB.
///
/// Returns `Ok(())` (no-op) when the mission's router is already
/// registered, or when the mission isn't in the `running` state.
pub(crate) async fn ensure_mission_router_mounted(state: &AppCore, mission_id: &str) -> Result<()> {
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

    mount_mission_router(state, &mission, Mount::Reopen)?;
    Ok(())
}

enum Mount<'a> {
    Fresh(&'a [(String, String)]),
    Reopen,
}

fn mount_mission_router(state: &AppCore, mission: &Mission, mount: Mount<'_>) -> Result<()> {
    use crate::event_bus::{BusEmitter, ChannelBusEvents};
    use crate::router::{
        open_log_for_mission, ChannelRouterUiNotifier, CompositeBusEmitter, Router,
        RouterSubscriber, StdinInjector,
    };
    use std::sync::Arc;
    let loaded = roster::load(&*state.db.get()?, &state.app_data_dir, mission)?;
    let crew_name = loaded.file.crew_name;
    let crew_addendum = loaded.file.crew_addendum;
    let roster = loaded.slots;
    let allowed_signals = all_known_signals();
    let session_pairs: Vec<(String, String)> = if let Mount::Fresh(pending) = mount {
        pending.to_vec()
    } else {
        let conn = state.db.get()?;
        let mut out = Vec::with_capacity(roster.len());
        for member in &roster {
            let session_id =
                repo::session::latest_unarchived_id_for_slot(&conn, &mission.id, &member.slot.id)
                    .ok()
                    .flatten();
            if let Some(sid) = session_id {
                out.push((member.slot.slot_handle.clone(), sid));
            }
        }
        out
    };

    let mission_dir =
        event_log::mission_dir(&state.app_data_dir, mission.crew_id.as_deref(), &mission.id);
    let log_arc = open_log_for_mission(&mission_dir)?;
    let injector: Arc<dyn StdinInjector> = Arc::clone(&state.sessions) as Arc<dyn StdinInjector>;
    let router = Router::new(
        mission.id.clone(),
        mission.crew_id.clone(),
        crew_name,
        &roster,
        allowed_signals,
        crew_addendum,
        Arc::clone(&log_arc),
        injector,
        Arc::new(ChannelRouterUiNotifier(state.events.clone())),
    )?;
    router.register_sessions(&session_pairs);

    // Restore pending asks and skip replaying historical events through live handlers.
    if matches!(mount, Mount::Reopen) {
        router.reconstruct_from_log()?;
    }

    let roster_handles: Vec<String> = roster.iter().map(|m| m.slot.slot_handle.clone()).collect();
    let channel_emitter: Arc<dyn BusEmitter> = Arc::new(ChannelBusEvents(state.events.clone()));
    let router_emitter: Arc<dyn BusEmitter> = Arc::new(RouterSubscriber(Arc::clone(&router)));
    let composite: Arc<dyn BusEmitter> = Arc::new(CompositeBusEmitter::new(vec![
        channel_emitter,
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
/// Runs once at app startup. The NDJSON log is the source of truth;
/// this just re-wires the in-memory fanout layer that died with the
/// old process. The PTY startup cleanup later demotes stale running
/// session rows; it does not reattach child processes from the prior
/// app process.
///
/// Per-mission isolation: if one mission's mount fails (corrupt log,
/// missing crew row, etc.), it gets logged and the loop continues.
pub async fn mount_all_running_mission_routers(state: &AppCore) {
    let mission_ids = match state.db.get() {
        Ok(conn) => repo::mission::running_ids(&conn).unwrap_or_else(|e| {
            log::error!("mount_all_running_mission_routers query failed: {e}");
            Vec::new()
        }),
        Err(e) => {
            log::error!("mount_all_running_mission_routers db pool unavailable: {e}");
            Vec::new()
        }
    };

    for mission_id in mission_ids {
        match ensure_mission_router_mounted(state, &mission_id).await {
            Ok(()) => {
                log::info!("mission startup mount: id={mission_id} action=mounted");
            }
            Err(e) => {
                log::info!("mission startup mount: id={mission_id} action=mount_failed");
                log::warn!("mission {mission_id} startup mount failed: {e}");
            }
        }
    }
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
pub async fn mission_stop_impl(state: &AppCore, id: String) -> Result<Mission> {
    log::info!("mission stop: id={id}");
    let stopped = state.sessions.kill_all_for_mission(&id);
    if let Some(router) = state.routers.get(&id) {
        let live: std::collections::HashSet<_> =
            state.sessions.live_session_ids().into_iter().collect();
        if let Err(error) = router.clear_interruptions(|session| !live.contains(session)) {
            log::warn!("clear interruptions for stopped mission {id}: {error}");
        }
    }
    stopped?;
    let conn = state.db.get()?;
    get(&conn, &id)
}

pub fn mission_resume(state: &AppCore, id: &str) -> Result<ResumeMissionOutput> {
    mission_resume_with_size(state, id, None)
}

pub fn mission_resume_with_size(
    state: &AppCore,
    id: &str,
    size: Option<(u16, u16)>,
) -> Result<ResumeMissionOutput> {
    mission_resume_with(state, id, |state, session_id| {
        crate::ops::session::session_resume(state, session_id, size.map(|s| s.0), size.map(|s| s.1))
            .map(|_| ())
    })
}

fn mission_resume_with(
    state: &AppCore,
    id: &str,
    mut resume: impl FnMut(&AppCore, &str) -> Result<()>,
) -> Result<ResumeMissionOutput> {
    let candidates = crate::ops::session::session_list(state, id)?;
    let mut resumed_session_ids = Vec::new();
    let mut first_error = None;
    for candidate in candidates {
        if matches!(candidate.session.status, SessionStatus::Running) {
            continue;
        }
        match resume(state, &candidate.session.id) {
            Ok(_) => resumed_session_ids.push(candidate.session.id),
            Err(error) => {
                let message = error.to_string();
                if !is_concurrent_resume_error(&message) {
                    first_error.get_or_insert((candidate.handle, candidate.session.id, error));
                }
            }
        }
    }
    if let Some((handle, session_id, error)) = first_error {
        if matches!(error, Error::RuntimeVersionGate(_)) {
            return Err(error);
        }
        return Err(Error::msg(format!(
            "mission {id} resume failed for @{handle} (session {session_id}): {error}; resumed session ids: [{}]",
            resumed_session_ids.join(", ")
        )));
    }
    Ok(ResumeMissionOutput {
        mission_id: id.to_owned(),
        resumed_session_ids,
        sessions: crate::ops::session::session_list(state, id)?,
    })
}

fn is_concurrent_resume_error(error: &str) -> bool {
    [
        "is already being resumed",
        "is already running — attach instead",
    ]
    .iter()
    .any(|fragment| error.contains(fragment))
}

/// Toggle a mission's pin. Pinned missions float to the top of the
/// sidebar's MISSION list (sort key: `pinned_at IS NULL, pinned_at
/// DESC, started_at DESC`). Setting `pinned = false` clears the
/// timestamp.
pub async fn mission_pin_impl(state: &AppCore, id: String, pinned: bool) -> Result<Mission> {
    // Pinning a linked mission pins its chat's group (#748).
    let head = link::group_head(&*state.db.get()?, &id)?;
    if let Some(head) = head {
        crate::ops::node::node_set_pinned(state, head.id, pinned)?;
        return mission_get(state, &id);
    }
    let conn = state.db.get()?;
    let pinned_at: Option<Timestamp> = if pinned { Some(now()) } else { None };
    let n = repo::mission::set_pinned_at(&conn, &id, pinned_at)?;
    if n != 1 {
        return Err(Error::msg(format!("mission not found: {id}")));
    }
    // The sidebar renders pin state from the node (feature 44); the
    // row's pinned_at stays for non-sidebar consumers.
    if let Some(node) = repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &id)? {
        repo::node::set_pinned(&conn, &node.id, pinned)?;
    }
    get(&conn, &id)
}

/// Rename a mission. Title is trimmed; empty values are rejected so
/// the sidebar never renders a blank row. The mission's event log is
/// untouched — the title only ever lived on the row.
pub async fn mission_rename_impl(state: &AppCore, id: String, title: String) -> Result<Mission> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(Error::msg("mission title must not be empty"));
    }
    let conn = state.db.get()?;
    let n = repo::mission::set_title(&conn, &id, trimmed)?;
    if n != 1 {
        return Err(Error::msg(format!("mission not found: {id}")));
    }
    get(&conn, &id)
}

/// Terminal end-of-mission. Kills every live PTY, writes the
/// `mission_stopped` event, flips the mission row to `completed`, and
/// drops the router + bus. Mirrors what `mission_stop` used to do
/// before the lifecycle split — preserved as a separate command so
/// the workspace UI can guard it behind an explicit confirm.
pub async fn mission_archive_impl(state: &AppCore, id: String) -> Result<Mission> {
    close_mission_drawer_shells(state, &id)?;
    state.sessions.kill_all_for_mission(&id)?;
    let mut conn = state.db.get()?;
    let mission = stop(&mut conn, &state.app_data_dir, &id)?;
    state.buses.unmount(&id);
    state.routers.unregister(&id);
    Ok(mission)
}

fn close_mission_drawer_shells(state: &AppCore, mission_id: &str) -> Result<()> {
    let shell_ids = {
        let conn = state.db.get()?;
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, mission_id)?
            .as_ref()
            .map(repo::node::session_ids)
            .unwrap_or_default()
    };
    for session_id in shell_ids {
        if let Err(error) = crate::ops::session::session_close(state, &session_id) {
            let conn = state.db.get()?;
            if repo::session::get_row(&conn, &session_id)?.is_some() {
                return Err(error);
            }
        }
    }
    Ok(())
}

pub async fn mission_unarchive_impl(state: &AppCore, id: String) -> Result<Mission> {
    let mission = {
        let mut conn = state.db.get()?;
        let tx = conn.transaction()?;
        let reopened = repo::mission::unarchive(&tx, &id)? == 1;
        let mission = get(&tx, &id)?;
        repo::node::ensure_mission_node(&tx, &id, mission.project_id.as_deref())?;
        if reopened {
            let mission_dir =
                event_log::mission_dir(&state.app_data_dir, mission.crew_id.as_deref(), &id);
            EventLog::open(&mission_dir)?.append(EventDraft::signal(
                mission.crew_id.as_deref(),
                &id,
                "system",
                SignalType::new("mission_reopened"),
                serde_json::json!({}),
            ))?;
        }
        tx.commit()?;
        mission
    };
    ensure_mission_router_mounted(state, &id).await?;
    Ok(mission)
}

/// Emits `mission/changed` after the flip — the sidebar's MISSION list
/// listens on that channel, so the restored row reappears without a
/// manual refresh.
pub async fn mission_unarchive(state: &AppCore, id: String) -> Result<Mission> {
    let mission = mission_unarchive_impl(state, id).await?;
    state.events.emit("mission/changed", &());
    Ok(mission)
}

/// Permanent delete of an archived mission: its session rows (with their
/// attention rows) and the mission row in one transaction — then its
/// on-disk footprint:
/// the mission dir (NDJSON log + roster sidecar) and the
/// `missions/<id>` scratch dir (per-slot runner shims,
/// cli_install::install_session_runner_shim). Refused for non-archived
/// missions: archive is the reversible step, delete is not. Dir
/// removal happens after commit and only logs on failure — the DB is
/// the source of truth, and a leaked dir is better than a
/// deleted-but-still-listed mission.
pub(crate) fn delete_archived(
    conn: &mut Connection,
    app_data_dir: &Path,
    id: &str,
) -> Result<Mission> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mission = repo::mission::get(&tx, id)?
        .ok_or_else(|| Error::msg(format!("mission not found: {id}")))?;
    if mission.archived_at.is_none() {
        return Err(Error::msg(format!(
            "mission {id} is not archived; archive it before deleting"
        )));
    }
    delete_rows(&tx, &[id.to_owned()])?;
    tx.commit()?;
    let mission_dir = event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), id);
    let scratch_dir = app_data_dir.join("missions").join(id);
    for dir in [mission_dir, scratch_dir] {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "mission {id} deleted but removing {} failed: {e}",
                    dir.display()
                );
            }
        }
    }
    Ok(mission)
}

/// Delete missions and their sessions, inside the caller's transaction.
/// The sessions are deleted, not unbound: a mission-scoped row with no
/// mission would surface as a direct chat.
pub(crate) fn delete_rows(tx: &rusqlite::Transaction<'_>, ids: &[String]) -> Result<usize> {
    super::session::delete_rows(tx, &repo::session::ids_for_missions(tx, ids)?)?;
    repo::mission_review::delete_for_missions(tx, ids)?;
    Ok(repo::mission::delete_many(tx, ids)?)
}

/// Undo a mission start that failed before any PTY forked: delete the session
/// rows registered so far and mark the mission aborted.
fn abort_start(conn: &mut Connection, mission_id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let session_ids = repo::session::ids_for_missions(&tx, &[mission_id.to_owned()])?;
    super::session::delete_rows(&tx, &session_ids)?;
    repo::mission::abort(&tx, mission_id, Utc::now())?;
    tx.commit()?;
    Ok(())
}

/// Settings → Archived permanent delete (feature 01 Phase 4). Archived
/// missions have no live PTYs, bus, or router — `mission_archive`
/// dropped them — so `delete_archived` covers everything.
pub fn mission_delete(state: &AppCore, id: &str) -> Result<()> {
    close_mission_drawer_shells(state, id)?;
    let mut conn = state.db.get()?;
    delete_archived(&mut conn, &state.app_data_dir, id)?;
    Ok(())
}

pub fn mission_list(state: &AppCore, crew_id: Option<String>) -> Result<Vec<Mission>> {
    let conn = state.db.get()?;
    list(&conn, crew_id.as_deref())
}

/// Archived missions, newest-archived first — the Settings → Archived
/// pane's read surface.
pub fn mission_list_archived(state: &AppCore, crew_id: Option<String>) -> Result<Vec<Mission>> {
    let conn = state.db.get()?;
    repo::mission::list_archived(&conn, crew_id.as_deref()).map_err(Into::into)
}

pub fn mission_roster(state: &AppCore, id: &str) -> Result<Vec<crate::model::SlotWithRole>> {
    let conn = state.db.get()?;
    let mission = get(&conn, id)?;
    Ok(roster::load(&conn, &state.app_data_dir, &mission)?.slots)
}

pub fn mission_get(state: &AppCore, id: &str) -> Result<Mission> {
    let conn = state.db.get()?;
    get(&conn, id)
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
    crate::router::pending_asks(&entries).len()
}

fn latest_session_statuses_from_log(
    mission_dir: &Path,
) -> std::collections::HashMap<String, MissionActivityState> {
    let log = match EventLog::open(mission_dir) {
        Ok(l) => l,
        Err(_) => return std::collections::HashMap::new(),
    };
    let entries = match log.read_from_lossy(0) {
        Ok((entries, _skipped)) => entries,
        Err(_) => return std::collections::HashMap::new(),
    };
    let mut latest = std::collections::HashMap::new();
    for entry in &entries {
        let event = &entry.event;
        if !matches!(event.kind, runner_core::model::EventKind::Signal) {
            continue;
        }
        let Some(t) = event.signal_type.as_ref() else {
            continue;
        };
        if !t.is_session_status() {
            continue;
        }
        let Some(state) = event.payload.get("state").and_then(|v| v.as_str()) else {
            continue;
        };
        let state = match state {
            "busy" => MissionActivityState::Busy,
            "idle" => MissionActivityState::Idle,
            _ => continue,
        };
        latest.insert(event.from.clone(), state);
    }
    latest
}

fn mission_activity_from_latest(
    live_handles: &[String],
    latest: &std::collections::HashMap<String, MissionActivityState>,
) -> Option<MissionActivityState> {
    if live_handles.is_empty() {
        return None;
    }
    if live_handles
        .iter()
        .any(|handle| !matches!(latest.get(handle), Some(MissionActivityState::Idle)))
    {
        Some(MissionActivityState::Busy)
    } else {
        Some(MissionActivityState::Idle)
    }
}

fn mission_activity_from_log(
    mission_dir: &Path,
    live_handles: &[String],
) -> Option<MissionActivityState> {
    if live_handles.is_empty() {
        return None;
    }
    let latest = latest_session_statuses_from_log(mission_dir);
    mission_activity_from_latest(live_handles, &latest)
}

#[cfg(test)]
fn live_session_handles(conn: &Connection, mission_id: &str) -> rusqlite::Result<Vec<String>> {
    repo::session::live_handles_for_mission(conn, mission_id)
}

fn all_mission_sessions_live(conn: &Connection, mission_id: &str) -> rusqlite::Result<bool> {
    let (total, running) = repo::session::unarchived_counts_for_mission(conn, mission_id)?;
    Ok(total > 0 && running == total)
}

pub async fn mission_list_summary_impl(
    state: &AppCore,
    crew_id: Option<String>,
) -> Result<Vec<MissionSummary>> {
    let missions = list(&*state.db.get()?, crew_id.as_deref())?;
    summaries(state, missions)
}

/// The non-archived missions linked to a chat, oldest first, for
/// `mission list --linked-here` (#748).
pub async fn mission_list_linked(state: &AppCore, session_id: &str) -> Result<Vec<MissionSummary>> {
    let missions = repo::mission::list_linked(&*state.db.get()?, session_id)?;
    summaries(state, missions)
}

fn summaries(state: &AppCore, missions: Vec<Mission>) -> Result<Vec<MissionSummary>> {
    let statuses = crate::ops::session::session_status_snapshot(state)?;
    let conn = state.db.get()?;

    let mut summaries = Vec::with_capacity(missions.len());
    for m in missions {
        let crew_name = m
            .crew_id
            .as_deref()
            .and_then(|id| crew::get(&conn, id).ok())
            .map(|crew| crew.name);
        let sessions = match super::session::list_for_mission(&conn, &state.app_data_dir, &m.id) {
            Ok(sessions) => sessions,
            Err(error) => {
                log::warn!("mission {} summary unavailable: {error}; skipping", m.id);
                continue;
            }
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
                let mission_dir =
                    event_log::mission_dir(&state.app_data_dir, m.crew_id.as_deref(), &m.id);
                count_pending_asks_from_log(&mission_dir)
            }
        };
        // The same query feeds both projections: `any_session_live`
        // preserves the existing paused/no-live behavior, while
        // `activity` overlays busy/idle only for live mission slots.
        let live_handles: Vec<_> = sessions
            .iter()
            .filter(|session| session.session.status == SessionStatus::Running)
            .map(|session| session.handle.clone())
            .collect();
        let any_session_live = !live_handles.is_empty();
        let all_sessions_live = all_mission_sessions_live(&conn, &m.id).unwrap_or(false);
        let activity = if any_session_live {
            let mission_dir =
                event_log::mission_dir(&state.app_data_dir, m.crew_id.as_deref(), &m.id);
            mission_activity_from_log(&mission_dir, &live_handles)
        } else {
            None
        };
        let phase = state.reports.phase(&m.id).unwrap_or_else(|| {
            report::derive(
                &report::phase_input(state, &m, &sessions, &statuses, pending_ask_count),
                None,
            )
        });
        let session_statuses = sessions
            .into_iter()
            .filter(|session| !Runtime::parse(&session.runtime).is_some_and(Runtime::is_shell))
            .map(|session| {
                let mut status = statuses
                    .get(&session.session.id)
                    .cloned()
                    .unwrap_or_default();
                status.lifecycle = match session.session.status {
                    SessionStatus::Running => crate::session::status::Lifecycle::Running,
                    SessionStatus::Stopped => crate::session::status::Lifecycle::Stopped,
                    SessionStatus::Crashed => crate::session::status::Lifecycle::Error,
                };
                (session.session.id, status)
            })
            .collect();
        summaries.push(MissionSummary {
            session_statuses,
            mission: m,
            crew_name,
            pending_ask_count,
            any_session_live,
            all_sessions_live,
            activity,
            phase: Some(phase),
        });
    }
    Ok(summaries)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod project_tests;

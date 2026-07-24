// Mission lifecycle — start, stop, list, get.
//
// A mission is the runtime container: it owns a directory, an NDJSON event
// log, and a set of sessions (spawned in C6). This module only does the
// bookkeeping layer — no PTYs yet.
//
// `mission_start` is the point where config crystallizes into runtime:
// validate the crew has ≥1 runner and exactly one lead, create the mission
// row, create the mission directory, and emit the two opening events —
// `mission_start` (system announces the run) and `mission_goal` (the
// human's intent, which the orchestrator routes to the lead via the
// built-in rule in C8).

use std::path::Path;

use chrono::Utc;
use runner_core::event_log::{self, EventLog};
use runner_core::model::{EventDraft, EventKind, KnownSignalType, SignalType};
use rusqlite::{params, Connection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use ulid::Ulid as UlidGen;

use crate::{
    commands::{crew, project, slot},
    error::{Error, Result},
    model::{Mission, MissionStatus, SessionStatus, Timestamp},
    repo, AppState,
};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StartMissionInput {
    pub crew_id: String,
    /// Optional project membership. Its cwd is used when cwd is omitted.
    #[serde(default)]
    pub project_id: Option<String>,
    pub title: String,
    /// Optional override of the crew's default goal. When `None`, the crew's
    /// `goal` column is used; if that is also unset the mission starts with
    /// an empty-goal event (valid — the human may post a `human_said` signal
    /// later instead of setting a goal up front).
    #[serde(default)]
    pub goal_override: Option<String>,
    /// Working directory exposed to every session as `$MISSION_CWD`.
    /// An explicit value overrides the project's bound cwd.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionActivityState {
    Busy,
    Idle,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

/// Full set of known signal types as `Vec<SignalType>`, the shape the
/// router + launch-prompt composer take.
fn all_known_signals() -> Vec<SignalType> {
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
    /// True iff at least one of the mission's session rows is `status =
    /// 'running'`. A mission without live sessions never shows the sidebar
    /// working indicator.
    pub any_session_live: bool,
    /// True iff every current, unarchived session row is `running`.
    /// Partial crash/resume states keep the sidebar mission icon muted.
    pub all_sessions_live: bool,
    /// Optional live activity projection derived from per-slot
    /// `runner_status` events. `None` means the mission has no live
    /// sessions and keeps the sidebar attention slot clear.
    pub activity: Option<MissionActivityState>,
}

pub fn get(conn: &Connection, id: &str) -> Result<Mission> {
    // Intentionally no `archived_at` filter — opening an archived
    // mission by direct URL has to still resolve so the workspace can
    // render it read-only.
    repo::mission::get(conn, id)?.ok_or_else(|| Error::msg(format!("mission not found: {id}")))
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

/// Guard one composed first-turn body against the positional `[PROMPT]`
/// argv ceiling (`router::runtime::FIRST_TURN_ARGV_MAX_BYTES`) before a
/// PTY is spawned. The individual persist-time caps
/// (`system_prompt` 16 KB, `crew.goal` / `mission_goal` 8 KB) do NOT
/// compose to this bound — brief + goal alone can reach 24 KB, and
/// `crew.system_prompt_addendum` (team conventions) is uncapped on top —
/// so the real invariant has to be enforced on the assembled body, at
/// the spawn boundary. Otherwise `first_turn_argv` silently drops the
/// entire first turn in release builds (empty argv) and trips a
/// `debug_assert!` in debug. See #247.
fn ensure_first_turn_fits(slot_handle: &str, body: &str) -> Result<()> {
    let max = crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES;
    if body.len() > max {
        return Err(Error::msg(format!(
            "composed first-turn prompt for slot `{slot_handle}` is {} bytes; exceeds the \
             {} KB runtime argv ceiling. Trim this crew's runner brief, mission goal, or team \
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
    mut input: StartMissionInput,
) -> Result<StartMissionOutput> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(Error::msg("mission title must not be empty"));
    }
    if let Some(g) = input.goal_override.as_deref() {
        validate_mission_goal(g)?;
    }
    input.cwd = project::resolve_cwd(conn, input.project_id.as_deref(), input.cwd)?;

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
    // mission.
    let tx = conn.transaction()?;

    let id = new_id();
    let started_at = now();
    repo::mission::insert(
        &tx,
        &repo::mission::MissionRow {
            id: id.clone(),
            crew_id: crew.id.clone(),
            project_id: input.project_id.clone(),
            title: title.clone(),
            status: MissionStatus::Running,
            goal_override: input.goal_override.clone(),
            cwd: input.cwd.clone(),
            started_at,
            stopped_at: None,
            pinned_at: None,
            archived_at: None,
        },
    )?;
    // Sidebar node, under the project's node when bound (feature 44).
    repo::node::ensure_mission_node(&tx, &id, input.project_id.as_deref())?;

    let mission_dir = event_log::mission_dir(app_data_dir, &crew.id, &id);
    std::fs::create_dir_all(&mission_dir)?;

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
    let affected = repo::mission::complete_and_archive_if_running(&tx, id, stopped_at)?;
    if affected == 0 {
        // Either the id doesn't exist or the mission isn't running anymore
        // (a concurrent stop won the race). Fetch for a precise error.
        let mission = get(&tx, id)?;
        return Err(Error::msg(format!(
            "mission {id} is not running; status = {:?}",
            mission.status
        )));
    }

    // Archiving removes the mission from the sidebar tree; unarchive
    // re-creates the node (feature 44).
    repo::node::delete_mission_node(&tx, id)?;

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

/// Write the per-mission roster snapshot to `roster.json` next to
/// `events.ndjson`. The CLI (`runner msg post --to`) reads this to
/// validate handles without DB access. Frozen at mission_start: if the
/// crew's membership changes mid-mission, the running mission still
/// validates against this snapshot.
///
/// Atomic write via `tempfile::NamedTempFile::persist` so a crash
/// mid-write can't leave a half-formed file the CLI would
/// parse-fail on.
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

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PostHumanSignalInput {
    pub mission_id: String,
    /// Signal type — restricted to the human-originated ones the workspace
    /// UI is allowed to emit. Anything else is rejected.
    pub signal_type: String,
    /// Free-form JSON object carried with the signal. Annotated so schemars
    /// emits a schema with an explicit `type`; a bare `serde_json::Value`
    /// derives a typeless schema (`{}`), which strict MCP clients reject —
    /// and the rejection drops the entire advertised tool list (#240).
    #[schemars(with = "std::collections::HashMap<String, serde_json::Value>")]
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

pub(crate) async fn mission_post_human_signal_impl(
    state: &AppState,
    input: PostHumanSignalInput,
) -> Result<runner_core::model::Event> {
    // Whitelist: only human_said from MCP and human_response from the
    // workspace UI. The router treats `from = "human"` as authoritative
    // for these, so a buggy client that posted `mission_goal` or
    // `ask_lead` could trigger handler side-effects from the wrong
    // identity.
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
pub async fn mission_post_human_signal(
    state: State<'_, AppState>,
    input: PostHumanSignalInput,
) -> Result<runner_core::model::Event> {
    mission_post_human_signal_impl(&state, input).await
}

pub(crate) async fn mission_start_impl(
    state: &AppState,
    app: &tauri::AppHandle,
    input: StartMissionInput,
) -> Result<StartMissionOutput> {
    mission_start_impl_with_size(state, app, input, None).await
}

async fn mission_start_impl_with_size(
    state: &AppState,
    app: &tauri::AppHandle,
    input: StartMissionInput,
    initial_size: Option<(u16, u16)>,
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
    let (crew_name, crew_default_goal, crew_addendum) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &out.mission.crew_id)?;
        (crew.name, crew.goal, crew.system_prompt_addendum)
    };
    let allowed_signals = all_known_signals();
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
    // See `docs/impls/archive/0007-spawn-time-prompt-delivery.md`.
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
                                crew_addendum: crew_addendum.as_deref(),
                            },
                        )
                    })
                } else {
                    Some(crate::router::prompt::compose_worker_first_turn(
                        m.runner.system_prompt.as_deref(),
                        crew_addendum.as_deref(),
                    ))
                }
            })
            .collect()
    };

    // Enforce the composed-body argv ceiling before spawning any PTY.
    // Per-field persist caps don't compose to this bound (see
    // `ensure_first_turn_fits`); on overflow, roll the half-open mission
    // back to `aborted` and surface an actionable error rather than
    // booting an agent with an empty first turn.
    for (member, body) in roster.iter().zip(&first_turns) {
        if let Some(body) = body {
            if let Err(e) = ensure_first_turn_fits(&member.slot.slot_handle, body) {
                if let Ok(conn) = state.db.get() {
                    let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
                }
                return Err(e);
            }
        }
    }

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
                let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
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
        crew_addendum.clone(),
        Arc::clone(&log_arc),
        injector,
    ) {
        Ok(r) => r,
        Err(e) => {
            if let Ok(conn) = state.db.get() {
                let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
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
    let mut pendings: Vec<crate::session::PendingMissionSpawn> = Vec::with_capacity(roster.len());
    // Two-phase spawn: `register_mission_session` is the synchronous
    // part — insert the DB row, generate the session id, compose the
    // SpawnSpec — and runs in this loop. The slow part (gate +
    // `runtime.spawn` + reader thread) is `complete_mission_session_spawn`
    // and is dispatched in a background task after router + bus mount,
    // so the Start-mission RPC returns in ~milliseconds instead of
    // ~1.5s × (claude_workers). See issue #171.
    //
    // Iteration is plain position order — the modal no longer blocks
    // on the gate, so there's no user-visible benefit to promoting
    // the lead ahead of position-zero workers.
    for (idx, member) in roster.iter().enumerate() {
        let first_turn = first_turns.get(idx).cloned().flatten();
        let register_res = state.sessions.register_mission_session(
            &out.mission,
            &member.runner,
            &member.slot,
            &state.app_data_dir,
            events_log_path.clone(),
            state.db.clone(),
            first_turn,
            initial_size,
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
                if let Ok(conn) = state.db.get() {
                    let _ = repo::session::delete_all_for_mission(&conn, &out.mission.id);
                    let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
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
        // Bus didn't attach — drop the pending PTY spawns, DELETE the
        // session rows (no PTYs ever forked for them at this point),
        // abort the mission. `pendings` going out of scope releases
        // any held resources without ever firing
        // `complete_mission_session_spawn`.
        drop(pendings);
        if let Ok(conn) = state.db.get() {
            let _ = repo::session::delete_all_for_mission(&conn, &out.mission.id);
            let _ = repo::mission::abort(&conn, &out.mission.id, Utc::now());
        }
        return Err(e);
    }

    state.routers.register(out.mission.id.clone(), router);

    // Dispatch the gate-blocked PTY-spawn phase to a background task
    // so this RPC returns now. The frontend already has every
    // `sessions` row from `register_mission_session`, so the
    // workspace mounts immediately and renders starting pills; each
    // pill clears when its PTY emits the TUI-ready signal (see
    // `chunkIndicatesTuiReady`). Spawns run sequentially inside the
    // task so the claude-code launch gate's lead-first ordering is
    // preserved.
    //
    // Per-mission cancel flag lets Stop / Archive / Reset abort
    // queued slots before their PTYs fork; without it, a slot
    // sleeping in the gate would spawn into a stopped mission. See
    // `SessionManager::cancel_pending_mission_spawns`.
    //
    // On per-slot spawn failure we mark just that row crashed and
    // emit `session/exit` so the workspace's row refresh fires and
    // the pane flips from "starting" to "session ended" without
    // a manual refresh. Rest of the mission proceeds — user gets a
    // Resume button on the broken slot.
    let manager = Arc::clone(&state.sessions);
    let pool_for_task = state.db.clone();
    let mission_id_for_task = out.mission.id.clone();
    let emitter_for_task = Arc::clone(&emitter);
    let cancel = state
        .sessions
        .register_pending_mission_cancel(&out.mission.id);
    let cancel_for_drop = Arc::clone(&cancel);
    tauri::async_runtime::spawn_blocking(move || {
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
        // still *this* task's flag. A concurrent mission_reset that
        // overwrote it with a fresh batch's flag must keep that flag
        // reachable from `cancel_pending_mission_spawns`.
        manager.drop_pending_mission_cancel(&mission_id_for_task, &cancel_for_drop);
    });

    log::info!(
        "mission started: id={} sessions={}",
        out.mission.id,
        spawned_pairs.len(),
    );
    Ok(out)
}

#[tauri::command]
pub async fn mission_start(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    input: StartMissionInput,
    initial_cols: Option<u16>,
    initial_rows: Option<u16>,
) -> Result<StartMissionOutput> {
    let initial_size = initial_cols
        .zip(initial_rows)
        .filter(|(cols, rows)| *cols > 0 && *rows > 0);
    let output = mission_start_impl_with_size(&state, &app, input, initial_size).await?;
    // The new mission node joined the sidebar tree (feature 44).
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(output)
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
///  * `mount_all_running_mission_routers` — app startup path; runs once
///    per running mission so router/event-bus fanout is restored for
///    missions that remain marked running in the DB.
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

    let (crew_name, crew_addendum) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &mission.crew_id)?;
        (crew.name, crew.system_prompt_addendum)
    };
    let allowed_signals = all_known_signals();
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
        crew_addendum,
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
/// Runs once at app startup. The NDJSON log is the source of truth;
/// this just re-wires the in-memory fanout layer that died with the
/// old process. The PTY startup cleanup later demotes stale running
/// session rows; it does not reattach child processes from the prior
/// app process.
///
/// Per-mission isolation: if one mission's mount fails (corrupt log,
/// missing crew row, etc.), it gets logged and the loop continues.
pub(crate) async fn mount_all_running_mission_routers(state: &AppState, app: &tauri::AppHandle) {
    let mission_ids = match state.db.get() {
        Ok(conn) => list_running_mission_ids(&conn).unwrap_or_else(|e| {
            log::error!("mount_all_running_mission_routers query failed: {e}");
            Vec::new()
        }),
        Err(e) => {
            log::error!("mount_all_running_mission_routers db pool unavailable: {e}");
            Vec::new()
        }
    };

    for mission_id in mission_ids {
        match ensure_mission_router_mounted(state, app, &mission_id).await {
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

/// Return the ids of every mission that's currently `running` and not
/// archived. Factored out of `mount_all_running_mission_routers` so the
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
pub(crate) async fn mission_stop_impl(state: &AppState, id: String) -> Result<Mission> {
    log::info!("mission stop: id={id}");
    state.sessions.kill_all_for_mission(&id)?;
    let conn = state.db.get()?;
    get(&conn, &id)
}

#[tauri::command]
pub async fn mission_stop(state: State<'_, AppState>, id: String) -> Result<Mission> {
    mission_stop_impl(&state, id).await
}

/// Toggle a mission's pin. Pinned missions float to the top of the
/// sidebar's MISSION list (sort key: `pinned_at IS NULL, pinned_at
/// DESC, started_at DESC`). Setting `pinned = false` clears the
/// timestamp.
pub(crate) async fn mission_pin_impl(
    state: &AppState,
    id: String,
    pinned: bool,
) -> Result<Mission> {
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

#[tauri::command]
pub async fn mission_pin(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    pinned: bool,
) -> Result<Mission> {
    let mission = mission_pin_impl(&state, id, pinned).await?;
    // Pin state renders from the node tree (feature 44).
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(mission)
}

/// Rename a mission. Title is trimmed; empty values are rejected so
/// the sidebar never renders a blank row. The mission's event log is
/// untouched — the title only ever lived on the row.
pub(crate) async fn mission_rename_impl(
    state: &AppState,
    id: String,
    title: String,
) -> Result<Mission> {
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

#[tauri::command]
pub async fn mission_rename(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<Mission> {
    mission_rename_impl(&state, id, title).await
}

#[tauri::command]
pub async fn mission_set_project(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    project_id: Option<String>,
) -> Result<Mission> {
    let mut conn = state.db.get()?;
    if repo::mission::set_project(&mut conn, &id, project_id.as_deref())? == 0 {
        return Err(Error::msg(format!("mission not found: {id}")));
    }
    // Keep the tree in step with the pointer: reparent the mission's
    // node under the new project's node (or root), appended at the end.
    if let Some(node) = repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &id)? {
        let parent = match project_id.as_deref() {
            Some(project_id) => Some(repo::node::ensure_project_node(&conn, project_id)?.id),
            None => None,
        };
        repo::node::reparent_append(&conn, &node.id, parent.as_deref())?;
    }
    let mission = get(&conn, &id)?;
    let _ = app.emit("mission/changed", serde_json::json!({ "mission_id": id }));
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(mission)
}

/// Reset a mission: wipe the run context (event log, agent session
/// keys, router state) and respawn every slot fresh against the same
/// mission row. Mostly for testing — gives you a clean slate without
/// having to rebuild the crew + start a new mission. Preserves the
/// mission's id, title, crew, cwd, and goal so links/bookmarks survive.
pub(crate) async fn mission_reset_impl(
    state: &AppState,
    app: &tauri::AppHandle,
    id: String,
    initial_size: Option<(u16, u16)>,
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
    let (crew_name, crew_goal, crew_addendum) = {
        let conn = state.db.get()?;
        let crew = crew::get(&conn, &mission_snap.crew_id)?;
        (crew.name, crew.goal, crew.system_prompt_addendum)
    };
    let allowed_signals = all_known_signals();
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
        repo::session::archive_all_for_mission(&conn, &id, now())?;
    }

    // 4. Wipe the event log + per-mission shim dir so the next spawn
    // starts from a clean slate. The roster sidecar gets rewritten
    // below from the current roster state.
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
        let n = repo::mission::reset_to_running(&conn, &id, started_at_dt)?;
        if n != 1 {
            return Err(Error::msg(format!("mission not found: {id}")));
        }
        // A reset returns the mission to the sidebar; re-create its
        // node if the archive removed it (feature 44).
        repo::node::ensure_mission_node(&conn, &id, mission_snap.project_id.as_deref())?;
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
    // boot — same contract as `mission_start`. Borrow of `crew_name`
    // ends here; it's moved into `Router::new` below.
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
                                crew_addendum: crew_addendum.as_deref(),
                            },
                        )
                    })
                } else {
                    Some(crate::router::prompt::compose_worker_first_turn(
                        m.runner.system_prompt.as_deref(),
                        crew_addendum.as_deref(),
                    ))
                }
            })
            .collect()
    };

    // Enforce the composed-body argv ceiling before respawning (same
    // guard as mission_start). A crew edited to oversized brief / goal /
    // conventions since the original start would otherwise boot agents
    // with an empty first turn; abort with an actionable error instead.
    // The mission is already torn down at this point, so `aborted` is the
    // consistent terminal state.
    for (member, body) in roster.iter().zip(&first_turns) {
        if let Some(body) = body {
            if let Err(e) = ensure_first_turn_fits(&member.slot.slot_handle, body) {
                if let Ok(conn) = state.db.get() {
                    let _ = repo::mission::abort(&conn, &id, now());
                }
                return Err(e);
            }
        }
    }

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
        allowed_signals,
        crew_addendum.clone(),
        Arc::clone(&log_arc),
        injector,
    )?;

    let emitter: Arc<dyn SessionEvents> = Arc::new(TauriSessionEvents(app.clone()));
    let mut spawned_pairs: Vec<(String, String)> = Vec::with_capacity(roster.len());
    let mut pendings: Vec<crate::session::PendingMissionSpawn> = Vec::with_capacity(roster.len());
    let mission_for_spawn = {
        let conn = state.db.get()?;
        get(&conn, &id)?
    };
    // Two-phase spawn: same shape as `mission_start`. Register
    // (synchronous, fast: DB row insert) here so router + bus mount
    // see the full session map; the slow `complete_*` phase (gate +
    // PTY fork + reader thread) is dispatched as a background task
    // after bus mount succeeds. Iteration is plain position order;
    // see the analogous block in `mission_start`. See issue #171.
    for (idx, member) in roster.iter().enumerate() {
        let first_turn = first_turns.get(idx).cloned().flatten();
        // `initial_size` matters beyond first paint: an unsized respawn
        // forks at 80×24 and seeds the resize purge gate at 80 cols, so
        // the agent's launch frames land in the ring at 80 cols and the
        // first slot-tab activation's real-cols push purges them all
        // (`SessionManager::resize` cols-gate). Sized respawns make that
        // push a same-width no-op and the opening history survives.
        let register_res = state.sessions.register_mission_session(
            &mission_for_spawn,
            &member.runner,
            &member.slot,
            &state.app_data_dir,
            events_log_path.clone(),
            state.db.clone(),
            first_turn,
            initial_size,
        );
        match register_res {
            Ok(pending) => {
                spawned_pairs.push((member.slot.slot_handle.clone(), pending.session_id.clone()));
                pendings.push(pending);
            }
            Err(e) => {
                // Rollback: delete the freshly-inserted session rows
                // (no PTYs forked yet — only DB inserts so far), abort
                // the mission. Bus / router aren't mounted yet so no
                // event-side cleanup.
                if let Ok(conn) = state.db.get() {
                    let _ = repo::session::delete_all_for_mission(&conn, &id);
                    let _ = repo::mission::abort(&conn, &id, now());
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
        // Bus didn't attach — drop the pending PTY spawns (none have
        // forked yet) and DELETE the inserted session rows. The bus
        // is the last gate before commit so a failure here means
        // mission stays aborted.
        drop(pendings);
        if let Ok(conn) = state.db.get() {
            let _ = repo::session::delete_all_for_mission(&conn, &id);
            let _ = repo::mission::abort(&conn, &id, now());
        }
        return Err(e);
    }
    state.routers.register(id.clone(), router);

    // Dispatch the gate-blocked PTY-spawn phase to a background task
    // so this RPC returns now. See the analogous block in
    // `mission_start` for the trade-offs (cancel-flag handling,
    // session/exit emission on failure, per-slot crashed status
    // instead of rolling back the whole reset).
    let manager = Arc::clone(&state.sessions);
    let pool_for_task = state.db.clone();
    let mission_id_for_task = id.clone();
    let emitter_for_task = Arc::clone(&emitter);
    let cancel = state.sessions.register_pending_mission_cancel(&id);
    let cancel_for_drop = Arc::clone(&cancel);
    tauri::async_runtime::spawn_blocking(move || {
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
                            now(),
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
                            now(),
                        );
                    }
                    emitter_for_task.exit(&crate::session::manager::ExitEvent {
                        session_id: session_id.clone(),
                        mission_id: Some(mission_id_for_task.clone()),
                        exit_code: None,
                        success: false,
                    });
                }
            }
        }
        manager.drop_pending_mission_cancel(&mission_id_for_task, &cancel_for_drop);
    });

    Ok(mission_for_spawn)
}

#[tauri::command]
pub async fn mission_reset(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    initial_cols: Option<u16>,
    initial_rows: Option<u16>,
) -> Result<Mission> {
    let initial_size = initial_cols
        .zip(initial_rows)
        .filter(|(cols, rows)| *cols > 0 && *rows > 0);
    mission_reset_impl(&state, &app, id, initial_size).await
}

/// Terminal end-of-mission. Kills every live PTY, writes the
/// `mission_stopped` event, flips the mission row to `completed`, and
/// drops the router + bus. Mirrors what `mission_stop` used to do
/// before the lifecycle split — preserved as a separate command so
/// the workspace UI can guard it behind an explicit confirm.
pub(crate) async fn mission_archive_impl(state: &AppState, id: String) -> Result<Mission> {
    state.sessions.kill_all_for_mission(&id)?;
    let mut conn = state.db.get()?;
    let mission = stop(&mut conn, &state.app_data_dir, &id)?;
    state.buses.unmount(&id);
    state.routers.unregister(&id);
    Ok(mission)
}

#[tauri::command]
pub async fn mission_archive(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<Mission> {
    let mission = mission_archive_impl(&state, id).await?;
    // The mission's node left the sidebar tree (feature 44).
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(mission)
}

/// Clear a mission's archive marker so it reappears in active lists.
/// Nothing else changes: the row keeps `status = 'completed'` and its
/// `stopped_at` — the same state the migration backfill created and
/// every surface already renders (impl 0026). Idempotent: unarchiving
/// an active mission is a no-op Ok; unknown ids error via the trailing
/// `get`.
pub(crate) async fn mission_unarchive_impl(state: &AppState, id: String) -> Result<Mission> {
    let conn = state.db.get()?;
    repo::mission::unarchive(&conn, &id)?;
    let mission = get(&conn, &id)?;
    // Restore the sidebar node, appended at its parent's end (original
    // position not remembered, matching chat restore).
    repo::node::ensure_mission_node(&conn, &id, mission.project_id.as_deref())?;
    Ok(mission)
}

/// Emits `mission/changed` after the flip — the sidebar's MISSION list
/// listens on that channel, so the restored row reappears without a
/// manual refresh.
#[tauri::command]
pub async fn mission_unarchive(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<Mission> {
    let mission = mission_unarchive_impl(&state, id).await?;
    let _ = app.emit("mission/changed", ());
    let _ = app.emit("chat/layout-changed", serde_json::json!({}));
    Ok(mission)
}

/// Permanent delete of an archived mission: session rows first
/// (`sessions.mission_id` is `ON DELETE SET NULL` — deleting the
/// mission row alone would orphan them into the direct-chat lists),
/// then the mission row, one transaction — then its on-disk footprint:
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
    repo::session::delete_all_for_mission(&tx, id)?;
    repo::mission::delete_archived(&tx, id)?;
    tx.commit()?;
    let mission_dir = event_log::mission_dir(app_data_dir, &mission.crew_id, id);
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

/// Settings → Archived permanent delete (feature 01 Phase 4). Archived
/// missions have no live PTYs, bus, or router — `mission_archive`
/// dropped them — so `delete_archived` covers everything.
#[tauri::command]
pub async fn mission_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    let mut conn = state.db.get()?;
    delete_archived(&mut conn, &state.app_data_dir, &id)?;
    Ok(())
}

#[tauri::command]
pub async fn mission_list(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<Mission>> {
    let conn = state.db.get()?;
    list(&conn, crew_id.as_deref())
}

/// Archived missions, newest-archived first — the Settings → Archived
/// pane's read surface.
#[tauri::command]
pub async fn mission_list_archived(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<Mission>> {
    let conn = state.db.get()?;
    repo::mission::list_archived(&conn, crew_id.as_deref()).map_err(Into::into)
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

fn latest_runner_statuses_from_log(
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
        if t.as_str() != "runner_status" {
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
    let latest = latest_runner_statuses_from_log(mission_dir);
    mission_activity_from_latest(live_handles, &latest)
}

fn live_session_handles(conn: &Connection, mission_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(sl.slot_handle, r.handle) AS handle
           FROM sessions s
           JOIN runners r ON r.id = s.runner_id
           LEFT JOIN slots sl ON sl.id = s.slot_id
          WHERE s.mission_id = ?1
            AND s.status = 'running'",
    )?;
    let rows = stmt.query_map(params![mission_id], |row| row.get::<_, String>(0))?;
    rows.collect()
}

fn all_mission_sessions_live(conn: &Connection, mission_id: &str) -> rusqlite::Result<bool> {
    let (total, running): (usize, usize) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0)
           FROM sessions
          WHERE mission_id = ?1
            AND archived_at IS NULL",
        params![mission_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(total > 0 && running == total)
}

pub(crate) async fn mission_list_summary_impl(
    state: &AppState,
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
        // The same query feeds both projections: `any_session_live`
        // preserves the existing paused/no-live behavior, while
        // `activity` overlays busy/idle only for live mission slots.
        let live_handles = live_session_handles(&conn, &m.id).unwrap_or_default();
        let any_session_live = !live_handles.is_empty();
        let all_sessions_live = all_mission_sessions_live(&conn, &m.id).unwrap_or(false);
        let activity = if any_session_live {
            let mission_dir = event_log::mission_dir(&state.app_data_dir, &m.crew_id, &m.id);
            mission_activity_from_log(&mission_dir, &live_handles)
        } else {
            None
        };
        summaries.push(MissionSummary {
            mission: m,
            crew_name,
            pending_ask_count,
            any_session_live,
            all_sessions_live,
            activity,
        });
    }
    Ok(summaries)
}

#[tauri::command]
pub async fn mission_list_summary(
    state: State<'_, AppState>,
    crew_id: Option<String>,
) -> Result<Vec<MissionSummary>> {
    mission_list_summary_impl(&state, crew_id).await
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
                goal: goal.map(String::from),
                ..Default::default()
            },
        )
        .unwrap();
        crew.id
    }

    fn add_runner(conn: &mut Connection, crew_id: &str, handle: &str) -> String {
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
        slot::create(conn, crew_id, &r.id, handle, None)
            .unwrap()
            .slot
            .id
    }

    fn append_runner_status(
        log: &EventLog,
        crew_id: &str,
        mission_id: &str,
        handle: &str,
        state: &str,
    ) {
        log.append(EventDraft::signal(
            crew_id.to_string(),
            mission_id.to_string(),
            handle,
            SignalType::new("runner_status"),
            serde_json::json!({ "state": state }),
        ))
        .unwrap();
    }

    #[test]
    fn ensure_first_turn_fits_guards_the_argv_ceiling() {
        // #247: the composed body must stay under the argv ceiling even
        // when every per-field persist cap is individually satisfied
        // (brief 16 KB + goal 8 KB + uncapped team conventions can
        // together overflow). Bodies at the limit pass; a byte over is
        // rejected with a slot-named, actionable error.
        let max = crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES;
        ensure_first_turn_fits("lead", &"x".repeat(max))
            .expect("a body exactly at the ceiling must be accepted");

        let err = ensure_first_turn_fits("reviewer", &"x".repeat(max + 1))
            .expect_err("a body one byte past the ceiling must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("reviewer"),
            "error must name the slot; got: {msg}"
        );
        assert!(
            msg.contains("argv ceiling"),
            "error must explain the ceiling; got: {msg}",
        );
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
        let project = repo::project::create(&conn, "Runner", "/tmp/work").unwrap();
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: Some(project.id.clone()),
                crew_id: crew_id.clone(),
                title: "first mission".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();

        assert_eq!(out.mission.title, "first mission");
        assert_eq!(out.mission.project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(out.mission.cwd.as_deref(), Some("/tmp/work"));
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

        // No signal_types sidecar — CLI validation is enum-based now.
        let stale_sidecar = event_log::crew_dir(tmp.path(), &crew_id).join("signal_types.json");
        assert!(
            !stale_sidecar.exists(),
            "mission_start must not write the legacy signal_types.json sidecar"
        );
    }

    #[test]
    fn start_explicit_cwd_overrides_project_default() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "Alpha", None);
        add_runner(&mut conn, &crew_id, "lead");
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: Some(project.id),
                crew_id,
                title: "override cwd".into(),
                goal_override: None,
                cwd: Some("/override".into()),
            },
        )
        .unwrap();

        assert_eq!(out.mission.cwd.as_deref(), Some("/override"));
    }

    #[test]
    fn start_unknown_project_creates_no_mission() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "Alpha", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let error = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: Some("missing".into()),
                crew_id,
                title: "unknown project".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "project not found: missing");
        let mission_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM missions", [], |row| row.get(0))
            .unwrap();
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mission_count, 0);
        assert_eq!(session_count, 0);
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
                project_id: None,
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
                project_id: None,
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
    fn delete_refuses_non_archived_mission() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: None,
                crew_id,
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();

        let err = delete_archived(&mut conn, tmp.path(), &out.mission.id).unwrap_err();
        assert!(
            err.to_string().contains("not archived"),
            "expected archive-first refusal, got: {err}"
        );

        let unknown = delete_archived(&mut conn, tmp.path(), "no-such-mission").unwrap_err();
        assert!(unknown.to_string().contains("not found"));
    }

    #[test]
    fn delete_archived_removes_mission_and_session_rows() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: None,
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();
        let id = out.mission.id.clone();
        // A stopped slot session — the cascade must take it with the
        // mission instead of leaving an orphan (`mission_id` is
        // ON DELETE SET NULL, which would leak it into direct lists).
        conn.execute(
            "INSERT INTO sessions (id, mission_id, slot_id, status, started_at)
             VALUES ('s-del', ?1, 's1', 'stopped', '2026-07-11T00:00:00Z')",
            params![id],
        )
        .unwrap();
        stop(&mut conn, tmp.path(), &id).unwrap();

        // On-disk footprint: the event-log dir (created by start) and
        // the per-slot shim scratch dir (created at spawn — faked here,
        // the bookkeeping-layer start doesn't spawn PTYs).
        let mission_dir = event_log::mission_dir(tmp.path(), &crew_id, &id);
        assert!(mission_dir.exists());
        let scratch_dir = tmp.path().join("missions").join(&id);
        std::fs::create_dir_all(scratch_dir.join("shims").join("lead").join("bin")).unwrap();

        let deleted = delete_archived(&mut conn, tmp.path(), &id).unwrap();
        assert_eq!(deleted.id, id);
        assert_eq!(deleted.crew_id, crew_id);

        let missions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM missions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(missions, 0);
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 's-del'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "session rows must die with the mission");
        assert!(!mission_dir.exists(), "event-log dir must be removed");
        assert!(!scratch_dir.exists(), "shim scratch dir must be removed");
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
    fn mission_activity_from_log_defaults_busy_until_all_live_slots_idle() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        add_runner(&mut conn, &crew_id, "lead");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: None,
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();
        let mission_dir = event_log::mission_dir(tmp.path(), &crew_id, &out.mission.id);
        let log = EventLog::open(&mission_dir).unwrap();
        let live_handles = vec!["lead".to_string(), "reviewer".to_string()];

        assert_eq!(
            mission_activity_from_log(&mission_dir, &[]),
            None,
            "no live sessions keeps paused/no-live activity empty"
        );
        assert_eq!(
            mission_activity_from_log(&mission_dir, &live_handles),
            Some(MissionActivityState::Busy),
            "live slot without runner_status defaults busy"
        );

        append_runner_status(&log, &crew_id, &out.mission.id, "lead", "idle");
        assert_eq!(
            mission_activity_from_log(&mission_dir, &live_handles),
            Some(MissionActivityState::Busy),
            "one missing live slot still reads busy"
        );

        append_runner_status(&log, &crew_id, &out.mission.id, "reviewer", "idle");
        assert_eq!(
            mission_activity_from_log(&mission_dir, &live_handles),
            Some(MissionActivityState::Idle),
            "all live slots idle reads idle"
        );

        append_runner_status(&log, &crew_id, &out.mission.id, "lead", "busy");
        assert_eq!(
            mission_activity_from_log(&mission_dir, &live_handles),
            Some(MissionActivityState::Busy),
            "latest busy status wins"
        );

        append_runner_status(&log, &crew_id, &out.mission.id, "lead", "idle");
        assert_eq!(
            mission_activity_from_log(&mission_dir, &live_handles),
            Some(MissionActivityState::Idle),
            "latest idle status wins after busy"
        );
    }

    #[test]
    fn mission_activity_uses_running_slot_handles_only() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew_id = seed_crew(&conn, "A", None);
        let lead_slot_id = add_runner(&mut conn, &crew_id, "lead");
        let worker_slot_id = add_runner(&mut conn, &crew_id, "worker");
        let tmp = tempfile::tempdir().unwrap();

        let out = start(
            &mut conn,
            tmp.path(),
            StartMissionInput {
                project_id: None,
                crew_id: crew_id.clone(),
                title: "m".into(),
                goal_override: None,
                cwd: None,
            },
        )
        .unwrap();

        let lead_runner_id: String = conn
            .query_row(
                "SELECT runner_id FROM slots WHERE id = ?1",
                params![lead_slot_id],
                |r| r.get(0),
            )
            .unwrap();
        let worker_runner_id: String = conn
            .query_row(
                "SELECT runner_id FROM slots WHERE id = ?1",
                params![worker_slot_id],
                |r| r.get(0),
            )
            .unwrap();
        let ts = now().to_rfc3339();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, slot_id, status, started_at)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
            params![
                new_id(),
                out.mission.id.clone(),
                lead_runner_id,
                lead_slot_id,
                ts
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, slot_id, status, started_at)
             VALUES (?1, ?2, ?3, ?4, 'stopped', ?5)",
            params![
                new_id(),
                out.mission.id.clone(),
                worker_runner_id,
                worker_slot_id,
                now().to_rfc3339()
            ],
        )
        .unwrap();

        let handles = live_session_handles(&conn, &out.mission.id).unwrap();
        assert_eq!(
            handles,
            vec!["lead".to_string()],
            "projection must key live sessions by slot_handle"
        );
        assert!(
            !all_mission_sessions_live(&conn, &out.mission.id).unwrap(),
            "one stopped slot keeps the mission-level live state false"
        );

        let mission_dir = event_log::mission_dir(tmp.path(), &crew_id, &out.mission.id);
        let log = EventLog::open(&mission_dir).unwrap();
        append_runner_status(&log, &crew_id, &out.mission.id, "lead", "idle");
        append_runner_status(&log, &crew_id, &out.mission.id, "worker", "busy");
        assert_eq!(
            mission_activity_from_log(&mission_dir, &handles),
            Some(MissionActivityState::Idle),
            "stopped slot statuses must not keep the mission busy"
        );

        conn.execute(
            "UPDATE sessions SET status = 'running' WHERE mission_id = ?1",
            params![out.mission.id],
        )
        .unwrap();
        assert!(
            all_mission_sessions_live(&conn, &out.mission.id).unwrap(),
            "all running slots make the mission-level live state true"
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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
                project_id: None,
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

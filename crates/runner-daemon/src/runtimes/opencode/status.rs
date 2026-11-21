use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::OptionalExtension;

use super::*;
use crate::session::hook_feed::HookFeed;
use crate::session::state::agent::{AdapterFeedback, AgentEvent, ProgramOutcomePolicy};
use crate::session::status::TurnOutcome;

const SOURCE: &str = include_str!("tui.mjs");
fn plugin_dir(app_data: &Path) -> PathBuf {
    app_data.join("opencode-hooks")
}
pub(super) fn install(app_data: &Path) -> crate::error::Result<()> {
    let directory = plugin_dir(app_data);
    std::fs::create_dir_all(&directory)?;
    write_config_atomically(
        &directory.join("package.json"),
        br#"{"name":"runner-opencode-status","type":"module","exports":{".":"./index.mjs","./tui":"./tui.mjs"}}"#,
    )?;
    write_config_atomically(&directory.join("index.mjs"), include_bytes!("model.mjs"))?;
    write_config_atomically(&directory.join("tui.mjs"), SOURCE.as_bytes())
}
pub(super) fn model_config(
    object: &mut serde_json::Map<String, Value>,
    app_data: &Path,
    model: &str,
    variant: &str,
) -> crate::error::Result<()> {
    let (provider, id) = model
        .split_once('/')
        .ok_or_else(|| crate::error::Error::msg("OpenCode model must use provider/model format"))?;
    install(app_data)?;
    let directory = plugin_dir(app_data).canonicalize()?;
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| crate::error::Error::msg("OpenCode plugins must be an array"))?;
    plugins.push(json!({
        "package": directory.to_string_lossy(),
        "options": {
            "model": {"providerID": provider, "id": id, "variant": variant},
        }
    }));
    Ok(())
}
pub(super) fn launch_config(
    app_data: &Path,
    env: &BTreeMap<String, String>,
) -> crate::error::Result<String> {
    install(app_data)?;
    let env = config::effective_env(env);
    let mut document = config::parse(
        env.get("OPENCODE_CLI_CONFIG_CONTENT")
            .map(String::as_str)
            .unwrap_or("{}"),
    )?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| crate::error::Error::msg("OpenCode CLI config must be an object"))?;
    if !object.contains_key("plugins") {
        if let Some(directory) =
            config::config_dir(&env, runner_core::app_paths::home_dir().as_deref())
        {
            if let Ok(raw) = std::fs::read_to_string(directory.join("cli.json")) {
                let native = config::parse(&raw)?;
                if let Some(plugins) = native.get("plugins") {
                    object.insert("plugins".into(), plugins.clone());
                }
            }
        }
    }
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| crate::error::Error::msg("OpenCode CLI plugins must be an array"))?;
    let directory = plugin_dir(app_data).canonicalize()?;
    let entry = json!({"package":directory.to_string_lossy()});
    if !plugins.contains(&entry) {
        plugins.push(entry);
    }
    Ok(document.to_string())
}

pub(crate) struct Watcher {
    feed: HookFeed,
    key: Option<String>,
    /// Offscreen sessions left while a turn might still settle; retired as
    /// soon as polling observes them idle/settled or missing.
    background_sessions: BTreeSet<String>,
    sequence: u64,
    epoch: u64,
    connected: bool,
    launched_at_ms: i64,
    db_path: Option<PathBuf>,
    db: Option<rusqlite::Connection>,
    has_session_message: bool,
    seen_time_idle: HashMap<String, i64>,
    /// Deduplicates an assistant `finish=error` message read from
    /// `session_message` when `session_v2.time_idle` lands moments later for
    /// the same turn (e.g. double-Esc interrupt).
    seen_assistant_turn: HashMap<String, AssistantTurn>,
    outcome_unavailable_emitted: bool,
}

struct AssistantTurn {
    busy_ms: Option<i64>,
    seq: i64,
    outcome: String,
}

enum RowPoll {
    RetireOffscreen(String),
    AdvanceWatermark {
        session_id: String,
        ended_at_ms: i64,
    },
    Emit {
        session_id: String,
        ended_at_ms: i64,
        superseded: bool,
        idle_outcome: Option<String>,
        assistant_turn: Option<AssistantTurn>,
    },
}

/// Reads the newest finalized `session_message` row when `session_v2.time_idle`
/// has not advanced past `time_busy`. OpenCode 2.3.2 writes permission
/// rejections as an `assistant` message with `finish = "error"` and
/// `error.type = "aborted"` without inserting an `idle` message or advancing
/// `session_v2.time_idle`.
fn latest_assistant_error(
    conn: &rusqlite::Connection,
    session_id: &str,
    time_busy: Option<i64>,
) -> rusqlite::Result<Option<(i64, i64, &'static str)>> {
    let mut stmt = conn.prepare(
        "SELECT seq, type, time_updated, data FROM session_message WHERE session_id = ?1 AND type IN ('assistant', 'idle') ORDER BY seq DESC LIMIT 4",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut saw_streaming_assistant = false;
    for row in rows {
        let (seq, msg_type, time_updated, raw_data) = row?;
        if msg_type != "assistant" {
            return Ok(None);
        }
        let Ok(data) = serde_json::from_str::<Value>(&raw_data) else {
            return Ok(None);
        };
        let error = data.get("error").filter(|v| !v.is_null());
        let completed_at = data
            .pointer("/time/completed")
            .and_then(Value::as_i64)
            .or_else(|| {
                (data.get("finish").and_then(Value::as_str) == Some("error"))
                    .then_some(time_updated)
            });
        let finished = error.is_some()
            || completed_at.is_some()
            || data.get("finish").and_then(Value::as_str).is_some();
        if !finished {
            saw_streaming_assistant = true;
            continue;
        }
        let Some(error) = error else {
            return Ok(None);
        };
        let Some(completed_at) = completed_at else {
            return Ok(None);
        };
        if saw_streaming_assistant && !time_busy.is_some_and(|busy| busy > completed_at) {
            return Ok(None);
        }
        let outcome = if error.get("type").and_then(Value::as_str) == Some("aborted") {
            "interrupted"
        } else {
            "failed"
        };
        return Ok(Some((seq, completed_at, outcome)));
    }
    Ok(None)
}

/// Returns `true` when `session_v2.time_idle` belongs to the exact same turn
/// whose error was already emitted from `session_message` at `marker.seq`.
fn is_same_assistant_turn_idle(
    conn: &rusqlite::Connection,
    session_id: &str,
    marker: &AssistantTurn,
    time_busy: Option<i64>,
    superseded: bool,
    idle_outcome: Option<&str>,
    has_session_message: bool,
) -> rusqlite::Result<bool> {
    if idle_outcome != Some(marker.outcome.as_str()) {
        return Ok(false);
    }
    if has_session_message {
        let latest_idle_seq: Option<i64> = conn
            .query_row(
                "SELECT seq FROM session_message WHERE session_id = ?1 AND type = 'idle' ORDER BY seq DESC LIMIT 1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(latest_seq) = latest_idle_seq {
            let next_after_marker: Option<(i64, String)> = conn
                .query_row(
                    "SELECT seq, type FROM session_message WHERE session_id = ?1 AND type IN ('assistant', 'idle') AND seq > ?2 ORDER BY seq ASC LIMIT 1",
                    rusqlite::params![session_id, marker.seq],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            return Ok(next_after_marker
                .is_some_and(|(seq, msg_type)| msg_type == "idle" && seq == latest_seq));
        }
    }
    Ok(!superseded && marker.busy_ms.is_some() && time_busy == marker.busy_ms)
}

impl Watcher {
    pub(super) fn new(
        receiver: crate::session::hook_queue::HookReceiver,
        spec: &SpawnSpec,
    ) -> Self {
        let key = spec
            .args
            .windows(2)
            .find(|a| a[0] == "--session")
            .map(|a| a[1].clone());
        let db_path = config::db_path(&spec.env);
        Self::with_db(
            receiver,
            key,
            db_path,
            chrono::Utc::now().timestamp_millis(),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_receiver(
        receiver: crate::session::hook_queue::HookReceiver,
        db_path: Option<PathBuf>,
    ) -> Self {
        Self::with_db(receiver, None, db_path, 0)
    }

    pub(crate) fn with_db(
        receiver: crate::session::hook_queue::HookReceiver,
        key: Option<String>,
        db_path: Option<PathBuf>,
        launched_at_ms: i64,
    ) -> Self {
        Self {
            feed: HookFeed::from_receiver(receiver),
            key,
            background_sessions: BTreeSet::new(),
            sequence: 0,
            epoch: 0,
            connected: false,
            launched_at_ms,
            db_path,
            db: None,
            has_session_message: false,
            seen_time_idle: HashMap::new(),
            seen_assistant_turn: HashMap::new(),
            outcome_unavailable_emitted: false,
        }
    }

    #[cfg(test)]
    fn polled_session_count(&self) -> usize {
        usize::from(self.key.is_some())
            + self
                .background_sessions
                .iter()
                .filter(|id| self.key.as_deref() != Some(id.as_str()))
                .count()
    }

    fn emit_outcome_unavailable(&mut self, emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback) {
        if self.connected && !self.outcome_unavailable_emitted {
            self.outcome_unavailable_emitted = true;
            emit(AgentEvent::ProgramOutcomeUnavailable);
        }
    }

    fn poll_session_outcome(&mut self, emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback) {
        let Some(path) = self.db_path.as_deref() else {
            self.emit_outcome_unavailable(emit);
            return;
        };
        if self.db.is_none() {
            if !path.is_file() {
                self.emit_outcome_unavailable(emit);
                return;
            }
            let Ok(conn) = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                self.emit_outcome_unavailable(emit);
                return;
            };
            let _ = conn.busy_timeout(Duration::from_millis(50));
            self.has_session_message = conn
                .prepare("SELECT 1 FROM session_message LIMIT 0")
                .is_ok();
            self.db = Some(conn);
        }
        let mut session_ids: Vec<String> = self.background_sessions.iter().cloned().collect();
        if let Some(active) = self.key.as_ref() {
            if !self.background_sessions.contains(active) {
                session_ids.push(active.clone());
            }
        }
        session_ids.sort();
        if session_ids.is_empty() {
            if self.connected {
                let readable = self
                    .db
                    .as_ref()
                    .is_some_and(|conn| conn.prepare("SELECT 1 FROM session_v2 LIMIT 0").is_ok());
                if !readable {
                    self.db = None;
                    self.emit_outcome_unavailable(emit);
                }
            }
            return;
        }
        let mut query_failed = false;
        let mut actions = Vec::new();
        {
            let Some(conn) = self.db.as_ref() else {
                return;
            };
            for session_id in &session_ids {
                let offscreen = self.key.as_deref() != Some(session_id.as_str());
                let row = conn
                    .query_row(
                        "SELECT time_busy, time_idle, idle_outcome FROM session_v2 WHERE id = ?1",
                        [session_id],
                        |row| {
                            Ok((
                                row.get::<_, Option<i64>>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )
                    .optional();
                let row = match row {
                    Ok(row) => row,
                    Err(_) => {
                        query_failed = true;
                        break;
                    }
                };
                let Some((time_busy, time_idle, idle_outcome)) = row else {
                    if offscreen {
                        actions.push(RowPoll::RetireOffscreen(session_id.clone()));
                    }
                    continue;
                };
                let mut effective_watermark = self
                    .seen_time_idle
                    .get(session_id)
                    .copied()
                    .unwrap_or_else(|| self.launched_at_ms.saturating_sub(1));
                // Process distinct unseen outcomes oldest-first: any unseen
                // session_v2.time_idle is strictly older than a newer
                // assistant-only error with completed_at > time_idle.
                if let Some(next_idle) = time_idle {
                    if next_idle > effective_watermark {
                        let v2_superseded = time_busy.is_some_and(|busy| busy > next_idle);
                        let same_turn = match self.seen_assistant_turn.get(session_id) {
                            Some(marker) => match is_same_assistant_turn_idle(
                                conn,
                                session_id,
                                marker,
                                time_busy,
                                v2_superseded,
                                idle_outcome.as_deref(),
                                self.has_session_message,
                            ) {
                                Ok(matched) => matched,
                                Err(_) => {
                                    query_failed = true;
                                    break;
                                }
                            },
                            None => false,
                        };
                        if same_turn {
                            actions.push(RowPoll::AdvanceWatermark {
                                session_id: session_id.clone(),
                                ended_at_ms: next_idle,
                            });
                        } else {
                            actions.push(RowPoll::Emit {
                                session_id: session_id.clone(),
                                ended_at_ms: next_idle,
                                superseded: v2_superseded,
                                idle_outcome,
                                assistant_turn: None,
                            });
                        }
                        effective_watermark = next_idle;
                    }
                }
                let unsettled =
                    time_idle.is_none() || time_busy.is_some_and(|busy| busy >= time_idle.unwrap());
                let mut still_active = match time_idle {
                    Some(idle) => time_busy.is_some_and(|busy| busy > idle),
                    None => time_busy.is_some(),
                };
                if self.has_session_message && unsettled {
                    let assistant_error = match latest_assistant_error(conn, session_id, time_busy)
                    {
                        Ok(err) => err,
                        Err(_) => {
                            query_failed = true;
                            break;
                        }
                    };
                    if let Some((seq, completed_at, outcome)) = assistant_error {
                        if completed_at > time_idle.unwrap_or(i64::MIN) {
                            let assistant_superseded =
                                time_busy.is_some_and(|busy| busy > completed_at);
                            if completed_at > effective_watermark {
                                actions.push(RowPoll::Emit {
                                    session_id: session_id.clone(),
                                    ended_at_ms: completed_at,
                                    superseded: assistant_superseded,
                                    idle_outcome: Some(outcome.to_owned()),
                                    assistant_turn: Some(AssistantTurn {
                                        busy_ms: (!assistant_superseded)
                                            .then_some(time_busy)
                                            .flatten(),
                                        seq,
                                        outcome: outcome.to_owned(),
                                    }),
                                });
                            }
                            still_active = assistant_superseded;
                        }
                    }
                }
                if offscreen && !still_active {
                    actions.push(RowPoll::RetireOffscreen(session_id.clone()));
                }
            }
        }
        // Emit every row read before any error and advance watermarks only as
        // each outcome is published so transient DB failures never drop prefix
        // outcomes or poison retries.
        for action in actions {
            match action {
                RowPoll::RetireOffscreen(session_id) => {
                    self.background_sessions.remove(&session_id);
                }
                RowPoll::AdvanceWatermark {
                    session_id,
                    ended_at_ms,
                } => {
                    self.seen_time_idle.insert(session_id.clone(), ended_at_ms);
                    self.seen_assistant_turn.remove(&session_id);
                }
                RowPoll::Emit {
                    session_id,
                    ended_at_ms,
                    superseded,
                    idle_outcome,
                    assistant_turn,
                } => {
                    self.seen_time_idle.insert(session_id.clone(), ended_at_ms);
                    if let Some(marker) = assistant_turn {
                        self.seen_assistant_turn.insert(session_id.clone(), marker);
                    } else {
                        self.seen_assistant_turn.remove(&session_id);
                    }
                    match idle_outcome.as_deref() {
                        Some("failed") => {
                            emit(AgentEvent::ProgramSessionOutcome {
                                outcome: TurnOutcome::Failed,
                                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                                session_id,
                                ended_at_ms,
                                superseded,
                            });
                        }
                        Some("interrupted") => {
                            emit(AgentEvent::ProgramSessionOutcome {
                                outcome: TurnOutcome::Interrupted,
                                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                                session_id,
                                ended_at_ms,
                                superseded,
                            });
                        }
                        Some("succeeded") => {
                            emit(AgentEvent::ProgramSessionFinished {
                                session_id,
                                ended_at_ms,
                                superseded,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        if query_failed {
            self.db = None;
            self.emit_outcome_unavailable(emit);
        }
    }
}

impl HookWatcher for Watcher {
    fn drain_events(
        &mut self,
        _cancel: u8,
        emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> crate::error::Result<()> {
        let key = &mut self.key;
        let background_sessions = &mut self.background_sessions;
        let sequence = &mut self.sequence;
        let epoch = &mut self.epoch;
        let connected = &mut self.connected;
        let result = self.feed.drain(|report| {
            let Some(current_epoch) = report["epoch"].as_u64().filter(|value| *value >= *epoch)
            else {
                return;
            };
            let Some(next) = report["sequence"]
                .as_u64()
                .filter(|next| current_epoch > *epoch || *next >= *sequence)
            else {
                return;
            };
            let Some(id) = report["session_id"]
                .as_str()
                .filter(|id| id.is_empty() || valid_key(id))
            else {
                return;
            };
            if report["child"] == true {
                return;
            }
            let event = report["hook_event_name"].as_str().unwrap_or_default();
            if event != "snapshot" {
                return;
            }
            let first_snapshot = !*connected;
            *connected = true;
            if current_epoch == *epoch && next == *sequence {
                return;
            }
            *sequence = next;
            *epoch = current_epoch;
            let new = (!id.is_empty()).then(|| id.to_owned());
            if *key != new {
                if let Some(previous) = key.as_ref() {
                    background_sessions.insert(previous.clone());
                }
                if let Some(current) = new.as_ref() {
                    background_sessions.remove(current);
                }
                let conversation_changed = key.is_some();
                *key = new;
                session_start(id.into());
                emit(AgentEvent::ProgramSessionStarted {
                    conversation_changed,
                    compacted: false,
                });
            } else if first_snapshot {
                emit(AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: false,
                });
            }
        });
        // Stop retires the route before killing the CLI; its last verified
        // conversation remains resumable, unlike a failed live bridge.
        if self.feed.is_retired() {
            return result;
        }
        if result.is_err() {
            self.key = None;
            session_start(String::new());
            return Err(crate::error::Error::msg("OpenCode hook bridge unavailable"));
        }
        self.poll_session_outcome(emit);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::hook_queue::TestHookRoute;

    fn make_opencode_db(dir: &Path) -> PathBuf {
        let path = dir.join("opencode/opencode.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();
        path
    }

    #[test]
    fn stopping_a_hook_route_preserves_the_last_verified_identity() {
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
        let mut watcher = Watcher::new(
            receiver,
            &SpawnSpec {
                args: strings(&["--session", "ses_launch"]),
                ..Default::default()
            },
        );
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":1,"session_id":"ses_switched"}))
                .unwrap(),
        );
        let mut keys = Vec::new();
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| keys.push(key))
            .unwrap();
        assert_eq!(keys, ["ses_switched"]);
        route.retire();
        assert!(watcher
            .drain_events(0, &mut |_| Default::default(), &mut |key| keys.push(key))
            .is_err());
        assert_eq!(keys, ["ses_switched"]);
    }

    #[test]
    fn bridge_unavailable_clears_identity_without_heartbeat_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = make_opencode_db(dir.path());
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
        let mut watcher = Watcher::with_db(receiver, Some("ses_owned".into()), Some(db_path), 0);
        let report =
            json!({"hook_event_name":"snapshot","epoch":1,"sequence":1,"session_id":"ses_owned"});
        let mut events = Vec::new();
        let mut keys = Vec::new();
        drop(route.admit(report.clone()).unwrap());
        watcher
            .drain_events(
                0,
                &mut |e| {
                    events.push(e);
                    Default::default()
                },
                &mut |k| keys.push(k),
            )
            .unwrap();
        assert_eq!(events.len(), 1);
        // Repeated identical sequence is a no-op and silence never expires the bridge.
        drop(route.admit(report).unwrap());
        watcher
            .drain_events(
                0,
                &mut |e| {
                    events.push(e);
                    Default::default()
                },
                &mut |k| keys.push(k),
            )
            .unwrap();
        assert_eq!(events.len(), 1);
        watcher
            .drain_events(0, &mut |_| Default::default(), &mut |k| keys.push(k))
            .unwrap();
        assert!(keys.is_empty());

        drop(route.admit_bridge_unavailable().unwrap());
        assert!(watcher
            .drain_events(0, &mut |_| Default::default(), &mut |k| keys.push(k))
            .is_err());
        assert_eq!(keys, [""]);
        route.retire();
    }

    #[test]
    fn sqlite_session_v2_polls_live_outcomes_without_replaying_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = make_opencode_db(dir.path());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let old_created = 1_000_000i64;
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_idle, idle_outcome) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["ses_resumed", old_created, old_created + 50, "failed"],
        )
        .unwrap();

        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
        let mut watcher = Watcher::with_db(
            receiver,
            Some("ses_resumed".into()),
            Some(db_path.clone()),
            2_000_000,
        );
        let mut events = Vec::new();
        let mut keys = Vec::new();
        let mut drain = |watcher: &mut Watcher| {
            events.clear();
            watcher
                .drain_events(
                    0,
                    &mut |e| {
                        events.push(e);
                        Default::default()
                    },
                    &mut |k| keys.push(k),
                )
                .unwrap();
            events.clone()
        };

        // Pre-existing failure on resumed session must not replay.
        assert!(drain(&mut watcher).is_empty());

        // Live failure advances time_idle and emits ProgramTurnOutcome(Failed).
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = 'ses_resumed'",
            [2_000_100i64],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_resumed".into(),
                ended_at_ms: 2_000_100,
                superseded: false,
            }]
        );
        // Same time_idle is never emitted twice.
        assert!(drain(&mut watcher).is_empty());

        // Live interruption and live success emit Interrupted and ProgramSessionFinished.
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'interrupted' WHERE id = 'ses_resumed'",
            [2_000_200i64],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_resumed".into(),
                ended_at_ms: 2_000_200,
                superseded: false,
            }]
        );
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_resumed'",
            [2_000_300i64],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionFinished {
                session_id: "ses_resumed".into(),
                ended_at_ms: 2_000_300,
                superseded: false,
            }]
        );

        // Brand-new session that already failed before tui.mjs reported its ID still emits Failed.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_idle, idle_outcome) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["ses_fresh", 2_000_400i64, 2_000_450i64, "failed"],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":1,"session_id":"ses_fresh"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: true,
                    compacted: false,
                },
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_fresh".into(),
                    ended_at_ms: 2_000_450,
                    superseded: false,
                },
            ]
        );
        // Switching away to home ("") while ses_fresh is busy keeps polling ses_fresh in the
        // background, reports superseded=true when time_busy > time_idle, and retires once settled.
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1 WHERE id = 'ses_fresh'",
            [2_000_500i64],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":2,"session_id":""}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            }]
        );
        assert_eq!(watcher.polled_session_count(), 1);
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1, time_idle = ?2, idle_outcome = 'failed' WHERE id = 'ses_fresh'",
            rusqlite::params![2_000_600i64, 2_000_550i64],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_fresh".into(),
                ended_at_ms: 2_000_550,
                superseded: true,
            }]
        );
        // Still busy offscreen (time_busy=2_000_600 > time_idle=2_000_550), so remains tracked
        // until its active turn settles.
        assert_eq!(watcher.polled_session_count(), 1);
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_fresh'",
            [2_000_650i64],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionFinished {
                session_id: "ses_fresh".into(),
                ended_at_ms: 2_000_650,
                superseded: false,
            }]
        );
        assert_eq!(
            watcher.polled_session_count(),
            0,
            "settled offscreen session must retire immediately"
        );

        // First poll of an older/resumed row whose time_idle is ALREADY >= launched_at_ms must
        // emit that live post-launch outcome rather than swallowing it as history.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_idle, idle_outcome) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["ses_older_live", old_created, 2_000_700i64, "failed"],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":3,"session_id":"ses_older_live"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: false,
                },
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_older_live".into(),
                    ended_at_ms: 2_000_700,
                    superseded: false,
                },
            ]
        );

        // Missing/:memory: DB emits ProgramOutcomeUnavailable once upon snapshot connect.
        let (mem_route, mem_receiver) = TestHookRoute::new(Runtime::OpenCode, "mem".into());
        let mut mem_watcher = Watcher::with_db(mem_receiver, None, None, 2_000_000);
        drop(
            mem_route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":1,"session_id":"ses_mem"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut mem_watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: false,
                },
                AgentEvent::ProgramOutcomeUnavailable,
            ]
        );
        assert!(drain(&mut mem_watcher).is_empty());

        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":4,"session_id":"ses_resumed"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            }]
        );
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":5,"session_id":"ses_fresh"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            }]
        );
        route.retire();
    }

    #[test]
    fn opencode_watcher_bounds_history_polling_and_preserves_prefix_across_transient_db_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = make_opencode_db(dir.path());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let launch_ms = 2_000_000i64;
        for i in 0..50 {
            let sid = format!("ses_hist_{i:02}");
            conn.execute(
                "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
                 VALUES (?1, ?2, ?3, ?4, 'succeeded')",
                rusqlite::params![sid, 1_000_000 + i, 1_000_000 + i, 1_000_010 + i],
            )
            .unwrap();
        }
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "hist".into());
        let mut watcher = Watcher::with_db(receiver, None, Some(db_path.clone()), launch_ms);
        let drain = |watcher: &mut Watcher| {
            let mut events = Vec::new();
            watcher
                .drain_events(
                    0,
                    &mut |ev| {
                        events.push(ev);
                        Default::default()
                    },
                    &mut |_| {},
                )
                .unwrap();
            events
        };

        for i in 0..50u64 {
            let sid = format!("ses_hist_{i:02}");
            drop(
                route
                    .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":i + 1,"session_id":sid}))
                    .unwrap(),
            );
            let _ = drain(&mut watcher);
        }
        // Visiting 50 idle history sessions leaves only the current route polled (1 session).
        assert_eq!(watcher.polled_session_count(), 1);
        drop(
            route
                .admit(
                    json!({"hook_event_name":"snapshot","epoch":1,"sequence":51,"session_id":""}),
                )
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        assert_eq!(
            watcher.polled_session_count(),
            0,
            "returning home after browsing 50 idle history sessions must leave zero polled rows"
        );

        // Transient DB error on a later row (ses_b) must emit the earlier prefix (ses_a),
        // advance ses_a's watermark (no duplicate), and deliver ses_b once the error clears.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_a', ?1, ?1, NULL, NULL)",
            [launch_ms + 10],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_b', ?1, ?1, NULL, NULL)",
            [launch_ms + 20],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":52,"session_id":"ses_a"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":53,"session_id":"ses_b"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        assert_eq!(watcher.polled_session_count(), 2);

        // ses_a settles cleanly, while ses_b temporarily has a non-integer value causing a
        // SQLite row read error right after ses_a is read.
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = 'ses_a'",
            [launch_ms + 100],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_v2 SET time_idle = 'transient-bad-type', idle_outcome = 'failed' WHERE id = 'ses_b'",
            [],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_a".into(),
                    ended_at_ms: launch_ms + 100,
                    superseded: false,
                },
                AgentEvent::ProgramOutcomeUnavailable,
            ]
        );
        // Once ses_b's transient error clears, retry emits ONLY ses_b (never re-emitting ses_a).
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = 'ses_b'",
            [launch_ms + 110],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_b".into(),
                ended_at_ms: launch_ms + 110,
                superseded: false,
            }]
        );
        route.retire();
    }

    #[test]
    fn opencode_watcher_reads_session_message_assistant_errors_and_dedupes_double_esc() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = make_opencode_db(dir.path());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_message(
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                type TEXT NOT NULL,
                seq INTEGER NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();

        let launch_ms = 2_000_000i64;
        // Pre-launch rejected assistant row must not replay.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_pre', ?1, ?1, NULL, NULL)",
            [launch_ms - 500],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('m_pre', 'ses_pre', 'assistant', 1, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms - 500,
                launch_ms - 400,
                json!({
                    "time": {"created": launch_ms - 500, "completed": launch_ms - 400},
                    "finish": "error",
                    "error": {"type": "aborted", "message": "Step interrupted"}
                })
                .to_string()
            ],
        )
        .unwrap();

        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "msg".into());
        let mut watcher = Watcher::with_db(
            receiver,
            Some("ses_pre".into()),
            Some(db_path.clone()),
            launch_ms,
        );
        let drain = |watcher: &mut Watcher| {
            let mut events = Vec::new();
            watcher
                .drain_events(
                    0,
                    &mut |ev| {
                        events.push(ev);
                        Default::default()
                    },
                    &mut |_| {},
                )
                .unwrap();
            events
        };
        assert!(drain(&mut watcher).is_empty());

        // 1. First-turn permission Reject: session_v2.time_idle stays NULL while
        // session_message records assistant finish=error error.type=aborted.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_rej', ?1, ?1, NULL, NULL)",
            [launch_ms + 10],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":1,"session_id":"ses_rej"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('m_1', 'ses_rej', 'assistant', 1, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 10,
                launch_ms + 50,
                json!({
                    "time": {"created": launch_ms + 10, "completed": launch_ms + 50},
                    "finish": "error",
                    "error": {"type": "aborted", "message": "Step interrupted"}
                })
                .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_rej".into(),
                ended_at_ms: launch_ms + 50,
                superseded: false,
            }]
        );
        assert!(drain(&mut watcher).is_empty());

        // 2. Subsequent turn after a completed turn is rejected without updating time_idle;
        // stale time_idle < time_busy still triggers session_message lookup.
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1, time_idle = ?2, idle_outcome = 'succeeded' WHERE id = 'ses_rej'",
            rusqlite::params![launch_ms + 100, launch_ms + 120],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('m_2', 'ses_rej', 'idle', 2, ?1, ?1, '{\"outcome\":\"succeeded\"}')",
            [launch_ms + 120],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionFinished {
                session_id: "ses_rej".into(),
                ended_at_ms: launch_ms + 120,
                superseded: false,
            }]
        );
        // Same-millisecond new turn start followed by non-aborted assistant error emits Failed.
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1 WHERE id = 'ses_rej'",
            [launch_ms + 120],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('m_3', 'ses_rej', 'assistant', 3, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 120,
                launch_ms + 180,
                json!({
                    "time": {"created": launch_ms + 120, "completed": launch_ms + 180},
                    "finish": "error",
                    "error": {"type": "provider", "message": "upstream 500"}
                })
                .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_rej".into(),
                ended_at_ms: launch_ms + 180,
                superseded: false,
            }]
        );

        // 3. Double-Esc race: assistant error polled first, then session_v2.time_idle lands
        // 1ms later (and even if Turn 2 already started before the second poll!), Interrupted
        // must be emitted ONLY ONCE and offscreen Turn 2 stays tracked.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_esc', ?1, ?1, NULL, NULL)",
            [launch_ms + 200],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":2,"session_id":"ses_esc"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('e_1', 'ses_esc', 'assistant', 10, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 200,
                launch_ms + 250,
                json!({
                    "time": {"created": launch_ms + 200, "completed": launch_ms + 250},
                    "finish": "error",
                    "error": {"type": "aborted", "message": "Step interrupted"}
                })
                .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_esc".into(),
                ended_at_ms: launch_ms + 250,
                superseded: false,
            }]
        );
        // Move ses_esc offscreen, land Turn 1's idle row 1ms later, AND start Turn 2 before polling.
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":3,"session_id":""}))
                .unwrap(),
        );
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('e_2', 'ses_esc', 'idle', 11, ?1, ?1, '{\"outcome\":\"interrupted\"}')",
            [launch_ms + 251],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'interrupted', time_busy = ?2 WHERE id = 'ses_esc'",
            rusqlite::params![launch_ms + 251, launch_ms + 280],
        )
        .unwrap();
        let after_double_esc = drain(&mut watcher);
        assert_eq!(
            after_double_esc,
            vec![AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            }],
            "delayed session_v2.time_idle from the same double-Esc turn must not emit a duplicate Interrupted"
        );
        assert_eq!(
            watcher.polled_session_count(),
            1,
            "superseded offscreen session must remain tracked for Turn 2"
        );

        // 4. Reject superseded by an already-streaming Turn 2 still emits Interrupted(superseded=true)
        // and does not retire offscreen while Turn 2 is active.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_sup', ?1, ?2, NULL, NULL)",
            rusqlite::params![launch_ms + 300, launch_ms + 360],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('s_1', 'ses_sup', 'assistant', 1, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 300,
                launch_ms + 340,
                json!({
                    "time": {"created": launch_ms + 300, "completed": launch_ms + 340},
                    "finish": "error",
                    "error": {"type": "aborted", "message": "Step interrupted"}
                })
                .to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('s_2', 'ses_sup', 'assistant', 2, ?1, ?1, '{\"time\":{\"created\":2000360}}')",
            [launch_ms + 360],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":4,"session_id":"ses_sup"}))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: false,
                },
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_sup".into(),
                    ended_at_ms: launch_ms + 340,
                    superseded: true,
                },
            ]
        );

        // 5. Two same-outcome failed turns inside 5 seconds with distinct idle rows:
        // after seeing Turn 1's assistant error (seq 10), Turn 1 writes idle (seq 11) and
        // Turn 2 writes assistant (seq 12) + idle (seq 13) before the next poll. Comparing
        // the latest idle seq (13) against the idle immediately after seq 10 (11) proves
        // Turn 2 is distinct, so Turn 2's Failed emits once instead of being deduplicated.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_two_fail', ?1, ?1, NULL, NULL)",
            [launch_ms + 900],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":5,"session_id":"ses_two_fail"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher);
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('tf_10', 'ses_two_fail', 'assistant', 10, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 900,
                launch_ms + 1_000,
                json!({
                    "time": {"created": launch_ms + 900, "completed": launch_ms + 1_000},
                    "finish": "error",
                    "error": {"type": "provider", "message": "err 1"}
                })
                .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_two_fail".into(),
                ended_at_ms: launch_ms + 1_000,
                superseded: false,
            }]
        );
        // Turn 1 settles seq 11 at +1001, then Turn 2 fails with seq 12/13 at +3000/+3001
        // (well inside 5 seconds) before the next poll.
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('tf_11', 'ses_two_fail', 'idle', 11, ?1, ?1, '{\"outcome\":\"failed\"}')",
            [launch_ms + 1_001],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('tf_12', 'ses_two_fail', 'assistant', 12, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 2_900,
                launch_ms + 3_000,
                json!({
                    "time": {"created": launch_ms + 2_900, "completed": launch_ms + 3_000},
                    "finish": "error",
                    "error": {"type": "provider", "message": "err 2"}
                })
                .to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('tf_13', 'ses_two_fail', 'idle', 13, ?1, ?1, '{\"outcome\":\"failed\"}')",
            [launch_ms + 3_001],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1, time_idle = ?2, idle_outcome = 'failed' WHERE id = 'ses_two_fail'",
            rusqlite::params![launch_ms + 2_900, launch_ms + 3_001],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_two_fail".into(),
                ended_at_ms: launch_ms + 3_001,
                superseded: false,
            }],
            "second same-outcome turn with distinct idle seq must emit once"
        );
        assert!(drain(&mut watcher).is_empty());

        // 6. Unseen earlier session_v2 failure (Turn 1 @ +100) + newer assistant-only
        // permission Reject (Turn 2 @ +300, time_idle still +100) seen on the same poll:
        // must emit oldest-first [Failed@+100(superseded=true), Interrupted@+300(superseded=false)],
        // and the next poll must emit neither.
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_both', ?1, ?1, ?2, 'succeeded')",
            rusqlite::params![launch_ms + 40, launch_ms + 50],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('b_0', 'ses_both', 'idle', 1, ?1, ?1, '{\"outcome\":\"succeeded\"}')",
            [launch_ms + 50],
        )
        .unwrap();
        drop(
            route
                .admit(json!({"hook_event_name":"snapshot","epoch":1,"sequence":6,"session_id":"ses_both"}))
                .unwrap(),
        );
        let _ = drain(&mut watcher); // watermark now launch_ms + 50
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed', time_busy = ?2 WHERE id = 'ses_both'",
            rusqlite::params![launch_ms + 100, launch_ms + 200],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('b_1', 'ses_both', 'idle', 2, ?1, ?1, '{\"outcome\":\"failed\"}')",
            [launch_ms + 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('b_2', 'ses_both', 'assistant', 3, ?1, ?2, ?3)",
            rusqlite::params![
                launch_ms + 200,
                launch_ms + 300,
                json!({
                    "time": {"created": launch_ms + 200, "completed": launch_ms + 300},
                    "finish": "error",
                    "error": {"type": "aborted", "message": "Step interrupted"}
                })
                .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_both".into(),
                    ended_at_ms: launch_ms + 100,
                    superseded: true,
                },
                AgentEvent::ProgramSessionOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    session_id: "ses_both".into(),
                    ended_at_ms: launch_ms + 300,
                    superseded: false,
                },
            ],
            "unseen earlier session_v2 failure and newer assistant Reject must emit oldest-first"
        );
        assert!(
            drain(&mut watcher).is_empty(),
            "next poll after combined emission must emit neither outcome"
        );
        route.retire();
    }
}

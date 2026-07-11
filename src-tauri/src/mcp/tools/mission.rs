use std::collections::{BTreeMap, HashMap};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use runner_core::event_log::{self, LogEntry, SkipReport};
use runner_core::model::{Event, EventKind::Signal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::commands::{crew, mission, session};
use crate::error::Result;
use crate::mcp::server::RunnerMcpHandler;
use crate::model::{Crew, Mission, SessionStatus, Timestamp};

const DEFAULT_FEED_LIMIT: usize = 50;
const MAX_FEED_LIMIT: usize = 500;
const STATUS_WARNING_LIMIT: usize = 5;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MissionIdArgs {
    /// Mission ID.
    pub id: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct MissionListArgs {
    /// Optional crew ID filter.
    #[serde(default)]
    pub crew_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MissionPinArgs {
    /// Mission ID.
    pub id: String,
    /// True pins the mission; false unpins it.
    pub pinned: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MissionRenameArgs {
    /// Mission ID.
    pub id: String,
    /// New mission title. The backend trims and rejects empty values.
    pub title: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MissionFeedOrder {
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct MissionFeedArgs {
    /// Mission ID.
    pub mission_id: String,
    /// Maximum number of events to return. Defaults to 50 and is capped at 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Sort order for returned events.
    #[serde(default)]
    pub order: MissionFeedOrder,
    /// Optional byte offset into events.ndjson. Use a returned next_offset as the next cursor.
    #[serde(default)]
    pub since_offset: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct MissionFeed {
    pub mission_id: String,
    pub events: Vec<MissionFeedEntry>,
    pub next_offset: Option<u64>,
    pub skipped: Vec<SkippedEventLine>,
}

#[derive(Debug, Serialize)]
pub struct MissionFeedEntry {
    pub next_offset: u64,
    pub event: Event,
}

#[derive(Debug, Serialize)]
pub struct SkippedEventLine {
    pub offset: u64,
    pub next_offset: u64,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct MissionStatusSnapshot {
    pub mission: Mission,
    pub crew: Crew,
    pub sessions: Vec<session::SessionRow>,
    pub latest_runner_status_by_handle: BTreeMap<String, RunnerStatusSnapshot>,
    pub pending_asks: Vec<PendingAskSnapshot>,
    pub pending_ask_count: usize,
    pub live_session_count: usize,
    pub stopped_session_count: usize,
    pub crashed_session_count: usize,
    pub recent_warnings: Vec<MissionWarningSnapshot>,
    pub last_event_id: Option<String>,
    pub last_event_offset: Option<u64>,
    pub skipped_event_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerStatusSnapshot {
    pub state: String,
    pub event_id: String,
    pub ts: Timestamp,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingAskSnapshot {
    pub question_id: String,
    pub asker: String,
    pub prompt: String,
    pub choices: Option<serde_json::Value>,
    pub on_behalf_of: Option<String>,
    pub event_id: String,
    pub ts: Timestamp,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissionWarningSnapshot {
    pub event_id: String,
    pub ts: Timestamp,
    pub from: String,
    pub message: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Default)]
struct EventProjection {
    latest_runner_status_by_handle: BTreeMap<String, RunnerStatusSnapshot>,
    pending_asks: BTreeMap<String, PendingAskSnapshot>,
    recent_warnings: Vec<MissionWarningSnapshot>,
}

impl EventProjection {
    fn from_entries(entries: &[LogEntry]) -> Self {
        let mut projection = Self::default();
        let mut ask_human_asker: HashMap<String, String> = HashMap::new();

        for entry in entries {
            let event = &entry.event;
            if !matches!(event.kind, Signal) {
                continue;
            }
            let Some(signal_type) = event.signal_type.as_ref() else {
                continue;
            };
            match signal_type.as_str() {
                "ask_human" => {
                    ask_human_asker.insert(event.id.clone(), event.from.clone());
                }
                "human_question" => {
                    let triggered_by = event.payload.get("triggered_by").and_then(|v| v.as_str());
                    let asker = triggered_by
                        .and_then(|ask_id| ask_human_asker.remove(ask_id))
                        .or_else(|| {
                            event
                                .payload
                                .get("on_behalf_of")
                                .and_then(|v| v.as_str())
                                .map(ToOwned::to_owned)
                        })
                        .unwrap_or_else(|| event.from.clone());
                    projection.pending_asks.insert(
                        event.id.clone(),
                        PendingAskSnapshot {
                            question_id: event.id.clone(),
                            asker,
                            prompt: event
                                .payload
                                .get("prompt")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            choices: event.payload.get("choices").cloned(),
                            on_behalf_of: event
                                .payload
                                .get("on_behalf_of")
                                .and_then(|v| v.as_str())
                                .map(ToOwned::to_owned),
                            event_id: event.id.clone(),
                            ts: event.ts,
                        },
                    );
                }
                "human_response" => {
                    if let Some(question_id) =
                        event.payload.get("question_id").and_then(|v| v.as_str())
                    {
                        projection.pending_asks.remove(question_id);
                    }
                }
                "runner_status" => {
                    if let Some(state) = event.payload.get("state").and_then(|v| v.as_str()) {
                        if matches!(state, "busy" | "idle") {
                            projection.latest_runner_status_by_handle.insert(
                                event.from.clone(),
                                RunnerStatusSnapshot {
                                    state: state.to_string(),
                                    event_id: event.id.clone(),
                                    ts: event.ts,
                                    source: event
                                        .payload
                                        .get("source")
                                        .and_then(|v| v.as_str())
                                        .map(ToOwned::to_owned),
                                },
                            );
                        }
                    }
                }
                "mission_warning" => {
                    projection.recent_warnings.push(MissionWarningSnapshot {
                        event_id: event.id.clone(),
                        ts: event.ts,
                        from: event.from.clone(),
                        message: event
                            .payload
                            .get("message")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned),
                        payload: event.payload.clone(),
                    });
                    if projection.recent_warnings.len() > STATUS_WARNING_LIMIT {
                        projection.recent_warnings.remove(0);
                    }
                }
                _ => {}
            }
        }

        projection
    }
}

fn mcp_error(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn mission_feed_from_entries(
    mission_id: String,
    entries: Vec<LogEntry>,
    skipped: Vec<SkipReport>,
    order: MissionFeedOrder,
    limit: usize,
) -> MissionFeed {
    let (page_entries, consumed_skips): (Vec<&LogEntry>, Vec<&SkipReport>) = match order {
        MissionFeedOrder::OldestFirst => {
            let page_entries: Vec<&LogEntry> = entries.iter().take(limit).collect();
            let next_unreturned_entry = entries.get(limit);
            let consumed_skips: Vec<&SkipReport> = match next_unreturned_entry {
                Some(next) => skipped
                    .iter()
                    .filter(|skip| skip.offset < next.next_offset)
                    .collect(),
                None => skipped.iter().collect(),
            };
            (page_entries, consumed_skips)
        }
        MissionFeedOrder::NewestFirst => (
            entries.iter().rev().take(limit).collect(),
            skipped.iter().collect(),
        ),
    };
    let max_skip_next = consumed_skips
        .iter()
        .map(|skip| skip.next_offset)
        .max()
        .unwrap_or(0);
    let max_entry_next = page_entries
        .iter()
        .map(|entry| entry.next_offset)
        .max()
        .unwrap_or(0);
    let max_next = max_skip_next.max(max_entry_next);
    let next_offset = (max_next > 0).then_some(max_next);
    let events: Vec<MissionFeedEntry> = page_entries
        .into_iter()
        .map(|entry| MissionFeedEntry {
            next_offset: entry.next_offset,
            event: entry.event.clone(),
        })
        .collect();
    let skipped: Vec<SkippedEventLine> = consumed_skips
        .into_iter()
        .map(|skip| SkippedEventLine {
            offset: skip.offset,
            next_offset: skip.next_offset,
            error: skip.error.clone(),
        })
        .collect();

    MissionFeed {
        mission_id,
        events,
        next_offset,
        skipped,
    }
}

fn read_log_entries(
    app_data_dir: &std::path::Path,
    conn: &rusqlite::Connection,
    mission_id: &str,
    offset: u64,
) -> Result<(Mission, Vec<LogEntry>, Vec<SkipReport>)> {
    let mission = mission::get(conn, mission_id)?;
    let mission_dir = event_log::mission_dir(app_data_dir, &mission.crew_id, mission_id);
    let log = event_log::EventLog::open(&mission_dir)?;
    let (entries, skipped) = log.read_from_lossy(offset)?;
    Ok((mission, entries, skipped))
}

fn emit_mission_changed(handler: &RunnerMcpHandler) {
    handler.state.app_handle.emit("mission/changed", ()).ok();
}

#[tool_router(router = mission_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List non-archived missions, optionally filtered by crew.")]
    pub async fn mission_list(
        &self,
        Parameters(args): Parameters<MissionListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.state.db.get().map_err(mcp_error)?;
        let missions = mission::list(&conn, args.crew_id.as_deref()).map_err(mcp_error)?;
        Ok(CallToolResult::success(vec![Content::json(&missions)?]))
    }

    #[tool(description = "Fetch one mission by ID, including archived missions.")]
    pub async fn mission_get(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.state.db.get().map_err(mcp_error)?;
        let mission = mission::get(&conn, &id).map_err(mcp_error)?;
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(
        description = "Return sidebar-style mission summaries with crew name, pending asks, live-session flag, and activity."
    )]
    pub async fn mission_list_summary(
        &self,
        Parameters(args): Parameters<MissionListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let summaries = mission::mission_list_summary_impl(&app_state, args.crew_id)
            .await
            .map_err(mcp_error)?;
        Ok(CallToolResult::success(vec![Content::json(&summaries)?]))
    }

    #[tool(description = "Read mission events from the NDJSON feed with optional offset cursor.")]
    pub async fn mission_feed(
        &self,
        Parameters(args): Parameters<MissionFeedArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.state.db.get().map_err(mcp_error)?;
        let (_mission, entries, skipped) = read_log_entries(
            &self.state.app_data_dir,
            &conn,
            &args.mission_id,
            args.since_offset.unwrap_or(0),
        )
        .map_err(mcp_error)?;
        let limit = args.limit.unwrap_or(DEFAULT_FEED_LIMIT).min(MAX_FEED_LIMIT);
        let feed = mission_feed_from_entries(args.mission_id, entries, skipped, args.order, limit);
        Ok(CallToolResult::success(vec![Content::json(&feed)?]))
    }

    #[tool(
        description = "Return an operational snapshot: mission, crew, sessions, latest runner status, pending asks, live counts, and recent warnings."
    )]
    pub async fn mission_status(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.state.db.get().map_err(mcp_error)?;
        let (mission, entries, skipped) =
            read_log_entries(&self.state.app_data_dir, &conn, &id, 0).map_err(mcp_error)?;
        let crew = crew::get(&conn, &mission.crew_id).map_err(mcp_error)?;
        let sessions = session::list_for_mission(&conn, &mission.id).map_err(mcp_error)?;
        let projection = EventProjection::from_entries(&entries);
        let live_session_count = sessions
            .iter()
            .filter(|s| matches!(s.session.status, SessionStatus::Running))
            .count();
        let stopped_session_count = sessions
            .iter()
            .filter(|s| matches!(s.session.status, SessionStatus::Stopped))
            .count();
        let crashed_session_count = sessions
            .iter()
            .filter(|s| matches!(s.session.status, SessionStatus::Crashed))
            .count();
        let last_event_id = entries.last().map(|entry| entry.event.id.clone());
        let last_event_offset = entries.last().map(|entry| entry.next_offset);
        let snapshot = MissionStatusSnapshot {
            mission,
            crew,
            sessions,
            latest_runner_status_by_handle: projection.latest_runner_status_by_handle,
            pending_ask_count: projection.pending_asks.len(),
            pending_asks: projection.pending_asks.into_values().collect(),
            live_session_count,
            stopped_session_count,
            crashed_session_count,
            recent_warnings: projection.recent_warnings,
            last_event_id,
            last_event_offset,
            skipped_event_count: skipped.len(),
        };
        Ok(CallToolResult::success(vec![Content::json(&snapshot)?]))
    }

    #[tool(description = "Start a mission for a crew using the same semantics as the app.")]
    pub async fn mission_start(
        &self,
        Parameters(input): Parameters<mission::StartMissionInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let out = mission::mission_start_impl(&app_state, &self.state.app_handle, input)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&out)?]))
    }

    #[tool(description = "Stop a running mission's live sessions without archiving it.")]
    pub async fn mission_stop(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_stop_impl(&app_state, id)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(
        description = "Archive a mission, stopping live sessions and hiding it from active lists."
    )]
    pub async fn mission_archive(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_archive_impl(&app_state, id)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(
        description = "Unarchive a mission: clear the archive marker so it reappears in active lists. Status stays completed."
    )]
    pub async fn mission_unarchive(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_unarchive_impl(&app_state, id)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(description = "Pin or unpin a mission.")]
    pub async fn mission_pin(
        &self,
        Parameters(MissionPinArgs { id, pinned }): Parameters<MissionPinArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_pin_impl(&app_state, id, pinned)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(description = "Rename a mission.")]
    pub async fn mission_rename(
        &self,
        Parameters(MissionRenameArgs { id, title }): Parameters<MissionRenameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_rename_impl(&app_state, id, title)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(description = "Reset and restart a mission using the same guarded behavior as the UI.")]
    pub async fn mission_reset(
        &self,
        Parameters(MissionIdArgs { id }): Parameters<MissionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let mission = mission::mission_reset_impl(&app_state, &self.state.app_handle, id)
            .await
            .map_err(mcp_error)?;
        emit_mission_changed(self);
        Ok(CallToolResult::success(vec![Content::json(&mission)?]))
    }

    #[tool(description = "Post a human-originated signal into a mission feed.")]
    pub async fn mission_post_human_signal(
        &self,
        Parameters(input): Parameters<mission::PostHumanSignalInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let app_state = self.state.app_state();
        let event = mission::mission_post_human_signal_impl(&app_state, input)
            .await
            .map_err(mcp_error)?;
        Ok(CallToolResult::success(vec![Content::json(&event)?]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use runner_core::event_log::EventLog;
    use runner_core::model::{EventDraft, SignalType};
    use rusqlite::params;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn signal(from: &str, ty: &str, payload: serde_json::Value) -> EventDraft {
        EventDraft::signal("crew", "mission", from, SignalType::new(ty), payload)
    }

    #[test]
    fn event_projection_tracks_status_pending_asks_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        log.append(signal(
            "coder",
            "runner_status",
            serde_json::json!({ "state": "busy" }),
        ))
        .unwrap();
        log.append(signal(
            "coder",
            "runner_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
        let ask = log
            .append(signal(
                "reviewer",
                "ask_human",
                serde_json::json!({ "prompt": "ship?" }),
            ))
            .unwrap();
        let question = log
            .append(signal(
                "router",
                "human_question",
                serde_json::json!({
                    "triggered_by": ask.id,
                    "prompt": "ship?",
                    "choices": ["yes", "no"],
                    "on_behalf_of": "reviewer"
                }),
            ))
            .unwrap();
        log.append(signal(
            "router",
            "mission_warning",
            serde_json::json!({ "message": "careful" }),
        ))
        .unwrap();
        let (entries, _) = log.read_from_lossy(0).unwrap();

        let projection = EventProjection::from_entries(&entries);

        assert_eq!(
            projection
                .latest_runner_status_by_handle
                .get("coder")
                .unwrap()
                .state,
            "idle"
        );
        assert_eq!(
            projection.pending_asks.get(&question.id).unwrap().asker,
            "reviewer"
        );
        assert_eq!(projection.recent_warnings.len(), 1);
        assert_eq!(
            projection.recent_warnings[0].message.as_deref(),
            Some("careful")
        );
    }

    #[test]
    fn event_projection_removes_answered_pending_asks() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        let ask = log
            .append(signal("reviewer", "ask_human", serde_json::json!({})))
            .unwrap();
        let question = log
            .append(signal(
                "router",
                "human_question",
                serde_json::json!({ "triggered_by": ask.id, "prompt": "ship?" }),
            ))
            .unwrap();
        log.append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": question.id, "choice": "yes" }),
        ))
        .unwrap();
        let (entries, _) = log.read_from_lossy(0).unwrap();

        let projection = EventProjection::from_entries(&entries);

        assert!(projection.pending_asks.is_empty());
    }

    #[test]
    fn mission_feed_oldest_first_matches_read_events_order() {
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew', 'Crew', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission', 'crew', 'Mission', 'running', ?1)",
            params![now],
        )
        .unwrap();
        let mission_dir = event_log::mission_dir(app_data.path(), "crew", "mission");
        let log = EventLog::open(&mission_dir).unwrap();
        log.append(signal(
            "system",
            "mission_start",
            serde_json::json!({ "title": "Mission" }),
        ))
        .unwrap();
        log.append(signal(
            "human",
            "mission_goal",
            serde_json::json!({ "text": "ship" }),
        ))
        .unwrap();

        let expected_ids: Vec<String> = mission::read_events(app_data.path(), &conn, "mission")
            .unwrap()
            .into_iter()
            .map(|event| event.id)
            .collect();
        let (_mission, entries, skipped) =
            read_log_entries(app_data.path(), &conn, "mission", 0).unwrap();
        let feed = mission_feed_from_entries(
            "mission".into(),
            entries,
            skipped,
            MissionFeedOrder::OldestFirst,
            10,
        );
        let feed_ids: Vec<String> = feed
            .events
            .into_iter()
            .map(|entry| entry.event.id)
            .collect();

        assert_eq!(feed_ids, expected_ids);
    }

    #[test]
    fn mission_feed_oldest_first_limit_cursor_pages_without_skipping_events() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        let first = log
            .append(signal("system", "mission_start", serde_json::json!({})))
            .unwrap();
        let second = log
            .append(signal("human", "mission_goal", serde_json::json!({})))
            .unwrap();
        let third = log
            .append(signal(
                "coder",
                "runner_status",
                serde_json::json!({ "state": "idle" }),
            ))
            .unwrap();
        let (entries, skipped) = log.read_from_lossy(0).unwrap();

        let feed = mission_feed_from_entries(
            "mission".into(),
            entries,
            skipped,
            MissionFeedOrder::OldestFirst,
            2,
        );

        let ids: Vec<String> = feed
            .events
            .iter()
            .map(|entry| entry.event.id.clone())
            .collect();
        assert_eq!(ids, vec![first.id, second.id]);
        assert_eq!(feed.next_offset, Some(feed.events[1].next_offset));

        let (next_entries, _) = log.read_from_lossy(feed.next_offset.unwrap()).unwrap();
        let next_ids: Vec<String> = next_entries
            .into_iter()
            .map(|entry| entry.event.id)
            .collect();
        assert_eq!(next_ids, vec![third.id]);
    }

    #[test]
    fn mission_feed_skip_only_advances_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        OpenOptions::new()
            .append(true)
            .open(log.path())
            .unwrap()
            .write_all(b"{bad json}\n")
            .unwrap();
        let (entries, skipped) = log.read_from_lossy(0).unwrap();
        let expected_next = skipped[0].next_offset;

        let feed = mission_feed_from_entries(
            "mission".into(),
            entries,
            skipped,
            MissionFeedOrder::OldestFirst,
            10,
        );

        assert!(feed.events.is_empty());
        assert_eq!(feed.skipped.len(), 1);
        assert_eq!(feed.next_offset, Some(expected_next));
    }

    #[test]
    fn mission_feed_trailing_skip_advances_past_returned_event() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        log.append(signal("system", "mission_start", serde_json::json!({})))
            .unwrap();
        OpenOptions::new()
            .append(true)
            .open(log.path())
            .unwrap()
            .write_all(b"{bad json}\n")
            .unwrap();
        let (entries, skipped) = log.read_from_lossy(0).unwrap();
        let expected_next = skipped[0].next_offset;

        let feed = mission_feed_from_entries(
            "mission".into(),
            entries,
            skipped,
            MissionFeedOrder::OldestFirst,
            1,
        );

        assert_eq!(feed.events.len(), 1);
        assert_eq!(feed.skipped.len(), 1);
        assert!(expected_next > feed.events[0].next_offset);
        assert_eq!(feed.next_offset, Some(expected_next));
    }
}

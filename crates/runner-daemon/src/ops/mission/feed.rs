use crate::error::Result;
use crate::model::SessionStatus;
use crate::ops::{crew, mission, session};
use runner_core::event_log::{self, LogEntry, SkipReport};
use runner_core::model::EventKind::Signal;
use runner_core::protocol::*;
use std::collections::{BTreeMap, HashMap};
const DEFAULT_FEED_LIMIT: usize = 50;
const MAX_FEED_LIMIT: usize = 500;
const STATUS_WARNING_LIMIT: usize = 5;
#[derive(Default)]
struct EventProjection {
    latest_session_status_by_handle: BTreeMap<String, SessionStatusSnapshot>,
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
                "session_status" | "runner_status" => {
                    if let Some(state) = event.payload.get("state").and_then(|v| v.as_str()) {
                        if matches!(state, "busy" | "idle") {
                            projection.latest_session_status_by_handle.insert(
                                event.from.clone(),
                                SessionStatusSnapshot {
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

pub fn mission_feed(state: &crate::AppCore, args: MissionFeedArgs) -> Result<MissionFeed> {
    let conn = state.db.get()?;
    let (_, entries, skipped) = read_log_entries(
        &state.app_data_dir,
        &conn,
        &args.mission_id,
        args.since_offset.unwrap_or(0),
    )?;
    Ok(mission_feed_from_entries(
        args.mission_id,
        entries,
        skipped,
        args.order,
        args.limit.unwrap_or(DEFAULT_FEED_LIMIT).min(MAX_FEED_LIMIT),
    ))
}

pub fn mission_status(state: &crate::AppCore, id: &str) -> Result<MissionStatusSnapshot> {
    let conn = state.db.get()?;
    let (mission, entries, skipped) = read_log_entries(&state.app_data_dir, &conn, id, 0)?;
    let crew = crew::get(&conn, &mission.crew_id)?;
    let sessions = session::list_for_mission(&conn, &mission.id)?;
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
        latest_session_status_by_handle: projection.latest_session_status_by_handle,
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
    Ok(snapshot)
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
    fn event_projection_reads_legacy_runner_status_rows() {
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
            "session_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
        log.append(signal(
            "reviewer",
            "runner_status",
            serde_json::json!({ "state": "busy", "source": "forwarder" }),
        ))
        .unwrap();
        let (entries, _) = log.read_from_lossy(0).unwrap();

        let projection = EventProjection::from_entries(&entries);

        let coder = &projection.latest_session_status_by_handle["coder"];
        assert_eq!(coder.state, "idle");
        let reviewer = &projection.latest_session_status_by_handle["reviewer"];
        assert_eq!(reviewer.state, "busy");
        assert_eq!(reviewer.source.as_deref(), Some("forwarder"));
    }

    #[test]
    fn event_projection_tracks_status_pending_asks_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path()).unwrap();
        log.append(signal(
            "coder",
            "session_status",
            serde_json::json!({ "state": "busy" }),
        ))
        .unwrap();
        log.append(signal(
            "coder",
            "session_status",
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
                .latest_session_status_by_handle
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
                "session_status",
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

use super::core::{failed, DaemonCore, Failure, Result};
use runner_core::{
    companion::*,
    model::{Event, EventKind},
    protocol as p,
};

pub const MAX_BATCH_BYTES: usize = 64 * 1024;
// Leave room for the result/push envelope and cursor.
const ENTRY_BUDGET: usize = MAX_BATCH_BYTES - 1024;

pub struct Updates {
    pub entries: Vec<FeedEntry>,
    pub cursor: u64,
    pub more: bool,
}
fn entry(event: Event) -> Result<Option<(FeedEntry, usize)>> {
    if event.kind == EventKind::Signal
        && matches!(
            event.signal_type.as_ref().map(|s| s.as_str()),
            Some("session_status" | "runner_status" | "inbox_read" | "ask_human")
        )
        || event.kind == EventKind::Message && event.payload.get("question_id").is_some()
    {
        return Ok(None);
    }
    let content = match event.kind {
        EventKind::Message => FeedContent::Message {
            text: event
                .payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into(),
        },
        EventKind::Signal => match event.signal_type.as_ref().map(|s| s.as_str()) {
            Some("human_question") => FeedContent::Ask {
                question: event.id.clone(),
                prompt: event.payload["prompt"].as_str().unwrap_or_default().into(),
                choices: serde_json::from_value(event.payload["choices"].clone()).unwrap_or(None),
            },
            Some("human_response") => FeedContent::Answer {
                question: event.payload["question_id"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                text: event.payload["choice"].as_str().unwrap_or_default().into(),
            },
            _ => FeedContent::Signal {
                signal: event.signal_type.map(|s| s.0).unwrap_or_default(),
                payload: event.payload,
            },
        },
    };
    let mut entry = FeedEntry {
        id: event.id,
        ts: event.ts.to_rfc3339(),
        from: event.from,
        to: event.to.as_slice().to_vec(),
        content,
    };
    let mut size = serde_json::to_vec(&entry).map_err(failed)?.len() + 1;
    if size > ENTRY_BUDGET {
        entry.from = "system".into();
        entry.to.clear();
        entry.content = FeedContent::Signal {
            signal: "feedEntryTooLarge".into(),
            payload: serde_json::json!({"message":"This feed entry is too large to send. View it on the host."}),
        };
        size = serde_json::to_vec(&entry).map_err(failed)?.len() + 1;
    }
    Ok(Some((entry, size)))
}
impl DaemonCore {
    pub(super) fn feed_cursor(&self, mission: &str) -> Result<u64> {
        Ok(
            crate::ops::mission::feed::mission_status(self.core()?.as_ref(), mission)
                .map_err(failed)?
                .last_event_offset
                .unwrap_or(0),
        )
    }
    pub(super) fn updates(&self, mission: &str, mut cursor: u64) -> Result<Updates> {
        let core = self.core()?;
        let page = crate::ops::mission::feed::mission_feed(
            &core,
            p::MissionFeedArgs {
                mission_id: mission.into(),
                since_offset: Some(cursor),
                limit: Some(500),
                order: p::MissionFeedOrder::OldestFirst,
            },
        )
        .map_err(failed)?;
        let more = page.events.len() == 500;
        let mut entries = vec![];
        let mut bytes = 0;
        for row in page.events {
            if let Some((entry, size)) = entry(row.event)? {
                if bytes + size > ENTRY_BUDGET {
                    return Ok(Updates {
                        entries,
                        cursor,
                        more: true,
                    });
                }
                bytes += size;
                entries.push(entry);
            }
            cursor = row.next_offset;
        }
        Ok(Updates {
            entries,
            cursor: page.next_offset.unwrap_or(cursor),
            more,
        })
    }
    pub(super) fn feed_page(&self, request: FeedRequest) -> Result<FeedPage> {
        let core = self.core()?;
        let mission = {
            let conn = core.db.get().map_err(failed)?;
            crate::ops::mission::get(&conn, &request.mission).map_err(failed)?
        };
        let path = runner_core::event_log::mission_dir(
            &core.app_data_dir,
            mission.crew_id.as_deref(),
            &mission.id,
        );
        let log = runner_core::event_log::EventLog::open(&path).map_err(failed)?;
        let (entries, _) = log.read_from_lossy(0).map_err(failed)?;
        select_page(request, entries.into_iter().map(|row| row.event))
    }
}
fn select_page(request: FeedRequest, events: impl IntoIterator<Item = Event>) -> Result<FeedPage> {
    let mut entries = std::collections::VecDeque::new();
    let mut bytes = 0;
    let mut older = false;
    let mut found = request.before.is_none();
    for event in events {
        if let Some((entry, size)) = entry(event)? {
            if request.before.as_deref() == Some(&entry.id) {
                found = true;
                break;
            }
            bytes += size;
            entries.push_back((entry, size));
            while entries.len() > request.limit.clamp(1, 500) as usize || bytes > ENTRY_BUDGET {
                bytes -= entries.pop_front().unwrap().1;
                older = true;
            }
        }
    }
    if !found {
        return Err(Failure(
            ErrorCode::BadRequest,
            "The feed cursor is no longer available.",
            None,
        ));
    }
    let before = older.then(|| entries.front().unwrap().0.id.clone());
    Ok(FeedPage {
        entries: entries.into_iter().rev().map(|(entry, _)| entry).collect(),
        before,
    })
}

impl DaemonCore {
    pub(super) fn answer_question(&self, request: AnswerRequest) -> Result<()> {
        let core = self.core()?;
        let pending =
            crate::ops::mission::feed::mission_status(&core, &request.mission).map_err(failed)?;
        if !pending
            .pending_asks
            .iter()
            .any(|ask| ask.question_id == request.question)
        {
            return Err(Failure(
                ErrorCode::NotFound,
                "That question is no longer pending.",
                None,
            ));
        }
        crate::daemon::block_on(crate::ops::mission::mission_signal_impl(
            &core,
            p::PostSignalInput {
                mission_id: request.mission,
                from: None,
                signal_type: "human_response".into(),
                payload: serde_json::json!({"question_id":request.question,"choice":request.text}),
            },
        ))
        .map_err(failed)?;
        Ok(())
    }
    pub(super) fn post_message(&self, request: PostRequest) -> Result<()> {
        let core = self.core()?;
        crate::daemon::block_on(crate::ops::mission::mission_post_impl(
            &core,
            p::PostMessageInput {
                mission_id: request.mission,
                from: None,
                text: request.text,
                to: serde_json::from_value(serde_json::json!(request.to)).map_err(failed)?,
            },
        ))
        .map_err(failed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_feed_pages_visit_each_event_once_and_stop_at_the_older_cursor() {
        let visited = std::cell::Cell::new(0);
        let events = || {
            (0..4000)
                .map(|index| Event {
                    id: format!("{index:026}"),
                    ts: chrono::Utc::now(),
                    crew_id: None,
                    mission_id: "m".into(),
                    kind: if index % 2 == 0 {
                        EventKind::Message
                    } else {
                        EventKind::Signal
                    },
                    from: "human".into(),
                    to: Default::default(),
                    signal_type: (index % 2 == 1)
                        .then(|| runner_core::model::SignalType::new("runner_status")),
                    payload: serde_json::json!({"text":"message", "state":"busy"}),
                })
                .inspect(|_| visited.set(visited.get() + 1))
        };
        let newest = select_page(
            FeedRequest {
                id: 1,
                mission: "m".into(),
                before: None,
                limit: 50,
            },
            events(),
        )
        .unwrap();
        assert_eq!(visited.get(), 4000);
        assert_eq!(newest.entries.len(), 50);
        assert_eq!(newest.entries[0].id, format!("{:026}", 3998));
        assert_eq!(
            newest.before.as_deref(),
            Some(format!("{:026}", 3900).as_str())
        );
        visited.set(0);
        let older = select_page(
            FeedRequest {
                id: 2,
                mission: "m".into(),
                before: newest.before,
                limit: 50,
            },
            events(),
        )
        .unwrap();
        assert_eq!(visited.get(), 3901);
        assert_eq!(older.entries.len(), 50);
        assert_eq!(older.entries[0].id, format!("{:026}", 3898));
        assert_eq!(older.entries[49].id, format!("{:026}", 3800));
    }
}

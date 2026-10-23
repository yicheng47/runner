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
        let mut entries = std::collections::VecDeque::new();
        let mut bytes = 0;
        let mut older = false;
        let mut cursor = 0;
        let mut found = request.before.is_none();
        'pages: loop {
            let updates = self.updates(&request.mission, cursor)?;
            for entry in updates.entries {
                if request.before.as_deref() == Some(&entry.id) {
                    found = true;
                    break 'pages;
                }
                let size = serde_json::to_vec(&entry).map_err(failed)?.len() + 1;
                bytes += size;
                entries.push_back((entry, size));
                while entries.len() > request.limit.clamp(1, 500) as usize || bytes > ENTRY_BUDGET {
                    bytes -= entries.pop_front().unwrap().1;
                    older = true;
                }
            }
            cursor = updates.cursor;
            if !updates.more {
                break;
            }
        }
        if !found {
            return Err(Failure(
                ErrorCode::BadRequest,
                "The feed cursor is no longer available.",
            ));
        }
        let before = older.then(|| entries.front().unwrap().0.id.clone());
        Ok(FeedPage {
            entries: entries.into_iter().rev().map(|(entry, _)| entry).collect(),
            before,
        })
    }
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

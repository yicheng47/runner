use std::collections::HashMap;

use chrono::Duration;
use runner_backend::model::{Event, EventKind, SignalType};

#[derive(Clone, Debug)]
pub(crate) enum FeedBlock {
    Divider(Event),
    MessageGroup { author: String, events: Vec<Event> },
    Signal(Event),
    AskCard(Event),
}

impl FeedBlock {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::MessageGroup { events, .. } => &events[0].id,
            Self::Divider(event) | Self::Signal(event) | Self::AskCard(event) => &event.id,
        }
    }
}

pub(crate) struct AskProjection {
    pub(crate) askers_by_question: HashMap<String, String>,
    pub(crate) resolved_asks: HashMap<String, String>,
}

pub(crate) fn project_asks(events: &[Event]) -> AskProjection {
    let mut ask_human_askers = HashMap::new();
    let mut askers_by_question = HashMap::new();
    let mut resolved_asks = HashMap::new();
    for event in events {
        if event.kind != EventKind::Signal {
            continue;
        }
        match event.signal_type.as_ref().map(SignalType::as_str) {
            Some("ask_human") => {
                ask_human_askers.insert(event.id.clone(), event.from.clone());
            }
            Some("human_question") => {
                let triggered_by = event
                    .payload
                    .get("triggered_by")
                    .and_then(serde_json::Value::as_str);
                if let Some(asker) = triggered_by.and_then(|id| ask_human_askers.remove(id)) {
                    askers_by_question.insert(event.id.clone(), asker);
                }
            }
            Some("human_response") => {
                if let Some(question_id) = event
                    .payload
                    .get("question_id")
                    .and_then(serde_json::Value::as_str)
                {
                    let choice = event
                        .payload
                        .get("choice")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    resolved_asks.insert(question_id.to_owned(), choice);
                }
            }
            _ => {}
        }
    }
    AskProjection {
        askers_by_question,
        resolved_asks,
    }
}

fn is_hidden_system_signal(event: &Event) -> bool {
    event.kind == EventKind::Signal
        && matches!(
            event.signal_type.as_ref().map(SignalType::as_str),
            Some("inbox_read" | "runner_status" | "ask_human")
        )
}

fn is_message_like(event: &Event) -> bool {
    event.kind == EventKind::Message
        || (event.kind == EventKind::Signal
            && matches!(
                event.signal_type.as_ref().map(SignalType::as_str),
                Some("human_said" | "human_response" | "mission_goal")
            ))
}

fn is_mission_goal(event: &Event) -> bool {
    event.kind == EventKind::Signal
        && event.signal_type.as_ref().map(SignalType::as_str) == Some("mission_goal")
}

fn grouping_route(event: &Event) -> String {
    if event.kind == EventKind::Message {
        return format!("target:{}", event.to.as_deref().unwrap_or_default());
    }
    match event.signal_type.as_ref().map(SignalType::as_str) {
        Some("human_said") => format!(
            "target:{}",
            event
                .payload
                .get("target")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        ),
        Some("human_response") => format!(
            "question:{}",
            event
                .payload
                .get("question_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        ),
        _ => String::new(),
    }
}

fn can_join_group(author: &str, grouped: &[Event], event: &Event) -> bool {
    if author != event.from || grouped.is_empty() {
        return false;
    }
    let first = &grouped[0];
    if is_mission_goal(first)
        || is_mission_goal(event)
        || grouping_route(first) != grouping_route(event)
    {
        return false;
    }
    let gap = event.ts.signed_duration_since(
        grouped
            .last()
            .expect("non-empty message group checked above")
            .ts,
    );
    gap >= Duration::zero() && gap <= Duration::minutes(5)
}

pub(crate) fn group_feed_blocks(events: &[Event]) -> Vec<FeedBlock> {
    let mut blocks = Vec::new();
    for event in events {
        if is_hidden_system_signal(event) {
            continue;
        }
        if is_message_like(event) {
            if let Some(FeedBlock::MessageGroup { author, events }) = blocks.last_mut() {
                if can_join_group(author, events, event) {
                    events.push(event.clone());
                    continue;
                }
            }
            blocks.push(FeedBlock::MessageGroup {
                author: event.from.clone(),
                events: vec![event.clone()],
            });
            continue;
        }
        match event.signal_type.as_ref().map(SignalType::as_str) {
            Some("mission_start") => blocks.push(FeedBlock::Divider(event.clone())),
            Some("human_question") => blocks.push(FeedBlock::AskCard(event.clone())),
            _ => blocks.push(FeedBlock::Signal(event.clone())),
        }
    }
    blocks
}

pub(crate) fn is_human_authored(event: &Event) -> bool {
    (event.kind == EventKind::Message && event.from == "human")
        || (event.kind == EventKind::Signal
            && matches!(
                event.signal_type.as_ref().map(SignalType::as_str),
                Some("human_said" | "human_response")
            ))
}

pub(crate) fn message_text(event: &Event) -> String {
    let key = if event.signal_type.as_ref().map(SignalType::as_str) == Some("human_response") {
        "choice"
    } else {
        "text"
    };
    event
        .payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn message_target(
    event: &Event,
    askers_by_question: &HashMap<String, String>,
) -> Option<String> {
    if event.kind == EventKind::Message {
        return event.to.clone();
    }
    match event.signal_type.as_ref().map(SignalType::as_str) {
        Some("human_said") | Some("mission_goal") => event
            .payload
            .get("target")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        Some("human_response") => Some(
            event
                .payload
                .get("question_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| askers_by_question.get(id))
                .cloned()
                .unwrap_or_else(|| "?".into()),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, seconds: i64, from: &str, kind: EventKind, signal: Option<&str>) -> Event {
        Event {
            id: id.into(),
            ts: chrono::Utc::now() + Duration::seconds(seconds),
            crew_id: "crew".into(),
            mission_id: "mission".into(),
            kind,
            from: from.into(),
            to: None,
            signal_type: signal.map(SignalType::new),
            payload: serde_json::json!({ "text": id }),
        }
    }

    #[test]
    fn grouping_matches_the_chat_feed_contract() {
        let events = vec![
            event(
                "start",
                0,
                "system",
                EventKind::Signal,
                Some("mission_start"),
            ),
            event("a", 1, "coder", EventKind::Message, None),
            event("b", 299, "coder", EventKind::Message, None),
            event(
                "status",
                300,
                "coder",
                EventKind::Signal,
                Some("runner_status"),
            ),
            event("signal", 301, "coder", EventKind::Signal, Some("ask_lead")),
            event("c", 302, "coder", EventKind::Message, None),
            event("goal", 303, "lead", EventKind::Signal, Some("mission_goal")),
            event("ask", 304, "lead", EventKind::Signal, Some("ask_human")),
            event(
                "question",
                305,
                "human",
                EventKind::Signal,
                Some("human_question"),
            ),
        ];
        let blocks = group_feed_blocks(&events);
        assert_eq!(blocks.len(), 6);
        assert!(matches!(blocks[0], FeedBlock::Divider(_)));
        assert!(matches!(
            &blocks[1],
            FeedBlock::MessageGroup { events, .. } if events.len() == 2
        ));
        assert!(matches!(blocks[2], FeedBlock::Signal(_)));
        assert!(matches!(blocks[3], FeedBlock::MessageGroup { .. }));
        assert!(matches!(blocks[4], FeedBlock::MessageGroup { .. }));
        assert!(matches!(blocks[5], FeedBlock::AskCard(_)));
    }

    #[test]
    fn grouping_breaks_on_route_and_time_window() {
        let mut first = event("a", 0, "coder", EventKind::Message, None);
        first.to = Some("lead".into());
        let mut route = event("b", 1, "coder", EventKind::Message, None);
        route.to = Some("reviewer".into());
        let late = event("c", 302, "coder", EventKind::Message, None);
        assert_eq!(group_feed_blocks(&[first, route, late]).len(), 3);
    }
}

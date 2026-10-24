use std::collections::HashMap;

use chrono::Duration;
use runner_core::protocol::model::{Event, EventKind, SignalType};

#[derive(Clone, Debug)]
pub(crate) enum FeedBlock {
    Divider(Event),
    MessageGroup { author: String, events: Vec<Event> },
    Signal(Event),
    AskCard(Event),
    PendingAsk(crate::surfaces::front_page::model::MissionQuestionDisplay),
}

impl FeedBlock {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::PendingAsk(question) => &question.id,
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
            Some("inbox_read" | "session_status" | "runner_status" | "ask_human")
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
        return format!("target:{}", event.recipients().join(","));
    }
    match event.signal_type.as_ref().map(SignalType::as_str) {
        Some("human_said") => {
            serde_json::from_value::<runner_core::Recipients>(event.payload["target"].clone())
                .map(|recipients| format!("target:{}", recipients.as_slice().join(",")))
                .unwrap_or_else(|_| format!("target:{}", event.payload["target"]))
        }
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
    let relayed: std::collections::HashSet<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::Message)
        .filter_map(|event| event.payload["source_event"].as_str())
        .collect();
    for event in events {
        if event.kind == EventKind::Signal
            && event.signal_type.as_ref().map(SignalType::as_str) == Some("human_said")
            && relayed.contains(event.id.as_str())
        {
            continue;
        }
        append_feed_block(&mut blocks, event);
    }
    blocks
}

/// Returns the first changed block for a tail append without a relay to earlier content.
pub(crate) fn append_feed_block(blocks: &mut Vec<FeedBlock>, event: &Event) -> Option<usize> {
    if is_hidden_system_signal(event)
        || (event.kind == EventKind::Message && event.payload.get("question_id").is_some())
    {
        return None;
    }
    let tail = blocks.len().saturating_sub(1);
    if is_message_like(event) {
        if let Some(FeedBlock::MessageGroup { author, events }) = blocks.last_mut() {
            if can_join_group(author, events, event) {
                events.push(event.clone());
                return Some(tail);
            }
        }
        blocks.push(FeedBlock::MessageGroup {
            author: event.from.clone(),
            events: vec![event.clone()],
        });
        return Some(blocks.len() - 1);
    }
    match event.signal_type.as_ref().map(SignalType::as_str) {
        Some(
            "mission_start" | "mission_stopped" | "mission_reopened" | "role_joined"
            | "role_removed",
        ) => blocks.push(FeedBlock::Divider(event.clone())),
        Some("human_question") => blocks.push(FeedBlock::AskCard(event.clone())),
        _ => blocks.push(FeedBlock::Signal(event.clone())),
    }
    Some(blocks.len() - 1)
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
        return (!event.recipients().is_empty()).then(|| event.recipients().join(", @"));
    }
    match event.signal_type.as_ref().map(SignalType::as_str) {
        Some("human_said") | Some("mission_goal") => {
            let recipients: runner_core::Recipients =
                serde_json::from_value(event.payload["target"].clone()).ok()?;
            (!recipients.is_empty()).then(|| recipients.as_slice().join(", @"))
        }
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
            crew_id: Some("crew".into()),
            mission_id: "mission".into(),
            kind,
            from: from.into(),
            to: Default::default(),
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
                Some("session_status"),
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
    fn mission_stop_and_reopen_are_dividers() {
        for kind in ["mission_stopped", "mission_reopened"] {
            let mut lifecycle = event("lifecycle", 2, "system", EventKind::Signal, Some(kind));
            lifecycle.payload = serde_json::json!({});
            let blocks = group_feed_blocks(&[
                event(
                    "start",
                    0,
                    "system",
                    EventKind::Signal,
                    Some("mission_start"),
                ),
                event("message", 1, "coder", EventKind::Message, None),
                lifecycle,
            ]);
            assert_eq!(blocks.len(), 3);
            assert!(matches!(blocks[0], FeedBlock::Divider(_)));
            assert!(matches!(blocks[1], FeedBlock::MessageGroup { .. }));
            assert!(matches!(
                &blocks[2],
                FeedBlock::Divider(event) if event.id == "lifecycle"
                    && event.signal_type.as_ref().map(SignalType::as_str) == Some(kind)
                    && event.payload == serde_json::json!({})
            ));
        }
    }

    #[test]
    fn several_recipients_render_in_messages_and_legacy_person_signals() {
        let mut message = event("a", 0, "coder", EventKind::Message, None);
        message.to = runner_core::Recipients::new(vec!["reviewer".into(), "qa".into()]);
        assert_eq!(
            message_target(&message, &HashMap::new()).as_deref(),
            Some("reviewer, @qa")
        );
        let mut person = event("b", 0, "human", EventKind::Signal, Some("human_said"));
        person.payload["target"] = serde_json::json!(["reviewer", "qa"]);
        assert_eq!(
            message_target(&person, &HashMap::new()).as_deref(),
            Some("reviewer, @qa")
        );
    }

    #[test]
    fn unconverted_human_said_groups_with_messages_to_the_same_recipients() {
        for target in [
            serde_json::json!(null),
            serde_json::json!("coder"),
            serde_json::json!(["coder", "reviewer"]),
        ] {
            let mut signal = event("signal", 0, "human", EventKind::Signal, Some("human_said"));
            signal.payload["target"] = target.clone();
            let mut message = event("message", 1, "human", EventKind::Message, None);
            message.to = serde_json::from_value(target).unwrap();
            let blocks = group_feed_blocks(&[signal, message]);
            assert_eq!(blocks.len(), 1);
            assert!(matches!(
                &blocks[0],
                FeedBlock::MessageGroup { author, events }
                    if author == "human" && events.iter().map(|event| event.id.as_str()).collect::<Vec<_>>() == ["signal", "message"]
            ));
        }
    }

    #[test]
    fn grouping_breaks_on_route_and_time_window() {
        let mut first = event("a", 0, "coder", EventKind::Message, None);
        first.to = Some("lead".into()).into();
        let mut route = event("b", 1, "coder", EventKind::Message, None);
        route.to = Some("reviewer".into()).into();
        let late = event("c", 302, "coder", EventKind::Message, None);
        assert_eq!(group_feed_blocks(&[first, route, late]).len(), 3);
    }
}

#[cfg(test)]
mod answer_delivery_tests {
    use super::*;
    use runner_core::model::EventDraft;
    fn envelope(draft: EventDraft, id: &str) -> Event {
        Event {
            id: id.into(),
            ts: chrono::Utc::now(),
            crew_id: draft.crew_id,
            mission_id: draft.mission_id,
            kind: draft.kind,
            from: draft.from,
            to: draft.to,
            signal_type: draft.signal_type,
            payload: draft.payload,
        }
    }
    #[test]
    fn answer_messages_hide_behind_the_existing_card_and_legacy_signals_still_render() {
        let question = envelope(
            EventDraft::signal(
                None,
                "m",
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"prompt":"continue?"}),
            ),
            "q",
        );
        let response = envelope(
            EventDraft::signal(
                None,
                "m",
                "human",
                SignalType::new("human_response"),
                serde_json::json!({"question_id":"q","choice":"yes"}),
            ),
            "r",
        );
        let mut answer = envelope(
            EventDraft::message(
                None,
                "m",
                "human",
                Some("worker".into()),
                "Answer to continue: yes",
            ),
            "a",
        );
        answer.payload["question_id"] = "q".into();
        let blocks = group_feed_blocks(&[question.clone(), response.clone(), answer]);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], FeedBlock::AskCard(_)));
        let projection = project_asks(&[question, response]);
        assert_eq!(projection.resolved_asks["q"], "yes");
    }
    #[test]
    fn a_relayed_human_said_is_shown_once_and_legacy_human_said_is_kept() {
        let signal = envelope(
            EventDraft::signal(
                None,
                "m",
                "human",
                SignalType::new("human_said"),
                serde_json::json!({"text":"hello"}),
            ),
            "s",
        );
        assert_eq!(group_feed_blocks(std::slice::from_ref(&signal)).len(), 1);
        let mut message = envelope(EventDraft::message(None, "m", "human", None, "hello"), "m");
        message.payload["source_event"] = "s".into();
        assert_eq!(group_feed_blocks(&[signal, message]).len(), 1);
    }
}

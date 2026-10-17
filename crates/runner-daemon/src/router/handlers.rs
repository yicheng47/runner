use runner_core::model::{Event, EventDraft};

use super::{PendingAsk, Router};
use crate::error::Result;

pub(super) fn mission_goal(_: &Router, _: &Event) {}

pub(super) fn human_said(router: &Router, event: &Event) {
    if let Err(error) = append_human_said(router, event) {
        router.warn(format!("human_said message append failed: {error}"));
    }
}

pub(super) fn append_human_said(router: &Router, event: &Event) -> Result<()> {
    let mut draft = EventDraft::message(
        router.crew_id.as_deref(),
        &router.mission_id,
        "human",
        None,
        event.payload["text"].as_str().unwrap_or_default(),
    );
    draft.to = serde_json::from_value(event.payload["target"].clone())?;
    draft.payload["source_event"] = event.id.clone().into();
    router.log.append(draft)?;
    Ok(())
}

pub(super) fn ask_human(router: &Router, event: &Event) {
    let prompt = event.payload["prompt"].as_str().unwrap_or_default();
    let choices = event
        .payload
        .get("choices")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let on_behalf_of = event.payload["on_behalf_of"].as_str();
    if let Some(card_id) = router.append_human_question(&event.id, prompt, &choices, on_behalf_of) {
        router.record_pending_ask(card_id, event.from.clone(), prompt);
    }
}

pub(super) fn human_response(router: &Router, event: &Event) {
    let Some(question_id) = event.payload["question_id"].as_str() else {
        router.warn("human_response missing payload.question_id");
        return;
    };
    let Some(ask) = router.take_pending_ask(question_id) else {
        router.warn(format!(
            "human_response references unknown question_id {question_id}"
        ));
        return;
    };
    if let Err(error) = append_answer(router, event, &ask) {
        router
            .state
            .lock()
            .unwrap()
            .pending_asks
            .insert(question_id.to_owned(), ask);
        router.warn(format!("human_response message append failed: {error}"));
    }
}

pub(super) fn append_answer(router: &Router, event: &Event, ask: &PendingAsk) -> Result<()> {
    let Some(asker) = &ask.asker else {
        router.warn("human_response has no recorded asker");
        return Ok(());
    };
    let prompt: String = ask.prompt.chars().take(80).collect();
    let choice = event.payload["choice"].as_str().unwrap_or_default();
    let mut draft = EventDraft::message(
        router.crew_id.as_deref(),
        &router.mission_id,
        "human",
        Some(asker.clone()),
        format!("Answer to \"{prompt}\": {choice}"),
    );
    draft.payload["question_id"] = event.payload["question_id"].clone();
    draft.payload["source_event"] = event.id.clone().into();
    router.log.append(draft)?;
    Ok(())
}

pub(super) fn message_nudge(router: &Router, event: &Event) {
    let handles = {
        let state = router.state.lock().unwrap();
        state
            .session_by_handle
            .keys()
            .filter(|handle| event.addresses(handle))
            .cloned()
            .collect::<Vec<_>>()
    };
    router
        .state
        .lock()
        .unwrap()
        .changed_backlogs
        .extend(handles);
}

// Hardcoded signal handlers. One function per built-in signal type.
// Per arch §5.2, signals always carry `to: null`; per-target routing lives
// in `payload.target` (only `human_said` uses this in v0).
//
// Messages also reach this module, but only as stdin nudges: when a
// directed message lands in a slot's inbox, the router pushes a one-line
// "[inbox] new message from @X — run `runner msg read`" notification to
// that slot's PTY so the agent wakes up and pulls. Without this nudge,
// pull-based inbox routing strands the worker — they have no clock to
// poll on. Broadcast messages nudge every slot except the sender.
//
// Stdin pushes are silent: handlers do NOT write `inject_stdin_*` audit
// events. The originating signal/message already records the cause.
// Only `ask_human` results in a derived event (`human_question`), because
// that one is consumed by the workspace UI as a card.

use runner_core::model::Event;

use super::{Router, RunnerStatus};

// Note: `LEAD_LAUNCH_PROMPT_DELAY` and the launch-prompt composition
// imports lived here previously. The mission_goal handler no longer
// drives launch-prompt delivery — that path moved to spawn-time
// positional argv in `commands::mission::mission_start` per
// `docs/impls/0007-spawn-time-prompt-delivery.md`. The resume-fresh-
// fallback (`Router::fire_lead_launch_prompt`) keeps the paste path
// alive but composes the prompt and selects its own delay internally.

/// Strip any trailing `\n`/`\r` so the body can be handed to
/// `Router::inject_and_submit` cleanly — the trailing carriage
/// return arrives as a separate stdin chunk, on a small delay, so
/// claude-code's TUI sees it as Enter rather than appending it to
/// the input buffer. Embedded `\n` characters are kept verbatim as
/// line breaks inside the input box.
fn submit_body(text: &str) -> Vec<u8> {
    text.trim_end_matches(['\n', '\r']).as_bytes().to_vec()
}

pub(super) fn mission_goal(router: &Router, event: &Event) {
    // Fresh-mission delivery moved to the spawn-time positional
    // `[PROMPT]` argv path (see
    // `docs/impls/0007-spawn-time-prompt-delivery.md` and
    // `router::runtime::first_turn_argv`). `commands::mission::
    // mission_start` composes the lead's launch prompt before the
    // spawn loop and passes it to `SessionManager::spawn`; the
    // agent reads it during process init, before the TUI binds raw
    // input, so the post-spawn paste race this handler used to
    // work around is gone.
    //
    // The resume-fresh-fallback path (`Router::fire_lead_launch_prompt`)
    // is unchanged — it still composes the prompt locally and routes
    // through `inject_and_submit_delayed` for paste-and-verify
    // delivery on a freshly-respawned-without-context lead.
    //
    // This handler intentionally stays subscribed (bus initial replay
    // still surfaces the `mission_goal` event for UI consumers and
    // future signal-routing extensions); only the launch-prompt
    // injection side effect is dropped.
    let _ = (router, event);
}

pub(super) fn human_said(router: &Router, event: &Event) {
    let text = event
        .payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let target = event
        .payload
        .get("target")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| router.lead_handle().to_string());

    if let Err(e) = router.inject_and_submit(&target, &submit_body(text)) {
        router.warn(format!(
            "human_said injection to @{target} failed: {e} (text: {text:?})"
        ));
    }
}

pub(super) fn ask_lead(router: &Router, event: &Event) {
    let question = event
        .payload
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let context = event.payload.get("context").and_then(|v| v.as_str());

    // Render a stdin template the lead can read in-stream. The asker handle
    // (`event.from`) is preserved in the prefix so the lead knows whom to
    // reply to with `runner msg post --to <asker>`.
    let mut text = format!(
        "[ask_lead from @{asker}] {question}\n",
        asker = event.from,
        question = question,
    );
    if let Some(ctx) = context {
        let ctx = ctx.trim();
        if !ctx.is_empty() {
            text.push_str("Context:\n");
            text.push_str(ctx);
            text.push('\n');
        }
    }

    let lead_handle = router.lead_handle().to_string();
    if let Err(e) = router.inject_and_submit(&lead_handle, &submit_body(&text)) {
        router.warn(format!("ask_lead injection to lead failed: {e}"));
    }
}

pub(super) fn ask_human(router: &Router, event: &Event) {
    let prompt = event
        .payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let choices = event
        .payload
        .get("choices")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let on_behalf_of = event.payload.get("on_behalf_of").and_then(|v| v.as_str());

    // Append the `human_question` card first; its id is the canonical
    // `question_id` per arch §5.5.0. The asker is the runner that emitted
    // the `ask_human` signal (typically the lead, or a worker in the
    // direct-fallback flow). Pending-ask map is keyed on the *card* id so
    // a matching `human_response` (which carries
    // `payload.question_id = human_question.id`) routes back to the
    // original asker. If the append fails, no mapping is recorded — the
    // human_response would have nothing to reference anyway, and the
    // failure is already logged inside `append_human_question`.
    if let Some(card_id) = router.append_human_question(&event.id, prompt, &choices, on_behalf_of) {
        router.record_pending_ask(card_id, event.from.clone());
    }
}

pub(super) fn human_response(router: &Router, event: &Event) {
    let Some(question_id) = event.payload.get("question_id").and_then(|v| v.as_str()) else {
        router.warn("human_response missing payload.question_id");
        return;
    };
    let Some(asker) = router.take_pending_ask(question_id) else {
        router.warn(format!(
            "human_response references unknown question_id {question_id}"
        ));
        return;
    };

    // Render the human's choice as a single line. Free-text answers (a
    // future v0.x extension) would land in `payload.text`; for now we
    // expect `choice` only.
    let choice = event
        .payload
        .get("choice")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = format!("[human_response] {choice}");
    if let Err(e) = router.inject_and_submit(&asker, &submit_body(&text)) {
        router.warn(format!("human_response injection to @{asker} failed: {e}"));
    }
}

/// Wakes the recipient(s) of a message by pushing a one-line stdin
/// nudge. The agent reads it in-stream and is expected to call
/// `runner msg read` to pull the actual message.
///
/// Routing rules:
///   - Directed (`to == Some(handle)`): nudge that handle's session.
///   - Broadcast (`to == None`): nudge every slot in the roster except
///     the sender. The sender already knows what they sent; nudging
///     them creates an echo loop.
pub(super) fn message_nudge(router: &Router, event: &Event) {
    let sender = event.from.as_str();
    if let Some(target) = event.to.as_deref() {
        if target == sender {
            return;
        }
        // `human` is a virtual handle: it identifies the workspace
        // UI as the message recipient. The workspace renders the
        // event in the feed via the bus's `event/appended` listener
        // — there's no PTY to nudge. Skipping here also keeps the
        // log clean of "no live session for handle @human" warnings.
        if target == "human" {
            return;
        }
        let text = format!("[inbox] new message from @{sender} — run `runner msg read` to view.");
        if let Err(e) = router.inject_and_submit(target, &submit_body(&text)) {
            router.warn(format!("message_nudge injection to @{target} failed: {e}"));
        }
        return;
    }

    // Broadcast: walk the roster, skip the sender, nudge each.
    let text = format!("[inbox] new broadcast from @{sender} — run `runner msg read` to view.");
    let handles: Vec<String> = router
        .launch()
        .roster()
        .iter()
        .map(|r| r.handle().to_string())
        .filter(|h| h != sender)
        .collect();
    for handle in handles {
        if let Err(e) = router.inject_and_submit(&handle, &submit_body(&text)) {
            router.warn(format!("message_nudge broadcast to @{handle} failed: {e}"));
        }
    }
}

pub(super) fn runner_status(router: &Router, event: &Event) {
    let state = match event.payload.get("state").and_then(|v| v.as_str()) {
        Some("busy") => RunnerStatus::Busy,
        Some("idle") => RunnerStatus::Idle,
        other => {
            router.warn(format!(
                "runner_status from @{} has unknown state {:?}",
                event.from, other
            ));
            return;
        }
    };
    router.set_status(event.from.clone(), state);

    // Wake the lead only when a non-lead reports idle. A worker reporting
    // busy is already implicit in the fact that they're working; spamming
    // the lead on every busy→still-busy transition would be noise. arch
    // §5.5.1.
    let lead_handle = router.lead_handle().to_string();
    if state == RunnerStatus::Idle && event.from != lead_handle {
        let note = event
            .payload
            .get("note")
            .and_then(|v| v.as_str())
            .map(|n| format!(" — {n}"))
            .unwrap_or_default();
        let text = format!(
            "[runner_status] @{worker} is idle{note}",
            worker = event.from
        );
        if let Err(e) = router.inject_and_submit(&lead_handle, &submit_body(&text)) {
            router.warn(format!("runner_status idle notice to lead failed: {e}"));
        }
    }
}

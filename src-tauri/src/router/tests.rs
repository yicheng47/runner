// Router unit tests. The list mirrors docs/tests/v0-mvp-tests.md C8.
//
// We bypass the event bus entirely here — the router exposes
// `handle_event(&Event)` synchronously so we can drive it with hand-crafted
// envelopes and assert what landed in the recording injector + the log.
// Bus integration is covered separately (mission lifecycle + mission_e2e).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use runner_core::event_log::EventLog;
use runner_core::model::{Event, EventDraft, EventKind, SignalType};

use super::{Router, RouterRegistry, StdinInjector};
use crate::error::Result;
use crate::model::{CrewRunner, Runner};

/// Records every `inject` call so handler outputs can be asserted.
#[derive(Default)]
struct RecordingInjector {
    pushes: Mutex<Vec<(String, Vec<u8>)>>,
    /// Optional `dead_session` set — `inject` errors when called with one
    /// of these ids, simulating a crashed PTY for `mission_warning` tests.
    dead: Mutex<Vec<String>>,
}

impl RecordingInjector {
    fn pushes_for(&self, session_id: &str) -> Vec<String> {
        self.pushes
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| s == session_id)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }

    fn all_pushes(&self) -> Vec<(String, String)> {
        self.pushes
            .lock()
            .unwrap()
            .iter()
            .map(|(s, b)| (s.clone(), String::from_utf8_lossy(b).into_owned()))
            .collect()
    }

    fn mark_dead(&self, session_id: &str) {
        self.dead.lock().unwrap().push(session_id.to_string());
    }
}

impl StdinInjector for RecordingInjector {
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        if self.dead.lock().unwrap().iter().any(|d| d == session_id) {
            return Err(crate::error::Error::msg(format!(
                "test: session {session_id} is dead"
            )));
        }
        self.pushes
            .lock()
            .unwrap()
            .push((session_id.to_string(), bytes.to_vec()));
        Ok(())
    }
}

fn runner(handle: &str, runtime: &str) -> Runner {
    Runner {
        id: format!("rid-{handle}"),
        handle: handle.into(),
        display_name: handle.to_uppercase(),
        role: "test".into(),
        runtime: runtime.into(),
        command: "/bin/sh".into(),
        args: vec![],
        working_dir: None,
        system_prompt: Some(format!("brief for {handle}")),
        env: HashMap::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn crew_runner(handle: &str, lead: bool) -> CrewRunner {
    CrewRunner {
        runner: runner(handle, "claude-code"),
        position: 0,
        lead,
        added_at: Utc::now(),
    }
}

/// Build a router around a fresh tempdir log + recording injector. Returns
/// `(router, injector, log, dir)` so tests can inspect everything without
/// re-opening the file. The dir is returned so tempdir cleanup is delayed
/// to test-end (otherwise the log path would be invalidated immediately).
fn fixture(
    roster: Vec<CrewRunner>,
    sessions: &[(&str, &str)],
) -> (
    Arc<Router>,
    Arc<RecordingInjector>,
    Arc<EventLog>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let injector = Arc::new(RecordingInjector::default());
    let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
    let router = Router::new(
        "mission-1".into(),
        "crew-1".into(),
        "Crew One".into(),
        &roster,
        vec![SignalType::new("mission_goal"), SignalType::new("ask_lead")],
        log.clone(),
        injector_dyn,
    )
    .unwrap();
    let session_pairs: Vec<(String, String)> = sessions
        .iter()
        .map(|(h, s)| (h.to_string(), s.to_string()))
        .collect();
    router.register_sessions(&session_pairs);
    (router, injector, log, dir)
}

fn signal(from: &str, ty: &str, payload: serde_json::Value) -> EventDraft {
    EventDraft::signal("crew-1", "mission-1", from, SignalType::new(ty), payload)
}

fn message(from: &str, to: Option<&str>, text: &str) -> EventDraft {
    EventDraft::message("crew-1", "mission-1", from, to.map(String::from), text)
}

fn read_signals(log: &EventLog) -> Vec<Event> {
    log.read_from(0)
        .unwrap()
        .into_iter()
        .map(|e| e.event)
        .filter(|e| matches!(e.kind, EventKind::Signal))
        .collect()
}

#[test]
fn messages_do_not_trigger_router_actions() {
    // Arch §5.5.0: messages flow through the inbox projection only; the
    // router's dispatcher must early-return on EventKind::Message. A
    // `mission_warning` from a missing handler would also surface here, so
    // an empty pushes Vec proves both that the dispatcher matched on kind
    // and that no handler ran.
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let bcast = log.append(message("lead", None, "broadcast")).unwrap();
    let direct = log.append(message("lead", Some("impl"), "go")).unwrap();
    router.handle_event(&bcast);
    router.handle_event(&direct);
    assert!(injector.all_pushes().is_empty());
}

#[test]
fn mission_goal_injects_composed_prompt_to_lead() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let ev = log
        .append(signal(
            "human",
            "mission_goal",
            serde_json::json!({ "text": "ship v0" }),
        ))
        .unwrap();
    router.handle_event(&ev);

    let lead_pushes = injector.pushes_for("S-LEAD");
    assert_eq!(lead_pushes.len(), 1, "lead receives one prompt push");
    let prompt = &lead_pushes[0];
    assert!(prompt.contains("Goal: ship v0"));
    assert!(prompt.contains("`impl`"));
    assert!(prompt.contains("Allowed signal types"));
    // Workers do not receive the launch prompt.
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn human_said_routes_to_target_or_lead() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );

    // Targeted: lands on the worker.
    let direct = log
        .append(signal(
            "human",
            "human_said",
            serde_json::json!({ "text": "look at line 42", "target": "impl" }),
        ))
        .unwrap();
    router.handle_event(&direct);
    let impl_pushes = injector.pushes_for("S-IMPL");
    assert_eq!(impl_pushes.len(), 1);
    assert!(impl_pushes[0].contains("look at line 42"));
    assert!(injector.pushes_for("S-LEAD").is_empty());

    // Untargeted: defaults to the lead.
    let bcast = log
        .append(signal(
            "human",
            "human_said",
            serde_json::json!({ "text": "status?" }),
        ))
        .unwrap();
    router.handle_event(&bcast);
    let lead_pushes = injector.pushes_for("S-LEAD");
    assert_eq!(lead_pushes.len(), 1);
    assert!(lead_pushes[0].contains("status?"));
}

#[test]
fn ask_lead_injects_question_and_context_to_lead() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let ev = log
        .append(signal(
            "impl",
            "ask_lead",
            serde_json::json!({ "question": "use notify-debouncer-full?", "context": "Pros: …\nCons: …" }),
        ))
        .unwrap();
    router.handle_event(&ev);

    let pushes = injector.pushes_for("S-LEAD");
    assert_eq!(pushes.len(), 1);
    let text = &pushes[0];
    assert!(text.contains("[ask_lead from @impl]"));
    assert!(text.contains("use notify-debouncer-full?"));
    assert!(text.contains("Pros:"));
    // Worker stdin must not see the relayed question.
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn ask_human_appends_human_question_card_and_records_pending_ask() {
    let (router, _injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let ev = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "Approve?",
                "choices": ["yes", "no"],
                "on_behalf_of": "impl",
            }),
        ))
        .unwrap();
    router.handle_event(&ev);

    // Append a `human_question` event referencing the original ask.
    let signals = read_signals(&log);
    let card = signals
        .iter()
        .find(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "human_question")
                .unwrap_or(false)
        })
        .expect("router must append human_question");
    assert_eq!(card.from, "router");
    assert_eq!(card.payload["prompt"], "Approve?");
    assert_eq!(card.payload["choices"], serde_json::json!(["yes", "no"]));
    assert_eq!(card.payload["on_behalf_of"], "impl");
    assert_eq!(card.payload["question_id"], ev.id);
    assert_eq!(card.payload["triggered_by"], ev.id);
}

#[test]
fn human_response_routes_back_to_asker() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let ask = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "Approve?",
                "choices": ["yes", "no"],
                "on_behalf_of": "impl",
            }),
        ))
        .unwrap();
    router.handle_event(&ask);

    let resp = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": ask.id, "choice": "yes" }),
        ))
        .unwrap();
    router.handle_event(&resp);

    let lead_pushes = injector.pushes_for("S-LEAD");
    assert!(
        lead_pushes.iter().any(|p| p.contains("[human_response] yes")),
        "lead must receive the routed answer; got {lead_pushes:?}",
    );
    // The pending-ask map is consumed; a duplicate response surfaces a
    // warning rather than re-injecting.
    let dup = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": ask.id, "choice": "no" }),
        ))
        .unwrap();
    router.handle_event(&dup);
    let warnings: Vec<_> = read_signals(&log)
        .into_iter()
        .filter(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "mission_warning")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        warnings
            .iter()
            .any(|w| w.payload["message"].as_str().unwrap().contains("unknown question_id")),
        "duplicate response must produce mission_warning; got {warnings:?}",
    );
}

#[test]
fn human_response_without_matching_question_emits_mission_warning() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true)],
        &[("lead", "S-LEAD")],
    );
    let resp = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": "01HUNKNOWN", "choice": "yes" }),
        ))
        .unwrap();
    router.handle_event(&resp);

    assert!(injector.all_pushes().is_empty());
    let warnings: Vec<_> = read_signals(&log)
        .into_iter()
        .filter(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "mission_warning")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(warnings.len(), 1);
}

#[test]
fn runner_status_idle_for_worker_notifies_lead_and_busy_does_not() {
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true), crew_runner("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );

    // busy from a worker — silent (no push to lead).
    let busy = log
        .append(signal(
            "impl",
            "runner_status",
            serde_json::json!({ "state": "busy" }),
        ))
        .unwrap();
    router.handle_event(&busy);
    assert!(injector.pushes_for("S-LEAD").is_empty());

    // idle from a worker — push to the lead, mentioning the worker.
    let idle = log
        .append(signal(
            "impl",
            "runner_status",
            serde_json::json!({ "state": "idle", "note": "ready for next task" }),
        ))
        .unwrap();
    router.handle_event(&idle);
    let pushes = injector.pushes_for("S-LEAD");
    assert_eq!(pushes.len(), 1);
    assert!(pushes[0].contains("@impl is idle"));
    assert!(pushes[0].contains("ready for next task"));

    // idle from the lead itself — does not self-notify.
    let lead_idle = log
        .append(signal(
            "lead",
            "runner_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&lead_idle);
    assert_eq!(
        injector.pushes_for("S-LEAD").len(),
        1,
        "lead going idle must not push to lead",
    );
}

#[test]
fn pending_ask_map_reconstructs_from_log_on_reopen() {
    // Append `ask_human`, drop the router, build a fresh one, replay the log
    // through it, then append `human_response`. The answer must still route
    // to the asker — no separate persistence required.
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![crew_runner("lead", true), crew_runner("impl", false)];

    let ask = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "Approve?",
                "choices": ["yes", "no"],
                "on_behalf_of": "impl",
            }),
        ))
        .unwrap();

    // First mount processes only the ask, then is dropped.
    {
        let injector = Arc::new(RecordingInjector::default());
        let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
        let router = Router::new(
            "mission-1".into(),
            "crew-1".into(),
            "Crew One".into(),
            &roster,
            vec![],
            log.clone(),
            injector_dyn,
        )
        .unwrap();
        router.register_sessions(&[
            ("lead".into(), "S-LEAD".into()),
            ("impl".into(), "S-IMPL".into()),
        ]);
        router.handle_event(&ask);
    }

    // Second mount: replay the log up to and including the ask, then route
    // the response.
    let injector = Arc::new(RecordingInjector::default());
    let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
    let router2 = Router::new(
        "mission-1".into(),
        "crew-1".into(),
        "Crew One".into(),
        &roster,
        vec![],
        log.clone(),
        injector_dyn,
    )
    .unwrap();
    router2.register_sessions(&[
        ("lead".into(), "S-LEAD".into()),
        ("impl".into(), "S-IMPL".into()),
    ]);
    // Mirror what `BusEmitter` would do during initial replay: feed every
    // historical envelope through `handle_event`. The router has no
    // dispatch ledger — it just rebuilds the pending-ask map from the
    // ask_human row it sees.
    for entry in log.read_from(0).unwrap() {
        router2.handle_event(&entry.event);
    }

    let resp = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": ask.id, "choice": "yes" }),
        ))
        .unwrap();
    router2.handle_event(&resp);

    let lead_pushes = injector.pushes_for("S-LEAD");
    assert!(
        lead_pushes.iter().any(|p| p.contains("[human_response] yes")),
        "after reopen + replay, response must route to original asker; got {lead_pushes:?}",
    );
}

#[test]
fn dead_session_for_handler_target_emits_mission_warning() {
    // The pending-ask map persists past a session crash by design — better
    // to surface the missed wake-up than to silently drop it. The router
    // attempts the inject, fails, and writes a mission_warning.
    let (router, injector, log, _dir) = fixture(
        vec![crew_runner("lead", true)],
        &[("lead", "S-LEAD")],
    );
    injector.mark_dead("S-LEAD");
    let ev = log
        .append(signal(
            "human",
            "human_said",
            serde_json::json!({ "text": "hi" }),
        ))
        .unwrap();
    router.handle_event(&ev);

    let warnings: Vec<_> = read_signals(&log)
        .into_iter()
        .filter(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "mission_warning")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].payload["message"]
        .as_str()
        .unwrap()
        .contains("human_said injection"));
}

#[test]
fn registry_register_get_unregister() {
    let (router, _i, _l, _d) = fixture(
        vec![crew_runner("lead", true)],
        &[("lead", "S-LEAD")],
    );
    let reg = RouterRegistry::new();
    reg.register("mission-1".into(), router.clone());
    assert!(reg.get("mission-1").is_some());
    reg.unregister("mission-1");
    assert!(reg.get("mission-1").is_none());
}

use super::dispatch::{Dispatcher, Input};
use super::*;
use crate::model::{Role, Slot, SlotWithRole};
use chrono::Utc;
use runner_core::event_log::EventLog;
use runner_core::model::{EventDraft, SignalType};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

#[derive(Default)]
struct FakeState {
    readiness: HashMap<String, DeliveryReadiness>,
    reserved: HashMap<String, u64>,
    pushes: Vec<(String, Vec<u8>)>,
    blocked: Vec<DeliveryBlockedEvent>,
    busy: usize,
    input: String,
    sticky: bool,
    unknown: bool,
    revision: u64,
    turn_started: u64,
    extra_rows: usize,
    ack_on_reserve: Option<Weak<Router>>,
}
#[derive(Default)]
struct Fake(Mutex<FakeState>);
impl StdinInjector for Fake {
    fn delivery_readiness(&self, session: &str) -> DeliveryReadiness {
        let state = self.0.lock().unwrap();
        if state.reserved.contains_key(session) {
            DeliveryReadiness::InFlight
        } else {
            state
                .readiness
                .get(session)
                .copied()
                .unwrap_or(DeliveryReadiness::Ready { idle: true })
        }
    }
    fn reserve_delivery(&self, session: &str) -> Result<DeliveryReservation> {
        if matches!(
            self.delivery_readiness(session),
            DeliveryReadiness::Ready { .. }
        ) {
            let ack = {
                let mut state = self.0.lock().unwrap();
                state.reserved.insert(session.into(), 1);
                state.ack_on_reserve.take()
            };
            if let Some(router) = ack.and_then(|router| router.upgrade()) {
                router
                    .state
                    .lock()
                    .unwrap()
                    .unread_by_handle
                    .insert("worker".into(), 0);
            }
            Ok(DeliveryReservation::Ready(1))
        } else {
            Ok(DeliveryReservation::Unavailable)
        }
    }
    fn inject_reserved(&self, session: &str, token: u64, bytes: &[u8]) -> Result<bool> {
        let mut state = self.0.lock().unwrap();
        if state.reserved.get(session) != Some(&token) {
            return Ok(false);
        }
        state.pushes.push((session.into(), bytes.to_vec()));
        if bytes == b"\r" {
            if !state.sticky {
                state.input.clear();
            }
        } else {
            state.input = String::from_utf8_lossy(bytes).into_owned();
        }
        Ok(true)
    }
    fn finish_delivery(&self, session: &str, _: u64) {
        self.0.lock().unwrap().reserved.remove(session);
    }
    fn delivery_snapshot(&self, _: &str) -> Option<DeliverySnapshot> {
        let state = self.0.lock().unwrap();
        Some(DeliverySnapshot {
            rows: if state.unknown {
                vec![]
            } else {
                let mut rows = vec!["output".into(), format!("› {}", state.input)];
                rows.extend(std::iter::repeat_n(String::new(), state.extra_rows));
                rows
            },
            generation: 1,
            input_revision: (state.revision, 0),
            turn_started: state.turn_started,
            runtime: "fake".into(),
        })
    }
    fn resubmit(
        &self,
        session: &str,
        snapshot: &DeliverySnapshot,
        probe: &runner_terminal::delivery_probe::InputProbe,
    ) -> Result<bool> {
        let current = self.delivery_snapshot(session).unwrap();
        if current.input_revision != snapshot.input_revision
            || probe.state(&current.rows) != runner_terminal::delivery_probe::ProbeState::Present
        {
            return Ok(false);
        }
        let mut state = self.0.lock().unwrap();
        state.pushes.push((session.into(), b"\r".to_vec()));
        if !state.sticky {
            state.input.clear();
        }
        Ok(true)
    }
    fn synthesize_wake_busy(&self, _: &str, _: EventDraft) -> Result<()> {
        self.0.lock().unwrap().busy += 1;
        Ok(())
    }
    fn register_delivery_listener(&self, _: &str, _: Weak<dyn SessionDeliveryListener>) {}
}
impl RouterUiNotifier for Fake {
    fn delivery_blocked(&self, event: &DeliveryBlockedEvent) {
        self.0.lock().unwrap().blocked.push(event.clone());
    }
}

struct Fixture {
    router: Arc<Router>,
    port: Arc<Fake>,
    dispatcher: Dispatcher,
    now: Instant,
    _dir: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let port = Arc::new(Fake::default());
        let now = Instant::now();
        let router = Router::new(
            "mission".into(),
            None,
            "crew".into(),
            &[
                slot_with_role("lead", true),
                slot_with_role("worker", false),
            ],
            vec![],
            None,
            Arc::new(EventLog::open(dir.path()).unwrap()),
            port.clone(),
            port.clone(),
        )
        .unwrap();
        router.register_sessions(&[("lead".into(), "L".into()), ("worker".into(), "W".into())]);
        Self {
            router,
            port,
            dispatcher: Dispatcher::new(now),
            now,
            _dir: dir,
        }
    }
    fn tick(&mut self, input: Option<Input>) -> Option<Instant> {
        self.dispatcher.advance(&self.router, input, self.now)
    }
    fn backlog(&mut self, count: usize) {
        self.router
            .state
            .lock()
            .unwrap()
            .unread_by_handle
            .insert("worker".into(), count);
        self.tick(Some(Input::Backlog {
            handle: "worker".into(),
            initial_backlog: false,
        }));
    }
    fn project_backlog(&self, count: usize) {
        self.router.update_inbox(&InboxUpdate {
            mission_id: "mission".into(),
            role_handle: "worker".into(),
            last_id: None,
            watermark: None,
            unread_count: count,
        });
        self.router.backlogs_projected();
    }
    fn session(&mut self, event: SessionDeliveryEvent) {
        self.tick(Some(Input::Session {
            session_id: "W".into(),
            event,
        }));
    }
    fn after(&mut self, duration: Duration) {
        self.now += duration;
        self.tick(None);
    }
    fn readiness(&self, readiness: DeliveryReadiness) {
        self.port
            .0
            .lock()
            .unwrap()
            .readiness
            .insert("W".into(), readiness);
    }
    fn writes(&self) -> Vec<String> {
        self.port
            .0
            .lock()
            .unwrap()
            .pushes
            .iter()
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }
    fn route(&self, draft: EventDraft) -> runner_core::model::Event {
        let event = self.router.log.append(draft).unwrap();
        let offset = self
            .router
            .log
            .read_from(0)
            .unwrap()
            .last()
            .unwrap()
            .next_offset;
        self.router.handle_event(&event, offset);
        event
    }
    fn signal(
        &self,
        from: &str,
        ty: &str,
        payload: serde_json::Value,
    ) -> runner_core::model::Event {
        self.route(EventDraft::signal(
            None,
            "mission",
            from,
            SignalType::new(ty),
            payload,
        ))
    }
}

fn role(handle: &str, runtime: &str) -> Role {
    Role {
        id: format!("rid-{handle}"),
        handle: handle.into(),
        display_name: handle.to_uppercase(),
        runtime: runtime.into(),
        command: "/bin/sh".into(),
        args: vec![],
        working_dir: None,
        system_prompt: Some(format!("brief for {handle}")),
        env: HashMap::new(),
        model: None,
        effort: None,
        codex_speed: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn slot_with_role(handle: &str, lead: bool) -> SlotWithRole {
    let role = role(handle, "claude-code");
    SlotWithRole {
        slot: Slot {
            id: format!("slot-{handle}"),
            crew_id: "crew-1".into(),
            role_id: role.id.clone(),
            slot_handle: handle.into(),
            position: 0,
            lead,
            runtime_override: None,
            model_override: None,
            effort_override: None,
            codex_speed_override: None,
            added_at: Utc::now(),
        },
        role,
    }
}

#[test]
fn attempts_use_current_backlog_and_submit_at_the_deadline() {
    let mut f = Fixture::new();
    f.backlog(1);
    assert_eq!(f.writes().len(), 1);
    f.backlog(3);
    f.after(SUBMIT_DELAY - Duration::from_millis(1));
    assert_eq!(f.writes().len(), 1);
    f.after(Duration::from_millis(1));
    assert_eq!(f.writes()[1], "\r");
    f.after(dispatch::CONFIRM_WINDOW);
    f.after(SUBMIT_DELAY);
    assert!(f.writes()[2].contains("3 new messages"));
}

#[test]
fn an_ack_cancels_a_held_notification() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Drafting);
    f.backlog(2);
    f.backlog(0);
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(Duration::from_secs(120));
    assert!(f.writes().is_empty());
}

#[test]
fn all_readiness_holds_release_on_the_matching_session_event() {
    for (readiness, event) in [
        (DeliveryReadiness::Pending, SessionDeliveryEvent::Respawned),
        (
            DeliveryReadiness::Starting,
            SessionDeliveryEvent::StartupReady,
        ),
        (
            DeliveryReadiness::Drafting,
            SessionDeliveryEvent::InputCleared,
        ),
        (
            DeliveryReadiness::RecentlyTyping(Duration::from_secs(2)),
            SessionDeliveryEvent::InputCleared,
        ),
        (
            DeliveryReadiness::NeedsYou,
            SessionDeliveryEvent::InputCleared,
        ),
        (
            DeliveryReadiness::InFlight,
            SessionDeliveryEvent::DeliveryFinished,
        ),
        (
            DeliveryReadiness::Unavailable,
            SessionDeliveryEvent::Respawned,
        ),
    ] {
        let mut f = Fixture::new();
        f.readiness(readiness);
        f.backlog(1);
        assert!(f.writes().is_empty(), "{readiness:?}");
        f.readiness(DeliveryReadiness::Ready { idle: false });
        f.session(event);
        f.after(Duration::from_secs(2));
        assert_eq!(
            f.writes()
                .iter()
                .filter(|text| text.as_str() != "\r")
                .count(),
            1,
            "{readiness:?}"
        );
    }
}

#[test]
fn input_clear_grace_is_rechecked_when_the_person_types_again() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Drafting);
    f.backlog(1);
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(Duration::from_millis(499));
    assert!(f.writes().is_empty());
    f.readiness(DeliveryReadiness::Drafting);
    f.after(Duration::from_millis(1));
    assert!(f.writes().is_empty());
    assert!(f.port.0.lock().unwrap().blocked.last().unwrap().blocked);
}

#[test]
fn redelivery_is_idle_only_and_obeys_backoff_even_on_idle_events() {
    let mut f = Fixture::new();
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    f.after(dispatch::CONFIRM_WINDOW);
    f.after(SUBMIT_DELAY);
    f.readiness(DeliveryReadiness::Ready { idle: false });
    f.after(Duration::from_secs(120));
    assert_eq!(f.writes().len(), 2);
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.writes().len(), 3);
    f.after(SUBMIT_DELAY);
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.writes().len(), 4);
}

#[test]
fn exit_keeps_backlog_and_respawn_attempts_again_and_removal_clears_backoff() {
    let mut f = Fixture::new();
    f.backlog(1);
    f.readiness(DeliveryReadiness::Unavailable);
    f.session(SessionDeliveryEvent::Exited);
    assert_eq!(f.router.state.lock().unwrap().unread_by_handle["worker"], 1);
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::Respawned);
    assert_eq!(f.writes().len(), 2);
    f.router.remove_roster_member("worker");
    f.tick(Some(Input::RosterChanged));
    assert!(f.port.0.lock().unwrap().reserved.is_empty());
}

#[test]
fn a_stopped_subscriber_does_not_repeat_its_queued_warning_on_remount() {
    for project_backlog_later in [false, true] {
        let mut f = Fixture::new();
        f.readiness(DeliveryReadiness::Unavailable);
        f.router
            .log
            .append(EventDraft::message(
                None,
                "mission",
                "human",
                Some("worker".into()),
                "historical message",
            ))
            .unwrap();
        f.router
            .warn("delivery to @worker queued until the session resumes");
        let before = f.router.log.read_from(0).unwrap();
        f.router.reconstruct_from_log().unwrap();
        if project_backlog_later {
            f.tick(Some(Input::RosterChanged));
        }
        f.project_backlog(1);
        f.tick(Some(Input::RosterChanged));
        f.router.dispatch_at(&mut f.dispatcher, f.now);
        f.after(Duration::from_secs(120));
        assert_eq!(f.router.log.read_from(0).unwrap().len(), before.len());
        assert!(f.writes().is_empty());

        f.route(EventDraft::message(
            None,
            "mission",
            "human",
            Some("worker".into()),
            "another message while stopped",
        ));
        f.project_backlog(2);
        f.router.dispatch_at(&mut f.dispatcher, f.now);
        assert_eq!(f.router.log.read_from(0).unwrap().len(), before.len() + 1);

        f.readiness(DeliveryReadiness::Pending);
        f.session(SessionDeliveryEvent::Respawned);
        f.readiness(DeliveryReadiness::Ready { idle: true });
        f.session(SessionDeliveryEvent::StartupReady);
        assert_eq!(f.router.log.read_from(0).unwrap().len(), before.len() + 1);
        assert_eq!(f.writes().len(), 1);
    }
}

#[test]
fn a_stopped_subscriber_mounted_empty_warns_when_its_first_message_arrives() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Unavailable);
    f.router.reconstruct_from_log().unwrap();
    f.project_backlog(0);
    f.tick(Some(Input::RosterChanged));
    assert!(f.router.log.read_from(0).unwrap().is_empty());

    f.route(EventDraft::message(
        None,
        "mission",
        "human",
        Some("worker".into()),
        "first message after restart",
    ));
    f.project_backlog(1);
    f.router.dispatch_at(&mut f.dispatcher, f.now);
    f.route(EventDraft::message(
        None,
        "mission",
        "human",
        Some("worker".into()),
        "second message after restart",
    ));
    f.project_backlog(2);
    f.router.dispatch_at(&mut f.dispatcher, f.now);
    f.after(Duration::from_secs(120));
    let entries = f.router.log.read_from(0).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[1]
            .event
            .signal_type
            .as_ref()
            .map(SignalType::as_str),
        Some("mission_warning")
    );
    assert_eq!(
        entries[1].event.payload["message"],
        "delivery to @worker queued until the session resumes"
    );
    assert!(f.writes().is_empty());
}

#[test]
fn an_exit_with_a_backlog_warns_once_per_stop_on_a_live_router() {
    let mut f = Fixture::new();
    f.backlog(1);
    for expected in [1, 2] {
        f.readiness(DeliveryReadiness::Unavailable);
        f.session(SessionDeliveryEvent::Exited);
        f.backlog(2);
        f.after(Duration::from_secs(120));
        let entries = f.router.log.read_from(0).unwrap();
        assert_eq!(entries.len(), expected);
        assert!(entries.iter().all(|entry| {
            entry.event.signal_type.as_ref().map(SignalType::as_str) == Some("mission_warning")
                && entry.event.payload["message"]
                    == "delivery to @worker queued until the session resumes"
        }));
        f.readiness(DeliveryReadiness::Ready { idle: true });
        f.session(SessionDeliveryEvent::Respawned);
    }
}

#[test]
fn stop_releases_the_gate_and_clears_the_pill() {
    let mut f = Fixture::new();
    f.backlog(1);
    assert_eq!(f.tick(Some(Input::Stop)), None);
    assert!(f.port.0.lock().unwrap().reserved.is_empty());
}

#[test]
fn cursor_is_an_offset_not_an_event_id_and_reconstruction_skips_history() {
    let f = Fixture::new();
    let mut event = f
        .router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "worker",
            SignalType::new("ask_human"),
            serde_json::json!({"prompt":"first"}),
        ))
        .unwrap();
    event.id = "ZZZ".into();
    f.router.handle_event(&event, 100);
    event.id = "AAA".into();
    f.router.handle_event(&event, 200);
    assert_eq!(f.router.pending_ask_count(), 2);
    f.router.handle_event(&event, 200);
    assert_eq!(f.router.pending_ask_count(), 2);
    f.router.reconstruct_from_log().unwrap();
    let count = f.router.pending_ask_count();
    for entry in f.router.log.read_from(0).unwrap() {
        f.router.handle_event(&entry.event, entry.next_offset);
    }
    assert_eq!(f.router.pending_ask_count(), count);
}

#[test]
fn registry_mount_and_unmount_joins_the_driver() {
    let f = Fixture::new();
    let registry = RouterRegistry::new();
    registry.register("mission".into(), f.router.clone());
    assert!(registry.get("mission").is_some());
    registry.unregister("mission");
    assert!(registry.get("mission").is_none());
}

#[test]
fn slot_restart_records_the_signal_before_notices_to_the_right_members() {
    for (has_lead, handle, recipients) in [
        (true, "worker", vec!["lead"]),
        (true, "lead", vec!["worker", "reviewer"]),
        (false, "worker", vec!["lead", "reviewer"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let port = Arc::new(Fake::default());
        let router = Router::new(
            "mission".into(),
            None,
            "crew".into(),
            &[
                slot_with_role("lead", has_lead),
                slot_with_role("worker", false),
                slot_with_role("reviewer", false),
            ],
            vec![],
            None,
            Arc::new(EventLog::open(dir.path()).unwrap()),
            port.clone(),
            port,
        )
        .unwrap();
        router
            .record_slot_restart(handle, "restarted-session", Some("prior-key"))
            .unwrap();
        let entries = router.log.read_from(0).unwrap();
        assert_eq!(entries.len(), recipients.len() + 1);
        let signal = &entries[0].event;
        assert_eq!(signal.kind, EventKind::Signal);
        assert_eq!(signal.from, "human");
        assert_eq!(
            signal.signal_type.as_ref().map(SignalType::as_str),
            Some("slot_restarted")
        );
        assert_eq!(
            signal.payload,
            serde_json::json!({"handle": handle, "session_id": "restarted-session", "prior_agent_session_key": "prior-key"})
        );
        for (entry, recipient) in entries[1..].iter().zip(recipients) {
            assert_eq!(entry.event.kind, EventKind::Message);
            assert_eq!(entry.event.from, "runner");
            assert_eq!(
                entry.event.recipients().first().map(String::as_str),
                Some(recipient)
            );
            assert_eq!(entry.event.payload["text"], format!("@{handle} was restarted by the human and starts over with only its brief. Nothing it was told in this mission survives; re-send the task, branch, and anything else it needs."));
        }
    }
}

#[test]
fn reconstruction_skips_malformed_lines_and_places_its_cursor_after_them() {
    use std::io::Write;

    let f = Fixture::new();
    let ask = f
        .router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "worker",
            SignalType::new("ask_human"),
            serde_json::json!({"prompt":"continue?"}),
        ))
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(f.router.log.path())
        .unwrap();
    file.write_all(b"malformed middle line\n").unwrap();
    let question = f
        .router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "router",
            SignalType::new("human_question"),
            serde_json::json!({"triggered_by":ask.id,"prompt":"continue?"}),
        ))
        .unwrap();
    file.write_all(b"malformed trailing line\n").unwrap();
    let end = f.router.log.size().unwrap();

    f.router.reconstruct_from_log().unwrap();
    assert_eq!(f.router.state.lock().unwrap().cursor, end);
    assert_eq!(f.router.pending_ask_count(), 1);
    let (entries, skipped) = f.router.log.read_from_lossy(0).unwrap();
    assert_eq!(skipped.len(), 2);
    for entry in entries {
        f.router.handle_event(&entry.event, entry.next_offset);
    }
    assert_eq!(f.router.log.size().unwrap(), end);
    let response = f
        .router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            SignalType::new("human_response"),
            serde_json::json!({"question_id":question.id,"choice":"yes"}),
        ))
        .unwrap();
    f.router
        .handle_event(&response, f.router.log.size().unwrap());
    let entries = f.router.log.read_from_lossy(end).unwrap().0;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].event.kind, EventKind::Message);
    assert_eq!(
        entries[1].event.recipients().first().map(String::as_str),
        Some("worker")
    );
    assert_eq!(entries[1].event.payload["question_id"], question.id);
    assert_eq!(f.router.pending_ask_count(), 0);
}

#[test]
fn an_unknown_human_response_warns_without_appending_a_message() {
    let f = Fixture::new();
    f.signal(
        "human",
        "human_response",
        serde_json::json!({"question_id":"unknown","choice":"yes"}),
    );
    let entries = f.router.log.read_from(0).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .all(|entry| entry.event.kind == EventKind::Signal));
    assert_eq!(
        entries[1]
            .event
            .signal_type
            .as_ref()
            .map(SignalType::as_str),
        Some("mission_warning")
    );
    assert_eq!(
        entries[1].event.payload["message"],
        "human_response references unknown question_id unknown"
    );
    assert!(f.writes().is_empty());
}

#[test]
fn ask_human_creates_a_card_and_routes_its_response_to_the_recorded_asker() {
    let f = Fixture::new();
    let ask = f.signal(
        "worker",
        "ask_human",
        serde_json::json!({"prompt":"continue?","choices":["yes","no"],"on_behalf_of":"lead"}),
    );
    let entries = f.router.log.read_from(0).unwrap();
    assert_eq!(entries.len(), 2);
    let card = &entries[1].event;
    assert_eq!(card.kind, EventKind::Signal);
    assert_eq!(card.from, "router");
    assert_eq!(
        card.signal_type.as_ref().map(SignalType::as_str),
        Some("human_question")
    );
    assert_eq!(
        card.payload,
        serde_json::json!({"triggered_by":ask.id,"prompt":"continue?","choices":["yes","no"],"on_behalf_of":"lead"})
    );
    assert_eq!(f.router.pending_ask_count(), 1);
    assert_eq!(
        f.router.state.lock().unwrap().pending_asks[&card.id]
            .asker
            .as_deref(),
        Some("worker")
    );
    let response = f.signal(
        "human",
        "human_response",
        serde_json::json!({"question_id":card.id,"choice":"yes"}),
    );
    let entries = f.router.log.read_from(0).unwrap();
    assert_eq!(entries.len(), 4);
    let answer = &entries[3].event;
    assert_eq!(answer.kind, EventKind::Message);
    assert_eq!(answer.from, "human");
    assert_eq!(
        answer.recipients().first().map(String::as_str),
        Some("worker")
    );
    assert_eq!(answer.payload["text"], "Answer to \"continue?\": yes");
    assert_eq!(answer.payload["question_id"], card.id);
    assert_eq!(answer.payload["source_event"], response.id);
    assert_eq!(f.router.pending_ask_count(), 0);
    assert!(f.writes().is_empty());
}

#[test]
fn confirmation_uses_runtime_turn_started_or_a_cleared_input() {
    for hook in [false, true] {
        let mut f = Fixture::new();
        f.port.0.lock().unwrap().sticky = hook;
        f.backlog(1);
        assert_eq!(f.port.0.lock().unwrap().busy, 0);
        f.after(SUBMIT_DELAY);
        assert_eq!(f.port.0.lock().unwrap().busy, 0);
        if hook {
            f.port.0.lock().unwrap().turn_started += 1;
        }
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.port.0.lock().unwrap().busy, 1);
        assert_eq!(
            f.writes()
                .iter()
                .filter(|text| text.as_str() == "\r")
                .count(),
            1
        );
    }
}

#[test]
fn unsubmitted_gets_exactly_one_more_enter_then_pauses_until_the_rows_clear() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.writes().len(), 3);
    f.after(dispatch::CONFIRM_WINDOW);
    let blocked = f.port.0.lock().unwrap().blocked.last().unwrap().clone();
    assert_eq!(blocked.reason.as_deref(), Some("unsubmitted"));
    f.backlog(2);
    f.after(Duration::from_secs(120));
    assert_eq!(f.writes().len(), 3);
    f.port.0.lock().unwrap().input.clear();
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(Duration::from_millis(500));
    assert!(f.writes().last().unwrap().contains("2 new messages"));
}

#[test]
fn an_unknown_probe_or_local_input_is_unverified_and_never_gets_a_second_enter() {
    for local_input in [false, true] {
        let mut f = Fixture::new();
        {
            let mut state = f.port.0.lock().unwrap();
            state.sticky = true;
            state.unknown = !local_input;
        }
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        if local_input {
            f.port.0.lock().unwrap().revision += 1;
        }
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.writes().len(), 2);
        assert_eq!(f.port.0.lock().unwrap().busy, 0);
    }
}

#[test]
fn live_and_reconstructed_pending_counts_include_orphaned_cards() {
    let f = Fixture::new();
    f.signal(
        "worker",
        "ask_human",
        serde_json::json!({"prompt":"paired"}),
    );
    let orphan = f.signal(
        "router",
        "human_question",
        serde_json::json!({"prompt":"orphan"}),
    );
    assert_eq!(f.router.pending_ask_count(), 2);
    f.router.reconstruct_from_log().unwrap();
    assert_eq!(f.router.pending_ask_count(), 2);
    f.signal(
        "human",
        "human_response",
        serde_json::json!({"question_id":orphan.id,"choice":"yes"}),
    );
    assert_eq!(f.router.pending_ask_count(), 1);
    assert_eq!(pending_asks(&f.router.log.read_from(0).unwrap()).len(), 1);
}

#[test]
fn a_replayed_ack_is_projected_before_any_notification_can_start() {
    let mut f = Fixture::new();
    f.router.update_inbox(&InboxUpdate {
        mission_id: "mission".into(),
        role_handle: "worker".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    f.session(SessionDeliveryEvent::Idle);
    assert!(f.writes().is_empty());
    f.router.update_watermark(&WatermarkUpdate {
        mission_id: "mission".into(),
        role_handle: "worker".into(),
        watermark: "ack".into(),
        unread_count: 0,
    });
    f.router.backlogs_projected();
    f.router.dispatch_at(&mut f.dispatcher, f.now);
    f.after(Duration::from_secs(120));
    assert!(f.writes().is_empty());
}

#[test]
fn answers_are_durable_messages_to_the_asker_with_a_unicode_safe_prompt_excerpt() {
    let f = Fixture::new();
    f.signal(
        "worker",
        "ask_human",
        serde_json::json!({"prompt":"中".repeat(90)}),
    );
    let question = f
        .router
        .log
        .read_from(0)
        .unwrap()
        .into_iter()
        .find(|entry| {
            entry.event.signal_type.as_ref().map(SignalType::as_str) == Some("human_question")
        })
        .unwrap()
        .event;
    f.signal(
        "human",
        "human_response",
        serde_json::json!({"question_id":question.id,"choice":"yes","delivery":"inbox"}),
    );
    let messages = f
        .router
        .log
        .read_from(0)
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event.kind == EventKind::Message)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].event.from, "human");
    assert_eq!(
        messages[0].event.recipients().first().map(String::as_str),
        Some("worker")
    );
    assert_eq!(messages[0].event.payload["question_id"], question.id);
    assert_eq!(
        messages[0].event.payload["text"],
        format!("Answer to \"{}\": yes", "中".repeat(80))
    );
    f.router.reconstruct_from_log().unwrap();
    assert_eq!(
        f.router
            .log
            .read_from(0)
            .unwrap()
            .iter()
            .filter(|entry| entry.event.kind == EventKind::Message)
            .count(),
        1
    );
    assert!(f.port.0.lock().unwrap().pushes.is_empty());
}

#[test]
fn targeted_and_untargeted_human_said_use_the_stream_without_a_lead_fallback() {
    let f = Fixture::new();
    f.signal(
        "human",
        "human_said",
        serde_json::json!({"text":"everyone"}),
    );
    f.signal(
        "human",
        "human_said",
        serde_json::json!({"text":"only worker","target":"worker"}),
    );
    let entries = f.router.log.read_from(0).unwrap();
    let messages = entries
        .iter()
        .filter(|entry| entry.event.kind == EventKind::Message)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].event.recipients().is_empty());
    assert_eq!(
        messages[1].event.recipients().first().map(String::as_str),
        Some("worker")
    );
    assert!(messages.iter().all(|entry| entry.event.from == "human"));
    assert!(f.port.0.lock().unwrap().pushes.is_empty());
}

#[test]
fn restart_repairs_the_gap_between_a_person_signal_and_its_message_once() {
    let f = Fixture::new();
    f.signal(
        "worker",
        "ask_human",
        serde_json::json!({"prompt":"continue?"}),
    );
    let question = f
        .router
        .log
        .read_from(0)
        .unwrap()
        .last()
        .unwrap()
        .event
        .clone();
    f.router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            SignalType::new("human_response"),
            serde_json::json!({"question_id":question.id,"choice":"yes","delivery":"inbox"}),
        ))
        .unwrap();
    f.router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            SignalType::new("human_said"),
            serde_json::json!({"text":"resume","delivery":"inbox"}),
        ))
        .unwrap();
    f.router.reconstruct_from_log().unwrap();
    f.router.reconstruct_from_log().unwrap();
    let entries = f.router.log.read_from(0).unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.event.kind == EventKind::Message)
            .count(),
        2
    );
    assert_eq!(f.router.pending_ask_count(), 0);
}

#[test]
fn legacy_typed_answer_and_human_said_logs_replay_without_new_messages() {
    let f = Fixture::new();
    f.router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            SignalType::new("human_said"),
            serde_json::json!({"text":"old typed message"}),
        ))
        .unwrap();
    f.router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            SignalType::new("human_response"),
            serde_json::json!({"question_id":"old","choice":"yes"}),
        ))
        .unwrap();
    f.router.reconstruct_from_log().unwrap();
    for entry in f.router.log.read_from(0).unwrap() {
        f.router.handle_event(&entry.event, entry.next_offset);
    }
    assert!(f
        .router
        .log
        .read_from(0)
        .unwrap()
        .iter()
        .all(|entry| entry.event.kind == EventKind::Signal));
}

#[test]
fn an_ack_during_reservation_cancels_the_attempt_before_typing() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().ack_on_reserve = Some(Arc::downgrade(&f.router));
    f.backlog(1);
    assert!(f.writes().is_empty());
    assert!(f.port.0.lock().unwrap().reserved.is_empty());
}

#[test]
fn a_resized_paused_input_resumes_on_input_cleared_without_interrupting_a_new_draft() {
    for new_draft in [false, true] {
        let mut f = Fixture::new();
        f.port.0.lock().unwrap().sticky = true;
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        f.after(dispatch::CONFIRM_WINDOW);
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(
            f.port
                .0
                .lock()
                .unwrap()
                .blocked
                .last()
                .unwrap()
                .reason
                .as_deref(),
            Some("unsubmitted")
        );
        {
            let mut state = f.port.0.lock().unwrap();
            state.extra_rows = 1;
            state.input.clear();
        }
        f.session(SessionDeliveryEvent::InputCleared);
        if new_draft {
            f.readiness(DeliveryReadiness::Drafting);
        }
        f.backlog(2);
        f.after(Duration::from_millis(500));
        if new_draft {
            assert_eq!(f.writes().len(), 3);
            f.readiness(DeliveryReadiness::Ready { idle: true });
            f.session(SessionDeliveryEvent::InputCleared);
            f.after(Duration::from_millis(500));
        }
        assert!(f.writes().last().unwrap().contains("2 new messages"));
        f.port.0.lock().unwrap().sticky = false;
        f.after(SUBMIT_DELAY);
        f.after(dispatch::CONFIRM_WINDOW);
        f.backlog(0);
        f.after(SUBMIT_DELAY);
        f.backlog(3);
        assert!(f.writes().last().unwrap().contains("3 new messages"));
    }
}

#[test]
fn approval_release_never_unpauses_unchanged_notification_even_after_resize() {
    for resized in [false, true] {
        let mut f = Fixture::new();
        f.port.0.lock().unwrap().sticky = true;
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        f.after(dispatch::CONFIRM_WINDOW);
        f.after(dispatch::CONFIRM_WINDOW);
        if resized {
            f.port.0.lock().unwrap().extra_rows = 1;
        }
        f.readiness(DeliveryReadiness::NeedsYou);
        f.session(SessionDeliveryEvent::ReadinessChanged);
        f.readiness(DeliveryReadiness::Ready { idle: false });
        f.session(SessionDeliveryEvent::ReadinessChanged);
        f.backlog(2);
        f.after(Duration::from_millis(500));
        f.after(Duration::from_secs(120));
        assert_eq!(f.writes().len(), 3);
        assert_eq!(
            f.port
                .0
                .lock()
                .unwrap()
                .blocked
                .last()
                .unwrap()
                .reason
                .as_deref(),
            Some("unsubmitted")
        );
        if !resized {
            f.session(SessionDeliveryEvent::InputCleared);
            f.after(Duration::from_millis(500));
            assert_eq!(
                f.writes().len(),
                3,
                "a positive probe must retain the pause"
            );
        }
        f.port.0.lock().unwrap().input.clear();
        f.session(SessionDeliveryEvent::InputCleared);
        f.after(Duration::from_millis(500));
        assert!(f.writes().last().unwrap().contains("2 new messages"));
    }
}

#[test]
fn addressed_members_alone_receive_notifications_on_the_manual_clock() {
    for (targets, expected) in [
        (vec!["worker"], vec!["worker"]),
        (vec!["worker", "qa"], vec!["worker", "qa"]),
        (vec![], vec!["worker", "qa", "reviewer"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let port = Arc::new(Fake::default());
        let handles = ["coder", "worker", "qa", "reviewer"];
        let router = Router::new(
            "mission".into(),
            None,
            "crew".into(),
            &handles.map(|handle| slot_with_role(handle, false)),
            vec![],
            None,
            Arc::new(EventLog::open(dir.path()).unwrap()),
            port.clone(),
            port.clone(),
        )
        .unwrap();
        router.register_sessions(&handles.map(|handle| (handle.to_owned(), handle.to_owned())));
        let mut draft = EventDraft::message(None, "mission", "coder", None, "please check");
        draft.to = runner_core::Recipients::new(targets.into_iter().map(str::to_owned).collect());
        let event = router.log.append(draft).unwrap();
        super::handlers::message_nudge(&router, &event);
        let notified = router.state.lock().unwrap().changed_backlogs.clone();
        assert_eq!(notified.len(), expected.len());
        assert!(expected.iter().all(|handle| notified.contains(*handle)));
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(now);
        for handle in &expected {
            router
                .state
                .lock()
                .unwrap()
                .unread_by_handle
                .insert((*handle).into(), 1);
        }
        for handle in notified {
            dispatcher.advance(
                &router,
                Some(Input::Backlog {
                    handle,
                    initial_backlog: false,
                }),
                now,
            );
        }
        dispatcher.advance(&router, None, now + SUBMIT_DELAY);
        let state = port.0.lock().unwrap();
        let notifications = state
            .pushes
            .iter()
            .filter(|(_, bytes)| bytes != b"\r")
            .collect::<Vec<_>>();
        assert_eq!(notifications.len(), expected.len());
        assert!(notifications
            .iter()
            .all(|(handle, bytes)| expected.contains(&handle.as_str())
                && bytes == "[inbox] 1 new messages — run `runner msg read` to view.".as_bytes()));
    }
}

#[test]
fn multi_target_human_messages_keep_recipients_live_and_during_reopen_repair() {
    let f = Fixture::new();
    f.signal(
        "human",
        "human_said",
        serde_json::json!({"text":"live","target":["lead","worker"]}),
    );
    f.router
        .log
        .append(EventDraft::signal(
            None,
            "mission",
            "human",
            "human_said",
            serde_json::json!({"text":"repair","target":["lead","worker"],"delivery":"inbox"}),
        ))
        .unwrap();
    f.router.reconstruct_from_log().unwrap();
    f.router.reconstruct_from_log().unwrap();
    let entries = f.router.log.read_from(0).unwrap();
    let messages = entries
        .iter()
        .filter(|entry| entry.event.kind == EventKind::Message)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    for entry in messages {
        assert_eq!(entry.event.recipients(), &["lead", "worker"]);
        assert!(!entry.event.addresses("qa"));
    }
}

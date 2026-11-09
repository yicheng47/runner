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
    moved: bool,
    unsent: bool,
    held: bool,
    revision: u64,
    turn_started: u64,
    prompted: bool,
    extra_rows: usize,
    ack_on_reserve: Option<Weak<Router>>,
    confirmed: usize,
    rechecks: usize,
    recheck_releases: bool,
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
            DeliveryReadiness::Ready { .. } | DeliveryReadiness::Working
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
                if state.moved {
                    rows.swap(0, 1);
                }
                rows.extend(std::iter::repeat_n(String::new(), state.extra_rows));
                rows
            },
            generation: 1,
            input_revision: (state.revision, 0),
            turn_started: state.turn_started,
            prompted: state.prompted,
            runtime: "fake".into(),
            unsent: state.unsent || state.readiness.get("W") == Some(&DeliveryReadiness::Unsent),
            held: state.held,
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
            || (probe.state(&current.rows) != runner_terminal::delivery_probe::ProbeState::Present
                && !current.unsent)
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
    fn confirm_delivery(&self, _: &str, _: &DeliverySnapshot) {
        self.0.lock().unwrap().confirmed += 1;
    }
    fn recheck_composer(&self, session: &str) {
        let mut state = self.0.lock().unwrap();
        state.rechecks += 1;
        if state.recheck_releases {
            state
                .readiness
                .insert(session.into(), DeliveryReadiness::Ready { idle: true });
        }
    }
    fn register_delivery_listener(&self, _: &str, _: Weak<dyn SessionDeliveryListener>) {}
    fn report_readiness(&self, session: &str) -> DeliveryReadiness {
        match self.delivery_readiness(session) {
            DeliveryReadiness::Working => DeliveryReadiness::Ready { idle: false },
            readiness => readiness,
        }
    }
    fn reserve_report(&self, session: &str) -> Result<Option<u64>> {
        if self.report_readiness(session) != (DeliveryReadiness::Ready { idle: true }) {
            return Ok(None);
        }
        self.0.lock().unwrap().reserved.insert(session.into(), 1);
        Ok(Some(1))
    }
    fn synthesize_report_busy(&self, _: &str) {
        self.0.lock().unwrap().busy += 1;
    }
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
    fn notices(&self) -> usize {
        self.writes()
            .iter()
            .filter(|text| text.as_str() != "\r")
            .count()
    }
    fn warnings(&self) -> Vec<String> {
        self.router
            .log
            .read_from(0)
            .unwrap()
            .into_iter()
            .filter(|entry| {
                entry.event.signal_type.as_ref().map(SignalType::as_str) == Some("mission_warning")
            })
            .map(|entry| entry.event.payload["message"].as_str().unwrap().to_owned())
            .collect()
    }
    fn block_reason(&self) -> Option<String> {
        self.port.0.lock().unwrap().blocked.last()?.reason.clone()
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
fn new_messages_wait_for_the_working_members_idle_transition() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Ready { idle: false });
    f.backlog(1);
    f.after(Duration::from_secs(10 * 60));
    assert!(f.writes().is_empty());
    f.backlog(2);
    assert!(f.writes().is_empty());
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.notices(), 1);
    assert!(f.writes()[0].contains("2 new messages"));
    f.after(SUBMIT_DELAY);
    assert_eq!(f.writes()[1], "\r");
}

#[test]
fn a_working_members_wait_is_a_logged_hold_that_never_warns() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Ready { idle: false });
    assert_eq!(f.dispatcher.hold_reason("worker"), None);
    f.backlog(1);
    assert_eq!(
        f.dispatcher.hold_reason("worker"),
        Some(DeliveryReadiness::Ready { idle: false })
    );
    f.after(Duration::from_secs(10 * 60));
    f.after(Duration::from_secs(10 * 60));
    assert!(f.writes().is_empty());
    assert!(
        f.warnings().is_empty(),
        "a long turn is not a stall: {:?}",
        f.warnings()
    );
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.notices(), 1);
    assert_eq!(f.dispatcher.hold_reason("worker"), None);
}

#[test]
fn a_busy_nothing_confirms_warns_once_after_the_backoff_and_delivers_on_release() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Unconfirmed);
    f.backlog(1);
    assert_eq!(
        f.dispatcher.hold_reason("worker"),
        Some(DeliveryReadiness::Unconfirmed)
    );
    f.after(Duration::from_secs(119));
    assert!(f.warnings().is_empty());
    f.after(Duration::from_secs(1));
    f.after(Duration::from_secs(300));
    assert_eq!(
        f.warnings(),
        ["delivery to @worker is held: it shows busy but its agent has not reported a turn"]
    );
    assert!(f.writes().is_empty());
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.notices(), 1);
    assert_eq!(f.warnings().len(), 1);
    assert_eq!(f.dispatcher.hold_reason("worker"), None);
}

#[test]
fn a_busy_that_the_agent_confirms_in_time_does_not_warn() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Unconfirmed);
    f.backlog(1);
    f.after(Duration::from_secs(100));
    f.readiness(DeliveryReadiness::Ready { idle: false });
    f.backlog(1);
    f.after(Duration::from_secs(10 * 60));
    assert!(f.warnings().is_empty(), "{:?}", f.warnings());
    assert_eq!(
        f.dispatcher.hold_reason("worker"),
        Some(DeliveryReadiness::Ready { idle: false })
    );
}

#[test]
fn a_working_mid_turn_member_gets_each_new_message_at_once_but_no_redelivery() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Working);
    f.backlog(1);
    assert_eq!(f.notices(), 1);
    assert!(f.writes()[0].contains("1 new messages"));
    f.after(SUBMIT_DELAY);
    assert_eq!(f.writes()[1], "\r");
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().busy, 1);
    f.after(Duration::from_secs(10 * 60));
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(
        f.writes().len(),
        2,
        "an unread backlog is not retyped mid-turn"
    );
    assert_eq!(f.dispatcher.hold_reason("worker"), None);
    assert!(f.warnings().is_empty(), "{:?}", f.warnings());
    f.backlog(2);
    assert_eq!(f.notices(), 2);
    assert!(f.writes().last().unwrap().contains("2 new messages"));
}

#[test]
fn confirmation_and_unsent_detection_hold_for_a_working_target() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Working);
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    assert_eq!(f.port.0.lock().unwrap().busy, 0);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(
        f.port.0.lock().unwrap().busy,
        1,
        "a cleared composer confirms the notice"
    );

    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.readiness(DeliveryReadiness::Working);
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.writes().len(), 3, "one more Enter for an ignored one");
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    assert_eq!(f.port.0.lock().unwrap().busy, 0);
    f.backlog(2);
    f.after(Duration::from_secs(10 * 60));
    assert_eq!(f.notices(), 1, "a leftover notice is never retyped over");
}

#[test]
fn cursor_input_keeps_a_seen_notice_paused_past_the_draft_cap_without_enter() {
    let mut f = Fixture::new();
    leave_unsent_notice(&mut f, Leftover::UnsubmittedThenMoved);
    let writes = f.writes();
    f.port.0.lock().unwrap().revision += 1;
    f.backlog(2);
    f.after(Duration::from_secs(11 * 60));
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.writes(), writes);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
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
        f.readiness(DeliveryReadiness::Ready { idle: true });
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
        (true, "worker", vec!["lead", "reviewer"]),
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
fn unsubmitted_gets_one_more_enter_then_only_enters_until_the_rows_clear() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.writes().len(), 3);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    f.backlog(2);
    f.after(Duration::from_secs(60));
    assert_eq!(f.writes().len(), 3, "the reconcile waits out the backoff");
    f.after(Duration::from_secs(60));
    assert_eq!(f.writes()[3..], ["\r"]);
    f.port.0.lock().unwrap().input.clear();
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().busy, 1);
    f.after(SUBMIT_DELAY);
    assert!(f.writes().last().unwrap().contains("2 new messages"));
}

#[derive(Clone, Copy, Debug)]
enum Leftover {
    Unsubmitted,
    UnsubmittedThenMoved,
    MovedDuringConfirm,
    UnverifiedThenVisible,
    UnverifiedTrackerOnly,
}

fn leave_unsent_notice(f: &mut Fixture, leftover: Leftover) {
    f.port.0.lock().unwrap().sticky = true;
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    if matches!(
        leftover,
        Leftover::UnverifiedThenVisible | Leftover::UnverifiedTrackerOnly
    ) {
        f.port.0.lock().unwrap().unknown = true;
    }
    if matches!(leftover, Leftover::MovedDuringConfirm) {
        f.port.0.lock().unwrap().moved = true;
        f.readiness(DeliveryReadiness::Unsent);
    }
    f.after(dispatch::CONFIRM_WINDOW);
    f.after(dispatch::CONFIRM_WINDOW);
    match leftover {
        Leftover::Unsubmitted | Leftover::MovedDuringConfirm => assert_eq!(f.writes().len(), 3),
        Leftover::UnsubmittedThenMoved => {
            assert_eq!(f.writes().len(), 3);
            f.port.0.lock().unwrap().moved = true;
            f.readiness(DeliveryReadiness::Unsent);
        }
        Leftover::UnverifiedThenVisible => {
            assert_eq!(f.writes().len(), 2);
            f.port.0.lock().unwrap().unknown = false;
        }
        Leftover::UnverifiedTrackerOnly => {
            assert_eq!(f.writes().len(), 2);
            f.readiness(DeliveryReadiness::Unsent);
        }
    }
    f.tick(None);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
}

#[test]
fn a_leftover_notice_is_resubmitted_when_the_turn_ends_and_never_retyped() {
    for leftover in [
        Leftover::Unsubmitted,
        Leftover::UnsubmittedThenMoved,
        Leftover::MovedDuringConfirm,
        Leftover::UnverifiedThenVisible,
        Leftover::UnverifiedTrackerOnly,
    ] {
        let mut f = Fixture::new();
        leave_unsent_notice(&mut f, leftover);
        let enters = f.writes().len();
        f.backlog(2);
        f.after(Duration::from_secs(60));
        assert_eq!(f.writes().len(), enters, "{leftover:?}");
        assert_eq!(
            f.block_reason().as_deref(),
            Some("unsubmitted"),
            "{leftover:?}"
        );
        f.port.0.lock().unwrap().sticky = false;
        if matches!(
            leftover,
            Leftover::Unsubmitted | Leftover::UnverifiedThenVisible
        ) {
            f.readiness(DeliveryReadiness::Ready { idle: true });
        }
        f.session(SessionDeliveryEvent::Idle);
        assert_eq!(f.writes()[enters..], ["\r"], "{leftover:?}");
        f.port.0.lock().unwrap().unknown = false;
        f.readiness(DeliveryReadiness::Ready { idle: false });
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.port.0.lock().unwrap().busy, 1, "{leftover:?}");
        assert_eq!(f.port.0.lock().unwrap().confirmed, 1, "{leftover:?}");
        assert_eq!(f.port.0.lock().unwrap().rechecks, 0, "{leftover:?}");
        assert_eq!(f.block_reason(), None, "{leftover:?}");
        assert_eq!(f.notices(), 1, "{leftover:?}");
        assert!(f.warnings().is_empty(), "{leftover:?}");
    }
}

#[test]
fn a_leftover_notice_is_resubmitted_by_the_reconcile_after_the_backoff() {
    for leftover in [
        Leftover::Unsubmitted,
        Leftover::UnsubmittedThenMoved,
        Leftover::MovedDuringConfirm,
        Leftover::UnverifiedThenVisible,
        Leftover::UnverifiedTrackerOnly,
    ] {
        let mut f = Fixture::new();
        leave_unsent_notice(&mut f, leftover);
        let enters = f.writes().len();
        f.after(Duration::from_secs(90));
        assert_eq!(f.writes().len(), enters, "{leftover:?}");
        f.after(Duration::from_secs(30));
        assert_eq!(f.writes()[enters..], ["\r"], "{leftover:?}");
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.port.0.lock().unwrap().busy, 0, "{leftover:?}");
        assert_eq!(
            f.warnings(),
            ["delivery to @worker is held: its inbox notice is still unsubmitted in its terminal"],
            "{leftover:?}"
        );
        f.after(Duration::from_secs(120));
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.writes()[enters..], ["\r", "\r"], "{leftover:?}");
        assert_eq!(f.port.0.lock().unwrap().busy, 0, "{leftover:?}");
        assert_eq!(f.notices(), 1, "{leftover:?}");
        assert_eq!(f.warnings().len(), 1, "one warning per hold");
    }
}

#[test]
fn silence_after_our_own_enter_cannot_drive_a_retry_loop() {
    let mut f = Fixture::new();
    leave_unsent_notice(&mut f, Leftover::Unsubmitted);
    let enters = f.writes().len();
    let mut redrawn = 0;
    for _ in 0..150 {
        // Each Enter redraws the composer, and silence detection goes idle two seconds later.
        let writes = f.writes().len();
        f.after(Duration::from_secs(2));
        if writes > redrawn {
            redrawn = writes;
            f.session(SessionDeliveryEvent::SilenceIdle);
        }
    }
    assert_eq!(f.writes()[enters..], ["\r", "\r"]);
    assert_eq!(f.notices(), 1);
    assert_eq!(f.warnings().len(), 1);
}

#[test]
fn a_reported_turn_end_soon_after_an_ignored_retry_presses_enter_at_once() {
    for (event, immediate) in [
        (SessionDeliveryEvent::Idle, true),
        (SessionDeliveryEvent::SilenceIdle, false),
    ] {
        let mut f = Fixture::new();
        leave_unsent_notice(&mut f, Leftover::Unsubmitted);
        f.session(SessionDeliveryEvent::ReadinessChanged);
        f.after(Duration::from_millis(500));
        assert_eq!(f.writes()[3..], ["\r"], "{event:?}");
        f.after(dispatch::CONFIRM_WINDOW);
        f.after(Duration::from_secs(5));
        f.session(event);
        assert_eq!(f.writes().len(), 4 + immediate as usize, "{event:?}");
        f.after(Duration::from_secs(150));
        assert_eq!(f.writes().len(), 5, "{event:?}");
        assert_eq!(f.notices(), 1, "{event:?}");
    }
}

#[test]
fn a_prompt_over_a_moved_leftover_keeps_it_for_an_enter_after_release() {
    let mut f = Fixture::new();
    leave_unsent_notice(&mut f, Leftover::UnsubmittedThenMoved);
    let enters = f.writes().len();
    f.port.0.lock().unwrap().unsent = true;
    f.readiness(DeliveryReadiness::NeedsYou);
    f.session(SessionDeliveryEvent::ReadinessChanged);
    f.backlog(2);
    f.after(Duration::from_secs(300));
    assert_eq!(f.writes().len(), enters);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    f.port.0.lock().unwrap().unsent = false;
    f.readiness(DeliveryReadiness::Unsent);
    f.session(SessionDeliveryEvent::ReadinessChanged);
    f.after(Duration::from_millis(500));
    assert_eq!(f.writes()[enters..], ["\r"]);
    assert_eq!(f.notices(), 1);
}

#[test]
fn a_person_edit_stops_the_leftovers_enters_but_still_blocks_a_second_notice() {
    let mut f = Fixture::new();
    leave_unsent_notice(&mut f, Leftover::Unsubmitted);
    {
        let mut state = f.port.0.lock().unwrap();
        state.revision += 1;
        state.input.push_str(" and a note");
    }
    f.readiness(DeliveryReadiness::Drafting);
    f.backlog(2);
    f.after(SUBMIT_DELAY);
    f.session(SessionDeliveryEvent::Idle);
    f.after(Duration::from_secs(300));
    assert_eq!(f.writes().len(), 3);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    assert_eq!(
        f.warnings(),
        ["delivery to @worker is held: its inbox notice is still unsubmitted in its terminal"]
    );
    f.port.0.lock().unwrap().input.clear();
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(Duration::from_millis(500));
    assert!(f.writes().last().unwrap().contains("2 new messages"));
}

#[test]
fn unsent_readiness_without_a_leftover_is_never_typed_over() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Unsent);
    f.backlog(1);
    f.session(SessionDeliveryEvent::Idle);
    f.after(Duration::from_secs(150));
    assert!(f.writes().is_empty());
    assert!(f.port.0.lock().unwrap().rechecks > 0);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    assert_eq!(
        f.warnings(),
        ["delivery to @worker is held: its inbox notice is still unsubmitted in its terminal"]
    );
}

#[test]
fn a_stale_unsent_hold_without_a_leftover_is_delivered_once_rechecked() {
    for backlog_while_held in [true, false] {
        let mut f = Fixture::new();
        f.readiness(DeliveryReadiness::Unsent);
        f.port.0.lock().unwrap().recheck_releases = true;
        if backlog_while_held {
            f.backlog(1);
        } else {
            // The turn ends with nothing unread; the next message arrives while it idles.
            f.session(SessionDeliveryEvent::Idle);
            assert_eq!(f.port.0.lock().unwrap().rechecks, 0);
            f.backlog(1);
        }
        assert_eq!(f.port.0.lock().unwrap().rechecks, 1);
        assert_eq!(f.notices(), 1, "backlog_while_held={backlog_while_held}");
        assert_eq!(f.block_reason(), None);
        f.after(SUBMIT_DELAY);
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.port.0.lock().unwrap().confirmed, 1);
        assert!(f.warnings().is_empty());
    }
}

#[test]
fn a_stale_draft_hold_is_rechecked_like_an_unsent_one() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Drafting);
    f.backlog(1);
    assert_eq!(f.port.0.lock().unwrap().rechecks, 1);
    assert!(f.writes().is_empty(), "a real draft still holds the notice");

    f.port.0.lock().unwrap().recheck_releases = true;
    f.session(SessionDeliveryEvent::Idle);
    f.after(Duration::from_secs(60));
    assert_eq!(f.notices(), 1);
}

#[test]
fn a_notice_the_transcript_keeps_stops_holding_once_its_turn_ran() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    {
        let mut state = f.port.0.lock().unwrap();
        state.unsent = true;
        state.held = true;
        state.turn_started += 1;
        state.revision += 1;
    }
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().confirmed, 0, "unverified");
    f.tick(None);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));

    f.port.0.lock().unwrap().unsent = false;
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.block_reason(), None);
}

#[test]
fn a_notice_the_tracker_let_go_of_before_the_check_does_not_hold() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    {
        let mut state = f.port.0.lock().unwrap();
        state.held = true;
        state.turn_started += 1;
        state.revision += 1;
    }
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().confirmed, 0, "unverified");
    f.session(SessionDeliveryEvent::Idle);
    f.after(Duration::from_secs(60));
    assert_eq!(f.block_reason(), None);
}

#[test]
fn a_turn_does_not_take_a_notice_the_tracker_never_held() {
    for before_check in [false, true] {
        let mut f = Fixture::new();
        f.port.0.lock().unwrap().sticky = true;
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        f.port.0.lock().unwrap().revision += 1;
        if before_check {
            f.port.0.lock().unwrap().turn_started += 1;
        }
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(f.port.0.lock().unwrap().confirmed, 0, "{before_check}");
        if !before_check {
            f.port.0.lock().unwrap().turn_started += 1;
        }
        f.session(SessionDeliveryEvent::Idle);
        f.after(Duration::from_secs(60));
        assert_eq!(
            f.block_reason().as_deref(),
            Some("unsubmitted"),
            "{before_check}"
        );
        assert_eq!(f.notices(), 1, "{before_check}");
    }
}

#[test]
fn a_turn_reporting_the_notice_as_its_prompt_confirms_it_despite_a_key() {
    for prompted in [true, false] {
        let mut f = Fixture::new();
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        {
            let mut state = f.port.0.lock().unwrap();
            state.unknown = true;
            state.revision += 1;
            state.turn_started += 1;
            state.prompted = prompted;
        }
        f.after(dispatch::CONFIRM_WINDOW);
        let state = f.port.0.lock().unwrap();
        assert_eq!(
            (state.confirmed, state.busy),
            if prompted { (1, 1) } else { (0, 0) },
            "prompted={prompted}"
        );
    }
}

#[test]
fn a_prompt_reported_after_the_check_releases_a_notice_the_tracker_never_held() {
    let mut f = Fixture::new();
    f.port.0.lock().unwrap().sticky = true;
    f.backlog(1);
    f.after(SUBMIT_DELAY);
    f.port.0.lock().unwrap().revision += 1;
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().confirmed, 0, "unverified");
    f.tick(None);
    assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
    {
        let mut state = f.port.0.lock().unwrap();
        state.turn_started += 1;
        state.prompted = true;
    }
    f.session(SessionDeliveryEvent::Idle);
    assert_eq!(f.block_reason(), None);
}

#[test]
fn only_a_confirmed_attempt_ends_the_sessions_ownership_of_its_notice() {
    for sticky in [false, true] {
        let mut f = Fixture::new();
        f.port.0.lock().unwrap().sticky = sticky;
        f.backlog(1);
        f.after(SUBMIT_DELAY);
        f.after(dispatch::CONFIRM_WINDOW);
        f.after(dispatch::CONFIRM_WINDOW);
        assert_eq!(
            f.port.0.lock().unwrap().confirmed,
            usize::from(!sticky),
            "sticky={sticky}"
        );
    }
}

#[test]
fn a_first_turn_or_pending_start_holds_without_a_warning() {
    for readiness in [DeliveryReadiness::Starting, DeliveryReadiness::Pending] {
        let mut f = Fixture::new();
        f.readiness(readiness);
        f.backlog(1);
        f.after(Duration::from_secs(600));
        assert!(f.writes().is_empty(), "{readiness:?}");
        assert!(f.warnings().is_empty(), "{readiness:?}: {:?}", f.warnings());
    }
}

#[test]
fn a_hold_with_unread_messages_warns_once_after_the_backoff() {
    let mut f = Fixture::new();
    f.readiness(DeliveryReadiness::Drafting);
    f.backlog(1);
    f.after(Duration::from_secs(119));
    assert!(f.warnings().is_empty());
    f.after(Duration::from_secs(1));
    f.after(Duration::from_secs(300));
    assert_eq!(f.warnings().len(), 1);
    f.readiness(DeliveryReadiness::Ready { idle: true });
    f.session(SessionDeliveryEvent::InputCleared);
    f.after(Duration::from_millis(500));
    f.after(SUBMIT_DELAY);
    f.after(dispatch::CONFIRM_WINDOW);
    assert_eq!(f.port.0.lock().unwrap().busy, 1);
    f.readiness(DeliveryReadiness::NeedsYou);
    f.backlog(2);
    f.after(SUBMIT_DELAY);
    f.after(Duration::from_secs(120));
    assert_eq!(
        f.warnings(),
        [
            "delivery to @worker is held: an unsent draft is in its terminal",
            "delivery to @worker is held: it is waiting for a response to its prompt"
        ]
    );
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

fn answered(payload: impl FnOnce(&str) -> serde_json::Value, recover: bool) -> String {
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
        .id
        .clone();
    let payload = payload(&question);
    if recover {
        f.router
            .log
            .append(EventDraft::signal(
                None,
                "mission",
                "human",
                SignalType::new("human_response"),
                payload,
            ))
            .unwrap();
        f.router.reconstruct_from_log().unwrap();
        f.router.reconstruct_from_log().unwrap();
    } else {
        f.signal("human", "human_response", payload);
    }
    let mut messages = f
        .router
        .log
        .read_from(0)
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event.kind == EventKind::Message)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    let answer = messages.remove(0).event;
    assert_eq!(
        answer.recipients().first().map(String::as_str),
        Some("worker")
    );
    assert_eq!(answer.payload["question_id"], question.as_str());
    assert_eq!(f.router.pending_ask_count(), 0);
    answer.payload["text"].as_str().unwrap().to_owned()
}

#[test]
fn an_answer_with_a_note_delivers_the_choice_and_the_note_in_one_message() {
    for recover in [false, true] {
        assert_eq!(
            answered(
                |id| serde_json::json!({"question_id":id,"choice":"yes","note":"Only on Friday","delivery":"inbox"}),
                recover,
            ),
            "Answer to \"continue?\": yes — Only on Friday",
            "recover={recover}"
        );
    }
}

#[test]
fn an_answer_without_a_usable_note_keeps_todays_text() {
    let unusable = [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!("")),
        Some(serde_json::json!(" \n\t ")),
        Some(serde_json::json!(7)),
        Some(serde_json::json!(true)),
        Some(serde_json::json!(["Only on Friday"])),
        Some(serde_json::json!({"text":"Only on Friday"})),
    ];
    for recover in [false, true] {
        for note in &unusable {
            let text = answered(
                |id| {
                    let mut payload =
                        serde_json::json!({"question_id":id,"choice":"yes","delivery":"inbox"});
                    if let Some(note) = note {
                        payload["note"] = note.clone();
                    }
                    payload
                },
                recover,
            );
            assert_eq!(
                text, "Answer to \"continue?\": yes",
                "recover={recover} note={note:?}"
            );
        }
    }
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
fn approval_release_resubmits_but_never_retypes_an_unchanged_notification_even_after_resize() {
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
        f.backlog(2);
        f.after(Duration::from_millis(500));
        assert_eq!(f.writes().len(), 3, "resized={resized}");
        // After a resize only the tracker still recognizes the router's text.
        f.readiness(if resized {
            DeliveryReadiness::Unsent
        } else {
            DeliveryReadiness::Ready { idle: false }
        });
        f.session(SessionDeliveryEvent::ReadinessChanged);
        f.after(Duration::from_millis(500));
        assert_eq!(f.writes()[3..], ["\r"], "resized={resized}");
        f.after(dispatch::CONFIRM_WINDOW);
        f.after(Duration::from_secs(120));
        assert_eq!(f.notices(), 1, "resized={resized}");
        assert_eq!(f.block_reason().as_deref(), Some("unsubmitted"));
        f.after(dispatch::CONFIRM_WINDOW);
        if !resized {
            f.session(SessionDeliveryEvent::InputCleared);
            f.after(Duration::from_millis(500));
            assert_eq!(f.notices(), 1, "a positive probe must retain the pause");
        }
        f.port.0.lock().unwrap().input.clear();
        f.readiness(DeliveryReadiness::Ready { idle: true });
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

mod report {
    use super::super::report::{Line, ReportDispatcher, ReportKind, ReportSource};
    use super::*;

    #[derive(Default)]
    struct Lines {
        closed: Mutex<bool>,
        stale: Mutex<Vec<String>>,
        unknown: Mutex<Vec<String>>,
    }
    impl ReportSource for Lines {
        fn line(&self, _: &str, mission: &str, kind: ReportKind) -> Line {
            if self.stale.lock().unwrap().iter().any(|m| m == mission) {
                Line::Drop
            } else if self.unknown.lock().unwrap().iter().any(|m| m == mission) {
                Line::Wait
            } else {
                Line::Send(format!("[runner] Mission {mission} {kind:?}"))
            }
        }
        fn chat_open(&self, _: &str) -> bool {
            !*self.closed.lock().unwrap()
        }
    }

    struct Reports {
        port: Fake,
        lines: Lines,
        dispatcher: ReportDispatcher,
        now: Instant,
    }
    impl Reports {
        fn new() -> Self {
            Self {
                port: Fake::default(),
                lines: Lines::default(),
                dispatcher: ReportDispatcher::default(),
                now: Instant::now(),
            }
        }
        fn tick(&mut self) -> Option<Instant> {
            self.dispatcher.advance(&self.port, &self.lines, self.now)
        }
        fn wait(&mut self, by: Duration) -> Option<Instant> {
            self.now += by;
            self.tick()
        }
        /// Type, press Enter and confirm one report.
        fn deliver(&mut self) {
            self.tick();
            self.wait(SUBMIT_DELAY);
            self.wait(super::super::dispatch::CONFIRM_WINDOW);
        }
        fn readiness(&self, readiness: DeliveryReadiness) {
            self.port
                .0
                .lock()
                .unwrap()
                .readiness
                .insert("C".into(), readiness);
        }
        fn typed(&self) -> Vec<String> {
            self.port
                .0
                .lock()
                .unwrap()
                .pushes
                .iter()
                .filter(|(_, bytes)| bytes != b"\r")
                .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
                .collect()
        }
        fn enters(&self) -> usize {
            self.port
                .0
                .lock()
                .unwrap()
                .pushes
                .iter()
                .filter(|(_, bytes)| bytes == b"\r")
                .count()
        }
    }

    #[test]
    fn a_report_is_typed_submitted_and_confirmed_once() {
        let mut r = Reports::new();
        r.dispatcher.queue("C", "m1", ReportKind::State);

        assert_eq!(r.tick(), Some(r.now + SUBMIT_DELAY));
        assert_eq!(r.typed(), ["[runner] Mission m1 State"]);
        assert_eq!(r.enters(), 0, "Enter waits for SUBMIT_DELAY");
        r.wait(SUBMIT_DELAY);
        assert_eq!(r.enters(), 1);
        r.wait(super::super::dispatch::CONFIRM_WINDOW);

        let state = r.port.0.lock().unwrap();
        assert_eq!((state.confirmed, state.busy), (1, 1));
        drop(state);
        assert!(r.dispatcher.pending("C").is_empty());
        r.wait(Duration::from_secs(60));
        assert_eq!(r.typed().len(), 1);
    }

    #[test]
    fn a_report_waits_for_an_idle_chat_on_every_runtime() {
        let mut r = Reports::new();
        r.dispatcher.queue("C", "m1", ReportKind::State);
        for held in [
            DeliveryReadiness::Working,
            DeliveryReadiness::Ready { idle: false },
            DeliveryReadiness::Unconfirmed,
            DeliveryReadiness::Drafting,
            DeliveryReadiness::Unsent,
            DeliveryReadiness::NeedsYou,
            DeliveryReadiness::Starting,
            DeliveryReadiness::Unavailable,
        ] {
            r.readiness(held);
            r.wait(Duration::from_secs(3600));
            assert!(r.typed().is_empty(), "{held:?}");
        }
        r.readiness(DeliveryReadiness::RecentlyTyping(Duration::from_secs(2)));
        assert_eq!(r.wait(Duration::ZERO), Some(r.now + Duration::from_secs(2)));
        r.readiness(DeliveryReadiness::Ready { idle: true });
        r.wait(Duration::from_secs(2));
        assert_eq!(r.typed().len(), 1);
    }

    #[test]
    fn reports_go_one_at_a_time_oldest_first_and_a_later_change_replaces_one_waiting() {
        let mut r = Reports::new();
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.dispatcher.queue("C", "m2", ReportKind::Linked);
        r.dispatcher.queue("C", "m1", ReportKind::Linked);
        assert_eq!(
            r.dispatcher.pending("C"),
            [
                ("m1".to_owned(), ReportKind::Linked),
                ("m2".to_owned(), ReportKind::Linked)
            ]
        );

        r.deliver();
        assert_eq!(r.typed(), ["[runner] Mission m1 Linked"]);
        r.readiness(DeliveryReadiness::Ready { idle: false });
        r.wait(Duration::from_secs(60));
        assert_eq!(
            r.typed().len(),
            1,
            "the next report waits for the chat to be idle again"
        );
        r.readiness(DeliveryReadiness::Ready { idle: true });
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.deliver();
        assert_eq!(
            r.typed(),
            ["[runner] Mission m1 Linked", "[runner] Mission m2 Linked"]
        );
        assert!(r.dispatcher.pending("C").is_empty());
    }

    #[test]
    fn an_unsubmitted_line_pauses_the_chat_until_it_is_cleared() {
        let mut r = Reports::new();
        r.port.0.lock().unwrap().sticky = true;
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.deliver();
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(
            r.enters(),
            2,
            "one extra Enter, then the line is unsubmitted"
        );
        for _ in 0..5 {
            r.wait(Duration::from_secs(60));
        }
        assert_eq!(r.typed().len(), 1, "no second line stacks on the first");
        assert_eq!(r.dispatcher.pending("C").len(), 1);

        r.port.0.lock().unwrap().input.clear();
        r.wait(Duration::ZERO);
        assert_eq!(
            r.typed().len(),
            2,
            "cleared without a turn: the report is retried"
        );

        let mut sent = Reports::new();
        sent.port.0.lock().unwrap().sticky = true;
        sent.dispatcher.queue("C", "m1", ReportKind::State);
        sent.deliver();
        sent.wait(super::super::dispatch::CONFIRM_WINDOW);
        {
            let mut state = sent.port.0.lock().unwrap();
            state.input.clear();
            state.turn_started += 1;
        }
        sent.wait(Duration::ZERO);
        assert!(
            sent.dispatcher.pending("C").is_empty(),
            "the person sent the line: the chat has the report"
        );
        assert_eq!(sent.typed().len(), 1);
    }

    #[derive(Clone, Copy, Debug)]
    enum Leftover {
        UnverifiedThenVisible,
        UnverifiedTrackerOnly,
    }

    /// An attempt whose confirmation could not see the composer.
    fn leave_unverified_line(r: &mut Reports, leftover: Leftover) {
        r.port.0.lock().unwrap().sticky = true;
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.tick();
        r.wait(SUBMIT_DELAY);
        r.port.0.lock().unwrap().unknown = true;
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(
            r.enters(),
            1,
            "{leftover:?}: no second Enter without seeing the line"
        );
        match leftover {
            Leftover::UnverifiedThenVisible => r.port.0.lock().unwrap().unknown = false,
            Leftover::UnverifiedTrackerOnly => r.port.0.lock().unwrap().unsent = true,
        }
    }

    #[test]
    fn an_unverified_line_that_is_still_there_is_never_typed_over() {
        for leftover in [
            Leftover::UnverifiedThenVisible,
            Leftover::UnverifiedTrackerOnly,
        ] {
            let mut r = Reports::new();
            leave_unverified_line(&mut r, leftover);
            r.dispatcher.queue("C", "m2", ReportKind::State);
            for _ in 0..5 {
                r.dispatcher
                    .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
                r.wait(Duration::from_secs(60));
            }
            assert_eq!(r.typed().len(), 1, "{leftover:?}");
            assert_eq!(r.dispatcher.pending("C").len(), 2, "{leftover:?}");
            assert_eq!(r.port.0.lock().unwrap().rechecks, 0, "{leftover:?}");

            {
                let mut state = r.port.0.lock().unwrap();
                state.input.clear();
                state.unsent = false;
                state.unknown = false;
            }
            r.wait(Duration::ZERO);
            assert_eq!(
                r.typed(),
                ["[runner] Mission m1 State", "[runner] Mission m1 State"],
                "{leftover:?}: once the line is gone the report is retried"
            );
        }
    }

    #[test]
    fn an_unverified_line_whose_turn_starts_late_is_not_typed_again() {
        for (person_typed, prompted) in [(false, false), (true, false), (true, true)] {
            let mut r = Reports::new();
            r.dispatcher.queue("C", "m1", ReportKind::Linked);
            r.tick();
            r.wait(SUBMIT_DELAY);
            // A native Codex fork's first turn reports after the window (QA run 5).
            r.port.0.lock().unwrap().unknown = true;
            r.wait(super::super::dispatch::CONFIRM_WINDOW);
            assert_eq!(r.port.0.lock().unwrap().confirmed, 0, "unverified");
            // The composer shows the line gone and the chat busy before its turn is reported.
            r.port.0.lock().unwrap().unknown = false;
            r.readiness(DeliveryReadiness::Ready { idle: false });
            r.wait(super::super::dispatch::CONFIRM_WINDOW);
            assert_eq!(r.dispatcher.pending("C").len(), 1);
            {
                let mut state = r.port.0.lock().unwrap();
                state.turn_started += 1;
                state.prompted = prompted;
                if person_typed {
                    state.revision += 1;
                }
            }
            r.dispatcher
                .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
            r.wait(Duration::from_secs(5));
            r.readiness(DeliveryReadiness::Ready { idle: true });
            r.dispatcher
                .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
            r.wait(Duration::from_secs(5));
            if person_typed && !prompted {
                assert_eq!(
                    r.typed().len(),
                    2,
                    "after the person's own input the turn may be theirs: retried"
                );
            } else {
                assert_eq!(r.typed().len(), 1, "the line's own turn: not typed again");
                assert!(r.dispatcher.pending("C").is_empty());
                assert_eq!(r.port.0.lock().unwrap().confirmed, 1);
            }
        }
    }

    #[test]
    fn a_deferred_retry_keeps_the_late_turn_evidence() {
        let mut r = Reports::new();
        r.dispatcher.queue("C", "m1", ReportKind::Linked);
        r.tick();
        r.wait(SUBMIT_DELAY);
        r.port.0.lock().unwrap().unknown = true;
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        r.port.0.lock().unwrap().unknown = false;
        r.readiness(DeliveryReadiness::Ready { idle: false });
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        // A ready pass before the turn shows defers the retry without typing.
        r.lines.unknown.lock().unwrap().push("m1".into());
        r.readiness(DeliveryReadiness::Ready { idle: true });
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.wait(Duration::ZERO);
        assert_eq!(r.typed().len(), 1);
        r.port.0.lock().unwrap().turn_started += 1;
        r.lines.unknown.lock().unwrap().clear();
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.wait(Duration::from_secs(5));
        assert_eq!(r.typed().len(), 1, "not typed again after the deferral");
        assert!(r.dispatcher.pending("C").is_empty());
        assert_eq!(r.port.0.lock().unwrap().confirmed, 1);
    }

    #[test]
    fn a_stale_unsent_hold_without_a_leftover_is_delivered_once_rechecked() {
        let mut r = Reports::new();
        r.readiness(DeliveryReadiness::Unsent);
        r.port.0.lock().unwrap().recheck_releases = true;
        r.dispatcher.queue("C", "m1", ReportKind::State);

        r.tick();
        assert_eq!(r.port.0.lock().unwrap().rechecks, 1);
        assert_eq!(r.typed(), ["[runner] Mission m1 State"]);
        r.wait(SUBMIT_DELAY);
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(r.port.0.lock().unwrap().confirmed, 1);
        assert!(r.dispatcher.pending("C").is_empty());
    }

    /// #1225: Claude Code keeps the submitted line as a transcript row at the composer's row,
    /// which reads as the line still sitting there.
    #[test]
    fn a_line_the_transcript_keeps_stops_holding_the_chat_once_its_turn_ran() {
        let mut r = Reports::new();
        r.port.0.lock().unwrap().sticky = true;
        r.dispatcher.queue("C", "m1", ReportKind::Linked);
        r.tick();
        r.wait(SUBMIT_DELAY);
        {
            let mut state = r.port.0.lock().unwrap();
            state.unsent = true;
            state.held = true;
            state.turn_started += 1;
            // A key the person pressed during the confirmation window.
            state.revision += 1;
        }
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(r.port.0.lock().unwrap().confirmed, 0, "unverified");
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.wait(Duration::from_secs(60));
        assert_eq!(
            r.typed().len(),
            1,
            "held while the tracker still reads the row as the composer"
        );

        // The turn ends with the cursor back at the empty composer.
        r.port.0.lock().unwrap().unsent = false;
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.port.0.lock().unwrap().sticky = false;
        r.deliver();
        assert_eq!(
            r.typed(),
            ["[runner] Mission m1 Linked", "[runner] Mission m1 State"]
        );
        assert!(r.dispatcher.pending("C").is_empty());
    }

    #[test]
    fn a_line_the_tracker_let_go_of_before_the_check_does_not_hold_the_chat() {
        let mut r = Reports::new();
        r.port.0.lock().unwrap().sticky = true;
        r.dispatcher.queue("C", "m1", ReportKind::Linked);
        r.tick();
        r.wait(SUBMIT_DELAY);
        {
            // The tracker held the line, then saw the empty composer before the check.
            let mut state = r.port.0.lock().unwrap();
            state.held = true;
            state.turn_started += 1;
            state.revision += 1;
        }
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(r.port.0.lock().unwrap().confirmed, 0, "unverified");
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.port.0.lock().unwrap().sticky = false;
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        r.wait(SUBMIT_DELAY);
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(
            r.typed(),
            ["[runner] Mission m1 Linked", "[runner] Mission m1 State"]
        );
        assert!(r.dispatcher.pending("C").is_empty());
    }

    #[test]
    fn a_turn_does_not_take_a_line_the_tracker_never_held() {
        // A turn that did not take the line, as from a scheduled wakeup, on either side of the
        // confirmation check.
        for before_check in [false, true] {
            let mut r = Reports::new();
            r.port.0.lock().unwrap().sticky = true;
            r.dispatcher.queue("C", "m1", ReportKind::State);
            r.tick();
            r.wait(SUBMIT_DELAY);
            r.port.0.lock().unwrap().revision += 1;
            if before_check {
                r.port.0.lock().unwrap().turn_started += 1;
            }
            r.wait(super::super::dispatch::CONFIRM_WINDOW);
            assert_eq!(r.port.0.lock().unwrap().confirmed, 0, "{before_check}");
            if !before_check {
                r.port.0.lock().unwrap().turn_started += 1;
            }
            r.dispatcher.queue("C", "m2", ReportKind::State);
            for _ in 0..3 {
                r.dispatcher
                    .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
                r.wait(Duration::from_secs(60));
            }
            assert_eq!(r.typed().len(), 1, "nothing stacks on the line");
            assert_eq!(r.dispatcher.pending("C").len(), 2, "{before_check}");
        }
    }

    /// #1225 L2: a key in the window moved the input revision and neither the probe nor the
    /// tracker could place the line. Only a turn reporting the line as its prompt confirms it.
    #[test]
    fn a_turn_reporting_the_line_as_its_prompt_confirms_it_despite_a_key() {
        for prompted in [true, false] {
            let mut r = Reports::new();
            r.dispatcher.queue("C", "m1", ReportKind::Linked);
            r.tick();
            r.wait(SUBMIT_DELAY);
            {
                let mut state = r.port.0.lock().unwrap();
                state.unknown = true;
                state.revision += 1;
                state.turn_started += 1;
                state.prompted = prompted;
            }
            r.wait(super::super::dispatch::CONFIRM_WINDOW);
            assert_eq!(
                r.port.0.lock().unwrap().confirmed,
                usize::from(prompted),
                "prompted={prompted}"
            );
            if !prompted {
                assert_eq!(r.dispatcher.pending("C").len(), 1);
                continue;
            }
            assert!(r.dispatcher.pending("C").is_empty());
            {
                let mut state = r.port.0.lock().unwrap();
                state.unknown = false;
                state.prompted = false;
            }
            r.dispatcher
                .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
            r.wait(Duration::from_secs(5));
            assert_eq!(r.typed().len(), 1, "not typed again");
            r.dispatcher.queue("C", "m1", ReportKind::State);
            r.deliver();
            assert_eq!(
                r.typed(),
                ["[runner] Mission m1 Linked", "[runner] Mission m1 State"]
            );
        }
    }

    #[test]
    fn a_prompt_reported_after_the_check_releases_a_line_the_tracker_never_held() {
        let mut r = Reports::new();
        r.port.0.lock().unwrap().sticky = true;
        r.dispatcher.queue("C", "m1", ReportKind::Linked);
        r.tick();
        r.wait(SUBMIT_DELAY);
        r.port.0.lock().unwrap().revision += 1;
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert_eq!(r.port.0.lock().unwrap().confirmed, 0, "unverified");
        {
            let mut state = r.port.0.lock().unwrap();
            state.turn_started += 1;
            state.prompted = true;
        }
        r.wait(super::super::dispatch::CONFIRM_WINDOW);
        assert!(r.dispatcher.pending("C").is_empty());
        assert_eq!(r.typed().len(), 1);
    }

    #[test]
    fn a_draft_hold_is_rechecked_and_released_once_the_composer_is_empty() {
        let mut r = Reports::new();
        r.readiness(DeliveryReadiness::Drafting);
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.tick();
        assert_eq!(r.port.0.lock().unwrap().rechecks, 1);
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.wait(Duration::from_secs(60));
        assert!(r.typed().is_empty(), "a real draft still holds the report");
        assert_eq!(r.port.0.lock().unwrap().rechecks, 2);

        r.port.0.lock().unwrap().recheck_releases = true;
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Idle);
        r.deliver();
        assert_eq!(r.typed(), ["[runner] Mission m1 State"]);
        assert!(r.dispatcher.pending("C").is_empty());
    }

    #[test]
    fn a_stopped_chat_receives_its_report_when_it_resumes() {
        let mut r = Reports::new();
        r.readiness(DeliveryReadiness::Unavailable);
        r.dispatcher.queue("C", "m1", ReportKind::State);
        assert_eq!(r.tick(), None, "nothing to time while the chat is stopped");
        assert!(r.typed().is_empty());

        r.readiness(DeliveryReadiness::Ready { idle: true });
        r.dispatcher
            .session_event(&r.port, "C", SessionDeliveryEvent::Respawned);
        r.tick();
        assert_eq!(r.typed().len(), 1);
    }

    #[test]
    fn a_closed_chat_or_a_stale_report_sends_nothing() {
        let mut r = Reports::new();
        r.dispatcher.queue("C", "m1", ReportKind::State);
        r.tick();
        *r.lines.closed.lock().unwrap() = true;
        r.wait(SUBMIT_DELAY);
        assert!(r.dispatcher.pending("C").is_empty());
        assert_eq!(r.enters(), 0);
        assert!(
            r.port.0.lock().unwrap().reserved.is_empty(),
            "the input gate is released"
        );

        let mut stale = Reports::new();
        stale.lines.stale.lock().unwrap().push("m1".into());
        stale.dispatcher.queue("C", "m1", ReportKind::State);
        stale.dispatcher.queue("C", "m2", ReportKind::State);
        stale.tick();
        assert_eq!(stale.typed(), ["[runner] Mission m2 State"]);
        assert_eq!(
            stale.dispatcher.pending("C"),
            [("m2".to_owned(), ReportKind::State)]
        );

        let mut unknown = Reports::new();
        unknown.lines.unknown.lock().unwrap().push("m1".into());
        unknown.dispatcher.queue("C", "m1", ReportKind::Linked);
        assert_eq!(unknown.tick(), Some(unknown.now + Duration::from_secs(1)));
        assert!(unknown.typed().is_empty());
        assert_eq!(
            unknown.dispatcher.pending("C").len(),
            1,
            "a mission not read yet waits"
        );
        unknown.lines.unknown.lock().unwrap().clear();
        unknown.wait(Duration::from_secs(1));
        assert_eq!(unknown.typed(), ["[runner] Mission m1 Linked"]);
    }
}

// Router unit tests. The list mirrors docs/tests/archive/v0-mvp-tests.md C8.
//
// We bypass the event bus entirely here — the router exposes
// `handle_event(&Event)` synchronously so we can drive it with hand-crafted
// envelopes and assert what landed in the recording injector + the log.
// Bus integration is covered separately (mission lifecycle + mission_e2e).

mod asks;
mod delivery_blocked;
mod nudges;
mod reconciliation;
mod reconstruct;
mod restart;
mod status;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use chrono::Utc;
use runner_core::event_log::EventLog;
use runner_core::model::{Event, EventDraft, EventKind, SignalType};

use super::{
    DeliveryBlockedEvent, DeliveryReservation, Router, RouterRegistry, RouterUiNotifier,
    SessionDeliveryEvent, SessionDeliveryListener, StdinInjector,
};
use super::{SessionActivityState, INPUT_CLEAR_FLUSH_GRACE};
use crate::error::Result;
use crate::model::{Role, Slot, SlotWithRole};
use crate::session::manager::InputState;

struct RecordingInjector {
    status_log: Arc<EventLog>,
    activity: Mutex<HashMap<String, super::SessionActivityState>>,
    pushes: Mutex<Vec<(String, Vec<u8>)>>,
    blocked_events: Mutex<Vec<DeliveryBlockedEvent>>,
    /// Optional `dead_session` set simulating a stopped or crashed PTY.
    dead: Mutex<Vec<String>>,
    input: Mutex<HashMap<String, RecordingInputState>>,
    listeners: Mutex<HashMap<String, Vec<Weak<dyn SessionDeliveryListener>>>>,
}

#[derive(Default)]
struct RecordingInputState {
    pending: bool,
    last_input_at: Option<Instant>,
    observed: Option<(InputState, Instant, bool)>,
    in_flight: bool,
    generation: u64,
}

impl RecordingInjector {
    fn new(status_log: Arc<EventLog>) -> Self {
        Self {
            status_log,
            activity: Mutex::new(HashMap::new()),
            pushes: Mutex::new(Vec::new()),
            blocked_events: Mutex::new(Vec::new()),
            dead: Mutex::new(Vec::new()),
            input: Mutex::new(HashMap::new()),
            listeners: Mutex::new(HashMap::new()),
        }
    }

    fn activity_for(&self, session_id: &str) -> Option<super::SessionActivityState> {
        self.activity.lock().unwrap().get(session_id).copied()
    }

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

    fn submitted_bodies_for(&self, session_id: &str) -> Vec<String> {
        self.pushes
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, bytes)| s == session_id && bytes.as_slice() != b"\r")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }

    fn clear_pushes(&self) {
        self.pushes.lock().unwrap().clear();
    }

    fn mark_dead(&self, session_id: &str) {
        self.dead.lock().unwrap().push(session_id.to_string());
    }

    fn set_pending(&self, session_id: &str) {
        let mut input = self.input.lock().unwrap();
        let input = input.entry(session_id.to_string()).or_default();
        input.pending = true;
        input.last_input_at = Some(Instant::now());
    }

    fn set_recent_typing(&self, session_id: &str) {
        let mut input = self.input.lock().unwrap();
        let input = input.entry(session_id.to_string()).or_default();
        input.pending = false;
        input.last_input_at = Some(Instant::now() - Duration::from_millis(1950));
    }

    fn set_observed_input(&self, session_id: &str, state: InputState, composer_visible: bool) {
        let input_cleared = {
            let mut input = self.input.lock().unwrap();
            let input = input.entry(session_id.to_string()).or_default();
            let input_cleared = input.observed.is_some_and(|(previous, _, _)| {
                previous != InputState::Idle && state == InputState::Idle
            });
            input.observed = Some((state, Instant::now(), composer_visible));
            if input_cleared {
                input.last_input_at = None;
            }
            input_cleared
        };
        if input_cleared {
            self.notify(session_id, SessionDeliveryEvent::InputCleared);
        }
    }

    fn set_in_flight(&self, session_id: &str) {
        let mut input = self.input.lock().unwrap();
        input.entry(session_id.to_string()).or_default().in_flight = true;
    }

    fn blocked_events(&self) -> Vec<DeliveryBlockedEvent> {
        self.blocked_events.lock().unwrap().clone()
    }

    fn clear_pending(&self, session_id: &str) {
        if let Some(input) = self.input.lock().unwrap().get_mut(session_id) {
            input.pending = false;
            input.last_input_at = None;
            input.observed = None;
        }
        self.notify(session_id, SessionDeliveryEvent::InputCleared);
    }

    fn respawn(&self, session_id: &str) {
        self.dead.lock().unwrap().retain(|dead| dead != session_id);
        {
            let mut input = self.input.lock().unwrap();
            let input = input.entry(session_id.to_string()).or_default();
            input.generation = input.generation.wrapping_add(1);
            input.in_flight = false;
            input.pending = false;
            input.last_input_at = None;
            input.observed = None;
        }
        self.notify(session_id, SessionDeliveryEvent::Respawned);
    }

    fn exit(&self, session_id: &str) {
        self.mark_dead(session_id);
        if let Some(input) = self.input.lock().unwrap().get_mut(session_id) {
            input.generation = input.generation.wrapping_add(1);
            input.in_flight = false;
            input.pending = false;
            input.last_input_at = None;
        }
        self.notify(session_id, SessionDeliveryEvent::Exited);
    }

    fn notify(&self, session_id: &str, event: SessionDeliveryEvent) {
        let listeners = self
            .listeners
            .lock()
            .unwrap()
            .get(session_id)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for listener in listeners {
            listener.session_delivery_event(session_id, event);
        }
    }
}

impl RouterUiNotifier for RecordingInjector {
    fn delivery_blocked(&self, event: &DeliveryBlockedEvent) {
        self.blocked_events.lock().unwrap().push(event.clone());
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

    fn input_quiescent(&self, session_id: &str) -> bool {
        if self
            .dead
            .lock()
            .unwrap()
            .iter()
            .any(|dead| dead == session_id)
        {
            return false;
        }
        self.input
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|input| {
                let observed_quiescent = match input.observed {
                    Some((InputState::Drafting, _, _)) => false,
                    Some((InputState::Submitted, since, _)) => {
                        since.elapsed() >= Duration::from_secs(2)
                    }
                    Some((InputState::Idle, _, _)) => true,
                    None => !input.pending,
                };
                !input.in_flight
                    && observed_quiescent
                    && input
                        .last_input_at
                        .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
    }

    fn session_live(&self, session_id: &str) -> bool {
        !self
            .dead
            .lock()
            .unwrap()
            .iter()
            .any(|dead| dead == session_id)
    }

    fn reserve_delivery(&self, session_id: &str) -> Result<DeliveryReservation> {
        if self
            .dead
            .lock()
            .unwrap()
            .iter()
            .any(|dead| dead == session_id)
        {
            return Ok(DeliveryReservation::Unavailable);
        }
        let mut input = self.input.lock().unwrap();
        let input = input.entry(session_id.to_string()).or_default();
        if input.in_flight {
            return Ok(DeliveryReservation::InFlight);
        }
        match input.observed {
            Some((InputState::Drafting, _, _)) => {
                return Ok(DeliveryReservation::PendingInput);
            }
            Some((InputState::Submitted, since, _)) => {
                let elapsed = since.elapsed();
                if elapsed < Duration::from_secs(2) {
                    return Ok(DeliveryReservation::RecentlyTyping(
                        Duration::from_secs(2) - elapsed,
                    ));
                }
            }
            Some((InputState::Idle, _, _)) => {}
            None if input.pending => return Ok(DeliveryReservation::PendingInput),
            None => {}
        }
        if let Some(last) = input.last_input_at {
            let elapsed = last.elapsed();
            if elapsed < Duration::from_secs(2) {
                return Ok(DeliveryReservation::RecentlyTyping(
                    Duration::from_secs(2) - elapsed,
                ));
            }
        }
        input.in_flight = true;
        Ok(DeliveryReservation::Ready(input.generation))
    }

    fn inject_reserved(&self, session_id: &str, token: u64, bytes: &[u8]) -> Result<bool> {
        if self
            .dead
            .lock()
            .unwrap()
            .iter()
            .any(|dead| dead == session_id)
        {
            return Ok(false);
        }
        let input = self.input.lock().unwrap();
        let Some(input) = input.get(session_id) else {
            return Ok(false);
        };
        if !input.in_flight || input.generation != token {
            return Ok(false);
        }
        self.pushes
            .lock()
            .unwrap()
            .push((session_id.to_string(), bytes.to_vec()));
        Ok(true)
    }

    fn finish_delivery(&self, session_id: &str, token: u64) {
        let finished = self
            .input
            .lock()
            .unwrap()
            .get_mut(session_id)
            .is_some_and(|input| {
                if input.in_flight && input.generation == token {
                    input.in_flight = false;
                    true
                } else {
                    false
                }
            });
        if finished {
            self.notify(session_id, SessionDeliveryEvent::DeliveryFinished);
        }
    }

    fn synthesize_wake_busy(&self, session_id: &str, draft: EventDraft) -> Result<()> {
        self.status_log.append(draft)?;
        self.activity
            .lock()
            .unwrap()
            .insert(session_id.to_string(), super::SessionActivityState::Busy);
        Ok(())
    }

    fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn SessionDeliveryListener>,
    ) {
        self.input
            .lock()
            .unwrap()
            .entry(session_id.to_string())
            .or_default();
        self.listeners
            .lock()
            .unwrap()
            .entry(session_id.to_string())
            .or_default()
            .push(listener);
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
            added_at: Utc::now(),
        },
        role,
    }
}

/// Build a router around a fresh tempdir log + recording injector. Returns
/// `(router, injector, log, dir)` so tests can inspect everything without
/// re-opening the file. The dir is returned so tempdir cleanup is delayed
/// to test-end (otherwise the log path would be invalidated immediately).
fn fixture(
    roster: Vec<SlotWithRole>,
    sessions: &[(&str, &str)],
) -> (
    Arc<Router>,
    Arc<RecordingInjector>,
    Arc<EventLog>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let injector = Arc::new(RecordingInjector::new(Arc::clone(&log)));
    let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
    let notifier: Arc<dyn RouterUiNotifier> = injector.clone();
    let router = Router::new(
        "mission-1".into(),
        "crew-1".into(),
        "Crew One".into(),
        &roster,
        vec![SignalType::new("mission_goal"), SignalType::new("ask_lead")],
        None,
        log.clone(),
        injector_dyn,
        notifier,
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
    // Lossy so the malformed-line test (which hand-injects a bad NDJSON
    // line to verify reconstruct's tolerance) can still inspect signals.
    let (entries, _skipped) = log.read_from_lossy(0).unwrap();
    entries
        .into_iter()
        .map(|e| e.event)
        .filter(|e| matches!(e.kind, EventKind::Signal))
        .collect()
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    // Deviation from main (CI accommodation): GitHub's shared macOS runners
    // oversleep 5ms ticks several-fold, so the local budgets flunk there.
    // Scaling under CI keeps these tests asserting ordering, not VM speed.
    let timeout = if std::env::var_os("CI").is_some() {
        timeout * 10
    } else {
        timeout
    };
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition did not become true");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn set_unread(router: &Router, handle: &str, unread_count: usize) {
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: handle.into(),
        last_id: None,
        watermark: None,
        unread_count,
    });
}

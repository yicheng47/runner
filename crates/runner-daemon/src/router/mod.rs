// Mission stream routing, subscriber notifications and human-question cards.

pub(crate) mod dispatch;
mod handlers;
pub mod prompt;
pub mod runtime;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::Duration;

use runner_core::event_log::{EventLog, LogEntry};
use runner_core::model::{Event, EventDraft, EventKind, SignalType};
use serde::Serialize;

use crate::error::Result;
use crate::event_bus::{AppendedEvent, BusEmitter, InboxUpdate, WatermarkUpdate};
use crate::events::EventChannel;
use crate::model::SlotWithRole;
use crate::session::manager::SessionManager;

/// Terminal rows and session counters observed for one delivery attempt.
#[derive(Clone, Debug)]
pub struct DeliverySnapshot {
    pub rows: Vec<String>,
    pub generation: u64,
    pub input_revision: (u64, u64),
    pub turn_started: u64,
    pub runtime: String,
    /// The composer still holds the router's own notice, whatever other gate masks readiness.
    pub unsent: bool,
}

/// Session-manager port for readiness, reservations and notification attempts.
pub trait StdinInjector: Send + Sync + 'static {
    fn delivery_readiness(&self, session_id: &str) -> DeliveryReadiness;

    fn reserve_delivery(&self, session_id: &str) -> Result<DeliveryReservation>;

    fn inject_reserved(&self, session_id: &str, token: u64, bytes: &[u8]) -> Result<bool>;

    fn finish_delivery(&self, session_id: &str, token: u64);
    fn delivery_snapshot(&self, session_id: &str) -> Option<DeliverySnapshot>;
    fn resubmit(
        &self,
        session_id: &str,
        snapshot: &DeliverySnapshot,
        probe: &runner_terminal::delivery_probe::InputProbe,
    ) -> Result<bool>;

    fn synthesize_wake_busy(&self, session_id: &str, draft: EventDraft) -> Result<()>;

    /// The notice was submitted, so the session stops holding it as its own unsent text unless
    /// input reached it after `snapshot`.
    fn confirm_delivery(&self, session_id: &str, snapshot: &DeliverySnapshot);

    /// Re-check an idle session's unsent notice against its composer.
    fn recheck_unsent(&self, session_id: &str);

    fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn SessionDeliveryListener>,
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeliveryBlockedEvent {
    pub mission_id: String,
    pub session_id: String,
    pub handle: String,
    pub unread_count: usize,
    pub blocked: bool,
    pub reason: Option<String>,
}

pub trait RouterUiNotifier: Send + Sync + 'static {
    fn delivery_blocked(&self, event: &DeliveryBlockedEvent);
}

pub struct ChannelRouterUiNotifier(pub EventChannel);

impl RouterUiNotifier for ChannelRouterUiNotifier {
    fn delivery_blocked(&self, event: &DeliveryBlockedEvent) {
        self.0.emit("router/delivery-blocked", event);
    }
}

impl StdinInjector for SessionManager {
    fn delivery_readiness(&self, session_id: &str) -> DeliveryReadiness {
        SessionManager::delivery_readiness(self, session_id)
    }

    fn reserve_delivery(&self, session_id: &str) -> Result<DeliveryReservation> {
        SessionManager::reserve_delivery(self, session_id)
    }

    fn inject_reserved(&self, session_id: &str, token: u64, bytes: &[u8]) -> Result<bool> {
        SessionManager::inject_reserved(self, session_id, token, bytes)
    }

    fn finish_delivery(&self, session_id: &str, token: u64) {
        SessionManager::finish_delivery(self, session_id, token)
    }

    fn delivery_snapshot(&self, session_id: &str) -> Option<DeliverySnapshot> {
        SessionManager::delivery_snapshot(self, session_id)
    }
    fn resubmit(
        &self,
        session_id: &str,
        snapshot: &DeliverySnapshot,
        probe: &runner_terminal::delivery_probe::InputProbe,
    ) -> Result<bool> {
        SessionManager::resubmit(self, session_id, snapshot, probe)
    }

    fn synthesize_wake_busy(&self, session_id: &str, draft: EventDraft) -> Result<()> {
        SessionManager::synthesize_wake_busy(self, session_id, draft)
    }

    fn confirm_delivery(&self, session_id: &str, snapshot: &DeliverySnapshot) {
        SessionManager::confirm_delivery(self, session_id, snapshot)
    }

    fn recheck_unsent(&self, session_id: &str) {
        SessionManager::recheck_unsent(self, session_id)
    }

    fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn SessionDeliveryListener>,
    ) {
        SessionManager::register_delivery_listener(self, session_id, listener)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryReadiness {
    Ready {
        idle: bool,
    },
    /// Busy, on a runtime whose composer queues input typed during a turn. A new message may be
    /// typed now; redelivery of an old backlog still waits for idle.
    Working,
    /// Busy on a submit or delivery alone: its agent has reported no turn. The session manager
    /// withdraws such a busy within seconds, so one that lasts is stuck.
    Unconfirmed,
    Pending,
    Starting,
    Drafting,
    /// The router's own notice is still in the composer, unsubmitted.
    Unsent,
    RecentlyTyping(Duration),
    NeedsYou,
    InFlight,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryReservation {
    Unavailable,
    Starting,
    Ready(u64),
    Drafting { composer_visible: bool },
    Unsent,
    LocalInputPending,
    HumanInteraction,
    RecentlyTyping(Duration),
    InFlight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDeliveryEvent {
    Idle,
    /// Idle inferred from output going quiet; the redraw after an ignored Enter produces one.
    SilenceIdle,
    InputCleared,
    ReadinessChanged,
    InputQueueDrained,
    DeliveryFinished,
    Respawned,
    StartupReady,
    Exited,
}

pub trait SessionDeliveryListener: Send + Sync + 'static {
    fn session_delivery_event(&self, session_id: &str, event: SessionDeliveryEvent);
}

pub use crate::session::runtime::SessionActivityState;

pub(crate) struct LaunchInputs {
    roster: Mutex<Vec<RosterRow>>,
}

#[derive(Clone)]
pub(crate) struct RosterRow {
    handle: String,
}

// Windows Codex suppresses Enter after rapid input. Let its paste-burst
// suppression window expire before submitting (#753).
const SUBMIT_DELAY: Duration = Duration::from_millis(if cfg!(windows) { 300 } else { 80 });
#[derive(Default)]
struct RouterState {
    session_by_handle: HashMap<String, String>,
    pending_asks: HashMap<String, PendingAsk>,
    cursor: u64,
    unread_by_handle: HashMap<String, usize>,
    projecting_unread: HashMap<String, usize>,
    changed_backlogs: HashSet<String>,
    backlogs_initialized: bool,
}

#[derive(Clone)]
pub(crate) struct PendingAsk {
    pub asker: Option<String>,
    pub prompt: String,
}

pub(crate) fn pending_asks(entries: &[LogEntry]) -> HashMap<String, PendingAsk> {
    let mut askers = HashMap::new();
    let mut pending = HashMap::new();
    for entry in entries {
        let event = &entry.event;
        if event.kind != EventKind::Signal {
            continue;
        }
        match event.signal_type.as_ref().map(|signal| signal.as_str()) {
            Some("ask_human") => {
                askers.insert(event.id.as_str(), event.from.as_str());
            }
            Some("human_question") => {
                let asker = event.payload["triggered_by"]
                    .as_str()
                    .and_then(|id| askers.get(id))
                    .map(|asker| (*asker).to_owned());
                pending.insert(
                    event.id.clone(),
                    PendingAsk {
                        asker,
                        prompt: event.payload["prompt"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    },
                );
            }
            Some("human_response") => {
                if let Some(id) = event.payload["question_id"].as_str() {
                    pending.remove(id);
                }
            }
            _ => {}
        }
    }
    pending
}

/// One mission's router. Mounted by `mission_start` after sessions spawn,
/// dropped by `mission_stop`. Wired into the event bus as a `BusEmitter`
/// subscriber so `handle_event` runs on every appended envelope.
pub struct Router {
    mission_id: String,
    crew_id: Option<String>,
    log: Arc<EventLog>,
    injector: Arc<dyn StdinInjector>,
    ui_notifier: Arc<dyn RouterUiNotifier>,
    launch: LaunchInputs,
    state: Mutex<RouterState>,
    dispatch_tx: mpsc::Sender<dispatch::Input>,
    dispatch_rx: Mutex<Option<mpsc::Receiver<dispatch::Input>>>,
    driver: Mutex<Option<dispatch::Driver>>,
    weak_self: Weak<Router>,
}

impl Router {
    /// Build a router from the mission's roster. `roster` is the same
    /// slice `mission_start` already loaded for the spawn loop.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mission_id: String,
        crew_id: Option<String>,
        _crew_name: String,
        roster: &[SlotWithRole],
        _allowed_signals: Vec<SignalType>,
        _crew_addendum: Option<String>,
        log: Arc<EventLog>,
        injector: Arc<dyn StdinInjector>,
        ui_notifier: Arc<dyn RouterUiNotifier>,
    ) -> Result<Arc<Self>> {
        let launch = LaunchInputs::new(roster);

        let (dispatch_tx, dispatch_rx) = mpsc::channel();
        Ok(Arc::new_cyclic(|weak_self| Self {
            mission_id,
            crew_id,
            log,
            injector,
            ui_notifier,
            launch,
            state: Mutex::new(RouterState::default()),
            dispatch_tx,
            dispatch_rx: Mutex::new(Some(dispatch_rx)),
            driver: Mutex::new(None),
            weak_self: weak_self.clone(),
        }))
    }

    pub fn add_roster_member(&self, member: &SlotWithRole) {
        self.launch.roster.lock().unwrap().push(RosterRow {
            handle: member.slot.slot_handle.clone(),
        });
    }

    pub fn remove_roster_member(&self, handle: &str) {
        self.launch
            .roster
            .lock()
            .unwrap()
            .retain(|slot| slot.handle != handle);
        {
            let mut state = self.state.lock().unwrap();
            state.session_by_handle.remove(handle);
            state.unread_by_handle.remove(handle);
            state.projecting_unread.remove(handle);
            state.changed_backlogs.remove(handle);
        }
        self.dispatch(dispatch::Input::RosterChanged);
    }

    pub fn register_sessions(&self, sessions: &[(String, String)]) {
        {
            let mut state = self.state.lock().unwrap();
            for (handle, session_id) in sessions {
                state
                    .session_by_handle
                    .insert(handle.clone(), session_id.clone());
            }
        }
        let listener: Weak<dyn SessionDeliveryListener> = self.weak_self.clone();
        for (_, session_id) in sessions {
            self.injector
                .register_delivery_listener(session_id, listener.clone());
        }
        self.dispatch(dispatch::Input::RosterChanged);
    }

    pub fn register_pending_sessions(&self, sessions: &[(String, String)]) {
        self.register_sessions(sessions);
    }

    /// Rebuild projection state without replaying historical notification attempts.
    pub fn reconstruct_from_log(&self) -> Result<()> {
        let (entries, skipped) = self.log.read_from_lossy(0)?;
        for skip in &skipped {
            log::warn!(
                "reconstruct skipping malformed line for mission {} at offset {} ({})",
                self.mission_id,
                skip.offset,
                skip.error
            );
        }
        let cursor = entries
            .iter()
            .map(|entry| entry.next_offset)
            .chain(skipped.iter().map(|skip| skip.next_offset))
            .max()
            .unwrap_or(0);
        let delivered: HashSet<_> = entries
            .iter()
            .filter(|entry| entry.event.kind == EventKind::Message)
            .filter_map(|entry| entry.event.payload["source_event"].as_str())
            .collect();
        for (index, entry) in entries.iter().enumerate() {
            let event = &entry.event;
            if event.kind != EventKind::Signal
                || event.payload["delivery"].as_str() != Some("inbox")
                || delivered.contains(event.id.as_str())
            {
                continue;
            }
            match event.signal_type.as_ref().map(SignalType::as_str) {
                Some("human_said") => handlers::append_human_said(self, event)?,
                Some("human_response") => {
                    if let Some(ask) = event.payload["question_id"]
                        .as_str()
                        .and_then(|id| pending_asks(&entries[..index]).remove(id))
                    {
                        handlers::append_answer(self, event, &ask)?;
                    }
                }
                _ => {}
            }
        }
        let mut state = self.state.lock().unwrap();
        state.pending_asks = pending_asks(&entries);
        state.cursor = cursor;
        Ok(())
    }

    /// Single dispatcher entry point. Bus calls this for every appended
    /// event in arrival order. On reopen, events at-or-below the replay
    /// high-water mark are short-circuited so the bus's initial replay
    /// doesn't re-inject historical stdin or re-emit cards (arch §5.5:
    /// "stdin pushes are deliberately silent" + plan's projection-only
    /// replay). Messages are nudged-only — the message body lives in
    /// the inbox projection per arch §5.5.0; the router just wakes the
    /// recipient with a one-line stdin notification.
    pub fn handle_event(&self, event: &Event, next_offset: u64) {
        {
            let mut state = self.state.lock().unwrap();
            if next_offset <= state.cursor {
                return;
            }
            state.cursor = next_offset;
        }
        match event.kind {
            EventKind::Message => handlers::message_nudge(self, event),
            EventKind::Signal => {
                let Some(signal) = event.signal_type.as_ref() else {
                    return;
                };
                match signal.as_str() {
                    "mission_goal" => handlers::mission_goal(self, event),
                    "human_said" => handlers::human_said(self, event),
                    "ask_human" => handlers::ask_human(self, event),
                    "human_response" => handlers::human_response(self, event),
                    "human_question" => {
                        self.state
                            .lock()
                            .unwrap()
                            .pending_asks
                            .entry(event.id.clone())
                            .or_insert_with(|| PendingAsk {
                                asker: None,
                                prompt: event.payload["prompt"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_owned(),
                            });
                    }
                    // mission_start, mission_stopped, inbox_read,
                    // human_question, mission_warning — observed but
                    // not routed here. inbox_read is owned by the
                    // bus's projection layer; mission_warning /
                    // human_question are events the router itself
                    // emits.
                    _ => {}
                }
            }
        }
    }

    fn dispatch(&self, input: dispatch::Input) {
        let _ = self.dispatch_tx.send(input);
    }

    fn update_inbox(&self, update: &InboxUpdate) {
        self.state
            .lock()
            .unwrap()
            .projecting_unread
            .insert(update.role_handle.clone(), update.unread_count);
    }

    fn update_watermark(&self, update: &WatermarkUpdate) {
        self.update_inbox(&InboxUpdate {
            mission_id: update.mission_id.clone(),
            role_handle: update.role_handle.clone(),
            unread_count: update.unread_count,
            last_id: None,
            watermark: Some(update.watermark.clone()),
        });
    }

    fn backlogs_projected(&self) {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let initial = !state.backlogs_initialized;
            state.backlogs_initialized = true;
            for (handle, count) in std::mem::take(&mut state.projecting_unread) {
                if state.unread_by_handle.insert(handle.clone(), count) != Some(count) {
                    state.changed_backlogs.insert(handle);
                }
            }
            std::mem::take(&mut state.changed_backlogs)
                .into_iter()
                .map(|handle| {
                    let initial_backlog =
                        initial && state.unread_by_handle.get(&handle).copied().unwrap_or(0) > 0;
                    (handle, initial_backlog)
                })
                .collect::<Vec<_>>()
        };
        for (handle, initial_backlog) in changed {
            self.dispatch(dispatch::Input::Backlog {
                handle,
                initial_backlog,
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch_at(
        &self,
        dispatcher: &mut dispatch::Dispatcher,
        now: std::time::Instant,
    ) {
        let rx = self.dispatch_rx.lock().unwrap();
        if let Some(rx) = rx.as_ref() {
            while let Ok(input) = rx.try_recv() {
                dispatcher.advance(self, Some(input), now);
            }
        }
        dispatcher.advance(self, None, now);
    }

    fn start_dispatcher(self: &Arc<Self>) {
        let mut driver = self.driver.lock().unwrap();
        if driver.is_none() {
            if let Some(rx) = self.dispatch_rx.lock().unwrap().take() {
                *driver = Some(dispatch::Driver::start(
                    Arc::downgrade(self),
                    self.dispatch_tx.clone(),
                    rx,
                ));
            }
        }
    }

    fn stop_dispatcher(&self) {
        if let Some(driver) = self.driver.lock().unwrap().take() {
            driver.stop();
        }
    }

    #[cfg(test)]
    pub(crate) fn launch(&self) -> &LaunchInputs {
        &self.launch
    }

    pub(crate) fn record_slot_restart(
        &self,
        handle: &str,
        session_id: &str,
        prior_agent_session_key: Option<&str>,
    ) -> Result<()> {
        self.log.append(EventDraft::signal(
            self.crew_id.as_deref(),
            &self.mission_id,
            "human",
            SignalType::new("slot_restarted"),
            serde_json::json!({"handle": handle, "session_id": session_id,
                "prior_agent_session_key": prior_agent_session_key}),
        ))?;
        let text = format!("@{handle} was restarted by the human and starts over with only its brief. Nothing it was told in this mission survives; re-send the task, branch, and anything else it needs.");
        let rows = self.launch.roster();
        let recipients: Vec<String> = rows
            .iter()
            .filter(|slot| slot.handle() != handle)
            .map(|slot| slot.handle().to_owned())
            .collect();
        for recipient in recipients {
            self.log.append(EventDraft::message(
                self.crew_id.as_deref(),
                &self.mission_id,
                "runner",
                Some(recipient.to_string()),
                &text,
            ))?;
        }
        Ok(())
    }

    pub(crate) fn record_pending_ask(&self, question_id: String, asker: String, prompt: &str) {
        self.state.lock().unwrap().pending_asks.insert(
            question_id,
            PendingAsk {
                asker: Some(asker),
                prompt: prompt.to_owned(),
            },
        );
    }

    pub(crate) fn take_pending_ask(&self, question_id: &str) -> Option<PendingAsk> {
        self.state.lock().unwrap().pending_asks.remove(question_id)
    }

    /// Snapshot of how many `human_question` cards are still waiting for the
    /// human's choice. The Missions list (C11) reads this to flag rows that
    /// have a card the user hasn't answered. Stable for unmounted missions
    /// (router is dropped on `mission_stop`, so the registry returns None
    /// and callers report zero — completed/aborted missions can't accrue
    /// new pending asks).
    pub fn pending_ask_count(&self) -> usize {
        self.state.lock().unwrap().pending_asks.len()
    }

    /// Append a `mission_warning` event when a handler hits an unexpected or
    /// deferred state (unavailable session, unmatched response, bad payload).
    /// Best-effort: a log-write failure here is logged but never panics
    /// the router thread.
    pub(crate) fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        let draft = runner_core::model::EventDraft::signal(
            self.crew_id.as_deref(),
            self.mission_id.clone(),
            "router",
            SignalType::new("mission_warning"),
            serde_json::json!({ "message": message }),
        );
        if let Err(e) = self.log.append(draft) {
            log::error!(
                "failed to append mission_warning for mission {} ({}): {e}",
                self.mission_id,
                message,
            );
        }
    }

    /// Append a `human_question` event for the workspace UI and return its
    /// id. Per arch §5.5.0 the canonical `question_id` is the appended
    /// event's own `id`; `human_response.payload.question_id` references
    /// that. We deliberately do *not* echo `question_id` into the payload —
    /// the spec calls that "echoed here for convenience" and constructing
    /// it would require knowing the id before append. Consumers should
    /// read `event.id`. `triggered_by` ties the card back to the
    /// originating `ask_human` for replay reconstruction and audit.
    pub(crate) fn append_human_question(
        &self,
        ask_human_id: &str,
        prompt: &str,
        choices: &serde_json::Value,
        on_behalf_of: Option<&str>,
    ) -> Option<String> {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "triggered_by".into(),
            serde_json::Value::String(ask_human_id.to_string()),
        );
        payload.insert(
            "prompt".into(),
            serde_json::Value::String(prompt.to_string()),
        );
        payload.insert("choices".into(), choices.clone());
        if let Some(on_behalf_of) = on_behalf_of {
            payload.insert(
                "on_behalf_of".into(),
                serde_json::Value::String(on_behalf_of.to_string()),
            );
        }
        let draft = runner_core::model::EventDraft::signal(
            self.crew_id.as_deref(),
            self.mission_id.clone(),
            "router",
            SignalType::new("human_question"),
            serde_json::Value::Object(payload),
        );
        match self.log.append(draft) {
            Ok(ev) => Some(ev.id),
            Err(e) => {
                log::error!(
                    "failed to append human_question for mission {}: {e}",
                    self.mission_id
                );
                None
            }
        }
    }
}

impl SessionDeliveryListener for Router {
    fn session_delivery_event(&self, session_id: &str, event: SessionDeliveryEvent) {
        self.dispatch(dispatch::Input::Session {
            session_id: session_id.to_owned(),
            event,
        });
    }
}

impl LaunchInputs {
    fn new(roster: &[SlotWithRole]) -> Self {
        Self {
            roster: Mutex::new(
                roster
                    .iter()
                    .map(|member| RosterRow {
                        handle: member.slot.slot_handle.clone(),
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn latest_mission_goal_text(log: &EventLog) -> String {
        let (entries, _skipped) = match log.read_from_lossy(0) {
            Ok(out) => out,
            Err(_) => return String::new(),
        };
        for entry in entries.iter().rev() {
            let ev = &entry.event;
            if !matches!(ev.kind, EventKind::Signal) {
                continue;
            }
            let Some(t) = ev.signal_type.as_ref() else {
                continue;
            };
            if t.as_str() == "mission_goal" {
                return ev
                    .payload
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
            }
        }
        String::new()
    }

    pub(crate) fn roster(&self) -> Vec<RosterRow> {
        self.roster.lock().unwrap().clone()
    }
}

impl RosterRow {
    pub(crate) fn handle(&self) -> &str {
        &self.handle
    }
}

/// `BusEmitter` adapter so the existing `BusRegistry::mount` machinery can
/// drive the router. Appended events feed the dispatcher; inbox and watermark
/// projections publish a backlog snapshot after each complete bus read batch.
pub struct RouterSubscriber(pub Arc<Router>);

impl BusEmitter for RouterSubscriber {
    fn appended(&self, ev: &AppendedEvent) {
        self.0.handle_event(&ev.event, ev.next_offset);
    }
    fn inbox_updated(&self, ev: &InboxUpdate) {
        self.0.update_inbox(ev);
    }
    fn watermark_advanced(&self, ev: &WatermarkUpdate) {
        self.0.update_watermark(ev);
    }
    fn caught_up(&self) {
        self.0.backlogs_projected();
    }
}

/// Fan a single bus emission to multiple subscribers. The bus accepts only
/// one emitter, so `mission_start` wraps the channel emitter and the router
/// in this composite. Each sub-emitter is called in registration order.
pub struct CompositeBusEmitter {
    subs: Vec<Arc<dyn BusEmitter>>,
}

impl CompositeBusEmitter {
    pub fn new(subs: Vec<Arc<dyn BusEmitter>>) -> Self {
        Self { subs }
    }
}

impl BusEmitter for CompositeBusEmitter {
    fn appended(&self, ev: &AppendedEvent) {
        for s in &self.subs {
            s.appended(ev);
        }
    }
    fn inbox_updated(&self, ev: &InboxUpdate) {
        for s in &self.subs {
            s.inbox_updated(ev);
        }
    }
    fn watermark_advanced(&self, ev: &WatermarkUpdate) {
        for s in &self.subs {
            s.watermark_advanced(ev);
        }
    }
    fn caught_up(&self) {
        for s in &self.subs {
            s.caught_up();
        }
    }
}

/// Process-wide registry of live routers, keyed by mission id. Mirrors
/// `event_bus::BusRegistry`.
pub struct RouterRegistry {
    routers: Mutex<HashMap<String, Arc<Router>>>,
}

impl RouterRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            routers: Mutex::new(HashMap::new()),
        })
    }

    pub fn register(&self, mission_id: String, router: Arc<Router>) {
        let previous = self
            .routers
            .lock()
            .unwrap()
            .insert(mission_id.clone(), router.clone());
        if let Some(previous) = previous {
            if !Arc::ptr_eq(&previous, &router) {
                previous.stop_dispatcher();
            }
        }
        router.start_dispatcher();
        log::info!("router mounted: mission={mission_id}");
    }

    pub fn unregister(&self, mission_id: &str) {
        let router = self.routers.lock().unwrap().remove(mission_id);
        if let Some(router) = router {
            router.stop_dispatcher();
            log::info!("router unmounted: mission={mission_id}");
        }
    }

    pub fn get(&self, mission_id: &str) -> Option<Arc<Router>> {
        self.routers.lock().unwrap().get(mission_id).cloned()
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop_dispatcher();
    }
}

/// Convenience for `mission_start`: open the events log Arc once. Both the
/// router (for log appends) and `mission_start`'s opening writes use the
/// same flock-guarded path, so multiple `EventLog` instances are safe.
pub fn open_log_for_mission(mission_dir: &Path) -> Result<Arc<EventLog>> {
    Ok(Arc::new(EventLog::open(mission_dir)?))
}

#[cfg(test)]
mod tests;

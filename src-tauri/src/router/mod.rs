// Signal router v0 — flat parent-process dispatcher.
//
// What this is. The lead runner is the agent that *thinks* about
// coordination — it plans, dispatches workers via directed messages,
// decides when to escalate. The router is the parent-process plumbing
// underneath: bootstrap (write the launch prompt to the lead's stdin on
// `mission_goal`), cross-process stdin push (`ask_lead`, `human_said`,
// `human_response`), the UI bridge (`ask_human` → `human_question` event),
// and the runner-availability map (`runner_status`). See arch §5.5 and
// docs/impls/archive/0001-v0-mvp.md `C8 — Signal router v0`.
//
// What this is not. There is no policy engine, no rule abstraction, no
// per-crew config in MVP. Handlers are a flat `match signal_type { … }`.
// The crew-level prompt layer is `crew.system_prompt_addendum`, spliced
// on mission spawns — not read or acted on here.
//
// Per arch §5.5.0 invariant: messages never trigger router actions. Only
// `EventKind::Signal` reaches the dispatcher; messages flow through the
// inbox projection in `event_bus`.

mod handlers;
pub mod prompt;
pub mod runtime;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use runner_core::event_log::EventLog;
use runner_core::model::{Event, EventKind, SignalType};

use crate::error::Result;
use crate::event_bus::{AppendedEvent, BusEmitter, InboxUpdate, WatermarkUpdate};
use crate::model::SlotWithRunner;
use crate::session::manager::SessionManager;

/// What the router uses to push bytes into a child's PTY. The full
/// `SessionManager` impls it; tests use a recording fake. Lives behind a
/// trait so the router doesn't pull a PTY runtime into unit tests.
pub trait StdinInjector: Send + Sync + 'static {
    /// Raw stdin bytes — used by `inject_and_submit` for the
    /// already-running-agent paths (`human_said`, `ask_lead`).
    /// `\r` becomes Enter, anything else is a literal byte stream.
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()>;

    /// Paste-and-submit for mission lead launch prompts. Writes the
    /// body to the agent's input, waits a short render gap, then
    /// submits with Enter. Earlier versions verified the paste
    /// landed by capturing the pane post-paste — that verification
    /// path was tmux-shaped and went away with the runtime
    /// migration (docs/impls/archive/0011); under the in-process PtyRuntime
    /// xterm.js owns the terminal model and the host has nothing
    /// to capture against. Callers MUST NOT sleep before calling.
    fn inject_paste_with_verify(&self, session_id: &str, body: &[u8]) -> Result<()>;

    /// Snapshot used by diagnostics/tests. Delivery uses the atomic
    /// reservation below so a keystroke cannot race a separate query.
    fn input_quiescent(&self, session_id: &str) -> bool;

    fn session_live(&self, session_id: &str) -> bool;

    fn reserve_delivery(&self, session_id: &str) -> Result<DeliveryReservation>;

    fn inject_reserved(&self, session_id: &str, token: u64, bytes: &[u8]) -> Result<bool>;

    fn finish_delivery(&self, session_id: &str, token: u64);

    fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn SessionDeliveryListener>,
    );
}

impl StdinInjector for SessionManager {
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        SessionManager::inject_stdin(self, session_id, bytes)
    }

    fn inject_paste_with_verify(&self, session_id: &str, body: &[u8]) -> Result<()> {
        SessionManager::inject_paste(self, session_id, body)
    }

    fn input_quiescent(&self, session_id: &str) -> bool {
        SessionManager::input_quiescent(self, session_id)
    }

    fn session_live(&self, session_id: &str) -> bool {
        SessionManager::session_live(self, session_id)
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

    fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn SessionDeliveryListener>,
    ) {
        SessionManager::register_delivery_listener(self, session_id, listener)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryReservation {
    Ready(u64),
    PendingInput,
    RecentlyTyping(Duration),
    InFlight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDeliveryEvent {
    InputCleared,
    InputQueueDrained,
    DeliveryFinished,
    Respawned,
    Exited,
}

pub trait SessionDeliveryListener: Send + Sync + 'static {
    fn session_delivery_event(&self, session_id: &str, event: SessionDeliveryEvent);
}

// `RunnerStatus` now lives in `session::runtime` because the forwarder
// is the authoritative source (issue #124). The router consumes it via
// `runner_status` events the forwarder appends. Agent-reported events
// from the deprecated `runner status` CLI verb feed the same map; both
// converge under latest-wins, so the router doesn't branch on
// `payload.source`.
pub use crate::session::runtime::RunnerStatus;

/// Inputs to the launch-prompt composer, captured at mount so the
/// `mission_goal` handler doesn't have to round-trip the DB. The lead row
/// also doubles as the lead-resolved handle the dispatcher routes to.
/// Fields are pre-merged from (slot, runner template) so the composer
/// doesn't need to know about the join shape.
pub(crate) struct LaunchInputs {
    crew_name: String,
    lead: LeadRow,
    roster: Vec<RosterRow>,
    allowed_signals: Vec<SignalType>,
    /// `crew.system_prompt_addendum` snapshot at mount/resume time
    /// (Layer 2 of the system-prompt stack, #54). Used by
    /// `fire_lead_launch_prompt` on the resume fresh-fallback so the
    /// relaunched lead sees the same composed prompt the first spawn
    /// got. `None` / empty → no splice.
    crew_addendum: Option<String>,
}

pub(crate) struct LeadRow {
    /// `slot.slot_handle` — the in-mission identity. The router
    /// routes by this everywhere; the underlying template handle is
    /// not used in mission contexts.
    handle: String,
    display_name: String,
    /// `runner.system_prompt` — the brief shown in the lead's launch
    /// prompt. Comes from the runner template since system_prompt
    /// isn't yet a per-slot override (deferred).
    system_prompt: Option<String>,
}

pub(crate) struct RosterRow {
    handle: String,
    display_name: String,
    lead: bool,
}

const SUBMIT_DELAY: Duration = Duration::from_millis(80);
const INPUT_CLEAR_FLUSH_GRACE: Duration = Duration::from_millis(500);
const RECONCILIATION_TICK_INTERVAL: Duration = Duration::from_secs(30);
const RECONCILIATION_RENUDGE_BACKOFF: Duration = Duration::from_secs(2 * 60);
const RECONCILIATION_NUDGE: &str = "[inbox] unread messages — run `runner msg read` to view.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeliveryKind {
    InboxNudge,
    Relay,
}

#[derive(Clone)]
struct QueuedDelivery {
    kind: DeliveryKind,
    body: Vec<u8>,
    count: usize,
}

#[derive(Default)]
struct SessionOutbox {
    handle: String,
    deliveries: VecDeque<QueuedDelivery>,
    submit_in_flight: bool,
    retry_scheduled: bool,
    retry_generation: u64,
}

impl SessionOutbox {
    fn enqueue(&mut self, delivery: QueuedDelivery) {
        if delivery.body.is_empty() {
            return;
        }
        if delivery.kind == DeliveryKind::InboxNudge {
            if let Some(existing) = self
                .deliveries
                .iter_mut()
                .find(|queued| queued.kind == DeliveryKind::InboxNudge)
            {
                existing.count += delivery.count;
                existing.body = format!(
                    "[inbox] {} new messages — run `runner msg read` to view.",
                    existing.count
                )
                .into_bytes();
                return;
            }
        }
        self.deliveries.push_back(delivery);
    }
}

/// Mutable per-mission state. Rebuilt on reopen by replaying the log into
/// `reconstruct_from_log` — no separate persistence layer.
#[derive(Default)]
struct RouterState {
    /// Resolved at mount from the spawned `SpawnedSession` rows. The map is
    /// authoritative for the mission's lifetime; if a child crashes the
    /// entry stays so subsequent injections fail visibly with a
    /// `mission_warning` (the desired behavior — better than silently
    /// dropping a `human_response`).
    session_by_handle: HashMap<String, String>,
    /// `human_question.id` → asker handle. Populated when an `ask_human`
    /// is dispatched (the appended card's id is the canonical question_id
    /// per arch §5.5.0) and consumed by the matching `human_response`.
    pending_asks: HashMap<String, String>,
    /// Latest `runner_status` per handle.
    status: HashMap<String, RunnerStatus>,
    /// Replay high-water ULID. Set by `reconstruct_from_log` on reopen;
    /// `handle_event` short-circuits any event whose `id` is `≤` this so
    /// the bus's initial replay doesn't re-inject historical stdin or
    /// re-emit `human_question` cards. `None` for fresh missions: the
    /// opening `mission_goal` event must reach the live dispatcher to
    /// bootstrap the lead.
    replay_high_water: Option<String>,
    outbox_by_session: HashMap<String, SessionOutbox>,
    live_sessions: HashSet<String>,
    unread_by_handle: HashMap<String, usize>,
    last_reconciliation_nudge: HashMap<String, Instant>,
}

struct ReconciliationClock {
    shutdown: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<JoinHandle<()>>,
}

impl ReconciliationClock {
    fn stop(mut self) {
        let (shutdown, wake) = self.shutdown.as_ref();
        *shutdown.lock().unwrap() = true;
        wake.notify_all();
        if let Some(handle) = self.handle.take() {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

/// One mission's router. Mounted by `mission_start` after sessions spawn,
/// dropped by `mission_stop`. Wired into the event bus as a `BusEmitter`
/// subscriber so `handle_event` runs on every appended envelope.
pub struct Router {
    mission_id: String,
    crew_id: String,
    log: Arc<EventLog>,
    injector: Arc<dyn StdinInjector>,
    launch: LaunchInputs,
    state: Mutex<RouterState>,
    reconciliation_clock: Mutex<Option<ReconciliationClock>>,
    weak_self: Weak<Router>,
}

impl Router {
    /// Build a router from the crew's roster and lead. `roster` is the same
    /// slice `mission_start` already loaded for the spawn loop.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mission_id: String,
        crew_id: String,
        crew_name: String,
        roster: &[SlotWithRunner],
        allowed_signals: Vec<SignalType>,
        crew_addendum: Option<String>,
        log: Arc<EventLog>,
        injector: Arc<dyn StdinInjector>,
    ) -> Result<Arc<Self>> {
        let lead = roster
            .iter()
            .find(|m| m.slot.lead)
            .map(|m| LeadRow {
                handle: m.slot.slot_handle.clone(),
                display_name: m.runner.display_name.clone(),
                system_prompt: m.runner.system_prompt.clone(),
            })
            .ok_or_else(|| {
                crate::error::Error::msg(format!("router mount: crew {crew_id} has no lead slot"))
            })?;
        let roster_rows = roster
            .iter()
            .map(|m| RosterRow {
                handle: m.slot.slot_handle.clone(),
                display_name: m.runner.display_name.clone(),
                lead: m.slot.lead,
            })
            .collect();

        Ok(Arc::new_cyclic(|weak_self| Self {
            mission_id,
            crew_id,
            log,
            injector,
            launch: LaunchInputs {
                crew_name,
                lead,
                roster: roster_rows,
                allowed_signals,
                crew_addendum,
            },
            state: Mutex::new(RouterState::default()),
            reconciliation_clock: Mutex::new(None),
            weak_self: weak_self.clone(),
        }))
    }

    /// Register the spawned session ids so handlers can find which PTY
    /// owns each handle. Called once after `mission_start`'s spawn loop
    /// succeeds. Live `mission_start` calls `register_sessions` *before*
    /// the bus mounts so the initial replay's `mission_goal` lands on a
    /// fully-wired router; reopen paths register against existing live
    /// PTYs (when reattach lands) or skip injection (the workspace
    /// surfaces `mission_warning` from `inject_to_handle` either way).
    pub fn register_sessions(&self, sessions: &[(String, String)]) {
        {
            let mut state = self.state.lock().unwrap();
            for (handle, session_id) in sessions {
                state
                    .session_by_handle
                    .insert(handle.clone(), session_id.clone());
                if self.injector.session_live(session_id) {
                    state.live_sessions.insert(session_id.clone());
                }
            }
        }
        let listener: Weak<dyn SessionDeliveryListener> = self.weak_self.clone();
        for (_, session_id) in sessions {
            self.injector
                .register_delivery_listener(session_id, listener.clone());
        }
    }

    /// Reopen path only — fold historical projection state from the log
    /// without firing handler side effects, and set the replay high-water
    /// mark so the subsequent bus mount's initial replay no-ops past it.
    ///
    /// What is rebuilt:
    /// - `pending_asks` from `ask_human` → `human_question` pairs (the
    ///   card id is the canonical `question_id`; we walk in append order
    ///   to match each ask with its following card via
    ///   `human_question.payload.triggered_by`). Asks already answered
    ///   by a `human_response` are removed.
    /// - `runner_status` from the latest `runner_status` row per handle.
    ///
    /// What is *not* rebuilt: stdin pushes. The launch prompt, ask_lead
    /// relays, human_said echoes, and message_nudge fan-outs are all
    /// live-only side effects. Per the C8 plan, replay does not re-inject
    /// prompts into a sleeping LLM.
    ///
    /// **MUST NOT be called for fresh missions.** Setting the watermark
    /// over the just-written opening `mission_goal` would cause the bus
    /// initial replay to no-op the bootstrap injection, leaving the lead
    /// without its launch prompt.
    pub fn reconstruct_from_log(&self) -> Result<()> {
        // Lossy read so a single malformed NDJSON line — a buggy CLI
        // release, a partial write the writer recovered from — doesn't
        // make the whole reopen fail. The bus uses the same forgiveness
        // (`read_from_lossy` in `event_bus::BusState::tick`); reopen must
        // tolerate at least the same set of histories the bus does.
        let (entries, skipped) = self.log.read_from_lossy(0)?;
        for skip in &skipped {
            log::warn!(
                "reconstruct skipping malformed line for mission {} at offset {} ({})",
                self.mission_id,
                skip.offset,
                skip.error,
            );
        }

        // Walk once, building a transient ask_human.id → asker map so we
        // can pair the next human_question with the right asker. Once the
        // pairing lands in pending_asks, the ask_human.id is no longer
        // needed.
        let mut ask_human_asker: HashMap<String, String> = HashMap::new();
        let mut pending: HashMap<String, String> = HashMap::new();
        let mut status: HashMap<String, RunnerStatus> = HashMap::new();
        let mut last_id: Option<String> = None;

        for entry in &entries {
            let event = &entry.event;
            last_id = Some(event.id.clone());
            if !matches!(event.kind, EventKind::Signal) {
                continue;
            }
            let Some(t) = event.signal_type.as_ref() else {
                continue;
            };
            match t.as_str() {
                "ask_human" => {
                    ask_human_asker.insert(event.id.clone(), event.from.clone());
                }
                "human_question" => {
                    let triggered_by = event.payload.get("triggered_by").and_then(|v| v.as_str());
                    if let Some(ask_id) = triggered_by {
                        if let Some(asker) = ask_human_asker.remove(ask_id) {
                            pending.insert(event.id.clone(), asker);
                        }
                    }
                }
                "human_response" => {
                    if let Some(qid) = event.payload.get("question_id").and_then(|v| v.as_str()) {
                        pending.remove(qid);
                    }
                }
                "runner_status" => {
                    let s = match event.payload.get("state").and_then(|v| v.as_str()) {
                        Some("busy") => Some(RunnerStatus::Busy),
                        Some("idle") => Some(RunnerStatus::Idle),
                        _ => None,
                    };
                    if let Some(s) = s {
                        status.insert(event.from.clone(), s);
                    }
                }
                _ => {}
            }
        }

        let mut state = self.state.lock().unwrap();
        state.pending_asks = pending;
        state.status = status;
        state.replay_high_water = last_id;
        Ok(())
    }

    pub fn lead_handle(&self) -> &str {
        &self.launch.lead.handle
    }

    /// Single dispatcher entry point. Bus calls this for every appended
    /// event in arrival order. On reopen, events at-or-below the replay
    /// high-water mark are short-circuited so the bus's initial replay
    /// doesn't re-inject historical stdin or re-emit cards (arch §5.5:
    /// "stdin pushes are deliberately silent" + plan's projection-only
    /// replay). Messages are nudged-only — the message body lives in
    /// the inbox projection per arch §5.5.0; the router just wakes the
    /// recipient with a one-line stdin notification.
    pub fn handle_event(&self, event: &Event) {
        // Watermark check covers both messages and signals in one place
        // — replay must not re-nudge inbox recipients with stale "you
        // have mail" lines they already saw on the original delivery.
        if let Some(w) = self.state.lock().unwrap().replay_high_water.as_deref() {
            if event.id.as_bytes() <= w.as_bytes() {
                return;
            }
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
                    "ask_lead" => handlers::ask_lead(self, event),
                    "ask_human" => handlers::ask_human(self, event),
                    "human_response" => handlers::human_response(self, event),
                    "runner_status" => handlers::runner_status(self, event),
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

    // ---- helpers used by handlers --------------------------------------

    #[allow(dead_code)] // Kept for tests + future single-shot injections.
    pub(crate) fn inject_to_handle(&self, handle: &str, bytes: &[u8]) -> Result<()> {
        let session_id = {
            let state = self.state.lock().unwrap();
            state.session_by_handle.get(handle).cloned()
        };
        let Some(session_id) = session_id else {
            return Err(crate::error::Error::msg(format!(
                "router: no live session for handle @{handle}"
            )));
        };
        self.injector.inject(&session_id, bytes)
    }

    /// Mark a runner as busy when the router is about to wake them via
    /// stdin injection (issue #32). Appends a synthetic `runner_status`
    /// busy event with `from = handle` so the workspace rail projection
    /// (MissionWorkspace.tsx:397-411) keys the badge against the
    /// recipient, and updates router state so back-to-back nudges within
    /// the same task don't churn the log. Skips for the virtual `human`
    /// handle and skips if the recipient is already marked busy.
    ///
    /// Centralized here so the policy applies uniformly to every wake
    /// source: directed/broadcast `message_nudge`, `ask_lead` relay,
    /// `human_said`, `human_response`, and the lead's `mission_goal`
    /// bootstrap.
    ///
    /// Post-issue-#124: the session forwarder also fires `runner_status`
    /// busy on the agent's first response byte. This path remains as a
    /// faster cover for the inject→idle race (we may inject before the
    /// agent has written any byte yet) and as defense against an agent
    /// that stays silent past the forwarder's 750ms threshold —
    /// without this, a slow-to-respond agent could appear `idle` to the
    /// user immediately after a nudge. Latest-wins absorbs the
    /// follow-up forwarder event without churn.
    fn synthesize_wake_busy(&self, handle: &str) {
        if handle == "human" {
            return;
        }
        {
            let state = self.state.lock().unwrap();
            if matches!(state.status.get(handle), Some(RunnerStatus::Busy)) {
                return;
            }
        }
        let draft = runner_core::model::EventDraft::signal(
            self.crew_id.clone(),
            self.mission_id.clone(),
            handle,
            SignalType::new("runner_status"),
            serde_json::json!({ "state": "busy" }),
        );
        if let Err(e) = self.log.append(draft) {
            log::error!(
                "failed to append synthetic runner_status busy for @{handle} on mission {}: {e}",
                self.mission_id,
            );
            return;
        }
        self.set_status(handle.to_string(), RunnerStatus::Busy);
    }

    pub(crate) fn inject_and_submit(&self, handle: &str, body: &[u8]) -> Result<()> {
        self.inject_delivery(handle, body, DeliveryKind::Relay)
            .map(|_| ())
    }

    pub(crate) fn inject_inbox_nudge(&self, handle: &str, body: &[u8]) -> Result<()> {
        self.inject_delivery(handle, body, DeliveryKind::InboxNudge)
            .map(|_| ())
    }

    /// Reserve a clean input box through the delayed Enter, or park the
    /// payload until the session manager reports that local input cleared.
    fn inject_delivery(&self, handle: &str, body: &[u8], kind: DeliveryKind) -> Result<bool> {
        self.inject_delivery_at(handle, body, kind, None)
    }

    fn inject_delivery_at(
        &self,
        handle: &str,
        body: &[u8],
        kind: DeliveryKind,
        reconciliation: Option<(Instant, Duration)>,
    ) -> Result<bool> {
        let delivery = QueuedDelivery {
            kind,
            body: body.to_vec(),
            count: 1,
        };
        // A delayed bare Enter has no payload left to deliver after the
        // user's draft clears, so parking it would create a stray submit.
        let deferable = !delivery.body.is_empty();
        let mut retry = None;
        let (session_id, ready) = {
            let mut state = self.state.lock().unwrap();
            let Some(session_id) = state.session_by_handle.get(handle).cloned() else {
                return Err(crate::error::Error::msg(format!(
                    "router: no live session for handle @{handle}"
                )));
            };
            if let Some((now, backoff)) = reconciliation {
                let delivery_pending = state
                    .outbox_by_session
                    .get(&session_id)
                    .is_some_and(|outbox| outbox.submit_in_flight || !outbox.deliveries.is_empty());
                let backoff_active = state
                    .last_reconciliation_nudge
                    .get(handle)
                    .is_some_and(|last| now.saturating_duration_since(*last) < backoff);
                if !state.live_sessions.contains(&session_id)
                    || state.unread_by_handle.get(handle).copied().unwrap_or(0) == 0
                    || !matches!(state.status.get(handle), Some(RunnerStatus::Idle))
                    || delivery_pending
                    || backoff_active
                {
                    return Ok(false);
                }
            }
            if let Some(outbox) = state.outbox_by_session.get_mut(&session_id) {
                if outbox.submit_in_flight || !outbox.deliveries.is_empty() {
                    outbox.enqueue(delivery);
                    return Ok(true);
                }
            }
            match self.injector.reserve_delivery(&session_id)? {
                DeliveryReservation::Ready(token) => {
                    if let Some((now, _)) = reconciliation {
                        state
                            .last_reconciliation_nudge
                            .insert(handle.to_string(), now);
                    }
                    let outbox = state
                        .outbox_by_session
                        .entry(session_id.clone())
                        .or_default();
                    outbox.handle = handle.to_string();
                    outbox.submit_in_flight = true;
                    (session_id, Some((delivery, token)))
                }
                DeliveryReservation::RecentlyTyping(delay) => {
                    if reconciliation.is_some() {
                        return Ok(false);
                    }
                    if deferable {
                        let outbox = state
                            .outbox_by_session
                            .entry(session_id.clone())
                            .or_default();
                        outbox.handle = handle.to_string();
                        outbox.enqueue(delivery);
                        retry = Some((session_id.clone(), delay));
                    }
                    (session_id, None)
                }
                DeliveryReservation::PendingInput | DeliveryReservation::InFlight => {
                    if reconciliation.is_some() {
                        return Ok(false);
                    }
                    if deferable {
                        let outbox = state
                            .outbox_by_session
                            .entry(session_id.clone())
                            .or_default();
                        outbox.handle = handle.to_string();
                        outbox.enqueue(delivery);
                    }
                    (session_id, None)
                }
            }
        };
        if let Some((session_id, delay)) = retry {
            self.schedule_outbox_retry(session_id, delay);
        }
        if let Some((delivery, token)) = ready {
            if let Err(error) = self.start_reserved_delivery(&session_id, handle, delivery, token) {
                if reconciliation.is_some() {
                    self.state
                        .lock()
                        .unwrap()
                        .last_reconciliation_nudge
                        .remove(handle);
                }
                return Err(error);
            }
        }
        Ok(true)
    }

    fn reconcile_inbox_at(&self, now: Instant, backoff: Duration) -> usize {
        let handles: Vec<String> = self
            .state
            .lock()
            .unwrap()
            .session_by_handle
            .keys()
            .cloned()
            .collect();
        handles
            .into_iter()
            .filter(|handle| {
                match self.inject_delivery_at(
                    handle,
                    RECONCILIATION_NUDGE.as_bytes(),
                    DeliveryKind::InboxNudge,
                    Some((now, backoff)),
                ) {
                    Ok(nudged) => nudged,
                    Err(error) => {
                        log::warn!(
                            "inbox reconciliation nudge to @{handle} on mission {} failed: {error}",
                            self.mission_id
                        );
                        false
                    }
                }
            })
            .count()
    }

    fn start_reconciliation_tick_with_timings(
        self: &Arc<Self>,
        interval: Duration,
        backoff: Duration,
    ) {
        let mut clock = self.reconciliation_clock.lock().unwrap();
        if clock.is_some() {
            return;
        }
        let shutdown = Arc::new((Mutex::new(false), Condvar::new()));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let router = Arc::downgrade(self);
        let mission_id = self.mission_id.clone();
        let handle = std::thread::Builder::new()
            .name(format!("inbox-reconcile-{mission_id}"))
            .spawn(move || {
                let (stopped, wake) = shutdown_for_thread.as_ref();
                loop {
                    let guard = stopped.lock().unwrap();
                    let (guard, _) = wake
                        .wait_timeout_while(guard, interval, |stopped| !*stopped)
                        .unwrap();
                    if *guard {
                        return;
                    }
                    drop(guard);
                    let Some(router) = router.upgrade() else {
                        return;
                    };
                    router.reconcile_inbox_at(Instant::now(), backoff);
                }
            })
            .expect("spawn inbox reconciliation clock");
        *clock = Some(ReconciliationClock {
            shutdown,
            handle: Some(handle),
        });
    }

    fn stop_reconciliation_tick(&self) {
        if let Some(clock) = self.reconciliation_clock.lock().unwrap().take() {
            clock.stop();
        }
    }

    fn set_unread(&self, handle: &str, unread_count: usize) {
        self.state
            .lock()
            .unwrap()
            .unread_by_handle
            .insert(handle.to_string(), unread_count);
    }

    fn update_inbox(&self, update: &InboxUpdate) {
        self.set_unread(&update.runner_handle, update.unread_count);
    }

    fn start_reserved_delivery(
        &self,
        session_id: &str,
        handle: &str,
        delivery: QueuedDelivery,
        token: u64,
    ) -> Result<()> {
        if !delivery.body.is_empty() {
            match self
                .injector
                .inject_reserved(session_id, token, &delivery.body)
            {
                Ok(true) => {}
                Ok(false) => {
                    self.cancel_started_delivery(session_id);
                    self.injector.finish_delivery(session_id, token);
                    return Err(crate::error::Error::msg(format!(
                        "router: session {session_id} changed before delivery"
                    )));
                }
                Err(error) => {
                    self.injector.finish_delivery(session_id, token);
                    return Err(error);
                }
            }
        }
        let injector = Arc::clone(&self.injector);
        let sid = session_id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(SUBMIT_DELAY);
            let _ = injector.inject_reserved(&sid, token, b"\r");
            injector.finish_delivery(&sid, token);
        });
        self.synthesize_wake_busy(handle);
        Ok(())
    }

    fn cancel_started_delivery(&self, session_id: &str) {
        let mut state = self.state.lock().unwrap();
        let Some(outbox) = state.outbox_by_session.get_mut(session_id) else {
            return;
        };
        outbox.submit_in_flight = false;
        if outbox.deliveries.is_empty() {
            state.outbox_by_session.remove(session_id);
        }
    }

    fn flush_outbox(&self, session_id: &str) {
        let handle = {
            let state = self.state.lock().unwrap();
            let Some(outbox) = state.outbox_by_session.get(session_id) else {
                return;
            };
            if outbox.submit_in_flight || outbox.deliveries.is_empty() {
                return;
            }
            outbox.handle.clone()
        };

        let reservation = match self.injector.reserve_delivery(session_id) {
            Ok(reservation) => reservation,
            Err(error) => {
                let dropped = self
                    .state
                    .lock()
                    .unwrap()
                    .outbox_by_session
                    .remove(session_id)
                    .map_or(0, |outbox| outbox.deliveries.len());
                self.warn(format!(
                    "router: dropped {dropped} deferred deliveries for {session_id}: {error}"
                ));
                return;
            }
        };
        match reservation {
            DeliveryReservation::Ready(token) => {
                let delivery = {
                    let mut state = self.state.lock().unwrap();
                    let Some(outbox) = state.outbox_by_session.get_mut(session_id) else {
                        self.injector.finish_delivery(session_id, token);
                        return;
                    };
                    let Some(delivery) = outbox.deliveries.pop_front() else {
                        self.injector.finish_delivery(session_id, token);
                        return;
                    };
                    outbox.submit_in_flight = true;
                    delivery
                };
                if let Err(error) =
                    self.start_reserved_delivery(session_id, &handle, delivery, token)
                {
                    log::warn!("deferred router delivery to {session_id} failed: {error}");
                }
            }
            DeliveryReservation::RecentlyTyping(delay) => {
                self.schedule_outbox_retry(session_id.to_string(), delay);
            }
            DeliveryReservation::PendingInput | DeliveryReservation::InFlight => {}
        }
    }

    fn schedule_outbox_retry(&self, session_id: String, delay: Duration) {
        let generation = {
            let mut state = self.state.lock().unwrap();
            let Some(outbox) = state.outbox_by_session.get_mut(&session_id) else {
                return;
            };
            if outbox.retry_scheduled {
                return;
            }
            outbox.retry_scheduled = true;
            outbox.retry_generation = outbox.retry_generation.wrapping_add(1);
            outbox.retry_generation
        };
        let router = self.weak_self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if let Some(router) = router.upgrade() {
                {
                    let mut state = router.state.lock().unwrap();
                    let Some(outbox) = state.outbox_by_session.get_mut(&session_id) else {
                        return;
                    };
                    if outbox.retry_generation != generation {
                        return;
                    }
                    outbox.retry_scheduled = false;
                }
                router.flush_outbox(&session_id);
            }
        });
    }

    fn schedule_outbox_flush(&self, session_id: String, delay: Duration) {
        let generation = {
            let mut state = self.state.lock().unwrap();
            let Some(outbox) = state.outbox_by_session.get_mut(&session_id) else {
                return;
            };
            if outbox.deliveries.is_empty() {
                return;
            }
            outbox.retry_generation = outbox.retry_generation.wrapping_add(1);
            outbox.retry_scheduled = false;
            outbox.submit_in_flight = false;
            outbox.retry_generation
        };
        let router = self.weak_self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if let Some(router) = router.upgrade() {
                let current = router
                    .state
                    .lock()
                    .unwrap()
                    .outbox_by_session
                    .get(&session_id)
                    .map(|outbox| outbox.retry_generation);
                if current != Some(generation) {
                    return;
                }
                router.flush_outbox(&session_id);
            }
        });
    }

    fn schedule_delivery_cooldown(&self, session_id: String) {
        let generation = {
            let mut state = self.state.lock().unwrap();
            let Some(outbox) = state.outbox_by_session.get_mut(&session_id) else {
                return;
            };
            outbox.retry_generation = outbox.retry_generation.wrapping_add(1);
            outbox.retry_generation
        };
        let router = self.weak_self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SUBMIT_DELAY);
            let Some(router) = router.upgrade() else {
                return;
            };
            {
                let mut state = router.state.lock().unwrap();
                let Some(outbox) = state.outbox_by_session.get_mut(&session_id) else {
                    return;
                };
                if outbox.retry_generation != generation {
                    return;
                }
                outbox.submit_in_flight = false;
            }
            router.flush_outbox(&session_id);
            let mut state = router.state.lock().unwrap();
            if state
                .outbox_by_session
                .get(&session_id)
                .is_some_and(|outbox| !outbox.submit_in_flight && outbox.deliveries.is_empty())
            {
                state.outbox_by_session.remove(&session_id);
            }
        });
    }

    /// Lead launch-prompt injection: routes through the verified
    /// paste-and-submit primitive (`inject_paste_with_verify`) so
    /// the body lands as one bracketed paste and Enter only fires
    /// once the pane confirms the paste rendered. Used for the
    /// lead's `mission_goal`-driven launch prompt and the
    /// resume-fresh-fallback path; both spawn a fresh agent that
    /// races us to bind raw-mode input.
    ///
    /// `delay` controls only thread-vs-inline execution:
    /// non-zero spawns a thread; zero (cfg(test)) runs inline.
    /// **The verified primitive owns pre-paste readiness waiting**
    /// — this method does NOT sleep before calling it, otherwise
    /// the lead path would stack `delay` on top of the verify
    /// loop's own initial_wait. The legacy outer-sleep budget
    /// (`LEAD_LAUNCH_PROMPT_DELAY`) is therefore vestigial post
    /// 0005-first-prompt-readback and should be removed once the
    /// constant has no other readers.
    ///
    /// Resolves the handle → session_id at schedule time. Mission
    /// boot is the only caller and the lead's session is fully
    /// registered before this fires, so the snapshot is stable.
    pub(crate) fn inject_and_submit_delayed(
        &self,
        handle: &str,
        body: Vec<u8>,
        delay: std::time::Duration,
    ) {
        let session_id = {
            let state = self.state.lock().unwrap();
            state.session_by_handle.get(handle).cloned()
        };
        let Some(session_id) = session_id else {
            self.warn(format!(
                "router: no live session for handle @{handle} (delayed submit)"
            ));
            return;
        };
        self.synthesize_wake_busy(handle);
        if body.is_empty() {
            return;
        }

        // Zero-delay path: run inline. Under `cfg(test)`
        // (LEAD_LAUNCH_PROMPT_DELAY = ZERO) the verified primitive's
        // own durations are also zero, so this stays a synchronous
        // millisecond no-op and existing `pushes_for(...)`
        // assertions still observe one body push.
        if delay.is_zero() {
            if let Err(e) = self.injector.inject_paste_with_verify(&session_id, &body) {
                log::error!("inline verified-paste to {session_id} failed: {e}");
            }
            return;
        }
        let injector = Arc::clone(&self.injector);
        std::thread::spawn(move || {
            // No outer sleep — the verified primitive owns the
            // readiness budget (initial_wait + render_wait). Stacking
            // `delay` here would push the lead launch prompt past
            // 4s before the first paste even tries.
            if let Err(e) = injector.inject_paste_with_verify(&session_id, &body) {
                log::error!("delayed verified-paste to {session_id} failed: {e}");
            }
        });
    }

    pub(crate) fn launch(&self) -> &LaunchInputs {
        &self.launch
    }

    /// Read the latest `mission_goal` text from the event log. Used by
    /// `fire_lead_launch_prompt` after a fresh-fallback resume — the
    /// bus's mission_goal handler can't replay (mission_attach's
    /// watermark suppresses it), so we have to compose the launch
    /// prompt ourselves and need the goal payload to feed into it.
    /// Returns an empty string if no goal is found, mirroring the
    /// handler's `unwrap_or("")` defensive read.
    fn latest_mission_goal_text(&self) -> String {
        let (entries, _skipped) = match self.log.read_from_lossy(0) {
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

    /// Compose + inject the lead's launch prompt manually. Same prompt
    /// the bus's `mission_goal` handler would build, but we call this
    /// directly when the resume path detects a missing claude-code
    /// conversation file for a lead slot: the bus can't replay
    /// `mission_goal` on resume (the `mission_attach` watermark
    /// suppresses it), so without this call the lead's freshly-spawned
    /// agent would come up with no system context. Reuses
    /// `inject_and_submit_delayed`'s 2.5s budget so claude-code's TUI
    /// has time to boot before the bytes land.
    pub fn fire_lead_launch_prompt(&self) {
        // Build the prompt body the same way `handlers::mission_goal`
        // does — single source of truth lives in
        // `router::prompt::compose_launch_prompt`, kept in sync via
        // the shared LaunchInputs view.
        let goal = self.latest_mission_goal_text();
        let lead_row = self.launch.lead();
        let roster_entries: Vec<crate::router::prompt::RosterEntry> = self
            .launch
            .roster()
            .iter()
            .map(|r| crate::router::prompt::RosterEntry {
                handle: r.handle(),
                display_name: r.display_name(),
                lead: r.is_lead(),
            })
            .collect();
        let prompt = crate::router::prompt::compose_launch_prompt(
            &crate::router::prompt::LaunchPromptInput {
                lead: crate::router::prompt::LeadView {
                    handle: lead_row.handle(),
                    display_name: lead_row.display_name(),
                    system_prompt: lead_row.system_prompt(),
                },
                crew_name: self.launch.crew_name(),
                mission_goal: &goal,
                roster: &roster_entries,
                allowed_signals: self.launch.allowed_signals(),
                crew_addendum: self.launch.crew_addendum(),
            },
        );
        let body = prompt.trim_end_matches(['\n', '\r']).as_bytes().to_vec();
        self.inject_and_submit_delayed(
            lead_row.handle(),
            body,
            std::time::Duration::from_millis(2500),
        );
    }

    pub(crate) fn record_pending_ask(&self, question_id: String, asker: String) {
        self.state
            .lock()
            .unwrap()
            .pending_asks
            .insert(question_id, asker);
    }

    pub(crate) fn take_pending_ask(&self, question_id: &str) -> Option<String> {
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

    pub(crate) fn set_status(&self, handle: String, status: RunnerStatus) {
        self.state.lock().unwrap().status.insert(handle, status);
    }

    /// Append a `mission_warning` event when a handler hits an unexpected
    /// state (dead session, unmatched `human_response`, malformed payload).
    /// Best-effort: a log-write failure here is logged but never panics
    /// the router thread.
    pub(crate) fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        let draft = runner_core::model::EventDraft::signal(
            self.crew_id.clone(),
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
            self.crew_id.clone(),
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
        match event {
            SessionDeliveryEvent::InputCleared => {
                self.schedule_outbox_flush(session_id.to_string(), INPUT_CLEAR_FLUSH_GRACE);
            }
            SessionDeliveryEvent::InputQueueDrained => self.flush_outbox(session_id),
            SessionDeliveryEvent::DeliveryFinished => {
                self.schedule_delivery_cooldown(session_id.to_string());
            }
            SessionDeliveryEvent::Respawned => {
                let mut state = self.state.lock().unwrap();
                state.live_sessions.insert(session_id.to_string());
                if let Some(outbox) = state.outbox_by_session.get_mut(session_id) {
                    outbox.submit_in_flight = false;
                    outbox.retry_scheduled = false;
                    outbox.retry_generation = outbox.retry_generation.wrapping_add(1);
                }
                drop(state);
                self.flush_outbox(session_id);
            }
            SessionDeliveryEvent::Exited => {
                let mut state = self.state.lock().unwrap();
                state.live_sessions.remove(session_id);
                let dropped = state
                    .outbox_by_session
                    .remove(session_id)
                    .map_or(0, |outbox| outbox.deliveries.len());
                drop(state);
                if dropped > 0 {
                    self.warn(format!(
                        "router: dropped {dropped} deferred deliveries because session {session_id} exited"
                    ));
                }
            }
        }
    }
}

impl LaunchInputs {
    pub(crate) fn crew_name(&self) -> &str {
        &self.crew_name
    }
    pub(crate) fn lead(&self) -> &LeadRow {
        &self.lead
    }
    pub(crate) fn roster(&self) -> &[RosterRow] {
        &self.roster
    }
    pub(crate) fn allowed_signals(&self) -> &[SignalType] {
        &self.allowed_signals
    }
    pub(crate) fn crew_addendum(&self) -> Option<&str> {
        self.crew_addendum.as_deref()
    }
}

impl LeadRow {
    pub(crate) fn handle(&self) -> &str {
        &self.handle
    }
    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }
    pub(crate) fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }
}

impl RosterRow {
    pub(crate) fn handle(&self) -> &str {
        &self.handle
    }
    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }
    pub(crate) fn is_lead(&self) -> bool {
        self.lead
    }
}

/// `BusEmitter` adapter so the existing `BusRegistry::mount` machinery can
/// drive the router. Appended events feed the dispatcher; inbox and watermark
/// projections keep the reconciliation clock's unread snapshot current.
pub struct RouterSubscriber(pub Arc<Router>);

impl BusEmitter for RouterSubscriber {
    fn appended(&self, ev: &AppendedEvent) {
        self.0.handle_event(&ev.event);
    }
    fn inbox_updated(&self, ev: &InboxUpdate) {
        self.0.update_inbox(ev);
    }
    fn watermark_advanced(&self, ev: &WatermarkUpdate) {
        self.0.set_unread(&ev.runner_handle, ev.unread_count);
    }
}

/// Fan a single bus emission to multiple subscribers. The bus accepts only
/// one emitter, so `mission_start` wraps the Tauri emitter and the router
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
        self.register_with_timings(
            mission_id,
            router,
            RECONCILIATION_TICK_INTERVAL,
            RECONCILIATION_RENUDGE_BACKOFF,
        );
    }

    fn register_with_timings(
        &self,
        mission_id: String,
        router: Arc<Router>,
        interval: Duration,
        backoff: Duration,
    ) {
        log::info!("router mounted: mission={mission_id}");
        let previous = self
            .routers
            .lock()
            .unwrap()
            .insert(mission_id, Arc::clone(&router));
        if let Some(previous) = previous {
            if !Arc::ptr_eq(&previous, &router) {
                previous.stop_reconciliation_tick();
            }
        }
        router.start_reconciliation_tick_with_timings(interval, backoff);
    }

    pub fn unregister(&self, mission_id: &str) {
        let router = self.routers.lock().unwrap().remove(mission_id);
        if let Some(router) = router {
            router.stop_reconciliation_tick();
            log::info!("router unmounted: mission={mission_id}");
        }
    }

    #[allow(dead_code)] // Exposed for the future workspace UI bridge.
    pub fn get(&self, mission_id: &str) -> Option<Arc<Router>> {
        self.routers.lock().unwrap().get(mission_id).cloned()
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop_reconciliation_tick();
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

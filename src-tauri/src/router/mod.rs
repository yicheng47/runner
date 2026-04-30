// Signal router v0 — flat parent-process dispatcher.
//
// What this is. The lead runner is the agent that *thinks* about
// coordination — it plans, dispatches workers via directed messages,
// decides when to escalate. The router is the parent-process plumbing
// underneath: bootstrap (write the launch prompt to the lead's stdin on
// `mission_goal`), cross-process stdin push (`ask_lead`, `human_said`,
// `human_response`), the UI bridge (`ask_human` → `human_question` event),
// and the runner-availability map (`runner_status`). See arch §5.5 and
// docs/impls/v0-mvp.md `C8 — Signal router v0`.
//
// What this is not. There is no policy engine, no rule abstraction, no
// per-crew config in MVP. Handlers are a flat `match signal_type { … }`.
// `crews.orchestrator_policy` is reserved for v0.x and is not read here.
//
// Per arch §5.5.0 invariant: messages never trigger router actions. Only
// `EventKind::Signal` reaches the dispatcher; messages flow through the
// inbox projection in `event_bus`.

mod handlers;
pub mod prompt;
pub mod runtime;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
}

impl StdinInjector for SessionManager {
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        SessionManager::inject_stdin(self, session_id, bytes)
    }
}

/// Latest-known availability for a runner. Populated from `runner_status`
/// signals; never inferred from PTY bytes (arch §5.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStatus {
    Busy,
    Idle,
}

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
}

impl Router {
    /// Build a router from the crew's roster and lead. `roster` is the same
    /// slice `mission_start` already loaded for the spawn loop.
    pub fn new(
        mission_id: String,
        crew_id: String,
        crew_name: String,
        roster: &[SlotWithRunner],
        allowed_signals: Vec<SignalType>,
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

        Ok(Arc::new(Self {
            mission_id,
            crew_id,
            log,
            injector,
            launch: LaunchInputs {
                crew_name,
                lead,
                roster: roster_rows,
                allowed_signals,
            },
            state: Mutex::new(RouterState::default()),
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
        let mut state = self.state.lock().unwrap();
        for (handle, session_id) in sessions {
            state
                .session_by_handle
                .insert(handle.clone(), session_id.clone());
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
    /// relays, human_said echoes, and idle nudges are all live-only side
    /// effects. Per the C8 plan, replay does not re-inject prompts into
    /// a sleeping LLM.
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
            eprintln!(
                "router[{}]: reconstruct skipping malformed line at offset {} ({})",
                self.mission_id, skip.offset, skip.error,
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

    /// Inject `body` to the handle's stdin, then send a separate
    /// carriage-return (`\r`) on a brief delay. claude-code's TUI
    /// editor treats `\r` as Enter, but bytes arriving in the same
    /// chunk as the body get appended to the input buffer rather
    /// than triggering submit — so the chord has to land as a
    /// distinct read on the slave end. ~80ms is empirically enough
    /// for the editor to process the body and re-bind its keypress
    /// reader. Body itself is written verbatim; embedded `\n`
    /// characters render as line breaks inside the input box.
    pub(crate) fn inject_and_submit(&self, handle: &str, body: &[u8]) -> Result<()> {
        let session_id = {
            let state = self.state.lock().unwrap();
            state.session_by_handle.get(handle).cloned()
        };
        let Some(session_id) = session_id else {
            return Err(crate::error::Error::msg(format!(
                "router: no live session for handle @{handle}"
            )));
        };
        if !body.is_empty() {
            self.injector.inject(&session_id, body)?;
        }
        let injector = Arc::clone(&self.injector);
        let sid = session_id.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let _ = injector.inject(&sid, b"\r");
        });
        Ok(())
    }

    /// Same contract as `inject_and_submit`, but the whole sequence
    /// (body + delayed `\r`) is deferred by `delay`. Used for the
    /// lead's launch prompt: the bus's initial replay fires
    /// `mission_goal` immediately after the lead PTY spawns, but on
    /// a warm app (mission_reset, fast mission_start) claude-code's
    /// TUI hasn't drawn yet, so synchronous bytes get swallowed by
    /// the boot / trust-folder screen and the lead never sees its
    /// system prompt. The 2.5s budget matches
    /// `SessionManager::schedule_first_prompt`, which solves the
    /// same race for non-lead workers.
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
        let injector = Arc::clone(&self.injector);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if !body.is_empty() {
                if let Err(e) = injector.inject(&session_id, &body) {
                    eprintln!("router: delayed inject to {session_id} failed: {e}");
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(80));
            let _ = injector.inject(&session_id, b"\r");
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
            },
        );
        let body = prompt
            .trim_end_matches(['\n', '\r'])
            .as_bytes()
            .to_vec();
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
            eprintln!(
                "router[{}]: failed to append mission_warning ({}): {e}",
                self.mission_id, message,
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
                eprintln!(
                    "router[{}]: failed to append human_question: {e}",
                    self.mission_id
                );
                None
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
/// drive the router. Only `appended` carries the work; the inbox/watermark
/// methods are no-ops because those are projections owned by the bus.
pub struct RouterSubscriber(pub Arc<Router>);

impl BusEmitter for RouterSubscriber {
    fn appended(&self, ev: &AppendedEvent) {
        self.0.handle_event(&ev.event);
    }
    fn inbox_updated(&self, _ev: &InboxUpdate) {}
    fn watermark_advanced(&self, _ev: &WatermarkUpdate) {}
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
        self.routers.lock().unwrap().insert(mission_id, router);
    }

    pub fn unregister(&self, mission_id: &str) {
        self.routers.lock().unwrap().remove(mission_id);
    }

    #[allow(dead_code)] // Exposed for the future workspace UI bridge.
    pub fn get(&self, mission_id: &str) -> Option<Arc<Router>> {
        self.routers.lock().unwrap().get(mission_id).cloned()
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

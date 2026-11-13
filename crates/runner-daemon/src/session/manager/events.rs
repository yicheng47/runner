use super::*;

/// Inputs the forwarder consumer needs to translate a
/// `RuntimeOutput::StatusTransition` into a real `session_status`
/// event on the mission's NDJSON log (issue #124). All fields are
/// correlated — a mission spawn has all of them; a direct chat has
/// none — so they live together in one optional struct. The
/// forwarder consumer carries an `Option<Self>`: `Some` for mission
/// sessions, `None` for direct chats. See
/// `docs/features/archive/13-pty-silence-idle-detection.md` §Scope for why
/// direct chats are skipped.
///
/// The `EventLog` handle is opened once at construction (on the caller's
/// thread, where a brief blocking flock during tail
/// repair is fine) and cached so the forwarder consumer thread's
/// hot path never calls `EventLog::open` — that path takes a
/// blocking flock to repair any dangling tail, and the forwarder
/// thread also drains terminal output and exit events through the
/// same channel; blocking it would freeze them.
#[derive(Clone)]
pub(crate) struct ForwarderEmitCtx {
    /// `mission.crew_id` — needed for the `EventDraft.crew_id`
    /// field so the appended row matches the event-log schema.
    pub crew_id: Option<String>,
    /// Mission id, redundant with the forwarder's outer
    /// `mission_id` argument but copied here so this struct is
    /// self-contained.
    pub mission_id: String,
    /// `slots.slot_handle` (mission spawns) — the `from` field on
    /// the appended event. The router projects state by `from`,
    /// not by session id.
    pub handle: String,
    /// Cached event-log handle. Constructed via `EventLog::open` on
    /// the spawn/resume path; the forwarder consumer
    /// reuses it for every `try_append` so it never blocks on the
    /// open-time tail-repair flock.
    pub event_log: Arc<EventLog>,
}

/// Open the mission's event log on the calling (non-forwarder)
/// thread. Used by spawn / resume to construct a
/// `ForwarderEmitCtx`. Logs at WARN and returns `None` if the open
/// fails — the forwarder still runs the detector for free; we just
/// can't surface its events.
pub(super) fn open_mission_event_log(
    app_data_dir: &Path,
    crew_id: Option<&str>,
    mission_id: &str,
) -> Option<Arc<EventLog>> {
    let mission_dir = runner_core::event_log::path::mission_dir(app_data_dir, crew_id, mission_id);
    match EventLog::open(&mission_dir) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            log::error!(
                "open event log for mission {mission_id} ({}): {e}",
                mission_dir.display(),
            );
            None
        }
    }
}

/// Outcome of a single forwarder-side `try_append` attempt. Drives
/// the streak counter in the consumer thread (P2 in the @reviewer
/// punch list — see issue #124 comments).
#[derive(Debug)]
pub(super) enum AppendOutcome {
    Ok,
    Contended,
    Failed,
}

impl ForwarderEmitCtx {
    pub(super) fn session_status_draft(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> EventDraft {
        let state_str = match state {
            SessionActivityState::Busy => "busy",
            SessionActivityState::Idle => "idle",
        };
        EventDraft::signal(
            self.crew_id.as_deref(),
            self.mission_id.clone(),
            self.handle.clone(),
            SignalType::new("session_status"),
            serde_json::json!({ "state": state_str, "source": source, "status": status }),
        )
    }

    /// Non-blocking append of a forwarder-emitted `session_status`
    /// row. The consumer thread runs this on every status
    /// transition; it must not block (it shares the mpsc receiver
    /// with the terminal output stream and the exit-event reap, so
    /// a stuck flock would freeze them too). Wire shape mirrors
    /// `crates/runner-cli/src/signal.rs::run_status` so router / UI projections
    /// can't tell the two apart except by `payload.source`.
    pub(super) fn try_append_session_status(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> AppendOutcome {
        match self.try_append_with_retry(self.session_status_draft(state, source, status)) {
            Ok(()) => AppendOutcome::Ok,
            Err(TryAppendError::Contended) => AppendOutcome::Contended,
            Err(TryAppendError::Failed(_)) => AppendOutcome::Failed,
        }
    }

    pub(super) fn try_append_with_retry(
        &self,
        draft: EventDraft,
    ) -> std::result::Result<(), TryAppendError> {
        for attempt in 1..=RUNNER_STATUS_APPEND_MAX_ATTEMPTS {
            match self.event_log.try_append(draft.clone()) {
                Ok(_) => return Ok(()),
                Err(TryAppendError::Contended) if attempt < RUNNER_STATUS_APPEND_MAX_ATTEMPTS => {
                    thread::sleep(RUNNER_STATUS_APPEND_RETRY_DELAY);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded append loop always returns")
    }

    pub(super) fn append_session_status(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> runner_core::Result<()> {
        self.event_log
            .append(self.session_status_draft(state, source, status))
            .map(|_| ())
    }
}

/// Streak indices at which the forwarder consumer logs a WARN about
/// dropped `session_status` events. Picked to cover the common
/// cases (first drop, sustained failure on a stuck mission log)
/// without spamming once it's clear the log is broken.
pub(super) fn drop_streak_is_loggable(streak: u64) -> bool {
    matches!(streak, 1 | 10 | 100 | 1000) || (streak >= 10_000 && streak.is_multiple_of(10_000))
}

/// Decouples the PTY layer from the app event channel so the reader thread
/// can be unit-tested with a fake. Prod uses `CoreSessionEvents`; tests use
/// a no-op or a channel-capture impl.
pub trait SessionEvents: Send + Sync + 'static {
    fn input_error(&self, _id: &str, _message: &str) {}
    /// An OSC 52 write; the text stays in memory and out of logs.
    fn clipboard(
        &self,
        _id: &str,
        _target: runner_terminal::terminal::ClipboardTarget,
        _text: &str,
    ) {
    }
    fn terminal_metadata_changed(&self, _id: &str) {}
    fn spawned(&self, _ev: &SessionSpawnedEvent) {}
    fn fork_started(&self, _ev: &SessionForkStartedEvent) {}
    fn exit(&self, ev: &ExitEvent);
    fn archived(&self, _ev: &SessionUpdatedEvent) {}
    /// Persisted session metadata changed without a lifecycle event
    /// (e.g. async agent_session_key capture). Default no-op so test
    /// fakes don't have to opt in.
    fn updated(&self, _ev: &SessionUpdatedEvent) {}
    /// Live direct-chat activity projection. Mission sessions keep using
    /// `session_status` rows in the mission log instead.
    fn status(&self, _ev: &SessionActivityEvent) {}
    /// Live activity counter for a role — emitted on every spawn/reap so
    /// the Roles list can update its "N sessions / M missions" badges
    /// without polling. Default no-op so test fakes don't have to opt in.
    fn role_activity(&self, _ev: &RoleActivityEvent) {}
    /// Non-fatal, user-facing advisory (resume fallback, etc.). Default
    /// no-op so test fakes don't have to opt in.
    fn warning(&self, _ev: &WarningEvent) {}
}

#[derive(Clone, Default)]
pub struct SessionEventObserverRegistry {
    observer: Arc<RwLock<Option<Weak<dyn SessionEvents>>>>,
    adapter: Arc<Mutex<Option<Arc<dyn SessionEvents>>>>,
}

impl SessionEventObserverRegistry {
    pub fn install(&self, observer: Weak<dyn SessionEvents>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    pub fn install_owned(&self, adapter: Arc<dyn SessionEvents>) {
        self.install(Arc::downgrade(&adapter));
        *self.adapter.lock().unwrap() = Some(adapter);
    }
    fn observer(&self) -> Option<Arc<dyn SessionEvents>> {
        self.observer
            .read()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
    }
}

/// Payload for `role/activity`. Derived from the same query as the role
/// activity operation, so a fresh page load and a live update agree.
#[derive(Debug, Clone, Serialize)]
pub struct RoleActivityEvent {
    pub role_id: String,
    pub handle: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    /// Most recent running direct-chat session id, if any. Mirrors
    /// `RoleActivity::direct_session_id` so the sidebar can re-attach
    /// to a live PTY without an extra round-trip.
    pub direct_session_id: Option<String>,
}

pub use crate::session::runtime::SessionActivityState;

/// Payload for `session/status`. Shared by direct chats and mission sessions, where
/// busy/idle is a live UI projection rather than persisted DB state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionActivityEvent {
    pub session_id: String,
    pub state: SessionActivityState,
    pub source: StatusSource,
    pub status: AgentStatus,
}

/// Lifecycle and metadata emitter. Terminal frames use the manager's
/// per-session subscriber queues.
///
/// Holds the manager as `Weak`: instances get stored inside the manager's
/// own session state (codex capture context, forwarder threads), so a
/// strong ref would create an Arc cycle that keeps both alive past app
/// teardown. `AppCore` owns the strong ref for the process lifetime, so
/// the upgrade only fails during shutdown — where skipping the
/// tab-completion hook is correct anyway.
pub struct CoreSessionEvents {
    db: Arc<DbPool>,
    sessions: std::sync::Weak<SessionManager>,
    windows: Arc<crate::windows::WindowRegistry>,
    events: crate::events::EventChannel,
    observer: SessionEventObserverRegistry,
}

impl CoreSessionEvents {
    pub fn new(
        db: Arc<DbPool>,
        sessions: std::sync::Weak<SessionManager>,
        windows: Arc<crate::windows::WindowRegistry>,
        events: crate::events::EventChannel,
        observer: SessionEventObserverRegistry,
    ) -> Self {
        Self {
            db,
            sessions,
            windows,
            events,
            observer,
        }
    }
}

impl SessionEvents for CoreSessionEvents {
    fn terminal_metadata_changed(&self, id: &str) {
        match self.observer.observer() {
            Some(observer) => {
                observer.updated(&SessionUpdatedEvent {
                    session_id: id.to_owned(),
                    mission_id: None,
                });
            }
            _ => {
                self.events
                    .emit("session/updated", &serde_json::json!({"session_id": id}));
            }
        }
    }
    fn input_error(&self, id: &str, message: &str) {
        self.events.emit(
            "session/input-error",
            &serde_json::json!({ "session_id": id, "message": message }),
        );
    }
    fn clipboard(&self, id: &str, target: runner_terminal::terminal::ClipboardTarget, text: &str) {
        self.events.emit(
            "session/clipboard",
            &serde_json::json!({ "session_id": id, "target": target.as_str(), "text": text }),
        );
    }
    fn spawned(&self, ev: &SessionSpawnedEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.spawned(ev);
        }
        self.events.emit("session/spawned", ev);
    }
    fn fork_started(&self, ev: &SessionForkStartedEvent) {
        self.events.emit("session/fork-started", ev);
    }
    fn exit(&self, ev: &ExitEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.exit(ev);
        }
        self.events.emit("session/exit", ev);
    }
    fn archived(&self, ev: &SessionUpdatedEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.archived(ev);
        }
        self.events.emit("session/archived", ev);
    }
    fn updated(&self, ev: &SessionUpdatedEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.updated(ev);
        }
        self.events.emit("session/updated", ev);
    }
    fn status(&self, ev: &SessionActivityEvent) {
        if ev.source != StatusSource::CodexAttention
            && ev.status.observation.outcome != Some(TurnOutcome::Interrupted)
            && ev.status.observation.outcome != Some(TurnOutcome::Failed)
        {
            if let Some(sessions) = self.sessions.upgrade() {
                let program_status_completion = ev.source == StatusSource::ProgramStatus
                    && ev.state == SessionActivityState::Idle
                    && ev.status.observation.outcome == Some(TurnOutcome::Completed)
                    && sessions.session_state(&ev.session_id).is_some_and(|state| {
                        state.lock().unwrap().model.program_status_authority()
                    });
                let baseline_or_hook_completion = ev.source != StatusSource::ProgramStatus
                    && ev.status.observation.program_status.is_none()
                    && {
                        let native_hook_completion = ev.source == StatusSource::Hook
                            && ev.status.observation.outcome == Some(TurnOutcome::Completed)
                            && sessions.session_state(&ev.session_id).is_some_and(|state| {
                                state.lock().unwrap().model.codex_attention_active()
                            });
                        (ev.state == SessionActivityState::Idle || native_hook_completion)
                            && matches!(
                                (ev.status.observation.activity, ev.status.observation.source),
                                (Activity::Idle, ObservationSource::Baseline)
                                    | (Activity::Ready, _)
                            )
                    };
                if program_status_completion || baseline_or_hook_completion {
                    if let Err(error) = crate::ops::node::record_session_completion(
                        &self.db,
                        &sessions,
                        &self.windows,
                        &self.events,
                        &ev.session_id,
                    ) {
                        log::warn!(
                            "record direct-chat completion for {} failed: {error}",
                            ev.session_id
                        );
                    }
                }
            }
        }
        let mut event = ev.clone();
        if let Some(sessions) = self.sessions.upgrade() {
            event.status = sessions.agent_status(&ev.session_id);
        }
        self.events.emit("session/status", &event);
    }
    fn role_activity(&self, ev: &RoleActivityEvent) {
        self.events.emit("role/activity", ev);
    }
    fn warning(&self, ev: &WarningEvent) {
        self.events.emit("session/warning", ev);
    }
}

/// Test carrier for the pre-existing output assertions.
#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub struct OutputEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    /// Monotonic per-session sequence number used by terminal readiness gates.
    pub seq: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSpawnedEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct SessionForkStartedEvent {
    pub source_session_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExitEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    pub exit_code: Option<i32>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionUpdatedEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
}

/// Non-fatal advisory the UI can render as a banner. Emitted on
/// `session/warning`. When a resumed child exits fast and unsuccessfully,
/// `resume_failed` says its conversation was confirmed missing and the
/// next spawn starts fresh; `resume_exited` says the key was kept for a
/// retry because nothing confirmed the conversation was gone.
#[derive(Debug, Clone, Serialize)]
pub struct WarningEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    /// Stable string the UI can switch on. Free-form strings are
    /// intentional — adding cases shouldn't require a frontend rebuild.
    pub kind: String,
    /// Human-readable detail. Safe to render verbatim.
    pub message: String,
}

pub use runner_core::protocol::session::SpawnedSession;

pub(super) fn emit_role_activity(pool: &DbPool, role: &Role, events: &dyn SessionEvents) {
    let Ok(conn) = pool.get() else { return };
    let activity = crate::repo::role::activity(&conn, &role.id).unwrap_or_default();
    // Count distinct crews this role is wired into via the slots
    // table. Mirrors the cold-path query in
    // `ops::role::role_activity` so live `role/activity`
    // events stay consistent with what the Roles list shows on a
    // refresh.
    events.role_activity(&RoleActivityEvent {
        role_id: role.id.clone(),
        handle: role.handle.clone(),
        active_sessions: activity.active_sessions,
        active_missions: activity.active_missions,
        crew_count: activity.crew_count,
        direct_session_id: activity.direct_session_id,
    });
}

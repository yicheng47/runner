use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_STALL_THRESHOLD: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

static GLOBAL_ACTIVE: AtomicBool = AtomicBool::new(false);
static GLOBAL_TRACKER: RwLock<Option<Weak<WatchdogTracker>>> = RwLock::new(None);

type ClockFn = Arc<dyn Fn() -> Instant + Send + Sync>;
type ProbeSpawner = Arc<dyn Fn(Arc<WatchdogTracker>, u64) + Send + Sync>;

struct PendingProbe {
    id: u64,
    enqueued_at: Instant,
    warned: bool,
}

struct WatchdogState {
    active_turn_depth: usize,
    turn_started_at: Option<Instant>,
    turn_seq: u64,
    turn_warned: bool,
    pending_probe: Option<PendingProbe>,
    next_probe_id: u64,
    last_breadcrumb: &'static str,
    manual_check_requested: bool,
    checks_completed: u64,
    stopped: bool,
    probe_spawner: Option<ProbeSpawner>,
}

pub(crate) struct WatchdogTracker {
    threshold: Duration,
    clock: ClockFn,
    state: Mutex<WatchdogState>,
    wake: Condvar,
}

impl WatchdogTracker {
    pub(crate) fn new(
        threshold: Duration,
        clock: ClockFn,
        probe_spawner: Option<ProbeSpawner>,
    ) -> Arc<Self> {
        Arc::new(Self {
            threshold,
            clock,
            state: Mutex::new(WatchdogState {
                active_turn_depth: 0,
                turn_started_at: None,
                turn_seq: 0,
                turn_warned: false,
                pending_probe: None,
                next_probe_id: 0,
                last_breadcrumb: "startup",
                manual_check_requested: false,
                checks_completed: 0,
                stopped: false,
                probe_spawner,
            }),
            wake: Condvar::new(),
        })
    }

    pub(crate) fn enter_turn(self: &Arc<Self>, breadcrumb: &'static str) -> TurnGuard {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap();
        state.last_breadcrumb = breadcrumb;
        if state.active_turn_depth == 0 {
            state.turn_seq = state.turn_seq.wrapping_add(1);
            state.turn_started_at = Some(now);
            state.turn_warned = false;
        }
        state.active_turn_depth += 1;
        TurnGuard {
            tracker: Some(Arc::clone(self)),
        }
    }

    pub(crate) fn record_breadcrumb(&self, breadcrumb: &'static str) {
        self.state.lock().unwrap().last_breadcrumb = breadcrumb;
    }

    fn exit_turn(&self) {
        let now = (self.clock)();
        let recovery = {
            let mut state = self.state.lock().unwrap();
            if state.active_turn_depth == 0 {
                return;
            }
            state.active_turn_depth -= 1;
            if state.active_turn_depth > 0 {
                return;
            }
            let already_warned = std::mem::replace(&mut state.turn_warned, false);
            state.turn_started_at.take().and_then(|started| {
                let elapsed = now.saturating_duration_since(started);
                (already_warned || elapsed >= self.threshold).then_some((
                    elapsed,
                    state.turn_seq,
                    state.last_breadcrumb,
                ))
            })
        };
        if let Some((elapsed, turn_seq, breadcrumb)) = recovery {
            tracing::warn!(
                recovered_after_ms = elapsed.as_millis() as u64,
                turn_seq = turn_seq,
                source = "turn",
                breadcrumb = breadcrumb,
                "main-thread stall recovered"
            );
        }
    }

    pub(crate) fn complete_probe(&self, probe_id: u64) {
        let now = (self.clock)();
        let recovery = {
            let mut state = self.state.lock().unwrap();
            if !state
                .pending_probe
                .as_ref()
                .is_some_and(|probe| probe.id == probe_id)
            {
                return;
            }
            let probe = state.pending_probe.take().unwrap();
            let elapsed = now.saturating_duration_since(probe.enqueued_at);
            (probe.warned || elapsed >= self.threshold).then_some((
                elapsed,
                probe.id,
                state.last_breadcrumb,
            ))
        };
        if let Some((elapsed, probe_seq, breadcrumb)) = recovery {
            tracing::warn!(
                recovered_after_ms = elapsed.as_millis() as u64,
                turn_seq = probe_seq,
                source = "heartbeat_probe",
                breadcrumb = breadcrumb,
                "main-thread stall recovered"
            );
        }
    }

    fn inspect_once(self: &Arc<Self>) {
        let now = (self.clock)();
        let mut warnings: Vec<(Duration, u64, &'static str, &'static str)> = Vec::new();
        let mut probe_to_spawn: Option<(ProbeSpawner, u64)> = None;
        {
            let mut state = self.state.lock().unwrap();
            if state.stopped {
                return;
            }
            if let Some(turn_started) = state.turn_started_at {
                let elapsed = now.saturating_duration_since(turn_started);
                if elapsed >= self.threshold && !state.turn_warned {
                    state.turn_warned = true;
                    warnings.push((elapsed, state.turn_seq, "turn", state.last_breadcrumb));
                }
            }
            let breadcrumb = state.last_breadcrumb;
            if let Some(probe) = state.pending_probe.as_mut() {
                let elapsed = now.saturating_duration_since(probe.enqueued_at);
                if elapsed >= self.threshold && !probe.warned {
                    probe.warned = true;
                    warnings.push((elapsed, probe.id, "heartbeat_probe", breadcrumb));
                }
            } else if state.active_turn_depth == 0 {
                if let Some(spawner) = state.probe_spawner.clone() {
                    state.next_probe_id = state.next_probe_id.wrapping_add(1);
                    let probe_id = state.next_probe_id;
                    state.pending_probe = Some(PendingProbe {
                        id: probe_id,
                        enqueued_at: now,
                        warned: false,
                    });
                    probe_to_spawn = Some((spawner, probe_id));
                }
            }
        }
        for (elapsed, seq, source, breadcrumb) in warnings {
            tracing::warn!(
                stalled_ms = elapsed.as_millis() as u64,
                threshold_ms = self.threshold.as_millis() as u64,
                turn_seq = seq,
                source = source,
                breadcrumb = breadcrumb,
                "main-thread turn stalled"
            );
        }
        if let Some((spawner, probe_id)) = probe_to_spawn {
            spawner(Arc::clone(self), probe_id);
        }
    }
}

pub(crate) struct TurnGuard {
    tracker: Option<Arc<WatchdogTracker>>,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if let Some(tracker) = self.tracker.take() {
            tracker.exit_turn();
        }
    }
}

pub(crate) fn enter_turn(breadcrumb: &'static str) -> TurnGuard {
    if !GLOBAL_ACTIVE.load(Ordering::Relaxed) {
        return TurnGuard { tracker: None };
    }
    let tracker = GLOBAL_TRACKER
        .read()
        .unwrap()
        .as_ref()
        .and_then(Weak::upgrade);
    match tracker {
        Some(tracker) => tracker.enter_turn(breadcrumb),
        None => TurnGuard { tracker: None },
    }
}

pub(crate) fn breadcrumb(breadcrumb: &'static str) {
    if !GLOBAL_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    if let Some(tracker) = GLOBAL_TRACKER
        .read()
        .unwrap()
        .as_ref()
        .and_then(Weak::upgrade)
    {
        tracker.record_breadcrumb(breadcrumb);
    }
}

pub(crate) struct StallWatchdogGuard {
    tracker: Arc<WatchdogTracker>,
    worker: Option<JoinHandle<()>>,
    registered_global: bool,
}

impl StallWatchdogGuard {
    #[cfg(not(test))]
    pub(crate) fn start_for_app(foreground: gpui::ForegroundExecutor) -> Self {
        use futures::StreamExt as _;
        let (probe_tx, mut probe_rx) =
            futures::channel::mpsc::unbounded::<(Arc<WatchdogTracker>, u64)>();
        foreground
            .spawn(async move {
                while let Some((tracker, probe_id)) = probe_rx.next().await {
                    tracker.complete_probe(probe_id);
                }
            })
            .detach();
        let spawner: ProbeSpawner = Arc::new(move |tracker, probe_id| {
            let _ = probe_tx.unbounded_send((tracker, probe_id));
        });
        let tracker = WatchdogTracker::new(
            DEFAULT_STALL_THRESHOLD,
            Arc::new(Instant::now),
            Some(spawner),
        );
        Self::spawn(tracker, DEFAULT_POLL_INTERVAL, true)
    }

    pub(crate) fn spawn(
        tracker: Arc<WatchdogTracker>,
        poll_interval: Duration,
        register_global: bool,
    ) -> Self {
        if register_global {
            *GLOBAL_TRACKER.write().unwrap() = Some(Arc::downgrade(&tracker));
            GLOBAL_ACTIVE.store(true, Ordering::Release);
        }
        let thread_tracker = Arc::clone(&tracker);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let worker = std::thread::Builder::new()
            .name("runner-ui-watchdog".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, move || loop {
                    {
                        let state = thread_tracker.state.lock().unwrap();
                        let (mut state, _) = thread_tracker
                            .wake
                            .wait_timeout_while(state, poll_interval, |s| {
                                !s.stopped && !s.manual_check_requested
                            })
                            .unwrap();
                        if state.stopped {
                            break;
                        }
                        state.manual_check_requested = false;
                    }
                    thread_tracker.inspect_once();
                    let mut state = thread_tracker.state.lock().unwrap();
                    state.checks_completed = state.checks_completed.wrapping_add(1);
                    thread_tracker.wake.notify_all();
                    if state.stopped {
                        break;
                    }
                });
            })
            .expect("spawn main-thread stall watchdog");
        Self {
            tracker,
            worker: Some(worker),
            registered_global: register_global,
        }
    }

    pub(crate) fn stop(&mut self) {
        if self.registered_global {
            GLOBAL_ACTIVE.store(false, Ordering::Release);
            let mut slot = GLOBAL_TRACKER.write().unwrap();
            if slot
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, &self.tracker))
            {
                *slot = None;
            }
            self.registered_global = false;
        }
        {
            let mut state = self.tracker.state.lock().unwrap();
            state.stopped = true;
            state.probe_spawner = None;
            state.pending_probe = None;
        }
        self.tracker.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    #[cfg(test)]
    fn trigger_check_and_wait(&self) {
        let target = {
            let mut state = self.tracker.state.lock().unwrap();
            let next = state.checks_completed.wrapping_add(1);
            state.manual_check_requested = true;
            self.tracker.wake.notify_all();
            next
        };
        let state = self.tracker.state.lock().unwrap();
        let _guard = self
            .tracker
            .wake
            .wait_timeout_while(state, Duration::from_secs(5), |s| {
                !s.stopped && s.checks_completed < target
            })
            .unwrap();
    }
}

impl Drop for StallWatchdogGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::AtomicU64;

    type QueuedProbes = Arc<Mutex<Vec<(Arc<WatchdogTracker>, u64)>>>;

    #[derive(Clone, Default)]
    struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

    struct CaptureGuard(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureGuard {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureBuffer {
        type Writer = CaptureGuard;
        fn make_writer(&'a self) -> Self::Writer {
            CaptureGuard(Arc::clone(&self.0))
        }
    }

    impl CaptureBuffer {
        fn take_string(&self) -> String {
            let bytes = std::mem::take(&mut *self.0.lock().unwrap());
            String::from_utf8(bytes).unwrap()
        }
    }

    #[test]
    fn watchdog_is_quiet_in_normal_turns_and_logs_stall_plus_recovery_without_backtrace_1377() {
        let base = Instant::now();
        let offset_ms = Arc::new(AtomicU64::new(0));
        let clock_offset = Arc::clone(&offset_ms);
        let clock: ClockFn =
            Arc::new(move || base + Duration::from_millis(clock_offset.load(Ordering::Acquire)));

        let logs = CaptureBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .with_writer(logs.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let tracker = WatchdogTracker::new(DEFAULT_STALL_THRESHOLD, clock, None);
            let mut guard =
                StallWatchdogGuard::spawn(Arc::clone(&tracker), Duration::from_secs(3600), false);

            // 1. Fast turn (< 3s) produces zero warnings.
            {
                let _turn = tracker.enter_turn("store.apply_refresh");
                offset_ms.fetch_add(1_500, Ordering::AcqRel);
                guard.trigger_check_and_wait();
            }
            assert!(logs.take_string().is_empty());

            // 2. Turn stalling past 3s logs once with breadcrumb + stalled_ms (never a watchdog Backtrace),
            // does not spam on repeated ticks, and logs recovery duration when the turn finishes.
            {
                let _turn = tracker.enter_turn("window.sync_activation");
                tracker.record_breadcrumb("root.render");
                offset_ms.fetch_add(3_250, Ordering::AcqRel);
                guard.trigger_check_and_wait();

                let stall_log = logs.take_string();
                assert!(
                    stall_log.contains("main-thread turn stalled"),
                    "expected stall warning, got: {stall_log}"
                );
                assert!(stall_log.contains("stalled_ms=3250"), "{stall_log}");
                assert!(
                    stall_log.contains("breadcrumb=\"root.render\""),
                    "{stall_log}"
                );
                assert!(
                    !stall_log.to_ascii_lowercase().contains("backtrace"),
                    "watchdog must never log its own thread backtrace: {stall_log}"
                );

                // Subsequent watchdog check during the same stall stays quiet.
                offset_ms.fetch_add(900, Ordering::AcqRel);
                guard.trigger_check_and_wait();
                assert!(logs.take_string().is_empty());

                // Completing the stalled turn logs total recovery duration.
                offset_ms.fetch_add(350, Ordering::AcqRel);
            }
            let recovery_log = logs.take_string();
            assert!(
                recovery_log.contains("main-thread stall recovered"),
                "expected recovery warning, got: {recovery_log}"
            );
            assert!(
                recovery_log.contains("recovered_after_ms=4500"),
                "{recovery_log}"
            );
            assert!(
                recovery_log.contains("breadcrumb=\"root.render\""),
                "{recovery_log}"
            );

            // 3. Clean teardown stops and joins the watchdog thread promptly.
            guard.stop();
            assert!(guard.worker.is_none());
        });
    }

    #[test]
    fn heartbeat_probe_detects_uninstrumented_main_thread_stall_and_stops_cleanly_1377() {
        let base = Instant::now();
        let offset_ms = Arc::new(AtomicU64::new(0));
        let clock_offset = Arc::clone(&offset_ms);
        let clock: ClockFn =
            Arc::new(move || base + Duration::from_millis(clock_offset.load(Ordering::Acquire)));

        let queued_probes: QueuedProbes = Arc::new(Mutex::new(Vec::new()));
        let probes_sink = Arc::clone(&queued_probes);
        let spawner: ProbeSpawner = Arc::new(move |tracker, probe_id| {
            probes_sink.lock().unwrap().push((tracker, probe_id));
        });

        let logs = CaptureBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .with_writer(logs.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let tracker = WatchdogTracker::new(DEFAULT_STALL_THRESHOLD, clock, Some(spawner));
            tracker.record_breadcrumb("mission_workspace.render");
            let guard =
                StallWatchdogGuard::spawn(Arc::clone(&tracker), Duration::from_secs(3600), false);

            // First check enqueues one foreground probe; no warning yet.
            guard.trigger_check_and_wait();
            assert_eq!(queued_probes.lock().unwrap().len(), 1);
            assert!(logs.take_string().is_empty());

            // Main thread fails to drain the probe for 3.4s -> stall warning fires with last breadcrumb.
            offset_ms.fetch_add(3_400, Ordering::AcqRel);
            guard.trigger_check_and_wait();
            assert_eq!(
                queued_probes.lock().unwrap().len(),
                1,
                "at most one heartbeat probe may be pending while stalled"
            );
            let stall_log = logs.take_string();
            assert!(
                stall_log.contains("main-thread turn stalled"),
                "{stall_log}"
            );
            assert!(
                stall_log.contains("source=\"heartbeat_probe\""),
                "{stall_log}"
            );
            assert!(stall_log.contains("stalled_ms=3400"), "{stall_log}");
            assert!(
                stall_log.contains("breadcrumb=\"mission_workspace.render\""),
                "{stall_log}"
            );

            // Main thread finally runs the queued probe 600ms later -> logs 4000ms recovery.
            offset_ms.fetch_add(600, Ordering::AcqRel);
            let (pending, probe_id) = queued_probes.lock().unwrap().pop().unwrap();
            pending.complete_probe(probe_id);
            let recovery_log = logs.take_string();
            assert!(
                recovery_log.contains("main-thread stall recovered"),
                "{recovery_log}"
            );
            assert!(
                recovery_log.contains("recovered_after_ms=4000"),
                "{recovery_log}"
            );

            drop(guard);
        });
    }

    #[test]
    fn subthreshold_turns_do_not_reset_or_multiply_pending_heartbeat_probe_1377() {
        let base = Instant::now();
        let offset_ms = Arc::new(AtomicU64::new(0));
        let clock_offset = Arc::clone(&offset_ms);
        let clock: ClockFn =
            Arc::new(move || base + Duration::from_millis(clock_offset.load(Ordering::Acquire)));

        let queued_probes: QueuedProbes = Arc::new(Mutex::new(Vec::new()));
        let probes_sink = Arc::clone(&queued_probes);
        let spawner: ProbeSpawner = Arc::new(move |tracker, probe_id| {
            probes_sink.lock().unwrap().push((tracker, probe_id));
        });

        let logs = CaptureBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .with_writer(logs.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let tracker = WatchdogTracker::new(DEFAULT_STALL_THRESHOLD, clock, Some(spawner));
            let guard =
                StallWatchdogGuard::spawn(Arc::clone(&tracker), Duration::from_secs(3600), false);

            // 1. Queue one foreground heartbeat probe at t=0.
            guard.trigger_check_and_wait();
            assert_eq!(queued_probes.lock().unwrap().len(), 1);
            assert!(logs.take_string().is_empty());

            // 2. Enter and exit multiple subthreshold (< 3s) turns without draining the foreground
            // probe, advancing past 3s overall while ticking the watchdog between/during turns.
            {
                let _turn = tracker.enter_turn("root.render");
                offset_ms.fetch_add(1_200, Ordering::AcqRel);
                guard.trigger_check_and_wait();
            }
            assert_eq!(queued_probes.lock().unwrap().len(), 1);
            assert!(logs.take_string().is_empty());

            guard.trigger_check_and_wait();
            assert_eq!(
                queued_probes.lock().unwrap().len(),
                1,
                "exiting a short turn must not clear pending_probe or queue a second probe"
            );

            {
                let _turn = tracker.enter_turn("store.apply_refresh");
                offset_ms.fetch_add(1_200, Ordering::AcqRel);
                guard.trigger_check_and_wait();
            }
            assert_eq!(queued_probes.lock().unwrap().len(), 1);
            assert!(logs.take_string().is_empty());

            {
                let _turn = tracker.enter_turn("window.sync_activation");
                offset_ms.fetch_add(1_100, Ordering::AcqRel);
            }
            // Total probe age is now 3,500ms (>= 3s) even though every turn was <= 1.2s.
            guard.trigger_check_and_wait();
            assert_eq!(
                queued_probes.lock().unwrap().len(),
                1,
                "still exactly one outstanding heartbeat probe"
            );
            let stall_log = logs.take_string();
            assert!(
                stall_log.contains("main-thread turn stalled"),
                "{stall_log}"
            );
            assert!(
                stall_log.contains("source=\"heartbeat_probe\""),
                "{stall_log}"
            );
            assert!(stall_log.contains("stalled_ms=3500"), "{stall_log}");
            assert!(
                stall_log.contains("breadcrumb=\"window.sync_activation\""),
                "{stall_log}"
            );

            // 3. Another subthreshold turn after warning must not claim premature recovery,
            // and a mismatched probe ID cannot acknowledge the pending probe.
            {
                let _turn = tracker.enter_turn("root.set_route");
                offset_ms.fetch_add(200, Ordering::AcqRel);
            }
            assert!(logs.take_string().is_empty());
            tracker.complete_probe(999);
            assert!(logs.take_string().is_empty());

            // 4. Draining the actual queued probe logs a single accurate recovery.
            offset_ms.fetch_add(200, Ordering::AcqRel);
            let (pending, probe_id) = queued_probes.lock().unwrap().pop().unwrap();
            pending.complete_probe(probe_id);
            let recovery_log = logs.take_string();
            assert_eq!(recovery_log.lines().count(), 1, "{recovery_log}");
            assert!(
                recovery_log.contains("main-thread stall recovered"),
                "{recovery_log}"
            );
            assert!(
                recovery_log.contains("source=\"heartbeat_probe\""),
                "{recovery_log}"
            );
            assert!(
                recovery_log.contains("recovered_after_ms=3900"),
                "{recovery_log}"
            );

            drop(guard);
        });
    }
}

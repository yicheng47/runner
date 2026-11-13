#[cfg(unix)]
use super::super::process::distinct_foreground_process;
use super::*;
#[cfg(unix)]
use runner_core::protocol::hook::GENERATION_ENV;
use std::collections::BTreeMap;

fn admit_hook(
    rt: &PtyRuntime,
    session: &str,
    runtime: crate::model::Runtime,
    event: &str,
    payload: serde_json::Value,
) {
    let caller_thread_id = (runtime == crate::model::Runtime::Codex)
        .then(|| payload["session_id"].as_str().unwrap().to_owned());
    drop(
        rt.hooks
            .admit(runner_core::protocol::hook::HookReport {
                bridge_unavailable: false,
                version: runner_core::protocol::hook::VERSION,
                runtime,
                session_id: session.into(),
                generation: "current".into(),
                event: event.into(),
                payload,
                caller_thread_id,
            })
            .unwrap(),
    );
}

#[test]
fn hook_startup_deadline_requires_output_and_quiet_and_stops_after_any_valid_report() {
    let start = Instant::now();
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    detector.awaiting_first_hook = true;
    assert!(!detector.hook_startup_timed_out_at(start + Duration::from_secs(3600)));
    detector.on_output_event_at(b"Starting MCP servers (6/7)", false, start);
    for seconds in [2, 10, 59] {
        assert!(!detector.hook_startup_timed_out_at(start + Duration::from_secs(seconds)));
    }
    detector.on_output_event_at(b"first answer", false, start + Duration::from_secs(59));
    assert!(!detector.hook_startup_timed_out_at(start + Duration::from_secs(60)));
    assert!(detector.hook_startup_timed_out_at(start + Duration::from_secs(61)));
    assert!(detector.accept_event(&super::super::state::agent::AgentEvent::TurnStarted));
    assert!(!detector.hook_startup_timed_out_at(start + Duration::from_secs(3600)));

    let mut startup = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    startup.awaiting_first_hook = true;
    startup.on_output_event_at(b"startup", false, start);
    // Codex suppresses startup-ready while argv is pending, but the bridge is alive.
    assert!(
        !startup.accept_event(&super::super::state::agent::AgentEvent::Batch {
            runtime: crate::model::Runtime::Codex,
            events: vec![super::super::state::agent::AgentEvent::StartupReady],
        })
    );
    assert!(!startup.hook_startup_timed_out_at(start + Duration::from_secs(3600)));
}

#[test]
#[cfg(unix)]
fn allocated_hook_receiver_with_no_reports_enters_output_fallback() {
    let routes = Arc::new(super::super::hook_queue::HookRoutes::default());
    let receiver = routes.register(
        crate::model::Runtime::Codex,
        "silent-bridge".into(),
        "launch".into(),
    );
    let spec = SpawnSpec {
        agent_runtime: Some(crate::model::Runtime::Codex),
        session_id: "silent-bridge".into(),
        mission: true,
        pending_turn: Some(true),
        ..Default::default()
    };
    let mut watcher = crate::runtimes::adapter(crate::model::Runtime::Codex)
        .status_hooks()
        .unwrap()
        .start_receiver(&spec, receiver)
        .unwrap();
    watcher
        .drain_events(
            0,
            &mut |_| panic!("no native reports were sent"),
            &mut |_| panic!("no native key report was sent"),
        )
        .unwrap();
    let start = Instant::now() - HOOK_STARTUP_TIMEOUT;
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    detector.awaiting_first_hook = true;
    detector.on_output_event_at(b"finished first answer", false, start);
    let detector = Arc::new(Mutex::new(detector));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let monitor = thread::spawn({
        let detector = detector.clone();
        let stop = stop.clone();
        move || {
            idle_monitor_thread(
                detector,
                tx,
                stop,
                Arc::new(AtomicBool::new(false)),
                Some(watcher),
            )
        }
    });
    let event = rx.recv_timeout(Duration::from_secs(5));
    stop.store(true, Ordering::Release);
    monitor.join().unwrap();
    assert!(
        matches!(event, Ok(RuntimeOutput::StatusBridgeFailed)),
        "{event:?}"
    );
    let detector = detector.lock().unwrap();
    assert!(!detector.awaiting_first_hook);
    assert!(detector.adapter.as_ref().unwrap().held_activity().is_none());
}

#[test]
#[cfg(unix)]
fn resumed_codex_reports_after_startup_timeout_restore_hook_status() {
    let routes = Arc::new(super::super::hook_queue::HookRoutes::default());
    let receiver = routes.register(
        crate::model::Runtime::Codex,
        "resumed-bridge".into(),
        "launch".into(),
    );
    let spec = SpawnSpec {
        agent_runtime: Some(crate::model::Runtime::Codex),
        session_id: "resumed-bridge".into(),
        mission: true,
        pending_turn: Some(false),
        ..Default::default()
    };
    let watcher = crate::runtimes::adapter(crate::model::Runtime::Codex)
        .status_hooks()
        .unwrap()
        .start_receiver(&spec, receiver)
        .unwrap();
    let start = Instant::now() - HOOK_STARTUP_TIMEOUT;
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, false);
    detector.awaiting_first_hook = true;
    detector.on_output_event_at(b"resumed history", false, start);
    let detector = Arc::new(Mutex::new(detector));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let monitor = thread::spawn({
        let detector = detector.clone();
        let stop = stop.clone();
        move || {
            idle_monitor_thread(
                detector,
                tx,
                stop,
                Arc::new(AtomicBool::new(false)),
                Some(watcher),
            )
        }
    });
    let failed = rx.recv_timeout(Duration::from_secs(5));
    {
        let mut detector = detector.lock().unwrap();
        detector.on_input(b"next task");
        detector.on_input(b"\r");
    }
    for (event, payload) in [
        (
            "SessionStart",
            serde_json::json!({"session_id": "thread", "source": "resume", "transcript_path": null}),
        ),
        (
            "UserPromptSubmit",
            serde_json::json!({"session_id": "thread", "turn_id": "turn", "transcript_path": null}),
        ),
    ] {
        routes
            .admit(runner_core::protocol::hook::HookReport {
                bridge_unavailable: false,
                version: runner_core::protocol::hook::VERSION,
                runtime: crate::model::Runtime::Codex,
                session_id: "resumed-bridge".into(),
                generation: "launch".into(),
                event: event.into(),
                payload,
                caller_thread_id: Some("thread".into()),
            })
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let restored = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(RuntimeOutput::AgentEvent { event, .. }) => break Some(event),
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    stop.store(true, Ordering::Release);
    monitor.join().unwrap();
    assert!(
        matches!(failed, Ok(RuntimeOutput::StatusBridgeFailed)),
        "{failed:?}"
    );
    // The late SessionStart is dropped so it cannot end the submitted turn.
    assert!(
        matches!(
            &restored,
            Some(super::super::state::agent::AgentEvent::Batch { events, .. })
                if matches!(
                    events.as_slice(),
                    [super::super::state::agent::AgentEvent::TurnStarted]
                )
        ),
        "{restored:?}"
    );
}

#[test]
#[cfg(unix)]
fn claude_feedback_wait_releases_detector_and_ends_on_detach() {
    struct Watcher(mpsc::Sender<()>);
    impl HookWatcher for Watcher {
        fn drain_events(
            &mut self,
            _cancel: u8,
            emit: &mut dyn FnMut(
                crate::session::state::agent::AgentEvent,
            ) -> crate::session::state::agent::AdapterFeedback,
            _session_start: &mut dyn FnMut(String),
        ) -> crate::error::Result<()> {
            emit(crate::session::state::agent::AgentEvent::Batch {
                runtime: crate::model::Runtime::ClaudeCode,
                events: vec![crate::session::state::agent::AgentEvent::TurnStarted],
            });
            self.0.send(()).unwrap();
            Ok(())
        }
    }
    let detector = Arc::new(Mutex::new(IdleDetector::new_at(
        DEFAULT_IDLE_THRESHOLD,
        Instant::now(),
    )));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let (ended_tx, ended_rx) = mpsc::channel();
    let (returned_tx, returned_rx) = mpsc::channel();
    let monitor = thread::spawn({
        let detector = Arc::clone(&detector);
        let stop = Arc::clone(&stop);
        move || {
            idle_monitor_thread(
                detector,
                tx,
                stop,
                Arc::new(AtomicBool::new(false)),
                Some(Box::new(Watcher(returned_tx))),
            );
            ended_tx.send(()).unwrap();
        }
    });
    let event = rx.recv_timeout(Duration::from_secs(2));
    let has_feedback = matches!(
        &event,
        Ok(RuntimeOutput::AgentEvent {
            feedback: Some(_),
            ..
        })
    );
    let unlocked = detector.try_lock().is_ok();
    let waiting = returned_rx.try_recv() == Err(mpsc::TryRecvError::Empty);
    stop.store(true, Ordering::Release);
    let ended = ended_rx.recv_timeout(Duration::from_secs(2));
    drop(event);
    monitor.join().unwrap();
    assert!(has_feedback);
    assert!(unlocked);
    assert!(waiting);
    ended.unwrap();
    returned_rx.try_recv().unwrap();
}

#[cfg(unix)]
#[test]
fn foreground_confirmation_only_applies_beyond_the_shell_process_group() {
    assert_eq!(distinct_foreground_process(Some(41), Some(41)), Some(false));
    assert_eq!(distinct_foreground_process(Some(41), Some(42)), Some(true));
    assert_eq!(distinct_foreground_process(Some(41), None), None);
}

fn spec(session_id: &str, command: &str, args: &[&str]) -> SpawnSpec {
    let env: BTreeMap<String, String> = BTreeMap::new();
    SpawnSpec {
        agent_runtime: None,
        pending_turn: None,
        session_id: session_id.to_string(),
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env,
        cwd: None,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: Some((80, 24)),
    }
}

// The ConPTY host asks the terminal for the cursor position and
// device attributes as soon as it starts and holds the client until
// something answers. The app's terminal answers these; the tests
// that spawn a bare runtime have no terminal, so this stands in for
// one (#524).
#[cfg(windows)]
#[derive(Default)]
struct HostHandshake {
    seen: Vec<u8>,
    answered: bool,
}

#[cfg(windows)]
impl HostHandshake {
    fn observe(&mut self, rt: &PtyRuntime, session: &RuntimeSession, bytes: &[u8]) {
        if self.answered {
            return;
        }
        self.seen.extend_from_slice(bytes);
        if self.seen.windows(4).any(|w| w == b"\x1b[6n") {
            rt.send_bytes(session, b"\x1b[1;1R\x1b[?6c").unwrap();
            self.answered = true;
        }
    }
}

#[cfg(unix)]
fn wait_for_command_identity(pid: i32, command: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let last_observation = match process_command_line(pid) {
            Ok(Some(line)) => {
                if command_line_matches_recorded_agent(&line, Some("test"), Some(command)) {
                    return;
                }
                format!("command {line:?}")
            }
            Ok(None) => "no command".into(),
            Err(error) => format!("lookup error: {error}"),
        };
        assert!(
            Instant::now() < deadline,
            "process {pid} never matched command {command}; last {last_observation}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn recorded_pid(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    session_id: &str,
) -> Option<i64> {
    pool.get()
        .unwrap()
        .query_row(
            "SELECT pid FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn translate_key_covers_production_names() {
    assert_eq!(translate_key("Enter").unwrap(), b"\r");
    assert_eq!(translate_key("Escape").unwrap(), b"\x1b");
    assert_eq!(translate_key("Tab").unwrap(), b"\t");
    assert_eq!(translate_key("BSpace").unwrap(), b"\x7f");
    assert_eq!(translate_key("Backspace").unwrap(), b"\x7f");
    assert_eq!(translate_key("Up").unwrap(), b"\x1b[A");
    assert_eq!(translate_key("C-c").unwrap(), vec![0x03]);
    assert_eq!(translate_key("C-D").unwrap(), vec![0x04]);
    assert!(translate_key("OhNoMyKey").is_err());
    assert!(translate_key("C-").is_err());
}

#[test]
fn interrupt_key_matches_only_complete_legacy_and_kitty_cancels() {
    for (bytes, kind) in [
        (b"\x03".as_slice(), CTRL_C_INTERRUPT),
        (b"\x1b", ESCAPE_INTERRUPT),
        (b"\x1b[99;5u", CTRL_C_INTERRUPT),
        (b"\x1b[27u", ESCAPE_INTERRUPT),
    ] {
        assert_eq!(interrupt_key(bytes), Some(kind), "{bytes:?}");
    }
    for bytes in [
        b"".as_slice(),
        b"text",
        b"\x1b[A",
        b"\x1bOP",
        b"\x1bd",
        b"\x1b[100;3u",
        b"\x1b[13;2u",
        b"\x1b[99;7u",
        b"\x1b[27;3u",
        b"\x1b[200~\x03\x1b\x1b[27u\x1b[99;5u\x1b[201~",
        b"text\x03",
        b"\x03text",
        b"\x1b\x1b",
        b"\x1b[27u\x1b[27u",
        b"\x1b[99;5utext",
        b"\x1b[27",
        b"[27u",
    ] {
        assert_eq!(interrupt_key(bytes), None, "{bytes:?}");
    }
}

#[test]
fn idle_detector_flips_to_idle_after_silence() {
    let threshold = Duration::from_millis(750);
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(threshold, start);

    assert_eq!(
        detector.tick_at(start + threshold - Duration::from_millis(1)),
        None
    );
    assert_eq!(
        detector.tick_at(start + threshold),
        Some(SessionActivityState::Idle)
    );
    assert_eq!(
        detector.tick_at(start + threshold + Duration::from_secs(1)),
        None
    );
}

#[test]
fn shell_detector_uses_silence_until_its_first_command_mark() {
    use SessionActivityState::{Busy, Idle};
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut detector = IdleDetector::new_at(DEFAULT_IDLE_THRESHOLD, start);
    detector.read_shell_marks();

    assert_eq!(detector.on_output_at(b"motd\r\n", false, at(10)), None);
    assert_eq!(
        detector.tick_at(at(10) + DEFAULT_IDLE_THRESHOLD),
        Some(Idle)
    );
    assert_eq!(detector.on_output_at(b"x", true, at(2100)), None);
    assert_eq!(detector.on_output_at(b"x", false, at(2200)), Some(Busy));

    assert_eq!(
        detector.on_output_at(b"\x1b]133;D\x1b\\\x1b]133;A\x1b", false, at(2300)),
        Some(Idle)
    );
    assert_eq!(detector.on_output_at(b"\\% ", false, at(2301)), None);
    for (bytes, quiet) in [(b"l".as_slice(), false), (b"\x1b[2J% ", true)] {
        assert_eq!(detector.on_output_at(bytes, quiet, at(2400)), None);
    }
    assert_eq!(
        detector.on_output_at(b"s\r\n\x1b]133;C\x07", false, at(2500)),
        Some(Busy)
    );
    assert_eq!(detector.tick_at(at(60_000)), None);
    assert_eq!(
        detector.on_output_at(b"\x1b]133;A\x07\x1b]133;C\x07", false, at(60_001)),
        None
    );
    assert_eq!(
        detector.on_output_at(b"out\x1b]133;D;0\x07\x1b]133;A\x07", false, at(60_002)),
        Some(Idle)
    );
    assert_eq!(
        detector.on_output_at(b"\x1b]133;A\x07", false, at(60_003)),
        None
    );
    assert_eq!(detector.tick_at(at(120_000)), None);
}

#[test]
fn shell_without_the_integration_keeps_silence_after_a_childs_c_marks() {
    use SessionActivityState::{Busy, Idle};
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut detector = IdleDetector::new_at(DEFAULT_IDLE_THRESHOLD, start);
    detector.read_shell_marks();
    assert_eq!(detector.tick_at(at(2000)), Some(Idle));
    let zones = b"\x1b]133;A\x07you\x1b]133;B\x07\x1b]133;C\x07agent";
    assert_eq!(detector.on_output_at(zones, false, at(2100)), Some(Busy));
    assert_eq!(
        detector.tick_at(at(2100) + DEFAULT_IDLE_THRESHOLD),
        Some(Idle)
    );
    assert_eq!(detector.on_output_at(b"$ ", false, at(5000)), Some(Busy));
    assert_eq!(
        detector.tick_at(at(5000) + DEFAULT_IDLE_THRESHOLD),
        Some(Idle)
    );
}

#[test]
fn shell_marks_in_one_read_apply_in_order() {
    use SessionActivityState::{Busy, Idle};
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(DEFAULT_IDLE_THRESHOLD, start);
    detector.read_shell_marks();
    assert_eq!(
        detector.on_output_at(b"\x1b]133;D\x07\x1b]133;A\x07$ ", false, start),
        Some(Idle)
    );
    let fast = b"\x1b]133;C\x07x\r\n\x1b]133;D\x07\x1b]133;A\x07$ ";
    assert_eq!(detector.on_output_at(fast, false, start), None);
    let next = b"\x1b]133;D\x07\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07";
    assert_eq!(detector.on_output_at(next, false, start), Some(Busy));
}

#[test]
fn agent_detector_ignores_shell_marks() {
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(DEFAULT_IDLE_THRESHOLD, start);
    assert_eq!(
        detector.tick_at(start + DEFAULT_IDLE_THRESHOLD),
        Some(SessionActivityState::Idle)
    );
    assert_eq!(
        detector.on_output_at(b"\x1b]133;C\x07", false, start + Duration::from_secs(3)),
        Some(SessionActivityState::Busy)
    );
    assert_eq!(
        detector.on_output_at(b"\x1b]133;D\x07", false, start + Duration::from_secs(4)),
        None
    );
    assert_eq!(
        detector.tick_at(start + Duration::from_secs(4) + DEFAULT_IDLE_THRESHOLD),
        Some(SessionActivityState::Idle)
    );
}

#[test]
fn idle_detector_wakes_on_first_byte_after_idle() {
    let threshold = Duration::from_millis(750);
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(threshold, start);

    assert_eq!(
        detector.tick_at(start + threshold),
        Some(SessionActivityState::Idle)
    );
    assert_eq!(
        detector.on_bytes_at(1, start + threshold + Duration::from_millis(1)),
        Some(SessionActivityState::Busy)
    );
    assert_eq!(
        detector.on_bytes_at(1, start + threshold + Duration::from_millis(2)),
        None
    );
}

fn osc_title(title: &str) -> Vec<u8> {
    format!("\x1b]0;{title}\x07").into_bytes()
}

fn codex_detector(threshold: Duration, pending_turn: bool) -> IdleDetector {
    IdleDetector::with_adapter_at(
        threshold,
        crate::runtimes::adapter(crate::model::Runtime::Codex).terminal_adapter(Some(pending_turn)),
        Instant::now(),
    )
}

fn accept_hook(
    detector: &mut IdleDetector,
    observation: &crate::session::status::AgentObservation,
) -> bool {
    let event = if observation.activity == crate::session::status::Activity::Idle
        && observation.outcome.is_none()
    {
        crate::session::state::agent::AgentEvent::StartupReady
    } else {
        crate::session::state::agent::AgentEvent::Working { detail: None }
    };
    detector.accept_event(&crate::session::state::agent::AgentEvent::Batch {
        runtime: crate::model::Runtime::Codex,
        events: vec![event],
    })
}

#[test]
fn codex_native_attention_bypasses_activity_dedup_and_survives_hook_takeover() {
    use crate::runtimes::TerminalEvent;
    use crate::session::state::agent::AgentEvent;
    for hooks in [false, true] {
        let mut detector = codex_detector(Duration::from_secs(2), false);
        let start = Instant::now();
        if hooks {
            detector.accept_event(&AgentEvent::TurnStarted);
        } else {
            detector.hooks_unavailable();
        }
        for (report, blocked) in [
            (b"\x1b]7501;state=blocked:app=codex\x1b\\".as_slice(), true),
            (b"\x1b]7501;state=blocked:app=codex\x07", true),
            (b"\x1b]7501;state=working:app=codex\x07", false),
            (b"\x1b]7501;state=idle:app=codex\x07", false),
            (b"\x1b]7501;state=clear\x1b\\", false),
        ] {
            detector.on_output_event_at(report, true, start);
            assert_eq!(
                detector.take_attention(),
                Some(TerminalEvent::CodexAttention(blocked))
            );
            assert_eq!(detector.current, SessionActivityState::Busy);
        }
    }
}

#[test]
fn codex_native_attention_preserves_nonquiet_activity_and_readiness_in_the_same_chunk() {
    use crate::runtimes::TerminalEvent as T;
    let start = Instant::now();
    for payload in [
        b"\x1b]7501;state=working:app=codex\x1b\\ordinary response".as_slice(),
        b"ordinary response\x1b]7501;state=idle:app=codex\x07",
    ] {
        let mut detector = codex_detector(Duration::from_secs(2), false);
        detector.hooks_unavailable();
        assert_eq!(
            detector.tick_event_at(start + Duration::from_secs(3)),
            Some(T::Activity(SessionActivityState::Idle))
        );
        assert_eq!(
            detector.on_output_event_at(payload, false, start + Duration::from_secs(4)),
            Some(T::Activity(SessionActivityState::Busy))
        );
        assert_eq!(detector.take_attention(), Some(T::CodexAttention(false)));
        assert_eq!(detector.current, SessionActivityState::Busy);
    }
    let mut detector = codex_detector(Duration::from_secs(2), false);
    detector.hooks_unavailable();
    assert_eq!(
        detector.on_output_event_at(
            b"\x1b[?2004h\x1b]7501;state=blocked:app=codex\x07",
            false,
            start
        ),
        Some(T::Ready(SessionActivityState::Idle))
    );
    assert_eq!(detector.take_attention(), Some(T::CodexAttention(true)));
    let mut detector = codex_detector(Duration::from_secs(2), false);
    detector.hooks_unavailable();
    detector.on_output_event_at(&osc_title("⠋ project"), false, start);
    detector.on_output_event_at(&osc_title("⠙ project"), false, start);
    let combined = [
        osc_title("project"),
        b"\x1b]7501;state=blocked:app=codex\x07".to_vec(),
    ]
    .concat();
    assert_eq!(
        detector.on_output_event_at(&combined, false, start),
        Some(T::Title(SessionActivityState::Idle))
    );
    assert_eq!(detector.take_attention(), Some(T::CodexAttention(true)));
}

#[test]
fn codex_title_idle_is_invalidated_by_submission_until_fresh_title_evidence() {
    let threshold = Duration::from_secs(2);
    let start = Instant::now();
    let mut detector = codex_detector(threshold, false);
    detector.hooks_unavailable();
    detector.last_byte = start;
    assert_eq!(
        detector.on_output_at(&osc_title("⠙ project"), false, start),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠹ project"),
            false,
            start + Duration::from_millis(100)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("project"),
            false,
            start + Duration::from_millis(200)
        ),
        Some(SessionActivityState::Idle)
    );

    let stale_working = osc_title("⠙ project");
    let stale_end = stale_working.len() - 1;
    assert_eq!(
        detector.on_output_at(
            &stale_working[..stale_end],
            false,
            start + Duration::from_millis(250)
        ),
        None
    );
    detector.on_input(b"\r");
    assert_eq!(detector.tick_state(), Some(SessionActivityState::Busy));
    assert_eq!(
        detector.on_output_at(
            &stale_working[stale_end..],
            false,
            start + Duration::from_millis(275)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("project"),
            false,
            start + Duration::from_millis(300)
        ),
        None
    );
    assert_eq!(detector.current, SessionActivityState::Busy);
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠙ project"),
            false,
            start + Duration::from_millis(400)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠹ project"),
            false,
            start + Duration::from_millis(450)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("project"),
            false,
            start + Duration::from_millis(500)
        ),
        Some(SessionActivityState::Idle)
    );
}

#[test]
fn hook_takeover_drops_title_authority_until_bridge_failure() {
    use super::super::status::{Activity, AgentObservation, ObservationSource};

    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    detector.hooks_unavailable();
    let start = detector.last_byte;
    assert_eq!(
        detector.on_output_at(&osc_title("⠙ project"), false, start),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠹ project"),
            false,
            start + Duration::from_millis(100)
        ),
        None
    );
    assert!(accept_hook(
        &mut detector,
        &AgentObservation {
            activity: Activity::Working,
            source: ObservationSource::Hook,
            ..Default::default()
        }
    ));
    for title in ["⠙ project", "⠹ project", "project"] {
        assert_eq!(
            detector.on_output_at(&osc_title(title), false, start + Duration::from_millis(200)),
            None
        );
    }
    let stale_rest = osc_title("project");
    let stale_end = stale_rest.len() - 1;
    assert_eq!(
        detector.on_output_at(
            &stale_rest[..stale_end],
            false,
            start + Duration::from_millis(200)
        ),
        None
    );

    detector.hooks_unavailable();
    assert_eq!(
        detector.on_output_at(
            &stale_rest[stale_end..],
            false,
            start + Duration::from_millis(250)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠙ project"),
            false,
            start + Duration::from_millis(300)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("⠹ project"),
            false,
            start + Duration::from_millis(400)
        ),
        None
    );
    assert_eq!(
        detector.on_output_at(
            &osc_title("project"),
            false,
            start + Duration::from_millis(500)
        ),
        Some(SessionActivityState::Idle)
    );
}

#[test]
fn codex_startup_without_readiness_retains_output_fallback() {
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, false);
    assert_eq!(
        detector.on_output_state(b"startup without terminal modes", false),
        None
    );
    assert_eq!(
        detector.tick_at(Instant::now() + DEFAULT_IDLE_THRESHOLD),
        Some(SessionActivityState::Idle)
    );
    assert_eq!(
        detector.on_output_state(b"more output", false),
        Some(SessionActivityState::Busy)
    );
    assert_eq!(
        detector.on_output_state(b"\x1b[?2004h", false),
        Some(SessionActivityState::Idle)
    );
    assert_eq!(detector.on_output_state(b"idle redraw", false), None);
}

#[test]
fn codex_startup_history_recall_submits_with_or_without_hooks() {
    for hooks_available in [false, true] {
        for arrow in [b"\x1b[A", b"\x1b[B", b"\x1bOA", b"\x1bOB"] {
            let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, false);
            if !hooks_available {
                detector.hooks_unavailable();
            }
            assert_eq!(
                detector.on_output_state(b"\x1b[?2004h", false),
                Some(SessionActivityState::Idle)
            );
            detector.on_input(arrow);
            assert_eq!(detector.on_output_state(b"recalled draft", false), None);
            detector.on_input(b"\r");
            assert_eq!(detector.tick_state(), Some(SessionActivityState::Busy));
            assert_eq!(
                detector.tick_at(Instant::now() + DEFAULT_IDLE_THRESHOLD),
                Some(SessionActivityState::Idle)
            );
            assert_eq!(
                detector.on_output_state(b"working output", false),
                Some(SessionActivityState::Busy)
            );
            if !hooks_available {
                assert_eq!(detector.adapter.as_ref().unwrap().held_activity(), None);
            }
        }
    }
}

#[test]
fn codex_startup_readiness_input_and_missing_hooks_are_baseline_only() {
    let frame: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/codex-0.155.1-idle.json")).unwrap();
    let ready = frame["readiness"].as_str().unwrap().as_bytes();
    let redraw = frame["redraw"].as_str().unwrap().as_bytes();
    for split in 1..b"\x1b[?2004h".len() {
        let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, false);
        assert_eq!(detector.on_output_state(&ready[..split], false), None);
        assert_eq!(
            detector.on_output_state(&ready[split..], true),
            Some(SessionActivityState::Idle)
        );
        for _ in 0..100 {
            assert_eq!(detector.on_output_state(redraw, false), None);
        }
        detector.on_input(b"\r");
        assert_eq!(detector.tick_state(), Some(SessionActivityState::Idle));
        detector.on_input(b"draft");
        assert_eq!(detector.on_output_state(redraw, false), None);
        detector.on_input(b"\r");
        assert_eq!(detector.tick_state(), Some(SessionActivityState::Busy));
        assert_eq!(
            detector.tick_at(Instant::now() + Duration::from_secs(60)),
            Some(SessionActivityState::Idle)
        );
        detector.hooks_unavailable();
        assert_eq!(detector.adapter.as_ref().unwrap().held_activity(), None);
    }
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, false);
    detector.on_input(b"early submission");
    detector.on_input(b"\r");
    assert_eq!(detector.tick_state(), Some(SessionActivityState::Busy));
    assert_eq!(detector.on_output_state(ready, false), None);
    assert_eq!(detector.current, SessionActivityState::Busy);
    assert!(!accept_hook(
        &mut detector,
        &super::super::status::AgentObservation {
            activity: super::super::status::Activity::Idle,
            source: super::super::status::ObservationSource::Hook,
            ..Default::default()
        }
    ));
    assert!(accept_hook(
        &mut detector,
        &super::super::status::AgentObservation {
            activity: super::super::status::Activity::Working,
            source: super::super::status::ObservationSource::Hook,
            ..Default::default()
        }
    ));
    assert_eq!(detector.adapter.as_ref().unwrap().held_activity(), None);
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    assert_eq!(detector.tick_state(), Some(SessionActivityState::Busy));
    assert_eq!(detector.on_output_state(ready, false), None);
}

#[test]
fn resize_grace_bytes_do_not_wake_idle_detector() {
    // Models the reader loop's grace branch: an idle session that
    // emits a SIGWINCH repaint burst stays idle, and its idle timer
    // is kept fresh so it doesn't immediately re-flip on the next tick.
    let threshold = Duration::from_millis(750);
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(threshold, start);

    assert_eq!(
        detector.tick_at(start + threshold),
        Some(SessionActivityState::Idle)
    );

    // Repaint bytes during the grace window: no Busy transition.
    let repaint = start + threshold + Duration::from_millis(10);
    detector.on_bytes_quiet_at(1, repaint);

    // Still idle (a real byte outside grace would have flipped Busy).
    assert_eq!(detector.tick_at(repaint + Duration::from_millis(1)), None);

    // The quiet burst refreshed last_byte, so a genuinely busy session
    // that was resized mid-work would not idle early. A later real byte
    // still wakes it.
    assert_eq!(
        detector.on_bytes_at(1, repaint + Duration::from_millis(20)),
        Some(SessionActivityState::Busy)
    );
}

#[derive(serde::Deserialize)]
struct RecordedOutput {
    ms: u64,
    data: String,
}

fn replay_byte_activity_fixture(name: &str) -> Vec<(u64, SessionActivityState)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runner-terminal/fixtures")
        .join(format!("{name}.ndjson"));
    let fixture = std::fs::read_to_string(path).unwrap();
    let outputs: Vec<RecordedOutput> = fixture
        .lines()
        .skip(1)
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|event: &RecordedOutput| !event.data.is_empty())
        .collect();
    let start = Instant::now();
    let mut detector = IdleDetector::new_at(DEFAULT_IDLE_THRESHOLD, start);
    let mut transitions = Vec::new();
    for output in &outputs {
        let at = start + Duration::from_millis(output.ms);
        if let Some(state) = detector.tick_at(at) {
            transitions.push((output.ms, state));
        }
        if let Some(state) = detector.on_bytes_at(output.data.len(), at) {
            transitions.push((output.ms, state));
        }
    }
    let last_ms = outputs.last().unwrap().ms;
    if let Some(state) =
        detector.tick_at(start + Duration::from_millis(last_ms) + DEFAULT_IDLE_THRESHOLD)
    {
        transitions.push((last_ms + DEFAULT_IDLE_THRESHOLD.as_millis() as u64, state));
    }
    transitions
}

#[test]
fn recorded_fixture_byte_activity_transitions() {
    assert_eq!(
            replay_byte_activity_fixture("codex-title-working"),
            vec![(12_893, SessionActivityState::Idle)],
            "Codex byte activity must stay Busy through its animation tail and become Idle only after its last output"
        );
    assert_eq!(
        replay_byte_activity_fixture("claude-session"),
        vec![
            (2_637, SessionActivityState::Idle),
            (2_637, SessionActivityState::Busy),
            (12_646, SessionActivityState::Idle),
            (12_646, SessionActivityState::Busy),
            (40_001, SessionActivityState::Idle),
            (40_001, SessionActivityState::Busy),
            (43_517, SessionActivityState::Idle),
        ]
    );
}

fn replay_codex_title_fixture(
    bridge_failure: bool,
    recovery: bool,
) -> Vec<(u64, SessionActivityState)> {
    use base64::Engine as _;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runner-terminal/fixtures/codex-title-working.ndjson");
    let fixture = std::fs::read_to_string(path).unwrap();
    let outputs: Vec<RecordedOutput> = fixture
        .lines()
        .skip(1)
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|event: &RecordedOutput| !event.data.is_empty())
        .collect();
    let start = Instant::now();
    let mut detector = codex_detector(DEFAULT_IDLE_THRESHOLD, true);
    detector.last_byte = start;
    if bridge_failure {
        assert!(accept_hook(
            &mut detector,
            &super::super::status::AgentObservation {
                activity: super::super::status::Activity::Working,
                source: super::super::status::ObservationSource::Hook,
                ..Default::default()
            }
        ));
    }
    let mut model = super::super::state::SessionModel::default();
    let now = super::super::clock::state_now();
    model.apply(
        super::super::state::SessionEvent::Transition {
            state: SessionActivityState::Busy,
            source: super::super::state::StatusSource::Forwarder,
            live: true,
        },
        now,
    );
    if bridge_failure {
        model.apply(
            super::super::state::SessionEvent::Agent {
                event: super::super::state::agent::AgentEvent::TurnStarted,
                live: true,
            },
            now,
        );
    }
    if recovery {
        let event = super::super::state::agent::AgentEvent::Batch {
            runtime: crate::model::Runtime::Codex,
            events: vec![super::super::state::agent::AgentEvent::ConversationRecovered],
        };
        assert!(detector.accept_event(&event));
        model.apply(
            super::super::state::SessionEvent::Agent { event, live: true },
            now,
        );
        assert_eq!(
            model.status().observation.source,
            super::super::status::ObservationSource::Baseline
        );
        assert!(!model.completion_armed());
    } else {
        detector.hooks_unavailable();
        model.apply(
            super::super::state::SessionEvent::BridgeFailed { live: true },
            now,
        );
    }
    let mut transitions = Vec::new();
    for output in &outputs {
        let at = start + Duration::from_millis(output.ms);
        if let Some(state) = detector.tick_at(at) {
            transitions.push((output.ms, state));
            model.apply(
                super::super::state::SessionEvent::Transition {
                    state,
                    source: super::super::state::StatusSource::Forwarder,
                    live: true,
                },
                now,
            );
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&output.data)
            .unwrap();
        if let Some(state) = detector.on_output_at(&bytes, false, at) {
            transitions.push((output.ms, state));
            model.apply(
                super::super::state::SessionEvent::Transition {
                    state,
                    source: super::super::state::StatusSource::Forwarder,
                    live: true,
                },
                now,
            );
        }
    }
    assert_eq!(detector.current, SessionActivityState::Idle);
    assert_eq!(
        model.status().observation.activity,
        super::super::status::Activity::Idle
    );
    assert_eq!(
        model.status().observation.source,
        super::super::status::ObservationSource::Baseline
    );
    transitions
}

#[test]
fn identity_only_recovery_releases_title_authority_through_detector_and_model() {
    for previously_hook_owned in [false, true] {
        assert_eq!(
            replay_codex_title_fixture(previously_hook_owned, true),
            vec![(7_428, SessionActivityState::Idle)]
        );
    }
}

#[test]
fn recorded_codex_title_fixture_overrides_idle_redraw_with_and_without_bridge_failure() {
    for bridge_failure in [false, true] {
        assert_eq!(
            replay_codex_title_fixture(bridge_failure, false),
            if bridge_failure {
                vec![(7_428, SessionActivityState::Idle)]
            } else {
                vec![
                    (36, SessionActivityState::Busy),
                    (7_428, SessionActivityState::Idle),
                ]
            },
            "bridge_failure={bridge_failure}"
        );
    }
}

#[test]
#[cfg(unix)]
fn codex_title_fallback_is_identical_for_direct_and_mission_sessions() {
    let mut observed = Vec::new();
    for mission in [false, true] {
        let rt = PtyRuntime::new();
        let mut spawn = spec(
                if mission {
                    "title-mission"
                } else {
                    "title-direct"
                },
                "/bin/sh",
                &[
                    "-c",
                    "printf '\\033]0;⠙ project\\007\\033]0;⠹ project\\007\\033]0;project\\007'; sleep 1",
                ],
            );
        spawn.pending_turn = Some(false);
        spawn.agent_runtime = Some(crate::model::Runtime::Codex);
        spawn.mission = mission;
        let (session, stream) = rt.spawn(spawn).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = None;
        while Instant::now() < deadline {
            match stream.recv_status_timeout(Duration::from_millis(50)) {
                Ok(super::super::runtime::TestOutput::StatusTransition {
                    state: next,
                    source,
                }) => {
                    assert_eq!(source.as_str(), "forwarder");
                    state = Some(next);
                    if next == SessionActivityState::Idle {
                        break;
                    }
                }
                Ok(
                    super::super::runtime::TestOutput::Stream(_)
                    | super::super::runtime::TestOutput::AgentObservation(_)
                    | super::super::runtime::TestOutput::ConversationStart
                    | super::super::runtime::TestOutput::StatusBridgeFailed,
                ) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        observed.push(state);
        rt.stop(&session).unwrap();
    }
    assert_eq!(
        observed,
        [
            Some(SessionActivityState::Idle),
            Some(SessionActivityState::Idle)
        ]
    );
}

#[test]
#[cfg(unix)]
fn codex_hook_runtime_events_bridge_failure_and_teardown() {
    use super::super::status::{Activity, TurnOutcome};
    let rt = PtyRuntime::new();
    let mut spawn = spec("codex-hooks", "/bin/cat", &[]);
    spawn.agent_runtime = Some(crate::model::Runtime::Codex);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    for event in ["UserPromptSubmit", "Stop", "PreToolUse", "Interrupt"] {
        admit_hook(
            &rt,
            "codex-hooks",
            crate::model::Runtime::Codex,
            event,
            serde_json::json!({"session_id":"main","turn_id":"one"}),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut values = Vec::new();
    while values.len() < 4 && Instant::now() < deadline {
        if let Ok(super::super::runtime::TestOutput::AgentObservation(value)) =
            stream.recv_status_timeout(Duration::from_millis(50))
        {
            values.push(value);
        }
    }
    rt.hooks.retire(&session.session_id);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut failed = false;
    while !failed && Instant::now() < deadline {
        failed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Ok(super::super::runtime::TestOutput::StatusBridgeFailed)
        );
    }
    rt.stop(&session).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut closed = false;
    while !closed && Instant::now() < deadline {
        closed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
    assert!(failed);
    assert!(closed);
    assert_eq!(
        values.iter().map(|v| v.activity).collect::<Vec<_>>(),
        [
            Activity::Working,
            Activity::Ready,
            Activity::Working,
            Activity::Unavailable
        ]
    );
    assert_eq!(
        values.last().unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
}

#[test]
#[cfg(unix)]
fn copilot_hook_runtime_uses_shared_reporter_interrupt_and_bridge_fallback() {
    use super::super::status::{Activity, TurnOutcome};
    let rt = PtyRuntime::new();
    let mut spawn = spec("copilot-hooks", "/bin/cat", &[]);
    spawn.agent_runtime = Some(crate::model::Runtime::Copilot);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    for (event, payload) in [
        ("UserPromptSubmit", serde_json::json!({"session_id":"main"})),
        (
            "Stop",
            serde_json::json!({"session_id":"main","stop_reason":"end_turn"}),
        ),
        (
            "PreToolUse",
            serde_json::json!({
                "session_id":"main",
                "tool_name":"Bash",
                "tool_input":{"command":"sleep 1"},
            }),
        ),
    ] {
        admit_hook(
            &rt,
            "copilot-hooks",
            crate::model::Runtime::Copilot,
            event,
            payload,
        );
    }
    rt.send_bytes(&session, b"\x1b").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut values = Vec::new();
    while values.len() < 4 && Instant::now() < deadline {
        if let Ok(super::super::runtime::TestOutput::AgentObservation(value)) =
            stream.recv_status_timeout(Duration::from_millis(50))
        {
            values.push(value);
        }
    }
    rt.hooks.retire(&session.session_id);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut failed = false;
    while !failed && Instant::now() < deadline {
        failed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Ok(super::super::runtime::TestOutput::StatusBridgeFailed)
        );
    }
    rt.stop(&session).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut closed = false;
    while !closed && Instant::now() < deadline {
        closed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
    assert!(failed);
    assert!(closed);
    assert_eq!(
        values.len(),
        4,
        "expected four Copilot observations: {values:?}"
    );
    assert_eq!(values[0].activity, Activity::Working);
    assert_eq!(values[1].activity, Activity::Ready);
    assert_eq!(values[2].activity, Activity::Working);
    assert_eq!(values[3].activity, Activity::Ready);
    assert_eq!(values[3].outcome, Some(TurnOutcome::Interrupted));
}

#[test]
#[cfg(unix)]
fn pi_hook_runtime_uses_external_extension_without_an_input_interrupt_signal() {
    use super::super::status::{Activity, TurnOutcome};
    let rt = PtyRuntime::new();
    let mut spawn = spec("pi-hooks", "/bin/cat", &[]);
    spawn.agent_runtime = Some(crate::model::Runtime::Pi);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    for report in [
        serde_json::json!({
            "generation":"current",
            "hook_event_name":"session_start",
            "reason":"startup",
            "session_id":"11111111-1111-4111-8111-111111111111",
        }),
        serde_json::json!({
            "generation":"current",
            "hook_event_name":"agent_start",
        }),
        serde_json::json!({
            "generation":"current",
            "hook_event_name":"message_end",
            "stopReason":"stop",
        }),
        serde_json::json!({
            "generation":"current",
            "hook_event_name":"agent_settled",
        }),
    ] {
        admit_hook(
            &rt,
            "pi-hooks",
            crate::model::Runtime::Pi,
            report["hook_event_name"].as_str().unwrap(),
            report.clone(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut values = Vec::new();
    while values.len() < 3 && Instant::now() < deadline {
        if let Ok(super::super::runtime::TestOutput::AgentObservation(value)) =
            stream.recv_status_timeout(Duration::from_millis(50))
        {
            values.push(value);
        }
    }
    rt.hooks.retire(&session.session_id);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut failed = false;
    while !failed && Instant::now() < deadline {
        failed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Ok(super::super::runtime::TestOutput::StatusBridgeFailed)
        );
    }
    rt.stop(&session).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut closed = false;
    while !closed && Instant::now() < deadline {
        closed = matches!(
            stream.recv_status_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }
    assert!(failed);
    assert!(closed);
    assert_eq!(
        values
            .iter()
            .map(|value| value.activity)
            .collect::<Vec<_>>(),
        [Activity::Idle, Activity::Working, Activity::Ready]
    );
    assert_eq!(values[2].outcome, Some(TurnOutcome::Completed));
}

#[test]
#[cfg(unix)]
fn claude_hook_admission_feeds_runtime_output_and_closes_on_exit() {
    let rt = PtyRuntime::new();
    let mut spawn = spec("hooks", "/bin/cat", &[]);
    spawn.agent_runtime = Some(crate::model::Runtime::ClaudeCode);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    for event in [
        "UserPromptSubmit",
        "Stop",
        "PreToolUse",
        "StopFailure",
        "Notification",
    ] {
        admit_hook(
            &rt,
            "hooks",
            crate::model::Runtime::ClaudeCode,
            event,
            serde_json::json!({"notification_type":"idle_prompt"}),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut statuses = Vec::new();
    while statuses.len() < 5 {
        assert!(
            Instant::now() < deadline,
            "missing hook transitions: {statuses:?}"
        );
        if let Ok(super::super::runtime::TestOutput::AgentObservation(observation)) =
            stream.recv_status_timeout(Duration::from_millis(50))
        {
            statuses.push(
                if observation.activity == super::super::status::Activity::Working {
                    SessionActivityState::Busy
                } else {
                    SessionActivityState::Idle
                },
            );
        }
    }
    assert_eq!(
        statuses,
        [
            SessionActivityState::Busy,
            SessionActivityState::Idle,
            SessionActivityState::Busy,
            SessionActivityState::Idle,
            SessionActivityState::Idle,
        ]
    );
    rt.stop(&session).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            Instant::now() < deadline,
            "hook watcher kept output channel alive"
        );
        if let Err(mpsc::RecvTimeoutError::Disconnected) =
            stream.recv_status_timeout(Duration::from_millis(50))
        {
            break;
        }
    }
}

#[test]
#[cfg(unix)]
fn claude_interrupt_keys_go_idle_without_a_hook_and_allow_dialog_recovery() {
    let rt = PtyRuntime::new();
    let mut spawn = spec(
        "interrupt",
        "/bin/sh",
        &["-c", "trap '' INT; printf ready; exec cat"],
    );
    spawn.agent_runtime = Some(crate::model::Runtime::ClaudeCode);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    let mut ready = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.windows(5).any(|bytes| bytes == b"ready") {
        assert!(Instant::now() < deadline, "test child did not become ready");
        if let Ok(super::super::runtime::TestOutput::Stream(bytes)) =
            stream.recv_status_timeout(IDLE_MONITOR_POLL)
        {
            ready.extend(bytes);
        }
    }
    let wait_for_status = |expected: SessionActivityState, expected_source: &str| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(
                Instant::now() < deadline,
                "missing {expected_source} {expected:?}"
            );
            if let Ok(super::super::runtime::TestOutput::AgentObservation(observation)) =
                stream.recv_status_timeout(IDLE_MONITOR_POLL)
            {
                let state = if observation.activity == super::super::status::Activity::Working {
                    SessionActivityState::Busy
                } else {
                    SessionActivityState::Idle
                };
                if state == expected {
                    assert_eq!(
                        observation.activity,
                        if expected == SessionActivityState::Busy {
                            super::super::status::Activity::Working
                        } else {
                            super::super::status::Activity::Ready
                        }
                    );
                    assert_eq!(
                        observation.outcome == Some(super::super::status::TurnOutcome::Interrupted),
                        expected_source != "hook"
                    );
                    break;
                }
            }
        }
    };
    for (interrupt, source) in [
        (b"\x03".as_slice(), "input-interrupt"),
        (b"\x1b", "input-escape"),
        (b"\x1b[99;5u", "input-interrupt"),
        (b"\x1b[27u", "input-escape"),
    ] {
        admit_hook(
            &rt,
            "interrupt",
            crate::model::Runtime::ClaudeCode,
            "PreToolUse",
            serde_json::json!({"tool_name":"AskUserQuestion"}),
        );
        wait_for_status(SessionActivityState::Busy, "hook");
        for bytes in [
            b"\x1b[A".as_slice(),
            b"\x1bd",
            b"\x1b[100;3u",
            b"\x1b[13;2u",
            b"x",
            b"\x1b[200~\x03\x1b[201~",
            b"\x1b[200~\x1b\x1b[201~",
            b"\x1b[200~\x1b[27u\x1b[99;5u\x1b[201~",
        ] {
            rt.send_bytes(&session, bytes).unwrap();
        }
        let deadline = Instant::now() + IDLE_MONITOR_POLL * 3;
        while Instant::now() < deadline {
            assert!(!matches!(
                stream.recv_status_timeout(IDLE_MONITOR_POLL),
                Ok(super::super::runtime::TestOutput::AgentObservation(
                    super::super::status::AgentObservation {
                        outcome: Some(super::super::status::TurnOutcome::Interrupted),
                        ..
                    }
                ))
            ));
        }
        rt.send_bytes(&session, interrupt).unwrap();
        wait_for_status(SessionActivityState::Idle, source);
    }
    // Cancelling a dialog can continue the turn without another prompt.
    admit_hook(
        &rt,
        "interrupt",
        crate::model::Runtime::ClaudeCode,
        "PreToolUse",
        serde_json::json!({}),
    );
    wait_for_status(SessionActivityState::Busy, "hook");
    rt.stop(&session).unwrap();
}

#[test]
#[cfg(unix)]
fn claude_streamed_kitty_escape_without_stop_allows_a_recovery_turn() {
    use super::super::status::{Activity, TurnOutcome};

    let rt = PtyRuntime::new();
    let mut spawn = spec("stream", "/bin/cat", &[]);
    spawn.agent_runtime = Some(crate::model::Runtime::ClaudeCode);
    spawn.env.insert(GENERATION_ENV.into(), "current".into());
    let (session, stream) = rt.spawn(spawn).unwrap();
    // The live #783 stream ended after UserPromptSubmit with no Stop or tool record.
    admit_hook(
        &rt,
        "stream",
        crate::model::Runtime::ClaudeCode,
        "UserPromptSubmit",
        serde_json::json!({"session_id":"main","prompt_id":"stream"}),
    );
    rt.send_bytes(&session, b"\x1b[27u").unwrap();
    let read_observations = || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut values = Vec::new();
        while values.len() < 2 && Instant::now() < deadline {
            if let Ok(super::super::runtime::TestOutput::AgentObservation(value)) =
                stream.recv_status_timeout(IDLE_MONITOR_POLL)
            {
                values.push(value);
            }
        }
        assert_eq!(values.len(), 2);
        values
    };
    let values = read_observations();
    assert_eq!(values[0].activity, Activity::Working);
    assert_eq!(values[1].activity, Activity::Ready);
    assert_eq!(values[1].outcome, Some(TurnOutcome::Interrupted));
    assert!(values[1].interactions.is_empty());
    assert_eq!(values[1].detail, None);

    for event in ["UserPromptSubmit", "Stop"] {
        admit_hook(
            &rt,
            "stream",
            crate::model::Runtime::ClaudeCode,
            event,
            serde_json::json!({"session_id":"main","prompt_id":"recovery"}),
        );
    }
    let values = read_observations();
    assert_eq!(values[0].activity, Activity::Working);
    assert_eq!(values[0].outcome, None);
    assert_eq!(values[1].activity, Activity::Ready);
    assert_eq!(values[1].outcome, Some(TurnOutcome::Completed));
    rt.stop(&session).unwrap();
}

#[test]
#[cfg(unix)]
fn spawn_cat_pipes_bytes_back() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt.spawn(spec("test-cat", "/bin/cat", &[])).unwrap();
    rt.send_bytes(&sess, b"hello\r").unwrap();

    // /bin/cat in canonical mode echoes the line back. Pull a few
    // batches until we see the substring (echoes can arrive split
    // across reads on macOS).
    let mut collected = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        match stream.recv_status_timeout(std::time::Duration::from_millis(200)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                collected.extend_from_slice(&bytes)
            }
            Ok(
                super::super::runtime::TestOutput::StatusTransition { .. }
                | super::super::runtime::TestOutput::AgentObservation(_)
                | super::super::runtime::TestOutput::ConversationStart
                | super::super::runtime::TestOutput::StatusBridgeFailed,
            ) => {}
            Err(_) => {}
        }
        if collected.windows(5).any(|w| w == b"hello") {
            break;
        }
    }
    assert!(
        collected.windows(5).any(|w| w == b"hello"),
        "expected 'hello' echo in stream, got: {:?}",
        String::from_utf8_lossy(&collected)
    );
    rt.stop(&sess).unwrap();
}

#[test]
#[cfg(unix)]
fn spawn_emits_idle_after_silence_and_busy_on_more_output() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec(
            "test-idle-detector",
            "/bin/sh",
            &["-c", "printf first; sleep 2.5; printf second; sleep 0.1"],
        ))
        .unwrap();

    let mut statuses = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        match stream.recv_status_timeout(Duration::from_millis(100)) {
            Ok(super::super::runtime::TestOutput::StatusTransition { state, source }) => {
                assert_eq!(source.as_str(), "forwarder");
                statuses.push(state);
                if statuses
                    .windows(2)
                    .any(|w| w == [SessionActivityState::Idle, SessionActivityState::Busy])
                {
                    break;
                }
            }
            Ok(
                super::super::runtime::TestOutput::Stream(_)
                | super::super::runtime::TestOutput::AgentObservation(_)
                | super::super::runtime::TestOutput::ConversationStart
                | super::super::runtime::TestOutput::StatusBridgeFailed,
            ) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    assert!(
        statuses
            .windows(2)
            .any(|w| w == [SessionActivityState::Idle, SessionActivityState::Busy]),
        "expected idle then busy transition, got {statuses:?}"
    );
    let _ = rt.stop(&sess);
}

#[test]
#[cfg(unix)]
fn spawn_keeps_symlinked_cwd_in_pwd() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut launch = spec(
        "test-logical-pwd",
        "/bin/sh",
        &["-c", "printf 'PWD=%s\\n' \"$PWD\""],
    );
    launch.cwd = Some(link.clone());
    let rt = PtyRuntime::new();
    let (_sess, stream) = rt.spawn(launch).unwrap();
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match stream.recv_status_timeout(Duration::from_millis(100)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => output.extend(bytes),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    let output = String::from_utf8_lossy(&output);
    assert!(
        output.contains(&format!("PWD={}\r\n", link.display())),
        "{output}"
    );
}

#[test]
#[cfg(unix)]
fn spawn_exit_seven_records_exit_code() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec("test-exit", "/bin/sh", &["-c", "exit 7"]))
        .unwrap();
    // Drain until EOF — the reader thread breaks on EOF and the
    // sender drops, so recv eventually returns Disconnected.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut disconnected = false;
    while std::time::Instant::now() < deadline {
        match stream.recv_status_timeout(std::time::Duration::from_millis(100)) {
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                disconnected = true;
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        }
    }
    assert!(
        disconnected,
        "output stream stayed connected after child exit"
    );
    // Reader thread also writes exit_code + alive=false before the
    // channel closes; give it a moment to settle.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let status = loop {
        let s = rt.status(&sess).unwrap().unwrap();
        if !s.alive || std::time::Instant::now() >= deadline {
            break s;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(!status.alive, "session should be marked not alive");
    assert_eq!(status.exit_code, Some(7), "exit code should be 7");
}

#[test]
fn unknown_session_status_returns_none() {
    let rt = PtyRuntime::new();
    let phantom = RuntimeSession {
        runtime: RUNTIME_LABEL.into(),
        session_id: "nonexistent".into(),
    };
    assert!(rt.status(&phantom).unwrap().is_none());
}

#[test]
#[cfg(unix)]
fn resize_succeeds_on_live_session() {
    let rt = PtyRuntime::new();
    let (sess, _stream) = rt
        .spawn(spec("test-resize", "/bin/sh", &["-c", "sleep 5"]))
        .unwrap();
    rt.resize(&sess, 120, 40).expect("resize should succeed");
    rt.stop(&sess).unwrap();
}

#[test]
fn poll_until_returns_the_first_observed_value() {
    // The deadline only bounds a hung poll; a loaded CI runner must not
    // be able to turn the third attempt into a timeout.
    let mut attempts = 0;
    let result = poll_until(Duration::from_millis(1), Duration::from_secs(30), || {
        attempts += 1;
        Ok((attempts == 3).then_some(attempts))
    })
    .unwrap();

    assert_eq!(result, Some(3));
    assert_eq!(attempts, 3);
}

#[cfg(unix)]
#[test]
fn foreground_process_tracks_shell_jobs() {
    let rt = PtyRuntime::new();
    let (session, _stream) = rt
        .spawn(spec("test-foreground", "/bin/sh", &["-i"]))
        .unwrap();
    let result = (|| -> RuntimeResult<()> {
        for expected in [false, true, false] {
            if expected {
                rt.send_bytes(&session, b"/bin/sleep 1\n")?;
            }
            poll_until(STOP_POLL, Duration::from_secs(5), || {
                Ok((rt.has_foreground_process(&session)? == Some(expected)).then_some(()))
            })?
            .ok_or_else(|| {
                RuntimeError::Msg(format!("foreground state never became {expected}"))
            })?;
        }
        Ok(())
    })();
    let stopped = rt.stop(&session);
    result.unwrap();
    stopped.unwrap();
}

#[cfg(unix)]
fn wait_for_disconnect(stream: &OutputStream) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "output stream never closed");
        if let Err(mpsc::RecvTimeoutError::Disconnected) =
            stream.recv_timeout(Duration::from_millis(50))
        {
            return;
        }
    }
}

#[cfg(unix)]
fn tracks(rt: &PtyRuntime, session_id: &str) -> bool {
    rt.sessions.lock().unwrap().contains_key(session_id)
}

#[cfg(unix)]
fn wait_until_forgotten(rt: &PtyRuntime, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while tracks(rt, session_id) {
        assert!(Instant::now() < deadline, "{session_id} was never released");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[cfg(unix)]
fn exited_session_closes_its_pty_and_is_forgotten_with_its_stream() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec("test-release-exited", "/bin/sh", &["-c", "exit 3"]))
        .unwrap();
    let handle = lookup(&rt, "test-release-exited").unwrap();

    wait_for_disconnect(&stream);

    assert!(handle.master.lock().unwrap().is_none());
    assert!(handle.writer.lock().unwrap().is_none());
    assert_eq!(rt.has_foreground_process(&sess).unwrap(), Some(false));
    assert!(rt.send_bytes(&sess, b"x").is_err());
    let status = rt
        .status(&sess)
        .unwrap()
        .expect("status while the stream lives");
    assert_eq!(status.exit_code, Some(3));
    rt.stop(&sess).unwrap();

    drop(stream);
    assert!(!tracks(&rt, "test-release-exited"));
}

#[test]
#[cfg(unix)]
fn stopped_session_is_forgotten_once_its_stream_is_dropped() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec("test-release-cancelled", "/bin/sleep", &["30"]))
        .unwrap();

    rt.stop(&sess).unwrap();
    drop(stream);

    wait_until_forgotten(&rt, "test-release-cancelled");
}

#[test]
#[cfg(unix)]
fn stopping_a_forgotten_session_succeeds() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec("test-release-late-stop", "/bin/sh", &["-c", "exit 0"]))
        .unwrap();
    wait_for_disconnect(&stream);
    drop(stream);
    wait_until_forgotten(&rt, "test-release-late-stop");

    rt.stop(&sess).unwrap();
}

#[test]
#[cfg(unix)]
fn dropping_an_old_stream_keeps_a_respawn_under_the_same_id() {
    let rt = PtyRuntime::new();
    let (_, old) = rt
        .spawn(spec("test-release-respawn", "/bin/sh", &["-c", "exit 0"]))
        .unwrap();
    wait_for_disconnect(&old);
    let (sess, new) = rt
        .spawn(spec("test-release-respawn", "/bin/sleep", &["30"]))
        .unwrap();

    drop(old);
    assert!(tracks(&rt, "test-release-respawn"));
    assert!(rt.status(&sess).unwrap().unwrap().alive);

    rt.stop(&sess).unwrap();
    drop(new);
    wait_until_forgotten(&rt, "test-release-respawn");
}

#[test]
#[cfg(unix)]
fn stop_reaps_descendants_the_agent_left_behind() {
    // Models codex: the agent dies on SIGHUP at once, forwarding nothing
    // (zsh execs into `sleep`), and its child sits in its own process
    // group (`set -m`) ignoring SIGHUP — out of reach of both the PTY
    // hangup and the process-group SIGKILL escalation.
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec(
            "test-descendant-sweep",
            if cfg!(target_os = "linux") {
                "/bin/bash"
            } else {
                "/bin/zsh"
            },
            &[
                "-c",
                "set -m; nohup sleep 300 >/dev/null 2>&1 & printf 'orphan=%d;' $!; exec sleep 299",
            ],
        ))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut output = Vec::new();
    let orphan = loop {
        assert!(
            Instant::now() < deadline,
            "child never reported its orphan pid"
        );
        match stream.recv_status_timeout(Duration::from_millis(50)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                output.extend_from_slice(&bytes)
            }
            Ok(
                super::super::runtime::TestOutput::StatusTransition { .. }
                | super::super::runtime::TestOutput::AgentObservation(_)
                | super::super::runtime::TestOutput::ConversationStart
                | super::super::runtime::TestOutput::StatusBridgeFailed,
            )
            | Err(_) => {}
        }
        let text = String::from_utf8_lossy(&output);
        if let Some(rest) = text.split("orphan=").nth(1) {
            if let Some(pid) = rest.split(';').next().and_then(|p| p.parse::<i32>().ok()) {
                break pid;
            }
        }
    };
    assert!(process_exists(orphan), "orphan not running before stop");

    rt.stop(&sess).unwrap();

    assert!(
        !process_exists(orphan),
        "descendant {orphan} survived the session stop"
    );
    let status = rt.status(&sess).unwrap().unwrap();
    assert!(!status.alive);
}

#[test]
#[cfg(unix)]
fn stop_sigkills_and_reaps_child_that_ignores_hup_and_term() {
    let rt = PtyRuntime::new();
    let (sess, stream) = rt
        .spawn(spec(
            "test-ignore-soft-signals",
            "/bin/sh",
            &[
                "-c",
                "trap '' HUP TERM; printf ready; while :; do sleep 1; done",
            ],
        ))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut output = Vec::new();
    while Instant::now() < deadline && !output.windows(5).any(|bytes| bytes == b"ready") {
        match stream.recv_status_timeout(Duration::from_millis(50)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                output.extend_from_slice(&bytes)
            }
            Ok(
                super::super::runtime::TestOutput::StatusTransition { .. }
                | super::super::runtime::TestOutput::AgentObservation(_)
                | super::super::runtime::TestOutput::ConversationStart
                | super::super::runtime::TestOutput::StatusBridgeFailed,
            ) => {}
            Err(_) => {}
        }
    }
    assert!(
        output.windows(5).any(|bytes| bytes == b"ready"),
        "child did not install signal traps"
    );
    let pid = rt.status(&sess).unwrap().unwrap().pid.unwrap();

    let started = Instant::now();
    rt.stop(&sess).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "stop exceeded latency budget: {:?}",
        started.elapsed()
    );

    let status = rt.status(&sess).unwrap().unwrap();
    assert!(!status.alive);
    assert!(status.exit_code.is_some());
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
#[cfg(unix)]
fn recorded_agent_identity_matches_command_path_or_runtime_default() {
    assert!(command_line_matches_recorded_agent(
        "claude --resume abc",
        Some("claude-code"),
        None,
    ));
    assert!(command_line_matches_recorded_agent(
        "/opt/tools/codex --model gpt-5",
        Some("codex"),
        Some("/custom/bin/codex"),
    ));
    assert!(command_line_matches_recorded_agent(
        "/Applications/Agent Tools/claude --resume abc",
        Some("claude-code"),
        Some("/Applications/Agent Tools/claude"),
    ));
    assert!(command_line_matches_recorded_agent(
        "/opt/homebrew/bin/claude --resume abc",
        Some("claude-code"),
        Some("/opt/homebrew/bin/claude"),
    ));
    assert!(!command_line_matches_recorded_agent(
        "python worker.py",
        Some("claude-code"),
        Some("claude"),
    ));
    assert!(!command_line_matches_recorded_agent(
        "claude-malicious --resume abc",
        Some("claude-code"),
        Some("claude"),
    ));
}

#[test]
fn startup_orphan_sweep_clears_resolved_pid_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::db::open_pool(&dir.path().join("runner.db")).unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions
                        (id, status, pid, agent_runtime, agent_command)
                     VALUES ('gone-orphan', 'stopped', 999999, 'test', '/bin/sleep')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                        (id, status, pid, agent_runtime, agent_command)
                     VALUES ('invalid-orphan', 'crashed', 2147483648, 'test', '/bin/sleep')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                        (id, status, pid, agent_runtime, agent_command)
                     VALUES ('running-session', 'running', 999998, 'test', '/bin/sleep')",
            [],
        )
        .unwrap();
    }

    assert_eq!(cleanup_orphan_processes_on_startup(&pool).unwrap(), 0);
    let recorded_pids: (Option<i64>, Option<i64>, Option<i64>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT
                    (SELECT pid FROM sessions WHERE id = 'gone-orphan'),
                    (SELECT pid FROM sessions WHERE id = 'invalid-orphan'),
                    (SELECT pid FROM sessions WHERE id = 'running-session')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(recorded_pids, (None, None, Some(999998)));
}

#[test]
#[cfg(unix)]
fn startup_orphan_sweep_kills_only_matching_processes() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::db::open_pool(&dir.path().join("runner.db")).unwrap();

    let mut matching = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let matching_pid = matching.id() as i32;
    wait_for_command_identity(matching_pid, "/bin/sleep");
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions
                        (id, status, pid, agent_runtime, agent_command)
                     VALUES ('matching-live', 'stopped', ?1, 'test', '/bin/sleep')",
            rusqlite::params![matching_pid],
        )
        .unwrap();
    }

    let (wait_ready_tx, wait_ready_rx) = mpsc::channel();
    let matching_wait = thread::spawn(move || {
        wait_ready_tx.send(()).unwrap();
        matching.wait().unwrap()
    });
    wait_ready_rx.recv().unwrap();
    let sweep_deadline = Instant::now() + Duration::from_secs(5);
    let reaped = loop {
        let reaped = cleanup_orphan_processes_on_startup(&pool).unwrap();
        if reaped == 1 {
            break reaped;
        }
        assert!(
            recorded_pid(&pool, "matching-live").is_some(),
            "matching pid cleared without a confirmed reap"
        );
        assert!(
            Instant::now() < sweep_deadline,
            "matching process was never reaped"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(reaped, 1);
    let matching_status = matching_wait.join().unwrap();
    assert!(!matching_status.success());
    assert_eq!(recorded_pid(&pool, "matching-live"), None);
    assert!(!process_exists(matching_pid));

    let mut mismatched = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mismatched_pid = mismatched.id() as i32;
    wait_for_command_identity(mismatched_pid, "/bin/sleep");
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions
                        (id, status, pid, agent_runtime, agent_command)
                     VALUES ('mismatched-live', 'crashed', ?1, 'test', 'claude')",
            rusqlite::params![mismatched_pid],
        )
        .unwrap();
    }

    let sweep_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert_eq!(cleanup_orphan_processes_on_startup(&pool).unwrap(), 0);
        if recorded_pid(&pool, "mismatched-live").is_none() {
            break;
        }
        assert!(
            Instant::now() < sweep_deadline,
            "mismatched pid was never resolved"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(mismatched.try_wait().unwrap().is_none());
    assert!(process_exists(mismatched_pid));
    mismatched.kill().unwrap();
    mismatched.wait().unwrap();
}

#[cfg(windows)]
#[test]
fn spawn_exit_seven_records_exit_code_windows() {
    let rt = PtyRuntime::new();
    let (session, stream) = rt
        .spawn(spec("exit-seven", "cmd", &["/c", "exit 7"]))
        .unwrap();
    let mut handshake = HostHandshake::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match stream.recv_status_timeout(Duration::from_millis(100)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                handshake.observe(&rt, &session, &bytes)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    let status = rt.status(&session).unwrap().unwrap();
    assert!(!status.alive);
    assert_eq!(status.exit_code, Some(7));
}

#[cfg(windows)]
#[test]
fn spawn_batch_roundtrips_arguments_windows() {
    let dir = tempfile::tempdir().unwrap();
    let batch = dir.path().join("argument echo.BaT");
    std::fs::write(&batch,
            "@echo off\r\n\"%RUNNER_BATCH_TEST_EXE%\" --exact session::pty_runtime::tests::batch_argument_probe --nocapture -- %*\r\n").unwrap();
    let args = [
        "one",
        "two words",
        "80% coverage",
        "say \"hello\"",
        "bang!",
        "left&right",
        "tail\\",
        "a^b>c|d",
        "%RUNNER_BATCH_TEST_EXPANSION%",
    ];
    let mut launch = spec("batch-echo", batch.to_str().unwrap(), &args);
    launch.cwd = Some(dir.path().to_path_buf());
    launch.env.insert(
        "RUNNER_BATCH_TEST_EXE".into(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    launch.env.insert(
        "RUNNER_BATCH_TEST_EXPANSION".into(),
        "must not expand".into(),
    );
    let rt = PtyRuntime::new();
    let (session, stream) = rt.spawn(launch).unwrap();
    let mut handshake = HostHandshake::default();
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match stream.recv_status_timeout(Duration::from_millis(100)) {
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                handshake.observe(&rt, &session, &bytes);
                output.extend(bytes);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    rt.stop(&session).unwrap();
    let output = String::from_utf8_lossy(&output);
    for (index, arg) in args.iter().enumerate() {
        assert!(
            output.contains(&format!("BATCH_ARG_{index}={arg}")),
            "missing argument {index}: {output}"
        );
    }
    assert!(output.contains("BATCH_ENV_CLEARED"), "{output}");
    assert!(
        output.contains(&format!("BATCH_ARG_COUNT={}", args.len())),
        "{output}"
    );
    assert!(rt
        .status(&session)
        .unwrap()
        .unwrap()
        .command
        .unwrap()
        .starts_with(batch.to_str().unwrap()));
}

#[cfg(windows)]
#[test]
fn batch_argument_probe() {
    if std::env::var_os("RUNNER_BATCH_TEST_EXE").is_none() {
        return;
    }
    let args = std::env::args()
        .skip_while(|arg| arg != "--")
        .skip(1)
        .collect::<Vec<_>>();
    println!("BATCH_ARG_COUNT={}", args.len());
    for (index, arg) in args.iter().enumerate() {
        println!("BATCH_ARG_{index}={arg}");
    }
    assert!(std::env::var_os("RUNNER_BATCH_COMMAND_LINE").is_none());
    println!("BATCH_ENV_CLEARED");
}

#[cfg(windows)]
#[test]
fn spawn_emits_idle_after_silence_and_busy_on_more_output_windows() {
    let rt = PtyRuntime::new();
    let (session, stream) = rt.spawn(spec(
            "idle-windows", "cmd", &["/d", "/c", r"echo first & %SystemRoot%\System32\timeout.exe /t 2 /nobreak >nul & %SystemRoot%\System32\timeout.exe /t 2 /nobreak >nul & echo second"],
        )).unwrap();
    let mut handshake = HostHandshake::default();
    let mut statuses = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        match stream.recv_status_timeout(Duration::from_millis(100)) {
            Ok(super::super::runtime::TestOutput::StatusTransition { state, source }) => {
                assert_eq!(source.as_str(), "forwarder");
                statuses.push(state);
                if statuses
                    .windows(2)
                    .any(|w| w == [SessionActivityState::Idle, SessionActivityState::Busy])
                {
                    break;
                }
            }
            Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
                handshake.observe(&rt, &session, &bytes)
            }
            Ok(
                super::super::runtime::TestOutput::AgentObservation(_)
                | super::super::runtime::TestOutput::ConversationStart
                | super::super::runtime::TestOutput::StatusBridgeFailed,
            ) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    rt.stop(&session).unwrap();
    assert!(
        statuses
            .windows(2)
            .any(|w| w == [SessionActivityState::Idle, SessionActivityState::Busy]),
        "got {statuses:?}"
    );
}

#[cfg(windows)]
#[test]
fn foreground_process_tracks_job_and_stop_reaps_child_windows() {
    let rt = PtyRuntime::new();
    let (session, stream) = rt.spawn(spec("job-stop", "cmd", &["/d", "/q"])).unwrap();
    assert_eq!(rt.has_foreground_process(&session).unwrap(), Some(false));
    let mut handshake = HostHandshake::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handshake.answered && Instant::now() < deadline {
        if let Ok(super::super::runtime::TestOutput::Stream(bytes)) =
            stream.recv_status_timeout(Duration::from_millis(100))
        {
            handshake.observe(&rt, &session, &bytes);
        }
    }
    assert!(
        handshake.answered,
        "host never asked for the cursor position"
    );
    rt.send_bytes(&session, b"ping -n 30 127.0.0.1 >nul\r")
        .unwrap();
    assert!(poll_until(STOP_POLL, Duration::from_secs(5), || {
        Ok((rt.has_foreground_process(&session)? == Some(true)).then_some(()))
    })
    .unwrap()
    .is_some());
    let pid = rt.status(&session).unwrap().unwrap().pid.unwrap();
    rt.stop(&session).unwrap();
    assert!(!process_exists(pid));
    assert!(!rt.status(&session).unwrap().unwrap().alive);
    assert!(poll_until(STOP_POLL, Duration::from_secs(5), || {
        Ok((rt.has_foreground_process(&session)? == Some(false)).then_some(()))
    })
    .unwrap()
    .is_some());
    rt.stop(&session).unwrap();
}

#[cfg(windows)]
#[test]
fn dropped_output_stops_conpty_and_drains_reader_windows() {
    let rt = PtyRuntime::new();
    let (session, stream) = rt
        .spawn(spec("dropped-output", "cmd", &["/d", "/q"]))
        .unwrap();
    let mut handshake = HostHandshake::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handshake.answered && Instant::now() < deadline {
        if let Ok(super::super::runtime::TestOutput::Stream(bytes)) =
            stream.recv_status_timeout(Duration::from_millis(100))
        {
            handshake.observe(&rt, &session, &bytes);
        }
    }
    assert!(
        handshake.answered,
        "host never asked for the cursor position"
    );
    while stream.recv_timeout(Duration::from_millis(200)).is_ok() {}
    let pid = rt.status(&session).unwrap().unwrap().pid.unwrap();
    drop(stream);
    let (done, completed) = mpsc::channel();
    let worker = thread::spawn(move || {
        rt.stop(&session).unwrap();
        rt.drain_workers();
        done.send(()).unwrap();
    });
    completed
        .recv_timeout(Duration::from_secs(3))
        .expect("ConPTY reader did not finish after output was dropped");
    worker.join().unwrap();
    assert!(!process_exists(pid));
}

#[cfg(windows)]
#[test]
fn stop_child_owned_by_reader_terminates_job_windows() {
    let rt = PtyRuntime::new();
    let (session, _stream) = rt.spawn(spec("reader-stop", "cmd", &["/d", "/q"])).unwrap();
    let handle = lookup(&rt, &session.session_id).unwrap();
    let mut child = handle.child.lock().unwrap().take().unwrap();
    rt.stop(&session).unwrap();
    assert!(child.try_wait().unwrap().is_some());
    assert!(!process_exists(handle.pid.unwrap()));
}

#[cfg(windows)]
#[test]
fn startup_demotes_rows_and_clears_pids_without_killing_windows() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::db::open_pool(&dir.path().join("runner.db")).unwrap();
    let pid = std::process::id();
    pool.get().unwrap().execute(
            "INSERT INTO sessions (id, status, pid, agent_runtime, agent_command) VALUES ('stale', 'running', ?1, 'test', 'runner-daemon.exe')",
            [pid],
        ).unwrap();
    assert_eq!(cleanup_stale_running_rows_on_startup(&pool).unwrap(), 1);
    assert_eq!(cleanup_orphan_processes_on_startup(&pool).unwrap(), 0);
    let (status, recorded_pid, stopped_at): (String, Option<i64>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status, pid, stopped_at FROM sessions WHERE id = 'stale'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "stopped");
    assert_eq!(recorded_pid, None);
    assert!(stopped_at.is_some());
    assert!(process_exists(pid as i32));
}

#[cfg(windows)]
fn windows_hook_bridge(
    runtime: crate::model::Runtime,
    id: &str,
    env: &[(&str, String)],
    hooks: impl FnOnce(&PtyRuntime),
    expected: usize,
) -> Vec<super::super::status::AgentObservation> {
    let rt = PtyRuntime::new();
    let mut spawn = spec(id, "cmd", &["/d", "/c", "ping -n 30 127.0.0.1 >nul"]);
    spawn.agent_runtime = Some(runtime);
    for (key, value) in env {
        spawn.env.insert((*key).into(), value.clone());
    }
    let (session, stream) = rt.spawn(spawn).unwrap();
    let mut handshake = HostHandshake::default();
    let mut recv = |timeout| match stream.recv_status_timeout(timeout) {
        Ok(super::super::runtime::TestOutput::Stream(bytes)) => {
            handshake.observe(&rt, &session, &bytes);
            None
        }
        other => Some(other),
    };
    hooks(&rt);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut values = Vec::new();
    while values.len() < expected && Instant::now() < deadline {
        if let Some(Ok(super::super::runtime::TestOutput::AgentObservation(value))) =
            recv(Duration::from_millis(50))
        {
            values.push(value);
        }
    }
    rt.hooks.retire(id);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut failed = false;
    while !failed && Instant::now() < deadline {
        failed = matches!(
            recv(Duration::from_millis(50)),
            Some(Ok(super::super::runtime::TestOutput::StatusBridgeFailed))
        );
    }
    rt.stop(&session).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut closed = false;
    while !closed && Instant::now() < deadline {
        closed = matches!(
            recv(Duration::from_millis(50)),
            Some(Err(mpsc::RecvTimeoutError::Disconnected))
        );
    }
    assert!(failed, "{id}: bridge loss not reported");
    assert!(closed, "{id}: hook watcher kept the output channel alive");
    values
}

#[test]
#[cfg(windows)]
fn codex_admitted_hooks_bridge_failure_and_teardown_windows() {
    use super::super::status::{Activity, TurnOutcome};
    let values = windows_hook_bridge(
        crate::model::Runtime::Codex,
        "codex-hooks",
        &[(
            runner_core::protocol::hook::GENERATION_ENV,
            "current".into(),
        )],
        |rt| {
            for event in ["UserPromptSubmit", "Stop", "PreToolUse", "Interrupt"] {
                admit_hook(
                    rt,
                    "codex-hooks",
                    crate::model::Runtime::Codex,
                    event,
                    serde_json::json!({"session_id":"main","turn_id":"one"}),
                );
            }
        },
        4,
    );
    assert_eq!(
        values.iter().map(|v| v.activity).collect::<Vec<_>>(),
        [
            Activity::Working,
            Activity::Ready,
            Activity::Working,
            Activity::Unavailable
        ]
    );
    assert_eq!(
        values.last().unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
}

#[test]
#[cfg(windows)]
fn copilot_admitted_hooks_bridge_failure_and_teardown_windows() {
    use super::super::status::Activity;
    let env = [(
        runner_core::protocol::hook::GENERATION_ENV,
        "current".into(),
    )];
    let values = windows_hook_bridge(
        crate::model::Runtime::Copilot,
        "copilot-hooks",
        &env,
        |rt| {
            for (event, payload) in [
                ("UserPromptSubmit", serde_json::json!({"session_id":"main"})),
                (
                    "Stop",
                    serde_json::json!({"session_id":"main","stop_reason":"end_turn"}),
                ),
                (
                    "PreToolUse",
                    serde_json::json!({
                        "session_id":"main",
                        "tool_name":"powershell",
                        "tool_input":{"command":"Start-Sleep 1"},
                    }),
                ),
            ] {
                admit_hook(
                    rt,
                    "copilot-hooks",
                    crate::model::Runtime::Copilot,
                    event,
                    payload,
                );
            }
        },
        3,
    );
    assert_eq!(
        values.iter().map(|v| v.activity).collect::<Vec<_>>(),
        [Activity::Working, Activity::Ready, Activity::Working]
    );
}

/// npm installs Codex and Claude Code as `.cmd` shims, which launch through cmd.exe
/// and its 8,191-character line. The composed hook argv must arrive intact.
#[cfg(windows)]
#[test]
fn hook_status_argv_survives_a_batch_shim_launch_windows() {
    use crate::model::Runtime;
    use crate::router::runtime::PermissionMode;
    let dir = tempfile::tempdir().unwrap();
    let app_data = std::path::Path::new(
        r"C:\Users\Jason Wang (Runner Windows Smoke)\AppData\Roaming\com.wycstudios.runner-dev",
    );
    let batch = dir.path().join("agent shim.cmd");
    std::fs::write(
            &batch,
            "@echo off\r\n\"%RUNNER_BATCH_TEST_EXE%\" --exact session::pty_runtime::tests::hook_argv_batch_probe --nocapture -- %*\r\n",
        )
        .unwrap();
    for (runtime, resume) in [
        (
            Runtime::Codex,
            vec!["resume", "019a0aab-ed06-f74f-180c-2b4b9a39a56e"],
        ),
        (
            Runtime::ClaudeCode,
            vec!["--resume", "64dec93a-961b-4647-9882-7068119c144c"],
        ),
    ] {
        let mut args = resume.into_iter().map(String::from).collect::<Vec<_>>();
        args.extend(
            crate::runtimes::adapter(runtime)
                .permissions()
                .mode_args(PermissionMode::Bypass),
        );
        args.extend(crate::runtimes::adapter(runtime).launch_args(
            &crate::runtimes::LaunchContext {
                default_model: None,
                env: &Default::default(),
                cwd: None,
                role_args: &[],
                app_data_dir: app_data,
                session_id: "01M2NCJRAFFVFJQBA0NGDWMDXR",
                resuming: true,
                model: Some("fixture-model-with-a-long-name"),
                effort: Some("xhigh"),
                codex_speed: None,
                system_prompt: None,
                first_turn: None,
                mission: false,
            },
        ));
        args.extend([
            "--add-dir".to_owned(),
            app_data
                .join(r"missions\01M2NCJRAFFVFJQBA0NGDWMDXR")
                .to_string_lossy()
                .into_owned(),
        ]);
        let length = args.iter().map(|arg| arg.len() + 3).sum::<usize>();
        eprintln!("{runtime:?} batch argv is about {length} characters");
        let out = dir.path().join(format!("{runtime:?}.json"));
        let mut launch = spec(
            "batch-hooks",
            batch.to_str().unwrap(),
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        launch.cwd = Some(dir.path().to_path_buf());
        launch.env.insert(
            "RUNNER_BATCH_TEST_EXE".into(),
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
        launch.env.insert(
            "RUNNER_BATCH_TEST_OUT".into(),
            out.to_string_lossy().into_owned(),
        );
        let rt = PtyRuntime::new();
        let (session, stream) = rt.spawn(launch).unwrap();
        let mut handshake = HostHandshake::default();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !out.exists() {
            if let Ok(super::super::runtime::TestOutput::Stream(bytes)) =
                stream.recv_status_timeout(Duration::from_millis(100))
            {
                handshake.observe(&rt, &session, &bytes);
            }
        }
        rt.stop(&session).unwrap();
        let received: Vec<String> =
            serde_json::from_slice(&std::fs::read(&out).unwrap_or_default())
                .unwrap_or_else(|_| panic!("{runtime:?}: the batch shim never ran the probe"));
        assert_eq!(received, args, "{runtime:?}");
    }
}

#[cfg(windows)]
#[test]
fn hook_argv_batch_probe() {
    let Some(out) = std::env::var_os("RUNNER_BATCH_TEST_OUT") else {
        return;
    };
    let args = std::env::args()
        .skip_while(|arg| arg != "--")
        .skip(1)
        .collect::<Vec<_>>();
    let temporary = std::path::PathBuf::from(&out).with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec(&args).unwrap()).unwrap();
    std::fs::rename(temporary, out).unwrap();
}
#[test]
#[cfg(unix)]
fn conversation_update_finishing_after_eof_is_not_forwarded() {
    struct ClosingWatcher(Arc<AtomicBool>);
    impl HookWatcher for ClosingWatcher {
        fn drain_events(
            &mut self,
            _cancel: u8,
            _emit: &mut dyn FnMut(
                crate::session::state::agent::AgentEvent,
            ) -> crate::session::state::agent::AdapterFeedback,
            session_start: &mut dyn FnMut(String),
        ) -> crate::error::Result<()> {
            self.0.store(true, Ordering::Release);
            session_start(String::new());
            Ok(())
        }
    }
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    idle_monitor_thread(
        Arc::new(Mutex::new(IdleDetector::new_at(
            DEFAULT_IDLE_THRESHOLD,
            Instant::now(),
        ))),
        tx,
        Arc::new(AtomicBool::new(false)),
        done.clone(),
        Some(Box::new(ClosingWatcher(done))),
    );
    assert!(rx
        .try_iter()
        .all(|event| !matches!(event, RuntimeOutput::ConversationStart(_))));
}

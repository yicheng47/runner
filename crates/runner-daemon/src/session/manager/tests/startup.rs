use super::*;
use crate::router::{DeliveryReservation, Router, RouterUiNotifier};
use crate::session::state::agent::AgentEvent;

struct NoUi;
impl RouterUiNotifier for NoUi {
    fn delivery_blocked(&self, _: &crate::router::DeliveryBlockedEvent) {}
}

struct StartupFixture {
    manager: Arc<SessionManager>,
    fake: Arc<FakeRuntime>,
    router: Arc<Router>,
    log: Arc<EventLog>,
    _dir: tempfile::TempDir,
    dispatcher: Mutex<crate::router::dispatch::Dispatcher>,
    now: Mutex<Instant>,
}

impl StartupFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = Arc::new(EventLog::open(dir.path()).unwrap());
        let fake = fake_runtime();
        let manager = mgr_with_fake(None, fake.clone());
        let role = role("fake", &[]);
        let router = Router::new(
            "mission".into(),
            None,
            String::new(),
            &[crate::model::SlotWithRole {
                slot: slot_for(&role),
                role,
            }],
            vec![],
            None,
            log.clone(),
            manager.clone(),
            Arc::new(NoUi),
        )
        .unwrap();
        router.register_pending_sessions(&[("tester".into(), "session".into())]);
        Self {
            manager,
            fake,
            router,
            log,
            _dir: dir,
            dispatcher: Mutex::new(crate::router::dispatch::Dispatcher::new(Instant::now())),
            now: Mutex::new(Instant::now()),
        }
    }

    fn queue(&self, text: &str) {
        let event = self
            .log
            .append(EventDraft::message(
                None,
                "mission",
                "human",
                Some("tester".into()),
                text,
            ))
            .unwrap();
        let offset = self.log.read_from(0).unwrap().last().unwrap().next_offset;
        self.router.handle_event(&event, offset);
        let count = self
            .log
            .read_from(0)
            .unwrap()
            .iter()
            .filter(|e| e.event.kind == runner_core::model::EventKind::Message)
            .count();
        crate::event_bus::BusEmitter::inbox_updated(
            &crate::router::RouterSubscriber(self.router.clone()),
            &crate::event_bus::InboxUpdate {
                mission_id: "mission".into(),
                role_handle: "tester".into(),
                last_id: Some(event.id),
                watermark: None,
                unread_count: count,
            },
        );
        crate::event_bus::BusEmitter::caught_up(&crate::router::RouterSubscriber(
            self.router.clone(),
        ));
        self.drive();
    }

    fn launch(&self, runtime: &str, pending_turn: bool, now: Instant) -> OutputStream {
        let (runtime_session, output) = self
            .fake
            .spawn(SpawnSpec {
                session_id: "session".into(),
                ..Default::default()
            })
            .unwrap();
        self.manager.install_handle_with_size_persistence(
            "session",
            SessionHandle {
                startup: Some(super::super::startup::StartupDelivery::new(
                    runtime,
                    pending_turn,
                    now,
                )),
                #[cfg(windows)]
                pending_first_turn: None,
                id: "session".into(),
                mission_id: Some("mission".into()),
                role_id: None,
                runtime_session,
                rollout_capture: None,
                forwarder: None,
                stop: output.stop_flag(),
            },
            Some(ForwarderEmitCtx {
                crew_id: None,
                mission_id: "mission".into(),
                handle: "tester".into(),
                event_log: self.log.clone(),
            }),
            Some(DEFAULT_PTY_SIZE),
            |_, _| true,
            &Capture::default(),
        );
        output
    }

    fn event(&self, runtime: Runtime, event: AgentEvent) {
        self.manager.publish_agent_event(
            "session",
            AgentEvent::Batch {
                runtime,
                events: vec![event],
            },
            &Capture::default(),
        );
    }

    fn drive(&self) {
        self.router.dispatch_at(
            &mut self.dispatcher.lock().unwrap(),
            *self.now.lock().unwrap(),
        );
    }

    fn held(&self) {
        self.drive();
        assert_eq!(
            self.manager.reserve_delivery("session").unwrap(),
            DeliveryReservation::Starting
        );
        assert!(self.fake.inputs.lock().unwrap().is_empty());
    }

    fn submitted(&self) {
        self.drive();
        *self.now.lock().unwrap() += Duration::from_secs(2);
        self.drive();
        *self.now.lock().unwrap() += Duration::from_secs(2);
        self.drive();
        assert!(
            self.fake
                .inputs
                .lock()
                .unwrap()
                .iter()
                .any(|input| matches!(input, FakeInput::Key { key, .. } if key == "Enter")),
            "readiness={:?}",
            self.manager.delivery_readiness("session")
        );
    }

    fn typed(&self) {
        self.manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::TestInputAt(Some(Instant::now())),
                crate::session::clock::state_now(),
            );
    }
}

impl Drop for StartupFixture {
    fn drop(&mut self) {
        self.manager.kill("session").unwrap();
    }
}

#[test]
fn router_holds_startup_deliveries_until_hook_turn_completion_and_resets_on_resume() {
    for runtime in [
        Runtime::ClaudeCode,
        Runtime::Codex,
        Runtime::Copilot,
        Runtime::Pi,
        Runtime::Antigravity,
    ] {
        let fixture = StartupFixture::new();
        fixture.queue("join broadcast before process launch");
        fixture.queue("task before process launch");
        let now = Instant::now();
        let output = fixture.launch(runtime.key(), true, now);
        fixture.held();
        fixture.event(runtime, AgentEvent::StartupReady);
        fixture.held();
        fixture.manager.startup_output(
            "session",
            &output.stop_flag(),
            b"Starting MCP servers (6/7)",
            now,
        );
        fixture.manager.release_settled_startup(
            "session",
            &output.stop_flag(),
            now + Duration::from_secs(60),
        );
        fixture.held();
        fixture.event(runtime, AgentEvent::TurnStarted);
        fixture.manager.release_settled_startup(
            "session",
            &output.stop_flag(),
            now + Duration::from_secs(3600),
        );
        fixture.held();
        fixture.event(
            runtime,
            AgentEvent::TurnEnded {
                outcome: TurnOutcome::Completed,
            },
        );
        fixture.submitted();
        assert!(fixture
            .fake
            .bytes_writes()
            .iter()
            .any(|(_, bytes)| String::from_utf8_lossy(bytes).contains("2 new messages")));
        fixture.manager.kill("session").unwrap();
        fixture.fake.inputs.lock().unwrap().clear();
        fixture.queue("task before resume");
        let resume = fixture.launch(runtime.key(), false, now);
        fixture.held();
        if runtime == Runtime::Antigravity {
            fixture
                .manager
                .startup_output("session", &resume.stop_flag(), b"resumed prompt", now);
            fixture.manager.release_settled_startup(
                "session",
                &resume.stop_flag(),
                now + Duration::from_secs(10),
            );
            fixture.manager.note_forwarder_transition(
                "session",
                SessionActivityState::Idle,
                StatusSource::Forwarder,
            );
        } else {
            fixture.event(runtime, AgentEvent::StartupReady);
        }
        fixture.submitted();
    }
}

#[test]
fn router_output_fallback_requires_output_startup_grace_and_idle_including_bridge_failure() {
    for runtime in [
        Runtime::OpenCode,
        Runtime::Cursor,
        Runtime::Grok,
        Runtime::Trae,
        Runtime::Shell,
        Runtime::Codex,
    ] {
        let fixture = StartupFixture::new();
        fixture.queue("task");
        let now = Instant::now();
        let output = fixture.launch(runtime.key(), true, now);
        let launch = output.stop_flag();
        if runtime == Runtime::Codex {
            fixture
                .manager
                .status_bridge_failed("session", &Capture::default());
        }
        fixture
            .manager
            .release_settled_startup("session", &launch, now + Duration::from_secs(60));
        fixture.held();
        fixture
            .manager
            .startup_output("session", &launch, b"startup screen", now);
        fixture
            .manager
            .release_settled_startup("session", &launch, now + Duration::from_secs(2));
        fixture.held();
        fixture.manager.startup_output(
            "session",
            &launch,
            b"first answer",
            now + Duration::from_secs(11),
        );
        fixture
            .manager
            .release_settled_startup("session", &launch, now + Duration::from_secs(12));
        fixture.held();
        fixture
            .manager
            .release_settled_startup("session", &launch, now + Duration::from_secs(13));
        fixture.manager.note_forwarder_transition(
            "session",
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        fixture.submitted();
    }
}

#[test]
fn codex_conversation_recovery_in_first_turn_falls_back_to_output_release() {
    let fixture = StartupFixture::new();
    fixture.queue("answer after recovery");
    let now = Instant::now();
    let output = fixture.launch("codex", true, now);
    let launch = output.stop_flag();
    fixture.event(Runtime::Codex, AgentEvent::TurnStarted);
    fixture
        .manager
        .startup_output("session", &launch, b"first turn", now);
    fixture.event(Runtime::Codex, AgentEvent::ConversationRecovered);
    fixture
        .manager
        .release_settled_startup("session", &launch, now + Duration::from_secs(1));
    fixture.held();
    fixture
        .manager
        .release_settled_startup("session", &launch, now + Duration::from_secs(12));
    fixture.submitted();
}

#[test]
fn codex_interrupted_first_turn_releases_the_startup_hold_once_the_abort_settles() {
    let fixture = StartupFixture::new();
    fixture.queue("task after interrupt");
    let now = Instant::now();
    let _output = fixture.launch("codex", true, now);
    fixture.event(Runtime::Codex, AgentEvent::TurnStarted);
    fixture.event(
        Runtime::Codex,
        AgentEvent::TurnEnded {
            outcome: crate::session::status::TurnOutcome::Interrupted,
        },
    );
    fixture.held();
    fixture.manager.publish_agent_event(
        "session",
        AgentEvent::Transcript {
            runtime: Runtime::Codex,
            events: vec![AgentEvent::AbortSettled],
        },
        &Capture::default(),
    );
    fixture.submitted();
}

#[test]
fn codex_failed_first_turn_releases_the_startup_hold() {
    let fixture = StartupFixture::new();
    fixture.queue("task after failure");
    let now = Instant::now();
    let _output = fixture.launch("codex", true, now);
    fixture.event(Runtime::Codex, AgentEvent::TurnStarted);
    fixture.held();
    fixture.manager.publish_agent_event(
        "session",
        AgentEvent::Transcript {
            runtime: Runtime::Codex,
            events: vec![AgentEvent::TurnEnded {
                outcome: crate::session::status::TurnOutcome::Failed,
            }],
        },
        &Capture::default(),
    );
    fixture.submitted();
}

#[test]
fn startup_exit_preserves_the_backlog_and_delivers_after_resume() {
    let fixture = StartupFixture::new();
    fixture.queue("task before exit");
    let now = Instant::now();
    let _output = fixture.launch("codex", true, now);
    fixture.held();
    fixture.manager.kill("session").unwrap();
    fixture.drive();
    assert!(!fixture.log.read_from(0).unwrap().iter().any(|entry| entry
        .event
        .payload
        .to_string()
        .contains("dropped")));
    let _resume = fixture.launch("codex", false, now);
    fixture.event(Runtime::Codex, AgentEvent::StartupReady);
    fixture.submitted();
}

#[test]
fn completed_startup_keeps_a_human_draft_held_until_input_clears() {
    let fixture = StartupFixture::new();
    fixture.queue("task");
    let now = Instant::now();
    let _output = fixture.launch("codex", true, now);
    fixture.typed();
    fixture.manager.report_input_state(
        "session",
        InputObservation {
            composing: false,
            state: InputState::Drafting,
            since: now,
            composer_visible: true,
        },
    );
    fixture.event(Runtime::Codex, AgentEvent::TurnStarted);
    fixture.event(
        Runtime::Codex,
        AgentEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
        },
    );
    assert_eq!(
        fixture.manager.reserve_delivery("session").unwrap(),
        DeliveryReservation::Drafting {
            composer_visible: true
        }
    );
    assert!(fixture.fake.inputs.lock().unwrap().is_empty());
    fixture.manager.report_input_state(
        "session",
        InputObservation {
            composing: false,
            state: InputState::Idle,
            since: now,
            composer_visible: true,
        },
    );
    fixture.submitted();
}

#[test]
fn readiness_is_owned_by_the_session_manager_before_and_after_attachment() {
    use crate::router::DeliveryReadiness as R;
    let fixture = StartupFixture::new();
    assert_eq!(
        fixture.manager.delivery_readiness("session"),
        R::Unavailable
    );
    fixture
        .manager
        .session_state_or_insert("session")
        .lock()
        .unwrap()
        .pending_spawn = true;
    assert_eq!(fixture.manager.delivery_readiness("session"), R::Pending);
    let _output = fixture.launch("codex", false, Instant::now());
    assert_eq!(fixture.manager.delivery_readiness("session"), R::Starting);
    fixture.event(Runtime::Codex, AgentEvent::StartupReady);
    assert!(matches!(
        fixture.manager.delivery_readiness("session"),
        R::Ready { .. }
    ));
    let token = match fixture.manager.reserve_delivery("session").unwrap() {
        DeliveryReservation::Ready(token) => token,
        other => panic!("{other:?}"),
    };
    assert_eq!(fixture.manager.delivery_readiness("session"), R::InFlight);
    fixture.manager.finish_delivery("session", token);
    fixture.manager.kill("session").unwrap();
    assert_eq!(
        fixture.manager.delivery_readiness("session"),
        R::Unavailable
    );
}

#[test]
fn a_synthetic_busy_publication_cannot_confirm_an_attempt() {
    let fixture = StartupFixture::new();
    let _output = fixture.launch("codex", false, Instant::now());
    fixture.event(Runtime::Codex, AgentEvent::StartupReady);
    let before = fixture.manager.delivery_snapshot("session").unwrap();
    fixture
        .manager
        .synthesize_wake_busy(
            "session",
            EventDraft::signal(
                None,
                "mission",
                "tester",
                SignalType::new("session_status"),
                serde_json::json!({"state":"busy"}),
            ),
        )
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .delivery_snapshot("session")
            .unwrap()
            .turn_started,
        before.turn_started
    );
    fixture.event(Runtime::Codex, AgentEvent::TurnStarted);
    assert_ne!(
        fixture
            .manager
            .delivery_snapshot("session")
            .unwrap()
            .turn_started,
        before.turn_started
    );
}

#[test]
fn approval_response_and_release_are_readiness_events_not_composer_clear_evidence() {
    #[derive(Default)]
    struct Events(Mutex<Vec<crate::router::SessionDeliveryEvent>>);
    impl crate::router::SessionDeliveryListener for Events {
        fn session_delivery_event(&self, _: &str, event: crate::router::SessionDeliveryEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    let fixture = StartupFixture::new();
    let _output = fixture.launch("claude-code", false, Instant::now());
    fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
    let events = Arc::new(Events::default());
    let listener: Arc<dyn crate::router::SessionDeliveryListener> = events.clone();
    fixture
        .manager
        .register_delivery_listener("session", Arc::downgrade(&listener));
    fixture.event(
        Runtime::ClaudeCode,
        AgentEvent::InteractionOpened {
            reason: crate::session::status::WaitReason::Approval,
            owners: vec!["approval".into()],
        },
    );
    fixture
        .manager
        .inject_direct_stdin("session", b"\r", &Capture::default())
        .unwrap();
    fixture.event(
        Runtime::ClaudeCode,
        AgentEvent::InteractionClosed {
            owner: "approval".into(),
            reason: None,
        },
    );
    for bytes in [b"\r".as_slice(), b"\x03".as_slice()] {
        fixture
            .manager
            .inject_direct_stdin("session", bytes, &Capture::default())
            .unwrap();
    }
    let observed = events.0.lock().unwrap().clone();
    assert!(observed.contains(&crate::router::SessionDeliveryEvent::ReadinessChanged));
    assert!(!observed.contains(&crate::router::SessionDeliveryEvent::InputCleared));
    for state in [InputState::Drafting, InputState::Idle] {
        fixture.manager.report_input_state(
            "session",
            InputObservation {
                state,
                since: Instant::now(),
                composing: false,
                composer_visible: true,
            },
        );
    }
    assert!(events
        .0
        .lock()
        .unwrap()
        .contains(&crate::router::SessionDeliveryEvent::InputCleared));
}

#[test]
fn resized_unsubmitted_input_waits_for_composer_output_after_a_successful_local_enter() {
    use crate::router::DeliveryReadiness;
    use runner_terminal::input_state::InputEvent;
    use runner_terminal::mappings::InputKind;

    let fixture = StartupFixture::new();
    let _output = fixture.launch("claude-code", false, Instant::now());
    fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
    let pool = db::open_pool(&fixture._dir.path().join("terminal.db")).unwrap();
    let events: Arc<dyn SessionEvents> = Arc::new(Capture::default());
    let terminal = fixture
        .manager
        .create_terminal("session", 80, 4, &pool, &events)
        .unwrap();
    fixture
        .manager
        .session_state("session")
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal.clone());
    let emit = |bytes: &[u8]| {
        fixture
            .manager
            .ingest_output_chunk("session", Some("mission"), bytes, events.as_ref());
    };
    let advance = |duration| {
        *fixture.now.lock().unwrap() += duration;
        let now = *fixture.now.lock().unwrap();
        crate::session::clock::at(now, 0, || fixture.drive());
    };
    emit(b"> ");
    fixture.drive();
    let revision = terminal.input_reset_guard();
    fixture.queue("first");
    let body = match &fixture.fake.inputs.lock().unwrap()[0] {
        FakeInput::Bytes { bytes, .. } => bytes.clone(),
        other => panic!("{other:?}"),
    };
    emit(&body);
    assert_eq!(terminal.input_reset_guard(), revision);
    advance(Duration::from_millis(300));
    advance(crate::router::dispatch::CONFIRM_WINDOW);
    advance(crate::router::dispatch::CONFIRM_WINDOW);
    let initial_inputs = fixture.fake.inputs.lock().unwrap().clone();
    assert_eq!(initial_inputs.len(), 3, "{initial_inputs:?}");
    terminal.resize(80, 5);
    terminal.observe_input(&InputEvent::Key {
        kind: InputKind::Submit,
    });
    let now = *fixture.now.lock().unwrap();
    crate::session::clock::at(now, 0, || {
        fixture
            .manager
            .inject_direct_stdin("session", b"\r", events.as_ref())
            .unwrap();
        fixture.queue("second");
    });
    advance(Duration::from_secs(120));
    let now = *fixture.now.lock().unwrap();
    assert_eq!(
        crate::session::clock::at(now, 0, || fixture.manager.delivery_readiness("session")),
        DeliveryReadiness::Unsent
    );
    assert_eq!(fixture.fake.inputs.lock().unwrap().len(), 4);
    assert!(terminal
        .input_rows()
        .iter()
        .any(|row| row.contains("[inbox]")));
    emit(b"\x1b[2J\x1b[H> ");
    fixture.event(
        Runtime::ClaudeCode,
        AgentEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
        },
    );
    crate::session::clock::at(now, 0, || fixture.drive());
    advance(Duration::from_millis(500));
    let inputs = fixture.fake.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 5);
    assert!(matches!(&inputs[4], FakeInput::Bytes { bytes, .. }
        if String::from_utf8_lossy(bytes).contains("2 new messages")));
    drop(inputs);
    fixture
        .manager
        .session_state("session")
        .unwrap()
        .lock()
        .unwrap()
        .terminal = None;
    let workers = terminal.take_workers();
    drop(terminal);
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn an_ignored_delivery_enter_is_resubmitted_at_turn_end_instead_of_holding_as_a_draft() {
    use crate::router::{DeliveryReadiness, SessionDeliveryEvent, SessionDeliveryListener};

    // With the echo after the Enter and no redraw, only the tracker can find the notice.
    for echo_before_enter in [true, false] {
        let fixture = StartupFixture::new();
        let _output = fixture.launch("codex", false, Instant::now());
        fixture.event(Runtime::Codex, AgentEvent::StartupReady);
        let pool = db::open_pool(&fixture._dir.path().join("terminal.db")).unwrap();
        let events: Arc<dyn SessionEvents> = Arc::new(Capture::default());
        let terminal = fixture
            .manager
            .create_terminal("session", 80, 4, &pool, &events)
            .unwrap();
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal.clone());
        let emit = |bytes: &[u8]| {
            fixture
                .manager
                .ingest_output_chunk("session", Some("mission"), bytes, events.as_ref());
        };
        let advance = |duration| {
            *fixture.now.lock().unwrap() += duration;
            let now = *fixture.now.lock().unwrap();
            crate::session::clock::at(now, 0, || fixture.drive());
        };
        let readiness = |later| {
            let now = *fixture.now.lock().unwrap() + later;
            crate::session::clock::at(now, 0, || fixture.manager.delivery_readiness("session"))
        };
        let inputs = || fixture.fake.inputs.lock().unwrap().clone();
        emit("\x1b[2;1H› Ask Codex".as_bytes());
        fixture.drive();
        fixture.queue("first");
        let body = match &inputs()[0] {
            FakeInput::Bytes { bytes, .. } => String::from_utf8(bytes.clone()).unwrap(),
            other => panic!("{other:?}"),
        };
        let unchanged = format!("\r\x1b[K› {body}");
        if echo_before_enter {
            emit(unchanged.as_bytes());
        }
        advance(Duration::from_millis(300));
        emit(unchanged.as_bytes());
        advance(crate::router::dispatch::CONFIRM_WINDOW);
        if echo_before_enter {
            emit(unchanged.as_bytes());
        }
        advance(crate::router::dispatch::CONFIRM_WINDOW);
        let enters = inputs().len();
        assert_eq!(enters, 3, "echo_before_enter={echo_before_enter}");
        assert_eq!(
            readiness(Duration::from_secs(26 * 60)),
            DeliveryReadiness::Unsent,
            "echo_before_enter={echo_before_enter}"
        );
        let now = *fixture.now.lock().unwrap();
        crate::session::clock::at(now, 0, || fixture.queue("second"));
        advance(Duration::from_secs(60));
        assert_eq!(inputs().len(), enters);

        fixture
            .router
            .session_delivery_event("session", SessionDeliveryEvent::Idle);
        advance(Duration::ZERO);
        let resubmitted = inputs();
        assert_eq!(resubmitted.len(), enters + 1, "{resubmitted:?}");
        assert!(
            matches!(resubmitted.last(), Some(FakeInput::Key { key, .. }) if key == "Enter"),
            "{resubmitted:?}"
        );
        emit("\r\x1b[K› Ask Codex".as_bytes());
        advance(crate::router::dispatch::CONFIRM_WINDOW);
        assert!(matches!(
            readiness(Duration::ZERO),
            DeliveryReadiness::Ready { .. } | DeliveryReadiness::Unconfirmed
        ));
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = None;
        let workers = terminal.take_workers();
        drop(terminal);
        for worker in workers {
            worker.join().unwrap();
        }
    }
}

#[test]
fn rejected_native_probe_submits_hold_readiness_without_more_output() {
    use crate::router::DeliveryReadiness;
    use runner_terminal::input_state::InputEvent;
    use runner_terminal::mappings::InputKind;

    for reprobe in [false, true] {
        let fixture = StartupFixture::new();
        let _output = fixture.launch("codex", false, Instant::now());
        fixture.event(Runtime::Codex, AgentEvent::StartupReady);
        let pool = db::open_pool(&fixture._dir.path().join("terminal.db")).unwrap();
        let events: Arc<dyn SessionEvents> = Arc::new(Capture::default());
        let terminal = fixture
            .manager
            .create_terminal("session", 80, 10, &pool, &events)
            .unwrap();
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal.clone());
        let emit = |bytes: &[u8]| {
            fixture
                .manager
                .ingest_output_chunk("session", Some("mission"), bytes, events.as_ref());
        };
        emit("\x1b[2;1H› Ask Codex".as_bytes());
        let submit = InputEvent::Key {
            kind: InputKind::Submit,
        };
        if reprobe {
            terminal.observe_input(&InputEvent::Paste { text: "old".into() });
            emit("\x1b[2;1H\x1b[K› old".as_bytes());
            terminal.observe_input(&submit);
        }
        terminal.observe_input(&InputEvent::Paste {
            text: "line one\nline two".into(),
        });
        terminal.observe_input(&submit);
        emit("\x1b[2;1H\x1b[K› line one\x1b[3;1H  line two\x1b[4;3H".as_bytes());
        fixture.typed();
        let (readiness, reservation) =
            crate::session::clock::at(Instant::now() + Duration::from_secs(120), 0, || {
                (
                    fixture.manager.delivery_readiness("session"),
                    fixture.manager.reserve_delivery("session").unwrap(),
                )
            });
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = None;
        let workers = terminal.take_workers();
        drop(terminal);
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(readiness, DeliveryReadiness::Drafting, "reprobe={reprobe}");
        assert_eq!(
            reservation,
            DeliveryReservation::Drafting {
                composer_visible: true
            },
            "reprobe={reprobe}"
        );
    }
}

#[test]
fn codex_submitted_history_releases_readiness_but_a_later_collapsed_draft_holds() {
    use crate::router::DeliveryReadiness;
    use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent, FixtureInput};

    let fixture = StartupFixture::new();
    let _output = fixture.launch("codex", false, Instant::now());
    fixture.event(Runtime::Codex, AgentEvent::StartupReady);
    let pool = db::open_pool(&fixture._dir.path().join("terminal.db")).unwrap();
    let events: Arc<dyn SessionEvents> = Arc::new(Capture::default());
    let terminal = fixture
        .manager
        .create_terminal("session", 80, 12, &pool, &events)
        .unwrap();
    fixture
        .manager
        .session_state("session")
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal.clone());
    let replay = Fixture::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../runner-terminal/fixtures/input-codex-submit-history.ndjson"),
    )
    .unwrap();
    for event in replay.events {
        let ms = match event {
            FixtureEvent::Data { ms, data } => {
                fixture.manager.ingest_output_chunk(
                    "session",
                    Some("mission"),
                    &decode_chunk(&data).unwrap(),
                    events.as_ref(),
                );
                ms
            }
            FixtureEvent::Input {
                input: FixtureInput::Event(input),
                ..
            } => {
                terminal.observe_input(&input);
                fixture.typed();
                continue;
            }
            _ => panic!("unexpected event in the submitted history fixture"),
        };
        let readiness =
            crate::session::clock::at(Instant::now() + Duration::from_secs(120), 0, || {
                fixture.manager.delivery_readiness("session")
            });
        match ms {
            30 | 7000 => assert!(
                matches!(readiness, DeliveryReadiness::Ready { .. }),
                "{ms}: {readiness:?}"
            ),
            2005 | 6000 | 6020 => assert_eq!(readiness, DeliveryReadiness::Drafting, "{ms}"),
            _ => {}
        }
    }
    fixture
        .manager
        .session_state("session")
        .unwrap()
        .lock()
        .unwrap()
        .terminal = None;
    let workers = terminal.take_workers();
    drop(terminal);
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn generated_submissions_accept_rotating_placeholders_and_deliver_later_backlogs() {
    for second_enter in [false, true] {
        let fixture = StartupFixture::new();
        let _output = fixture.launch("claude-code", false, Instant::now());
        fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
        let pool = db::open_pool(&fixture._dir.path().join("terminal.db")).unwrap();
        let events: Arc<dyn SessionEvents> = Arc::new(Capture::default());
        let terminal = fixture
            .manager
            .create_terminal("session", 80, 4, &pool, &events)
            .unwrap();
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal.clone());
        let emit = |bytes: &[u8]| {
            fixture
                .manager
                .ingest_output_chunk("session", Some("mission"), bytes, events.as_ref());
        };
        let advance = |duration| {
            *fixture.now.lock().unwrap() += duration;
            let now = *fixture.now.lock().unwrap();
            crate::session::clock::at(now, 0, || fixture.drive());
        };
        emit("\x1b[2;1H❯ First placeholder".as_bytes());
        fixture.drive();
        let revision = terminal.input_reset_guard();
        fixture.queue("first");
        let body = match &fixture.fake.inputs.lock().unwrap()[0] {
            FakeInput::Bytes { bytes, .. } => String::from_utf8(bytes.clone()).unwrap(),
            other => panic!("{other:?}"),
        };
        emit(format!("\r\x1b[K❯ {body}").as_bytes());
        advance(Duration::from_millis(300));
        if second_enter {
            advance(crate::router::dispatch::CONFIRM_WINDOW);
        }
        emit("\r\x1b[K❯ Different placeholder".as_bytes());
        let now = *fixture.now.lock().unwrap();
        crate::session::clock::at(now, 0, || fixture.drive());
        advance(crate::router::dispatch::CONFIRM_WINDOW);
        let now = *fixture.now.lock().unwrap();
        assert!(matches!(
            crate::session::clock::at(now, 0, || fixture.manager.delivery_readiness("session")),
            crate::router::DeliveryReadiness::Ready { .. }
                | crate::router::DeliveryReadiness::Unconfirmed
        ));
        assert_eq!(terminal.input_reset_guard(), revision);
        crate::session::clock::at(now, 0, || fixture.queue("second"));
        assert!(
            !matches!(fixture.fake.inputs.lock().unwrap().last(), Some(FakeInput::Bytes { bytes, .. }) if String::from_utf8_lossy(bytes).contains("2 new messages"))
        );
        fixture.event(
            Runtime::ClaudeCode,
            AgentEvent::TurnEnded {
                outcome: TurnOutcome::Completed,
            },
        );
        advance(Duration::from_millis(300));
        let inputs = fixture.fake.inputs.lock().unwrap().clone();
        assert!(
            matches!(inputs.last(),
            Some(FakeInput::Bytes { bytes, .. }) if String::from_utf8_lossy(bytes).contains("2 new messages")),
            "second_enter={second_enter} inputs={inputs:?}"
        );
        fixture
            .manager
            .session_state("session")
            .unwrap()
            .lock()
            .unwrap()
            .terminal = None;
        let workers = terminal.take_workers();
        drop(terminal);
        for worker in workers {
            worker.join().unwrap();
        }
    }
}

#[test]
fn generated_submit_cannot_publish_over_a_newer_composition_observation() {
    use runner_terminal::delivery_probe::InputProbe;
    use runner_terminal::input_state::InputEvent;
    use runner_terminal::terminal::{HostResult, ModelOptions, TerminalHost, TerminalModel};
    use std::sync::{mpsc, Condvar, Weak};

    struct Host {
        manager: Weak<SessionManager>,
        start: Mutex<Option<mpsc::Sender<()>>>,
        composed: (Mutex<bool>, Condvar),
    }
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> HostResult<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> HostResult<()> {
            Ok(())
        }
        fn report_input_state(&self, id: &str, observation: InputObservation) {
            if observation.state == InputState::Submitted {
                if let Some(start) = self.start.lock().unwrap().take() {
                    start.send(()).unwrap();
                }
                let (composed, _) = self
                    .composed
                    .1
                    .wait_timeout_while(
                        self.composed.0.lock().unwrap(),
                        Duration::from_secs(2),
                        |composed| !*composed,
                    )
                    .unwrap();
                assert!(*composed);
            }
            self.manager
                .upgrade()
                .unwrap()
                .report_input_state(id, observation);
            if observation.composing {
                *self.composed.0.lock().unwrap() = true;
                self.composed.1.notify_all();
            }
        }
        fn set_live_title(&self, _: &str, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> HostResult<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
    }

    for guarded in [false, true] {
        let fixture = StartupFixture::new();
        let _output = fixture.launch("claude-code", false, Instant::now());
        fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
        let (start, go) = mpsc::channel();
        let (ready, attempted) = mpsc::channel();
        let host = Arc::new(Host {
            manager: Arc::downgrade(&fixture.manager),
            start: Mutex::new(Some(start)),
            composed: (Mutex::new(false), Condvar::new()),
        });
        let terminal = TerminalModel::new(
            host.clone(),
            "session".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        terminal.publish_input(terminal.feed_output(1, b"> "));
        let text = "[inbox] task";
        let mut probe = InputProbe::new(terminal.input_rows(), text);
        terminal.track_delivery_input(text);
        terminal.publish_input(terminal.feed_output(2, text.as_bytes()));
        probe.locate(&terminal.input_rows());
        let observing = terminal.clone();
        let observer = std::thread::spawn(move || {
            go.recv_timeout(Duration::from_secs(2)).unwrap();
            ready.send(()).unwrap();
            observing.observe_input(&InputEvent::Composing { composing: true });
        });
        if guarded {
            assert!(terminal
                .with_unchanged_input(terminal.input_reset_guard(), &probe, || {
                    host.start.lock().unwrap().take().unwrap().send(()).unwrap();
                    attempted.recv_timeout(Duration::from_secs(2)).unwrap();
                })
                .is_some());
        } else {
            terminal.track_delivery_submit();
            if let Some(start) = host.start.lock().unwrap().take() {
                start.send(()).unwrap();
            }
        }
        observer.join().unwrap();
        crate::session::clock::at(Instant::now() + Duration::from_secs(120), 0, || {
            assert_eq!(
                fixture.manager.delivery_readiness("session"),
                crate::router::DeliveryReadiness::Drafting
            );
            assert!(matches!(
                fixture.manager.reserve_delivery("session").unwrap(),
                DeliveryReservation::Drafting { .. }
            ));
        });
        let workers = terminal.take_workers();
        drop(terminal);
        for worker in workers {
            worker.join().unwrap();
        }
    }
}

#[test]
fn a_return_that_starts_no_turn_frees_the_member_for_its_queued_notice() {
    for runtime in [Runtime::ClaudeCode, Runtime::Codex] {
        let fixture = StartupFixture::new();
        let output = fixture.launch(runtime.key(), false, Instant::now());
        fixture.event(runtime, AgentEvent::StartupReady);
        let stop = output.stop_flag();
        let events = Capture::default();
        let start = *fixture.now.lock().unwrap();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let advance_to = |secs: u64| {
            *fixture.now.lock().unwrap() = at(secs);
            crate::session::clock::at(at(secs), 0, || fixture.drive());
        };
        let activity = || fixture.manager.activity_snapshot()["session"];
        let notices = || {
            fixture
                .fake
                .inputs
                .lock()
                .unwrap()
                .iter()
                .filter(|input| matches!(input, FakeInput::Bytes { bytes, .. } if String::from_utf8_lossy(bytes).contains("new message")))
                .count()
        };
        let statuses = || -> Vec<(String, String)> {
            fixture
                .log
                .read_from(0)
                .unwrap()
                .into_iter()
                .filter(|entry| {
                    entry.event.signal_type.as_ref().map(SignalType::as_str)
                        == Some("session_status")
                })
                .map(|entry| {
                    (
                        entry.event.payload["state"].as_str().unwrap().to_owned(),
                        entry.event.payload["source"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };

        // Return on an empty composer, from the keyboard rather than the router.
        crate::session::clock::at(at(0), 0, || {
            fixture
                .manager
                .inject_direct_stdin("session", b"\r", &events)
                .unwrap()
        });
        assert_eq!(activity(), SessionActivityState::Busy, "{runtime:?}");
        if runtime == Runtime::Codex {
            // Codex repaints its composer on an Enter it ignores; that is not a turn.
            fixture.manager.ingest_output_chunk(
                "session",
                Some("mission"),
                b"\x1b[2;1H\xe2\x80\xba Ask Codex",
                &events,
            );
        }
        advance_to(3);
        crate::session::clock::at(at(3), 0, || fixture.queue("hello"));
        assert_eq!(
            crate::session::clock::at(at(3), 0, || fixture.manager.delivery_readiness("session")),
            crate::router::DeliveryReadiness::Unconfirmed,
            "{runtime:?}"
        );
        assert_eq!(notices(), 0, "{runtime:?}");

        crate::session::clock::at(at(9), 0, || {
            fixture
                .manager
                .expire_submit_busy("session", &stop, &events)
        });
        advance_to(9);
        assert_eq!(activity(), SessionActivityState::Busy, "{runtime:?}");
        assert_eq!(notices(), 0, "{runtime:?}");

        crate::session::clock::at(at(11), 0, || {
            fixture
                .manager
                .expire_submit_busy("session", &stop, &events)
        });
        assert_eq!(activity(), SessionActivityState::Idle, "{runtime:?}");
        advance_to(11);
        assert_eq!(notices(), 1, "{runtime:?}: the queued notice is delivered");
        assert!(
            statuses().contains(&("idle".to_owned(), "input-timeout".to_owned())),
            "{runtime:?}: {:?}",
            statuses()
        );
        let published = events.status.lock().unwrap();
        assert!(
            published
                .iter()
                .any(|event| event.state == SessionActivityState::Idle
                    && event.source.as_str() == "input-timeout"),
            "{runtime:?}"
        );
    }
}

#[test]
fn a_turn_that_reports_in_time_is_never_reverted_and_a_late_one_returns_to_busy() {
    for late in [false, true] {
        let fixture = StartupFixture::new();
        let output = fixture.launch("claude-code", false, Instant::now());
        fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
        let stop = output.stop_flag();
        let events = Capture::default();
        let start = *fixture.now.lock().unwrap();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let activity = || fixture.manager.activity_snapshot()["session"];
        crate::session::clock::at(at(0), 0, || {
            fixture
                .manager
                .inject_direct_stdin("session", b"\r", &events)
                .unwrap()
        });
        let expire = |secs: u64| {
            crate::session::clock::at(at(secs), 0, || {
                fixture
                    .manager
                    .expire_submit_busy("session", &stop, &events)
            })
        };
        if !late {
            crate::session::clock::at(at(2), 0, || {
                fixture.event(Runtime::ClaudeCode, AgentEvent::TurnStarted)
            });
        }
        expire(11);
        if late {
            assert_eq!(activity(), SessionActivityState::Idle);
            crate::session::clock::at(at(14), 0, || {
                fixture.event(Runtime::ClaudeCode, AgentEvent::TurnStarted)
            });
        }
        assert_eq!(activity(), SessionActivityState::Busy, "late={late}");
        assert_eq!(
            fixture.manager.agent_status("session").observation.activity,
            Activity::Working,
            "late={late}"
        );
        crate::session::clock::at(at(40), 0, || {
            fixture.event(
                Runtime::ClaudeCode,
                AgentEvent::TurnEnded {
                    outcome: TurnOutcome::Completed,
                },
            )
        });
        assert_eq!(activity(), SessionActivityState::Idle, "late={late}");
        expire(60);
        assert_eq!(activity(), SessionActivityState::Idle, "late={late}");
    }
}

#[test]
fn the_forwarder_loop_withdraws_a_submit_busy_that_outlived_its_window() {
    let fixture = StartupFixture::new();
    let output = fixture.launch("claude-code", false, Instant::now());
    fixture.event(Runtime::ClaudeCode, AgentEvent::StartupReady);
    let events = Arc::new(Capture::default());
    // The forwarder reads the real clock, so the Return goes in a window ago.
    let past = Instant::now() - crate::session::state::SUBMIT_TURN_START_WINDOW;
    crate::session::clock::at(past, 0, || {
        fixture
            .manager
            .inject_direct_stdin("session", b"\r", events.as_ref())
            .unwrap()
    });
    assert_eq!(
        fixture.manager.activity_snapshot()["session"],
        SessionActivityState::Busy
    );
    let rt_session = fixture.manager.live_runtime_session("session").unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let forwarder = fixture.manager.start_forwarder_thread(
        "session".into(),
        String::new(),
        Some("mission".into()),
        rt_session,
        output,
        pool_with_schema(),
        events.clone(),
        role("fake", &[]),
        None,
        true,
        None,
        app_data.path().to_path_buf(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.manager.activity_snapshot()["session"] != SessionActivityState::Idle {
        assert!(
            Instant::now() < deadline,
            "the forwarder never withdrew the busy"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(events
        .status
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.state == SessionActivityState::Idle
            && event.source.as_str() == "input-timeout"));
    fixture.manager.kill("session").unwrap();
    forwarder.join().unwrap();
}

#[test]
fn output_after_a_return_leaves_an_output_only_session_to_its_idle_detector() {
    for output_follows in [false, true] {
        let fixture = StartupFixture::new();
        let output = fixture.launch("opencode", false, Instant::now());
        let stop = output.stop_flag();
        let events = Capture::default();
        assert!(fixture.manager.note_forwarder_transition(
            "session",
            SessionActivityState::Idle,
            StatusSource::Forwarder
        ));
        let start = *fixture.now.lock().unwrap();
        crate::session::clock::at(start, 0, || {
            fixture
                .manager
                .inject_direct_stdin("session", b"\r", &events)
                .unwrap()
        });
        if output_follows {
            fixture.manager.ingest_output_chunk(
                "session",
                Some("mission"),
                b"working\r\n",
                &events,
            );
        }
        crate::session::clock::at(start + Duration::from_secs(11), 0, || {
            fixture
                .manager
                .expire_submit_busy("session", &stop, &events)
        });
        assert_eq!(
            fixture.manager.activity_snapshot()["session"],
            if output_follows {
                SessionActivityState::Busy
            } else {
                SessionActivityState::Idle
            },
            "output_follows={output_follows}"
        );
    }
}

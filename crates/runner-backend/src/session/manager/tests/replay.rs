use super::*;
use crate::runtimes::with_conversation_home;
use crate::session::{clock, hook_feed::HookWatcher, pty_runtime::IdleDetector};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;

mod render;

const ID: &str = "scenario-session";
const START: &str = "2026-01-01T00:00:00+00:00";
const EPOCH: i64 = 1_767_225_600_000;

#[derive(Deserialize)]
struct Header {
    runtime: String,
    name: String,
    #[serde(default)]
    rules: Vec<u8>,
    #[serde(default)]
    bugs: Vec<u16>,
    #[serde(default)]
    known_wrong: Vec<u16>,
    #[serde(default)]
    pending_turn: bool,
    composer_fixture: Option<String>,
}

#[derive(Deserialize)]
struct Step {
    ms: u64,
    #[serde(flatten)]
    event: Event,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Output {
        bytes: String,
        #[serde(default)]
        quiet: bool,
    },
    Tick,
    Input {
        bytes: String,
    },
    Composer {
        state: String,
        #[serde(default = "visible")]
        visible: bool,
    },
    Hook {
        report: Value,
    },
    Record {
        record: Value,
    },
    Observation {
        observation: AgentObservation,
    },
    LegacyTransition {
        state: String,
        source: String,
    },
    BridgeFailed,
    Reserve,
    Finish,
    Wake,
    Completion {
        #[serde(default)]
        viewed: bool,
    },
    Viewed,
    Exit {
        code: Option<i32>,
        #[serde(default)]
        crashed: bool,
    },
    Respawn,
    Key {
        key: String,
        mode: String,
        #[serde(default)]
        stale: bool,
    },
    History {
        key: String,
        #[serde(default)]
        alias: bool,
    },
    Probe {
        key: String,
        #[serde(default)]
        alias: bool,
    },
    CaptureHome,
}

fn visible() -> bool {
    true
}

#[derive(Default)]
struct DeliveryEvents(Mutex<Vec<String>>);
impl router::SessionDeliveryListener for DeliveryEvents {
    fn session_delivery_event(&self, _: &str, event: router::SessionDeliveryEvent) {
        self.0.lock().unwrap().push(format!("{event:?}"));
    }
}

fn watcher(runtime: &str, feed: &Path, home: &Path, generation: &str) -> Box<dyn HookWatcher> {
    use crate::runtimes::{
        antigravity::agy_status::AgyStatusWatcher, claude_code::claude_status::ClaudeStatusWatcher,
        codex::codex_status::CodexStatusWatcher, copilot::copilot_status::CopilotStatusWatcher,
        pi::pi_status::PiStatusWatcher,
    };
    match runtime {
        "claude-code" => Box::new(ClaudeStatusWatcher::start(feed, generation.into()).unwrap()),
        "codex" => Box::new(CodexStatusWatcher::start(feed, generation.into()).unwrap()),
        "copilot" => Box::new(
            CopilotStatusWatcher::start(feed, generation.into(), home.join(".copilot")).unwrap(),
        ),
        "pi" => Box::new(PiStatusWatcher::start(feed, generation.into()).unwrap()),
        "antigravity" => Box::new(AgyStatusWatcher::start(feed, generation.into()).unwrap()),
        _ => panic!("unknown scenario runtime {runtime}"),
    }
}

fn append(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap(),
        "{value}"
    )
    .unwrap();
}

fn persist(
    manager: &SessionManager,
    conn: &rusqlite::Connection,
    key: &str,
    mode: &str,
    stale: bool,
    current_start: &str,
) -> bool {
    let started = if stale {
        "2025-01-01T00:00:00+00:00"
    } else {
        current_start
    };
    let origin = match mode {
        "capture" => KeyOrigin::Captured,
        "rekey" => KeyOrigin::Rekeyed,
        "assigned" => KeyOrigin::Assigned,
        _ => panic!("unknown key mode"),
    };
    manager
        .report_key(
            ID,
            PersistKey {
                key: Some(key.into()),
                generation: started.into(),
                origin,
            },
            |effect| SessionManager::write_key_effect(conn, ID, effect),
        )
        .unwrap()
}

fn probe_cwd(home: &Path, alias: bool) -> String {
    if alias {
        #[cfg(unix)]
        return home.join("project-alias").to_string_lossy().into_owned();
        #[cfg(windows)]
        return home.join("alias/../project").to_string_lossy().into_owned();
    }
    let canonical = home.join("project").canonicalize().unwrap();
    #[cfg(windows)]
    let canonical = crate::runtimes::ordinary_windows_path(&canonical);
    canonical.to_string_lossy().into_owned()
}

fn history_path(runtime: &str, home: &Path, key: &str, alias: bool) -> PathBuf {
    match runtime {
        "claude-code" => {
            let cwd = probe_cwd(home, alias);
            #[cfg(windows)]
            let project: String = cwd
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            #[cfg(not(windows))]
            let project: String = cwd
                .chars()
                .map(|c| if c == '/' || c == '.' { '-' } else { c })
                .collect();
            home.join(".claude/projects")
                .join(project)
                .join(format!("{key}.jsonl"))
        }
        "copilot" => home
            .join(".copilot/session-state")
            .join(key)
            .join("events.jsonl"),
        "pi" => home
            .join("pi-sessions")
            .join(format!("2026-01-01_{key}.jsonl")),
        "antigravity" => home
            .join(".gemini/antigravity-cli/conversations")
            .join(format!("{key}.db")),
        "codex" => home
            .join(".codex/sessions/2026/01/01")
            .join(format!("rollout-2026-01-01T00-00-00-{key}.jsonl")),
        _ => unreachable!(),
    }
}

fn install_replay_handle(
    manager: &SessionManager,
    conn: &rusqlite::Connection,
    events: &dyn SessionEvents,
    log: &Arc<EventLog>,
) {
    let handle = SessionHandle {
        #[cfg(windows)]
        pending_first_turn: None,
        id: ID.into(),
        mission_id: Some("scenario-mission".into()),
        role_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: ID.into(),
        },
        codex_capture: None,
        forwarder: None,
        stop: Arc::new(AtomicBool::new(false)),
    };
    let sink = ForwarderEmitCtx {
        crew_id: "scenario-crew".into(),
        mission_id: "scenario-mission".into(),
        handle: "scenario-slot".into(),
        event_log: log.clone(),
    };
    manager.install_handle_with_size_persistence(
        ID,
        handle,
        Some(sink),
        Some(DEFAULT_PTY_SIZE),
        |cols, rows| crate::repo::session::update_last_size(conn, ID, cols, rows).is_ok(),
        events,
    );
}

fn replay(path: &Path) -> Value {
    let script = fs::read_to_string(path).unwrap();
    let mut lines = script.lines();
    let header: Header = serde_json::from_str(lines.next().unwrap()).unwrap();
    let steps: Vec<Step> = lines
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(steps.windows(2).all(|pair| pair[0].ms <= pair[1].ms));
    if let Some(name) = header.composer_fixture.as_ref() {
        let expected = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../runner-terminal/fixtures")
                .join(format!("{name}.expected.txt")),
        )
        .unwrap();
        let observations = steps
            .iter()
            .filter_map(|step| match &step.event {
                Event::Composer { state, visible } => Some(format!(
                    "{} {} composing=false visible={}\n",
                    step.ms,
                    match state.as_str() {
                        "idle" => "Idle",
                        "drafting" => "Drafting",
                        "submitted" => "Submitted",
                        _ => unreachable!(),
                    },
                    visible
                )),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(
            observations, expected,
            "recorded composer input drift: {name}"
        );
    }
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("project")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(home.join("project"), home.join("project-alias")).unwrap();
    #[cfg(windows)]
    fs::create_dir_all(home.join("alias")).unwrap();
    let feed = root.path().join("session-status/feed.ndjson");
    let transcript = home.join("transcript.jsonl");
    fs::write(&transcript, "").unwrap();
    for reporter in [
        crate::runtimes::copilot::copilot_status::reporter_path(root.path()),
        crate::runtimes::pi::pi_status::extension_path(root.path()),
        crate::runtimes::antigravity::agy_status::reporter_path(root.path()),
    ] {
        fs::create_dir_all(reporter.parent().unwrap()).unwrap();
        fs::write(reporter, "").unwrap();
    }
    let mut spawn_generation = 1;
    let mut generation = format!("spawn-{spawn_generation}");
    let mut current_start = START.to_owned();
    let mut watcher = Some(watcher(&header.runtime, &feed, &home, &generation));
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake);
    let events = capture();
    let log = Arc::new(EventLog::open(&root.path().join("mission")).unwrap());
    let listener = Arc::new(DeliveryEvents::default());
    let listener_dyn: Arc<dyn router::SessionDeliveryListener> = listener.clone();
    manager.register_delivery_listener(ID, Arc::downgrade(&listener_dyn));
    let conn = db::test_connection().unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running(ID.into());
    row.started_at = Some(
        DateTime::parse_from_rfc3339(START)
            .unwrap()
            .with_timezone(&Utc),
    );
    crate::repo::session::insert(&conn, &row).unwrap();
    let start = Instant::now();
    let mut detector = IdleDetector::with_adapter_at(
        Duration::from_secs(2),
        crate::runtimes::for_key(&header.runtime)
            .terminal_adapter((header.runtime == "codex").then_some(header.pending_turn)),
        start,
    );
    let mut cancel = 0;
    let mut timeline = Vec::new();
    let mut token = None;
    with_conversation_home(&home, || {
        clock::at(start, EPOCH, || {
            install_replay_handle(&manager, &conn, events.as_ref(), &log);
            manager.publish_mission_activity(
                ID,
                SessionActivityState::Busy,
                crate::session::state::StatusSource::Spawn,
                events.as_ref(),
            )
        });
        for step in steps {
            let now = start + Duration::from_millis(step.ms);
            clock::at(now, EPOCH + step.ms as i64, || {
                let mut result = Value::Null;
                let mut transition = None;
                match step.event {
                    Event::Output { bytes, quiet } => {
                        transition = detector.on_output_event_at(bytes.as_bytes(), quiet, now)
                    }
                    Event::Tick => transition = detector.tick_event_at(now),
                    Event::Input { bytes } => {
                        manager
                            .inject_direct_stdin(ID, bytes.as_bytes(), events.as_ref())
                            .unwrap();
                        detector.on_input_at(bytes.as_bytes(), now);
                        if let Some(interrupt) =
                            crate::session::pty_runtime::interrupt_key(bytes.as_bytes())
                        {
                            cancel |= interrupt;
                        }
                    }
                    Event::Composer { state, visible } => manager.report_input_state(
                        ID,
                        InputObservation {
                            state: match state.as_str() {
                                "idle" => InputState::Idle,
                                "drafting" => InputState::Drafting,
                                "submitted" => InputState::Submitted,
                                _ => panic!("unknown composer state"),
                            },
                            since: now,
                            composing: false,
                            composer_visible: visible,
                        },
                    ),
                    Event::Hook { mut report } => {
                        if report.get("generation").is_none() {
                            report["generation"] = json!(generation);
                        }
                        if report["transcript_path"] == "<TRANSCRIPT>" {
                            report["transcript_path"] = json!(transcript);
                        }
                        append(&feed, &report);
                    }
                    Event::Record { record } => append(&transcript, &record),
                    Event::Observation { observation } => {
                        manager.publish_observation(ID, observation, events.as_ref())
                    }
                    Event::LegacyTransition { state, source } => {
                        let state = match state.as_str() {
                            "busy" => SessionActivityState::Busy,
                            "idle" => SessionActivityState::Idle,
                            _ => panic!("unknown legacy activity"),
                        };
                        let source = match source.as_str() {
                            "hook" => StatusSource::Hook,
                            "input-interrupt" => StatusSource::InputInterrupt,
                            "input-escape" => StatusSource::InputEscape,
                            _ => panic!("unknown legacy source"),
                        };
                        manager.publish_mission_activity(ID, state, source, events.as_ref());
                    }
                    Event::BridgeFailed => {
                        watcher = None;
                        detector.hooks_unavailable();
                        manager.status_bridge_failed(ID, events.as_ref());
                    }
                    Event::Reserve => {
                        let reservation = manager.reserve_delivery(ID).unwrap();
                        result = match reservation {
                            router::DeliveryReservation::Ready(value) => {
                                token = Some(value);
                                json!({"gate":"ready"})
                            }
                            router::DeliveryReservation::RecentlyTyping(delay) => {
                                json!({"gate":"recently_typing", "remaining_ms":delay.as_millis()})
                            }
                            router::DeliveryReservation::Drafting { composer_visible } => {
                                json!({"gate":"drafting", "composer_visible":composer_visible})
                            }
                            router::DeliveryReservation::Unavailable => {
                                json!({"gate":"unavailable"})
                            }
                            router::DeliveryReservation::LocalInputPending => {
                                json!({"gate":"local_input_pending"})
                            }
                            router::DeliveryReservation::HumanInteraction => {
                                json!({"gate":"human_interaction"})
                            }
                            router::DeliveryReservation::InFlight => json!({"gate":"in_flight"}),
                        };
                    }
                    Event::Finish => {
                        if let Some(token) = token.take() {
                            manager.finish_delivery(ID, token);
                        }
                    }
                    Event::Wake => {
                        manager
                            .synthesize_wake_busy(
                                ID,
                                EventDraft::signal(
                                    "scenario-crew",
                                    "scenario-mission",
                                    "scenario-slot",
                                    SignalType::new("session_status"),
                                    json!({"state":"busy","source":"wake"}),
                                ),
                            )
                            .unwrap();
                    }
                    Event::Completion { viewed } => {
                        let armed = manager.take_completion_armed(&[ID.into()]);
                        if armed {
                            manager.record_unread(ID, viewed);
                        }
                        result = json!({"completion_consumed":armed,"viewed":viewed});
                    }
                    Event::Viewed => manager.mark_status_viewed(&[ID.into()]),
                    Event::Exit { code, crashed } => {
                        watcher = None;
                        manager.record_exit_status(ID, code, crashed);
                        manager
                            .forget_runtime_handle(ID, &manager.live_runtime_session(ID).unwrap())
                            .unwrap();
                        conn.execute("UPDATE sessions SET status = 'stopped' WHERE id = ?1", [ID])
                            .unwrap();
                    }
                    Event::Respawn => {
                        spawn_generation += 1;
                        generation = format!("spawn-{spawn_generation}");
                        current_start = DateTime::from_timestamp_millis(EPOCH + step.ms as i64)
                            .unwrap()
                            .to_rfc3339();
                        conn.execute(
                            "UPDATE sessions SET status = 'running', started_at = ?2 WHERE id = ?1",
                            params![ID, current_start],
                        )
                        .unwrap();
                        watcher = Some(self::watcher(&header.runtime, &feed, &home, &generation));
                        install_replay_handle(&manager, &conn, events.as_ref(), &log);
                        detector = IdleDetector::with_adapter_at(
                            Duration::from_secs(2),
                            crate::runtimes::for_key(&header.runtime)
                                .terminal_adapter((header.runtime == "codex").then_some(false)),
                            now,
                        );
                        manager.publish_mission_activity(
                            ID,
                            SessionActivityState::Busy,
                            crate::session::state::StatusSource::Spawn,
                            events.as_ref(),
                        );
                    }
                    Event::Key { key, mode, stale } => {
                        result = json!({"key_written":persist(&manager, &conn,&key,&mode,stale,&current_start)})
                    }
                    Event::History { key, alias } => {
                        let path = history_path(&header.runtime, &home, &key, alias);
                        fs::create_dir_all(path.parent().unwrap()).unwrap();
                        fs::write(path, "").unwrap();
                    }
                    Event::Probe { key, alias } => {
                        let cwd = probe_cwd(&home, alias);
                        let env = HashMap::from([(
                            "PI_CODING_AGENT_SESSION_DIR".into(),
                            home.join("pi-sessions").to_string_lossy().into_owned(),
                        )]);
                        let adapter = crate::runtimes::for_key(&header.runtime);
                        result = json!({"history_exists":adapter.conversation_exists(&key, &crate::runtimes::ProbeContext {cwd:Some(&cwd),role_env:&env}),
                            "missing_reuses_key":adapter.missing_conversation().reuse_key,
                            "missing_resume_on_launch":adapter.missing_conversation().resume_on_launch});
                    }
                    Event::CaptureHome => {
                        let configured = home.join("custom-codex");
                        fs::create_dir_all(configured.join("sessions")).unwrap();
                        let capture = crate::golden::with_config_env(
                            BTreeMap::from([("CODEX_HOME", configured.as_os_str().to_owned())]),
                            || crate::runtimes::for_key("codex").key_capture(),
                        );
                        let crate::runtimes::KeyCapture::RolloutScan { sessions_root } = capture
                        else {
                            panic!("expected rollout scan")
                        };
                        let started = DateTime::parse_from_rfc3339(START)
                            .unwrap()
                            .with_timezone(&Utc);
                        let date = started
                            .with_timezone(&chrono::Local)
                            .format("%Y/%m/%d")
                            .to_string();
                        let rollout = configured
                            .join("sessions")
                            .join(&date)
                            .join("rollout-sample-33333333-3333-4333-8333-333333333333.jsonl");
                        append(
                            &rollout,
                            &json!({"type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"/scenario/project","timestamp":START}}),
                        );
                        let captured = sessions_root.as_ref().and_then(|root| {
                            crate::session::codex_capture::replay_scan(
                                root,
                                "/scenario/project",
                                started,
                            )
                        });
                        if let Some(key) = captured.as_ref() {
                            persist(&manager, &conn, key, "capture", false, &current_start);
                        }
                        result = json!({"capture_uses_configured_home":sessions_root == Some(configured.join("sessions")),"captured_key":captured});
                    }
                }
                if let Some(state) = transition {
                    manager.publish_mission_terminal_event(ID, state, events.as_ref());
                }
                let mut starts = Vec::new();
                if let Some(active_watcher) = watcher.as_mut() {
                    match active_watcher.drain_events(
                        std::mem::take(&mut cancel),
                        &mut |event| {
                            if detector.accept_event(&event) {
                                manager.publish_agent_event(ID, event, events.as_ref())
                            } else {
                                Default::default()
                            }
                        },
                        &mut |key| starts.push(key),
                    ) {
                        Ok(()) => {}
                        Err(_) => {
                            detector.hooks_unavailable();
                            manager.status_bridge_failed(ID, events.as_ref());
                            watcher = None;
                        }
                    }
                }
                for key in starts {
                    persist(&manager, &conn, &key, "rekey", false, &current_start);
                }
                let status = manager.agent_status(ID);
                let (activity, completion_armed) = manager
                    .session_state(ID)
                    .map(|state| {
                        let state = state.lock().unwrap();
                        (state.model.activity(), state.model.completion_armed())
                    })
                    .unwrap_or((None, false));
                timeline.push(json!({"ms":step.ms,"result":result,"status":status,
                    "activity":activity,"completion_armed":completion_armed,
                    "quiescent":manager.input_quiescent(ID),
                    "published":events.status.lock().unwrap().drain(..).map(|event| json!(event)).collect::<Vec<_>>(),
                    "delivery_events":listener.0.lock().unwrap().drain(..).collect::<Vec<_>>(),
                    "key":crate::repo::session::get_row(&conn, ID).unwrap().unwrap().agent_session_key}));
            });
        }
    });
    let rows = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|row| row.event.payload)
        .collect::<Vec<_>>();
    json!({"runtime":header.runtime,"name":header.name,"rules":header.rules,"bugs":header.bugs,"known_wrong":header.known_wrong,"timeline":timeline,"session_status_rows":rows})
}

#[test]
fn session_scenario_goldens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session/fixtures");
    let mut paths = Vec::new();
    for runtime in ["claude-code", "codex", "copilot", "pi", "antigravity"] {
        let mut runtime_paths = fs::read_dir(root.join("scenarios").join(runtime))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "ndjson")
            })
            .collect::<Vec<_>>();
        runtime_paths.sort();
        assert!(!runtime_paths.is_empty(), "empty corpus for {runtime}");
        paths.extend(runtime_paths);
    }
    let mut rules = BTreeSet::new();
    let mut bugs = BTreeSet::new();
    let mut known_wrong = BTreeSet::new();
    let checkpoint = std::env::var("RUNNER_SESSION_GOLDEN_CHECKPOINT").ok();
    let mut compared = 0;
    for path in paths {
        let actual = replay(&path);
        rules.extend(
            actual["rules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_u64().unwrap()),
        );
        bugs.extend(
            actual["bugs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_u64().unwrap()),
        );
        known_wrong.extend(
            actual["known_wrong"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_u64().unwrap()),
        );
        let expected_path = root
            .join("expectations")
            .join(path.parent().unwrap().file_name().unwrap())
            .join(path.file_stem().unwrap())
            .with_extension("txt");
        let text = render::render(&actual);
        if std::env::var("RUNNER_UPDATE_SESSION_GOLDEN").as_deref() == Ok("1") {
            fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
            fs::write(&expected_path, &text).unwrap();
        }
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
        assert_eq!(text, expected, "session replay drift: {}", path.display());
        if let Some(checkpoint) = checkpoint.as_ref() {
            let original_path = format!(
                "crates/runner-backend/src/session/fixtures/expectations/{}/{}.json",
                path.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap(),
                path.file_stem().unwrap().to_str().unwrap()
            );
            let original = std::process::Command::new("git")
                .args(["-C", env!("CARGO_MANIFEST_DIR"), "show"])
                .arg(format!("{checkpoint}:{original_path}"))
                .output()
                .unwrap();
            assert!(
                original.status.success(),
                "checkpoint read failed: {original_path}"
            );
            let original: Value = serde_json::from_slice(&original.stdout).unwrap();
            assert_eq!(
                actual, original,
                "full checkpoint timeline drift: {original_path}"
            );
            assert_eq!(
                render::render(&original),
                expected,
                "checkpoint rendering drift: {original_path}"
            );
            compared += 1;
        }
    }
    assert_eq!(rules, (1..=10).collect());
    assert_eq!(known_wrong, BTreeSet::from([781]));
    assert_eq!(
        bugs,
        BTreeSet::from([459, 583, 623, 659, 670, 687, 738, 753, 766, 783, 781, 784, 785, 786, 736])
    );
    if checkpoint.is_some() {
        assert_eq!(compared, 124);
        eprintln!("Checkpoint equivalence: all {compared} full JSON timelines and compact golden bytes match");
    }
}

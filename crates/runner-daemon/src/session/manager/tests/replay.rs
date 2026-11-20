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
    IdentityUnavailable,
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

fn watcher(
    runtime: &str,
    routes: &Arc<crate::session::hook_queue::HookRoutes>,
    home: &Path,
    generation: &str,
) -> Option<Box<dyn HookWatcher>> {
    use crate::runtimes::{
        claude_code::claude_status::ClaudeStatusWatcher, codex::codex_status::CodexStatusWatcher,
        copilot::copilot_status::CopilotStatusWatcher, grok::status::Watcher as GrokStatusWatcher,
        opencode::status::Watcher as OpenCodeStatusWatcher, pi::pi_status::PiStatusWatcher,
    };
    if runtime == "antigravity" {
        return None;
    }
    let receiver = routes.register(
        Runtime::parse(runtime).unwrap(),
        ID.into(),
        generation.into(),
    );
    Some(match runtime {
        "claude-code" => Box::new(ClaudeStatusWatcher::from_receiver(receiver)),
        "codex" => Box::new(CodexStatusWatcher::from_receiver(receiver)),
        "copilot" => Box::new(CopilotStatusWatcher::from_receiver(
            receiver,
            home.join(".copilot"),
        )),
        "grok" => Box::new(GrokStatusWatcher::with_home(
            receiver,
            Some(home.join(".grok")),
            0,
        )),
        "opencode" => Box::new(OpenCodeStatusWatcher::from_receiver(
            receiver,
            Some(opencode_replay_db(home)),
        )),
        "pi" => Box::new(PiStatusWatcher::from_receiver(receiver)),
        _ => panic!("unknown scenario runtime {runtime}"),
    })
}

fn opencode_replay_db(home: &Path) -> PathBuf {
    home.join(".local/share/opencode/opencode.db")
}

fn ensure_opencode_replay_db(home: &Path) {
    let path = opencode_replay_db(home);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_v2(
            id TEXT PRIMARY KEY,
            time_created INTEGER NOT NULL,
            time_busy INTEGER,
            time_idle INTEGER,
            idle_outcome TEXT
        );
        CREATE TABLE IF NOT EXISTS session_message(
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            type TEXT NOT NULL,
            seq INTEGER NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL
        );",
    )
    .unwrap();
}

fn apply_opencode_record(home: &Path, record: &Value) {
    ensure_opencode_replay_db(home);
    let conn = rusqlite::Connection::open(opencode_replay_db(home)).unwrap();
    let id = record["id"].as_str().unwrap();
    let time_created = record["time_created"].as_i64().unwrap_or_else(|| {
        if record["preexisting"].as_bool() == Some(true) {
            -1_000
        } else {
            1
        }
    });
    let time_busy = record["time_busy"].as_i64();
    let time_idle = record["time_idle"].as_i64();
    let idle_outcome = record["idle_outcome"].as_str();
    conn.execute(
        "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
           time_busy = COALESCE(excluded.time_busy, session_v2.time_busy),
           time_idle = COALESCE(excluded.time_idle, session_v2.time_idle),
           idle_outcome = COALESCE(excluded.idle_outcome, session_v2.idle_outcome)",
        params![id, time_created, time_busy, time_idle, idle_outcome],
    )
    .unwrap();
    if let Some(message) = record.get("message") {
        let seq = message["seq"].as_i64().unwrap_or(1);
        let msg_id = message["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("msg_{id}_{seq}"));
        let msg_type = message["type"].as_str().unwrap_or("assistant");
        let msg_updated = message["time_updated"].as_i64().unwrap_or(time_created);
        let msg_created = message["time_created"].as_i64().unwrap_or(msg_updated);
        let data = message["data"].to_string();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET time_updated = excluded.time_updated, data = excluded.data",
            params![msg_id, id, msg_type, seq, msg_created, msg_updated, data],
        )
        .unwrap();
    }
}

struct ReplayTerminalHost;

impl runner_terminal::terminal::TerminalHost for ReplayTerminalHost {
    fn write_input(&self, _: &str, _: &[u8]) -> runner_terminal::terminal::HostResult<()> {
        Ok(())
    }
    fn write_reply(&self, _: &str, _: &[u8]) -> runner_terminal::terminal::HostResult<()> {
        Ok(())
    }
    fn report_input_state(&self, _: &str, _: InputObservation) {}
    fn set_live_title(&self, _: &str, _: &str) -> runner_terminal::terminal::HostResult<()> {
        Ok(())
    }
    fn resize(&self, _: &str, _: u16, _: u16) -> runner_terminal::terminal::HostResult<()> {
        Ok(())
    }
    fn is_local_host(&self, _: &str) -> bool {
        true
    }
    fn clipboard_store(&self, _: &str, _: runner_terminal::terminal::ClipboardTarget, _: &str) {}
}

fn attach_replay_program_terminal(manager: &SessionManager, runtime: &str) {
    let terminal = runner_terminal::terminal::TerminalModel::new(
        Arc::new(ReplayTerminalHost),
        ID.to_owned(),
        DEFAULT_PTY_SIZE.0,
        DEFAULT_PTY_SIZE.1,
        runner_terminal::terminal::ModelOptions {
            runtime: runner_core::Runtime::parse(runtime),
            ..Default::default()
        },
    )
    .unwrap();
    let semantics = crate::runtimes::for_key(runtime).status_semantics();
    let state = manager.session_state_or_insert(ID);
    let mut state = state.lock().unwrap();
    state
        .model
        .set_program_status_authority(semantics.program_status_authority);
    state
        .model
        .set_program_status_root_app(semantics.program_status_root_app);
    state
        .model
        .set_program_status_transcript_outcomes(semantics.program_status_transcript_outcomes);
    state
        .model
        .set_program_status_cancel_interrupt(semantics.program_status_cancel_interrupt);
    state
        .model
        .set_program_status_idle_completes(semantics.program_status_idle_completes);
    state
        .model
        .set_program_status_suppress_child_tasks(semantics.program_status_suppress_child_tasks);
    if let Some(previous) = state.terminal.replace(terminal) {
        for worker in previous.stop_workers() {
            let _ = worker.join();
        }
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
                key: (!key.is_empty()).then(|| key.into()),
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
            let project = crate::runtimes::claude_code::claude_code_project_dir(&cwd);
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
        startup: None,
        #[cfg(windows)]
        pending_first_turn: None,
        id: ID.into(),
        mission_id: Some("scenario-mission".into()),
        role_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: ID.into(),
        },
        rollout_capture: None,
        forwarder: None,
        input: Default::default(),
        stop: Arc::new(AtomicBool::new(false)),
    };
    let sink = ForwarderEmitCtx {
        crew_id: Some("scenario-crew".into()),
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
    let routes = Arc::new(crate::session::hook_queue::HookRoutes::default());
    let transcript = home.join("transcript.jsonl");
    fs::write(&transcript, "").unwrap();
    let mut spawn_generation = 1;
    let mut generation = format!("spawn-{spawn_generation}");
    let mut current_start = START.to_owned();
    let mut grok_used_explicit_transcript = false;
    let mut watcher = watcher(&header.runtime, &routes, &home, &generation);
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
    crate::test_support::insert_session_row(&conn, &row).unwrap();
    let start = Instant::now();
    let uses_program_status = crate::runtimes::for_key(&header.runtime)
        .status_semantics()
        .program_status_authority;
    let mut detector = IdleDetector::with_adapter_at(
        Duration::from_secs(2),
        crate::runtimes::for_key(&header.runtime)
            .terminal_adapter((header.runtime == "codex").then_some(header.pending_turn)),
        start,
    );
    if uses_program_status {
        detector.hooks_unavailable();
    }
    let mut cancel = 0;
    let mut timeline = Vec::new();
    let mut token = None;
    with_conversation_home(&home, || {
        clock::at(start, EPOCH, || {
            if uses_program_status {
                attach_replay_program_terminal(&manager, &header.runtime);
            }
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
                        if uses_program_status {
                            manager.ingest_output_chunk(
                                ID,
                                Some("scenario-mission"),
                                bytes.as_bytes(),
                                events.as_ref(),
                            );
                        }
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
                        if report["transcriptPath"] == "<TRANSCRIPT>" {
                            report["transcriptPath"] = json!(transcript);
                            grok_used_explicit_transcript = true;
                        }
                        if report["session_file"] == "<TRANSCRIPT>" {
                            report["session_file"] = json!(transcript);
                        }
                        let caller = report
                            .get("agent_id")
                            .and_then(Value::as_str)
                            .or_else(|| report.get("session_id").and_then(Value::as_str))
                            .map(str::to_owned);
                        let bridge_unavailable = report
                            .get("bridge_unavailable")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let report = runner_core::protocol::hook::HookReport {
                            bridge_unavailable,
                            version: runner_core::protocol::hook::VERSION,
                            runtime: Runtime::parse(&header.runtime).unwrap(),
                            session_id: ID.into(),
                            generation: report["generation"].as_str().unwrap().into(),
                            event: report["hook_event_name"].as_str().unwrap().into(),
                            payload: report,
                            caller_thread_id: caller,
                        };
                        drop(routes.admit(report));
                    }
                    Event::Record { record } => {
                        if header.runtime == "opencode" {
                            apply_opencode_record(&home, &record);
                        } else if header.runtime == "grok" && !grok_used_explicit_transcript {
                            let session_id = record
                                .pointer("/params/sessionId")
                                .and_then(Value::as_str)
                                .expect("grok record missing params.sessionId");
                            let path = home
                                .join(".grok/sessions/%2Fproject")
                                .join(session_id)
                                .join("updates.jsonl");
                            fs::create_dir_all(path.parent().unwrap()).unwrap();
                            append(&path, &record);
                        } else {
                            append(&transcript, &record);
                        }
                    }
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
                    Event::IdentityUnavailable => {
                        watcher = None;
                        detector.hooks_unavailable();
                        manager.publish_agent_event(
                            ID,
                            crate::session::state::agent::AgentEvent::ProgramIdentityUnavailable,
                            events.as_ref(),
                        );
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
                            router::DeliveryReservation::Unsent => json!({"gate":"unsent"}),
                            router::DeliveryReservation::Starting => json!({"gate":"starting"}),
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
                                    Some("scenario-crew"),
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
                        watcher = self::watcher(&header.runtime, &routes, &home, &generation);
                        if uses_program_status {
                            attach_replay_program_terminal(&manager, &header.runtime);
                        }
                        install_replay_handle(&manager, &conn, events.as_ref(), &log);
                        detector = IdleDetector::with_adapter_at(
                            Duration::from_secs(2),
                            crate::runtimes::for_key(&header.runtime)
                                .terminal_adapter((header.runtime == "codex").then_some(false)),
                            now,
                        );
                        if uses_program_status {
                            detector.hooks_unavailable();
                        }
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
                    Event::History { key, alias } => match header.runtime.as_str() {
                        "grok" => {
                            let cwd = probe_cwd(&home, alias);
                            let path = home
                                .join(".grok/sessions/workspace")
                                .join(&key)
                                .join("summary.json");
                            fs::create_dir_all(path.parent().unwrap()).unwrap();
                            fs::write(path, json!({"info":{"id":key,"cwd":cwd}}).to_string())
                                .unwrap();
                        }
                        "opencode" => {
                            apply_opencode_record(&home, &json!({"id":key,"preexisting":true}));
                        }
                        _ => {
                            let path = history_path(&header.runtime, &home, &key, alias);
                            fs::create_dir_all(path.parent().unwrap()).unwrap();
                            fs::write(path, "").unwrap();
                        }
                    },
                    Event::Probe { key, alias } => {
                        let cwd = probe_cwd(&home, alias);
                        let env = HashMap::from([
                            (
                                "PI_CODING_AGENT_SESSION_DIR".into(),
                                home.join("pi-sessions").to_string_lossy().into_owned(),
                            ),
                            (
                                "OPENCODE_DB".into(),
                                opencode_replay_db(&home).to_string_lossy().into_owned(),
                            ),
                        ]);
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
                            crate::runtimes::codex::capture::replay_scan(
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
                            if uses_program_status {
                                manager.publish_agent_event(
                                    ID,
                                    crate::session::state::agent::AgentEvent::ProgramIdentityUnavailable,
                                    events.as_ref(),
                                );
                            } else {
                                manager.status_bridge_failed(ID, events.as_ref());
                            }
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
    if let Some(state) = manager.session_state(ID) {
        if let Some(terminal) = state.lock().unwrap().terminal.take() {
            for worker in terminal.stop_workers() {
                let _ = worker.join();
            }
        }
    }
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
    for runtime in [
        "claude-code",
        "codex",
        "copilot",
        "pi",
        "antigravity",
        "grok",
        "opencode",
    ] {
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
    assert!(known_wrong.is_empty());
    assert_eq!(
        bugs,
        BTreeSet::from([
            459, 583, 623, 659, 670, 687, 738, 753, 766, 783, 781, 784, 785, 786, 736, 799,
        ])
    );
    if checkpoint.is_some() {
        assert_eq!(compared, 124);
        eprintln!("Checkpoint equivalence: all {compared} full JSON timelines and compact golden bytes match");
    }
}

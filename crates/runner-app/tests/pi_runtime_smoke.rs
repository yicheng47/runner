use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use runner_backend::model::{Mission, MissionStatus, Runtime};
use runner_backend::ops::{crew, role, slot};
use runner_backend::router::runtime::{MissionPermissionMode, PermissionMode};
use runner_backend::session::pty_runtime::PtyRuntime;
use runner_backend::session::SessionManager;
use runner_backend::AppCore;
use runner_terminal::replay::visible_lines;
use runner_terminal::terminal::{TerminalBridge, TerminalSession};

const DEFAULT_MODEL: &str = "deepseek/deepseek-v4-flash";

#[derive(Default)]
struct StopSessions(Vec<(Arc<SessionManager>, String)>);

impl Drop for StopSessions {
    fn drop(&mut self) {
        for (manager, id) in &self.0 {
            manager.kill(id).expect("stop smoke PTY");
        }
    }
}

fn core_at(app_data_dir: PathBuf, db: Arc<runner_backend::db::DbPool>) -> AppCore {
    let runtime_shell_env = Arc::new(RwLock::new(Default::default()));
    let runtime_discovery = Arc::new(RwLock::new(
        runner_backend::shell_path::DiscoveryState::startup(None, None),
    ));
    AppCore {
        sessions: SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(PtyRuntime::new()),
        ),
        db,
        app_data_dir,
        runtime_shell_env,
        runtime_discovery,
        buses: runner_backend::event_bus::BusRegistry::new(),
        routers: runner_backend::router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        mcp: Arc::new(runner_backend::mcp::McpHandle::new()),
        windows: Arc::new(runner_backend::windows::WindowRegistry::new()),
        events: runner_backend::events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "pi-smoke".into(),
    }
}

fn session_file(agent_dir: &Path, key: &str) -> Option<PathBuf> {
    let suffix = format!("_{key}.jsonl");
    std::fs::read_dir(agent_dir.join("sessions"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .flat_map(|entry| {
            std::fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
        })
        .find(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .map(|entry| entry.path())
}

fn transcript(agent_dir: &Path, key: &str) -> Vec<serde_json::Value> {
    session_file(agent_dir, key)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn message_text(event: &serde_json::Value) -> String {
    event["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|content| content["type"] == "text")
        .filter_map(|content| content["text"].as_str())
        .collect()
}

fn user_message_count(events: &[serde_json::Value]) -> usize {
    events
        .iter()
        .filter(|event| event["type"] == "message" && event["message"]["role"] == "user")
        .count()
}

fn grid_text(terminal: &TerminalSession) -> String {
    visible_lines(&*terminal.term.lock()).join("\n")
}

fn wait_for_ready(terminal: &TerminalSession) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !grid_text(terminal).contains("escape interrupt") {
        assert!(
            Instant::now() < deadline,
            "pi TUI did not paint before input"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_turn(terminal: &TerminalSession, agent_dir: &Path, key: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let events = transcript(agent_dir, key);
        let exact_reply = events.iter().any(|event| {
            event["type"] == "message"
                && event["message"]["role"] == "assistant"
                && event["message"]["stopReason"] == "stop"
                && message_text(event).trim() == expected
        });
        if exact_reply && grid_text(terminal).contains(expected) {
            println!(
                "Observed completed turn and terminal grid: {expected}; title={:?}\n{}",
                terminal.title(),
                grid_text(terminal)
            );
            assert_eq!(
                user_message_count(&events),
                1,
                "first turn must arrive once"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pi turn timed out; events={:?}; grid={}",
            events
                .iter()
                .map(|event| (&event["type"], &event["message"]["role"]))
                .collect::<Vec<_>>(),
            grid_text(terminal)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn copy_auth(agent_dir: &Path) {
    std::fs::create_dir_all(agent_dir).unwrap();
    if let Some(source) = std::env::var_os("RUNNER_PI_SMOKE_AUTH") {
        std::fs::copy(PathBuf::from(source), agent_dir.join("auth.json"))
            .expect("copy RUNNER_PI_SMOKE_AUTH into the isolated pi directory");
    }
}

#[test]
#[ignore = "spends provider credits; requires a logged-in real pi binary"]
fn pi_real_binary_direct_mission_and_relaunch_resume() {
    let command = std::env::var("RUNNER_PI_SMOKE_BINARY")
        .expect("set RUNNER_PI_SMOKE_BINARY to the real binary");
    let model =
        std::env::var("RUNNER_PI_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let agent_dir = root.join("pi-agent");
    let cwd = root.join("never-trusted-project");
    std::fs::create_dir_all(&cwd).unwrap();
    copy_auth(&agent_dir);
    let db = Arc::new(runner_backend::db::open_pool(&root.join("smoke.db")).unwrap());
    let core = core_at(root.join("app"), db.clone());
    let mut cleanup = StopSessions::default();
    let role = role::create(
        &db.get().unwrap(),
        role::CreateRoleInput {
            handle: "pi-smoke".into(),
            display_name: "pi smoke".into(),
            runtime: Runtime::Pi,
            command,
            args: vec![],
            working_dir: Some(cwd.to_string_lossy().into_owned()),
            system_prompt: Some("You are the Runner smoke agent.".into()),
            env: HashMap::from([(
                "PI_CODING_AGENT_DIR".into(),
                agent_dir.to_string_lossy().into_owned(),
            )]),
            model: Some(model.clone()),
            effort: Some("off".into()),
            permission_mode: PermissionMode::Default,
        },
    )
    .unwrap();
    let bridge = TerminalBridge::new(core.clone(), Arc::new(|| {})).unwrap();
    let direct = core.sessions.spawn_direct(&role, None, None, None, None, None, Some(100), Some(30), &core.app_data_dir, db.clone(), Arc::new(core.session_events()), Some("When the user asks for the smoke result, reply with exactly RUNNER_PI_DIRECT_OK. Do not call tools or change files.".into())).unwrap();
    cleanup.0.push((core.sessions.clone(), direct.id.clone()));
    let direct_row = runner_backend::repo::session::get_row(&db.get().unwrap(), &direct.id)
        .unwrap()
        .unwrap();
    let key = direct_row.agent_session_key.unwrap();
    println!("Direct key persisted: {key}");
    let terminal = bridge.session(&direct.id).unwrap();
    let view = terminal.view();
    wait_for_ready(&terminal);
    terminal
        .submit_text("Give the required smoke result.")
        .unwrap();
    wait_for_turn(&terminal, &agent_dir, &key, "RUNNER_PI_DIRECT_OK");
    core.sessions.kill(&direct.id).unwrap();
    cleanup.0.clear();
    drop(view);
    drop(terminal);

    let relaunched = core_at(core.app_data_dir.clone(), db.clone());
    let resumed_bridge = TerminalBridge::new(relaunched.clone(), Arc::new(|| {})).unwrap();
    relaunched
        .sessions
        .resume(
            &direct.id,
            Some(100),
            Some(30),
            &relaunched.app_data_dir,
            db.clone(),
            Arc::new(relaunched.session_events()),
        )
        .unwrap();
    cleanup
        .0
        .push((relaunched.sessions.clone(), direct.id.clone()));
    let resumed = resumed_bridge.session(&direct.id).unwrap();
    let resumed_view = resumed.view();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if grid_text(&resumed).contains("RUNNER_PI_DIRECT_OK") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "resume did not paint history: {}",
            grid_text(&resumed)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        runner_backend::repo::session::get_row(&db.get().unwrap(), &direct.id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .as_deref(),
        Some(key.as_str())
    );
    assert_eq!(user_message_count(&transcript(&agent_dir, &key)), 1);
    println!(
        "Observed resume by {key} in a new manager, old response painted, no first-turn replay"
    );
    relaunched.sessions.kill(&direct.id).unwrap();
    cleanup.0.clear();
    drop(resumed_view);
    drop(resumed);

    let crew = crew::create(
        &db.get().unwrap(),
        crew::CreateCrewInput {
            name: "pi smoke".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let slot = slot::create(
        &mut db.get().unwrap(),
        &crew.id,
        &role.id,
        "smoke",
        None,
        None,
    )
    .unwrap()
    .slot;
    let mission = Mission {
        id: "PISMOKEMISSION".into(),
        crew_id: crew.id,
        project_id: None,
        title: "pi smoke".into(),
        status: MissionStatus::Running,
        goal_override: None,
        cwd: Some(cwd.to_string_lossy().into_owned()),
        started_at: Utc::now(),
        stopped_at: None,
        pinned_at: None,
        archived_at: None,
    };
    runner_backend::repo::mission::insert(&db.get().unwrap(), &(&mission).into()).unwrap();
    relaunched
        .sessions
        .set_mission_permission_mode(MissionPermissionMode::Bypass);

    let events_path = relaunched
        .app_data_dir
        .join("crews")
        .join(&mission.crew_id)
        .join("missions")
        .join(&mission.id)
        .join("events.ndjson");
    let pending = relaunched
        .sessions
        .register_mission_session(
            &mission,
            &role,
            &slot,
            &relaunched.app_data_dir,
            events_path,
            db.clone(),
            Some("You are smoke in a Runner mission. Do not call tools or change files.".into()),
            Some("Reply with exactly RUNNER_PI_MISSION_OK.".into()),
            Some((100, 30)),
            "pi-smoke",
        )
        .unwrap();
    let spawned_id = pending.session_id.clone();
    relaunched
        .sessions
        .complete_mission_session_spawn(
            pending,
            Arc::new(relaunched.session_events()),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    cleanup
        .0
        .push((relaunched.sessions.clone(), spawned_id.clone()));
    let mission_key = runner_backend::repo::session::get_row(&db.get().unwrap(), &spawned_id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    let terminal = resumed_bridge.session(&spawned_id).unwrap();
    let mission_view = terminal.view();
    wait_for_turn(&terminal, &agent_dir, &mission_key, "RUNNER_PI_MISSION_OK");
    println!(
        "Observed mission slot under Bypass with temporary PI_CODING_AGENT_DIR; model={model}; thinking=off"
    );
    relaunched.sessions.kill(&spawned_id).unwrap();
    cleanup.0.clear();
    drop(mission_view);
    drop(terminal);

    relaunched
        .sessions
        .resume(
            &spawned_id,
            Some(100),
            Some(30),
            &relaunched.app_data_dir,
            db.clone(),
            Arc::new(relaunched.session_events()),
        )
        .unwrap();
    cleanup
        .0
        .push((relaunched.sessions.clone(), spawned_id.clone()));
    let resumed_mission = resumed_bridge.session(&spawned_id).unwrap();
    let _resumed_mission_view = resumed_mission.view();
    wait_for_ready(&resumed_mission);
    let history_deadline = Instant::now() + Duration::from_secs(30);
    while !grid_text(&resumed_mission).contains("RUNNER_PI_MISSION_OK") {
        assert!(
            Instant::now() < history_deadline,
            "mission resume did not paint history: {}",
            grid_text(&resumed_mission),
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let no_replay_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < no_replay_deadline {
        assert_eq!(
            user_message_count(&transcript(&agent_dir, &mission_key)),
            1,
            "a genuine mission resume must not replay the lead goal",
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("Observed genuine mission resume by {mission_key} with no lead-goal replay");
}

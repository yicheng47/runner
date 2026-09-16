use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        app_version: "copilot-smoke".into(),
    }
}

fn transcript(home: &Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(home.join("session-state").join(key).join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn grid_text(terminal: &TerminalSession) -> String {
    visible_lines(&*terminal.term.lock()).join("\n")
}

fn wait_for_turn(terminal: &TerminalSession, home: &Path, key: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let events = transcript(home, key);
        if events
            .iter()
            .any(|event| event["type"] == "assistant.turn_end")
            && grid_text(terminal).contains(expected)
        {
            println!(
                "Observed completed turn and terminal grid: {expected}; title={:?}\n{}",
                terminal.title(),
                grid_text(terminal)
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event["type"] == "user.message")
                    .count(),
                1,
                "first turn must arrive once"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Copilot turn timed out; events={:?}; grid={} ",
            events
                .iter()
                .map(|event| &event["type"])
                .collect::<Vec<_>>(),
            grid_text(terminal)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "spends Copilot credits; requires a logged-in real copilot binary"]
fn copilot_real_binary_direct_mission_and_relaunch_resume() {
    let command = std::env::var("RUNNER_COPILOT_SMOKE_BINARY")
        .expect("set RUNNER_COPILOT_SMOKE_BINARY to the real binary");
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let home = root.join("copilot-home");
    let cwd = root.join("never-trusted-project");
    std::fs::create_dir_all(&cwd).unwrap();
    let db = Arc::new(runner_backend::db::open_pool(&root.join("smoke.db")).unwrap());
    let core = core_at(root.join("app"), db.clone());
    let mut cleanup = StopSessions::default();
    let role = role::create(
        &db.get().unwrap(),
        role::CreateRoleInput {
            handle: "copilot-smoke".into(),
            display_name: "Copilot smoke".into(),
            runtime: Runtime::Copilot,
            command,
            args: vec![],
            working_dir: Some(cwd.to_string_lossy().into_owned()),
            system_prompt: Some("You are the Runner smoke agent.".into()),
            env: HashMap::from([("COPILOT_HOME".into(), home.to_string_lossy().into_owned())]),
            model: Some("gpt-5.4-mini".into()),
            effort: Some("low".into()),
            permission_mode: PermissionMode::Default,
        },
    )
    .unwrap();
    let bridge = TerminalBridge::new(core.clone(), Arc::new(|| {})).unwrap();
    let direct = core.sessions.spawn_direct(&role, None, None, None, None, None, Some(100), Some(30), &core.app_data_dir, db.clone(), Arc::new(core.session_events()), Some("You are the Runner smoke agent. Reply with exactly RUNNER_COPILOT_DIRECT_OK. Do not call tools or change files.".into())).unwrap();
    cleanup.0.push((core.sessions.clone(), direct.id.clone()));
    let key = runner_backend::repo::session::get_row(&db.get().unwrap(), &direct.id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    println!("Direct key persisted: {key}");
    let terminal = bridge.session(&direct.id).unwrap();
    let view = terminal.view();
    wait_for_turn(&terminal, &home, &key, "RUNNER_COPILOT_DIRECT_OK");
    core.sessions.kill(&direct.id).unwrap();
    cleanup.0.clear();
    drop(view);
    drop(terminal);

    // A new manager is the backend relaunch boundary; the app itself is not restarted.
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
        if grid_text(&resumed).contains("RUNNER_COPILOT_DIRECT_OK") {
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
        transcript(&home, &key)
            .iter()
            .filter(|event| event["type"] == "user.message")
            .count(),
        1
    );
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
            name: "Copilot smoke".into(),
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
        id: "COPILOTSMOKEMISSION".into(),
        crew_id: crew.id,
        project_id: None,
        title: "Copilot smoke".into(),
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
    let spawned = relaunched.sessions.spawn(&mission, &role, &slot, &relaunched.app_data_dir, events_path, db.clone(), Arc::new(relaunched.session_events()), Some("You are smoke in a Runner mission. Reply with exactly RUNNER_COPILOT_MISSION_OK. Do not call tools or change files.".into())).unwrap();
    cleanup
        .0
        .push((relaunched.sessions.clone(), spawned.id.clone()));
    let mission_key = runner_backend::repo::session::get_row(&db.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    let terminal = resumed_bridge.session(&spawned.id).unwrap();
    let _view = terminal.view();
    wait_for_turn(&terminal, &home, &mission_key, "RUNNER_COPILOT_MISSION_OK");
    let raw_config = std::fs::read_to_string(home.join("config.json")).unwrap();
    let json_start = raw_config.find('{').expect("Copilot config JSON body");
    let config: serde_json::Value = serde_json::from_str(&raw_config[json_start..]).unwrap();
    assert!(config["trustedFolders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|folder| folder == cwd.to_str().unwrap()));
    println!("Observed mission slot under Bypass; exact cwd trusted; both spawns used temporary COPILOT_HOME");
}

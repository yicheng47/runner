mod attention;
mod codex;
mod copilot;
mod fork;
mod forwarder;
mod hook_status;
mod input;
mod launch_gate;
mod mission_lifecycle;
mod permissions;
mod pi;
mod resume;
mod runtime_direct;
mod runtime_override;
mod slot_restart;
mod spawn_env;
mod status;
mod terminal_size;
mod wake;
#[cfg(windows)]
mod windows_batch;

use super::*;
use crate::model::Runtime;

// These tests don't touch the GPUI frontend — they hit the PTY layer directly. We
// build a minimal `Role` row, skip the DB (the SessionManager writes
// to DB on spawn), and cover: spawn-echo-readback, inject-stdin-roundtrip,
// and exit-emits-correct-status. For DB coverage we use the app's
// file-backed pool helper.

use crate::db;
use crate::model::{MissionStatus, Role};
use crate::router::runtime::MissionPermissionMode;
use crate::session::runtime::{
    OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
    SpawnSpec,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant};

/// Use this for paths consumed by spawn/resume, including stored cwd fields.
#[cfg(unix)]
fn fixture_tmp_dir() -> &'static Path {
    Path::new("/tmp")
}

#[cfg(windows)]
fn fixture_tmp_dir() -> &'static Path {
    static FIXTURE_TMP_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FIXTURE_TMP_DIR.get_or_init(|| std::env::temp_dir().components().collect())
}

// Deviation from main (CI accommodation): GitHub's shared macOS runners
// oversleep millisecond ticks several-fold, so tight elapsed budgets flunk
// there while proving the same boundedness property.
fn ci_scaled_budget(budget: Duration) -> Duration {
    if std::env::var_os("CI").is_some() {
        budget * 10
    } else {
        budget
    }
}

/// Test stand-in for `SessionRuntime`. Most legacy tests exercise
/// paths that should not touch the runtime field. This stub
/// errors on every method so any accidental runtime call surfaces.
struct InertRuntime;

impl SessionRuntime for InertRuntime {
    fn spawn(&self, _: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        Err(RuntimeError::Msg(
            "InertRuntime: spawn unsupported in unit tests".into(),
        ))
    }
    fn stop(&self, _: &RuntimeSession) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("InertRuntime: stop unsupported".into()))
    }
    fn send_bytes(&self, _: &RuntimeSession, _: &[u8]) -> RuntimeResult<()> {
        Err(RuntimeError::Msg(
            "InertRuntime: send_bytes unsupported".into(),
        ))
    }
    fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
        Err(RuntimeError::Msg(
            "InertRuntime: send_key unsupported".into(),
        ))
    }
    fn resize(&self, _: &RuntimeSession, _: u16, _: u16) -> RuntimeResult<()> {
        Err(RuntimeError::Msg("InertRuntime: resize unsupported".into()))
    }
    fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Err(RuntimeError::Msg("InertRuntime: status unsupported".into()))
    }
}

fn inert_runtime() -> Arc<dyn SessionRuntime> {
    Arc::new(InertRuntime)
}

fn manager_with_runtime(
    shell_env: crate::shell_path::LoginShellEnv,
    runtime: Arc<dyn SessionRuntime>,
) -> Arc<SessionManager> {
    SessionManager::new(
        Arc::new(std::sync::RwLock::new(shell_env)),
        Arc::new(std::sync::RwLock::new(
            crate::shell_path::DiscoveryState::startup(None, None),
        )),
        runtime,
    )
}

/// Test stand-in that captures every call so assertions can read
/// back what the manager handed to the runtime layer (env vars,
/// argv, byte writes, key names, resize dimensions). Lets
/// tests that depend on runtime-side behavior — DB writes after
/// spawn, output delivery, kill semantics, first-prompt
/// scheduling, agent_session_key resume preservation — run
/// without forking a real PTY.
#[derive(Default)]
struct FakeRuntime {
    spawns: std::sync::Mutex<Vec<FakeSpawn>>,
    inputs: std::sync::Mutex<Vec<FakeInput>>,
    stops: std::sync::Mutex<Vec<String>>,
    stop_failures: std::sync::Mutex<HashSet<String>>,
    resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
    /// Runs inside `spawn` once the fork is recorded — lets a test land
    /// work (a resize) between the fork and the handle install.
    spawn_hook: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    stop_gate: std::sync::Mutex<Option<RuntimeGate>>,
    write_gate: std::sync::Mutex<Option<RuntimeGate>>,
    stop_barrier: std::sync::Mutex<Option<Arc<Barrier>>>,
    /// What `status()` returns for any pane lookup. Most tests
    /// want exit_code=0 (clean stop); the kill-semantics test
    /// wants exit_code=143 (SIGTERM) to verify the
    /// stop-vs-crash discrimination still flips correctly.
    status_response: std::sync::Mutex<SessionStatus>,
}

/// One spawn/resume capture. `tx` is the live channel the
/// forwarder thread is reading; tests can `push_output` to
/// emit fake bytes or `close` to simulate exit.
struct FakeSpawn {
    spec: SpawnSpec,
    rt_session: RuntimeSession,
    tx: Option<std::sync::mpsc::Sender<RuntimeOutput>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FakeInput {
    Bytes { session_id: String, bytes: Vec<u8> },
    Key { session_id: String, key: String },
}

struct RuntimeGate {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl FakeRuntime {
    fn new() -> Self {
        Self {
            status_response: std::sync::Mutex::new(SessionStatus {
                alive: false,
                exit_code: Some(0),
                pid: Some(99999),
                command: Some("/bin/sh".into()),
            }),
            ..Default::default()
        }
    }

    /// Push a `Stream` event through the forwarder channel for
    /// the spawn at index `i`. Returns Err if the channel was
    /// already closed (test-side error).
    fn push_output(&self, i: usize, bytes: &[u8]) {
        let spawns = self.spawns.lock().unwrap();
        if let Some(tx) = spawns.get(i).and_then(|s| s.tx.as_ref()) {
            let _ = tx.send(RuntimeOutput::Stream(bytes.to_vec()));
        }
    }

    fn push_status(&self, i: usize, state: SessionActivityState) {
        self.push_status_from(i, state, "forwarder");
    }

    fn push_status_from(&self, i: usize, state: SessionActivityState, source: &'static str) {
        let spawns = self.spawns.lock().unwrap();
        if let Some(tx) = spawns.get(i).and_then(|s| s.tx.as_ref()) {
            let _ = tx.send(RuntimeOutput::StatusTransition { state, source });
        }
    }

    /// Drop the `Sender` for spawn `i` so the forwarder thread
    /// sees `Disconnected` and exits — the manager-side path
    /// that simulates a pane dying cleanly.
    fn close_spawn(&self, i: usize) {
        let mut spawns = self.spawns.lock().unwrap();
        if let Some(s) = spawns.get_mut(i) {
            s.tx = None;
        }
    }

    /// Update the canned `status()` reply. Use to make the
    /// next `kill`/exit reconciliation observe a non-zero exit
    /// code. (Reserved for future tests; currently every
    /// converted test runs against the default exit_code=0.)
    #[allow(dead_code)]
    fn set_status_exit_code(&self, code: Option<i32>) {
        let mut s = self.status_response.lock().unwrap();
        s.exit_code = code;
    }

    fn spawn_count(&self) -> usize {
        self.spawns.lock().unwrap().len()
    }

    fn fail_stop_for(&self, session_id: &str) {
        self.stop_failures
            .lock()
            .unwrap()
            .insert(session_id.to_string());
    }

    fn allow_stop_for(&self, session_id: &str) {
        self.stop_failures.lock().unwrap().remove(session_id);
    }

    fn arm_stop_gate(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.stop_gate.lock().unwrap() = Some(RuntimeGate {
            entered: entered_tx,
            release: release_rx,
        });
        (entered_rx, release_tx)
    }

    fn last_spawn_spec(&self) -> Option<SpawnSpec> {
        self.spawns.lock().unwrap().last().map(|s| s.spec.clone())
    }

    fn keys(&self) -> Vec<(String, String)> {
        self.inputs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|i| match i {
                FakeInput::Key { session_id, key } => Some((session_id.clone(), key.clone())),
                _ => None,
            })
            .collect()
    }

    fn bytes_writes(&self) -> Vec<(String, Vec<u8>)> {
        self.inputs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|i| match i {
                FakeInput::Bytes { session_id, bytes } => Some((session_id.clone(), bytes.clone())),
                _ => None,
            })
            .collect()
    }
}

impl SessionRuntime for FakeRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        let (tx, rx) = std::sync::mpsc::channel::<RuntimeOutput>();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rt_session = RuntimeSession {
            runtime: "fake".into(),
            session_id: spec.session_id.clone(),
        };
        self.spawns.lock().unwrap().push(FakeSpawn {
            spec: spec.clone(),
            rt_session: rt_session.clone(),
            tx: Some(tx),
        });
        if let Some(hook) = self.spawn_hook.lock().unwrap().as_ref() {
            hook();
        }
        Ok((rt_session, OutputStream::new(rx, stop)))
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        self.stops.lock().unwrap().push(session.session_id.clone());
        let barrier = self.stop_barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        let gate = self.stop_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.recv();
        }
        if self
            .stop_failures
            .lock()
            .unwrap()
            .contains(&session.session_id)
        {
            return Err(RuntimeError::Msg(format!(
                "injected stop failure for {}",
                session.session_id
            )));
        }
        // Drop the matching tx so the forwarder sees Disconnected.
        let target_session_id = session.session_id.clone();
        let mut spawns = self.spawns.lock().unwrap();
        for s in spawns.iter_mut() {
            if s.rt_session.session_id == target_session_id {
                s.tx = None;
            }
        }
        Ok(())
    }

    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        if let Some(gate) = self.write_gate.lock().unwrap().take() {
            let _ = gate.entered.send(());
            let _ = gate.release.recv();
        }
        self.inputs.lock().unwrap().push(FakeInput::Bytes {
            session_id: session.session_id.clone(),
            bytes: bytes.to_vec(),
        });
        Ok(())
    }

    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        self.inputs.lock().unwrap().push(FakeInput::Key {
            session_id: session.session_id.clone(),
            key: key.to_string(),
        });
        Ok(())
    }

    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()> {
        self.resizes
            .lock()
            .unwrap()
            .push((session.session_id.clone(), cols, rows));
        Ok(())
    }

    fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Ok(Some(self.status_response.lock().unwrap().clone()))
    }
}

fn fake_runtime() -> Arc<FakeRuntime> {
    Arc::new(FakeRuntime::new())
}

/// Build a manager backed by the supplied FakeRuntime. Returns
/// the Arc so tests can introspect the captured calls.
fn mgr_with_fake(shell: Option<String>, fake: Arc<FakeRuntime>) -> Arc<SessionManager> {
    manager_with_runtime(
        crate::shell_path::LoginShellEnv {
            path: shell,
            vars: Default::default(),
        },
        fake,
    )
}

/// Test emitter that just records every event. Replaces the app event channel
/// in unit tests — no frontend dependency.
#[derive(Default)]
struct Capture {
    output: Mutex<Vec<OutputEvent>>,
    exit: Mutex<Vec<ExitEvent>>,
    updated: Mutex<Vec<SessionUpdatedEvent>>,
    fork_started: Mutex<Vec<SessionForkStartedEvent>>,
    status: Mutex<Vec<SessionActivityEvent>>,
    activity: Mutex<Vec<RoleActivityEvent>>,
}

impl SessionEvents for Capture {
    fn output(&self, ev: &OutputEvent) {
        self.output.lock().unwrap().push(ev.clone());
    }
    fn exit(&self, ev: &ExitEvent) {
        self.exit.lock().unwrap().push(ev.clone());
    }
    fn updated(&self, ev: &SessionUpdatedEvent) {
        self.updated.lock().unwrap().push(ev.clone());
    }
    fn fork_started(&self, ev: &SessionForkStartedEvent) {
        self.fork_started.lock().unwrap().push(ev.clone());
    }
    fn status(&self, ev: &SessionActivityEvent) {
        self.status.lock().unwrap().push(ev.clone());
    }
    fn role_activity(&self, ev: &RoleActivityEvent) {
        self.activity.lock().unwrap().push(ev.clone());
    }
}

fn role(command: &str, args: &[&str]) -> Role {
    Role {
        id: ulid::Ulid::new().to_string(),
        handle: "tester".into(),
        display_name: "Tester".into(),
        runtime: "shell".into(),
        command: command.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        working_dir: None,
        system_prompt: None,
        env: HashMap::new(),
        model: None,
        effort: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn insert_role_row(conn: &rusqlite::Connection, role: &Role) {
    crate::repo::role::insert(conn, &crate::repo::role::RoleRow::from(role)).unwrap();
}

fn update_role_row(conn: &rusqlite::Connection, role: &Role) {
    crate::repo::role::update(conn, &crate::repo::role::RoleRow::from(role)).unwrap();
}

fn slot_for(role: &Role) -> crate::model::Slot {
    crate::model::Slot {
        id: ulid::Ulid::new().to_string(),
        crew_id: "c".into(),
        role_id: role.id.clone(),
        slot_handle: role.handle.clone(),
        position: 0,
        lead: true,
        runtime_override: None,
        model_override: None,
        effort_override: None,
        added_at: Utc::now(),
    }
}

fn mission() -> Mission {
    Mission {
        id: ulid::Ulid::new().to_string(),
        crew_id: "crew-ignored-in-tests".into(),
        project_id: None,
        title: "t".into(),
        status: MissionStatus::Running,
        goal_override: None,
        cwd: None,
        started_at: Utc::now(),
        stopped_at: None,
        pinned_at: None,
        archived_at: None,
    }
}

fn capture() -> Arc<Capture> {
    Arc::new(Capture::default())
}

fn wait_for_session_status_event(
    cap: &Capture,
    session_id: &str,
    state: SessionActivityState,
) -> SessionActivityEvent {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(ev) = cap
            .status
            .lock()
            .unwrap()
            .iter()
            .find(|ev| ev.session_id == session_id && ev.state == state)
            .cloned()
        {
            return ev;
        }
        if Instant::now() > deadline {
            panic!("session/status event never arrived for {session_id} state {state:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_output_event(cap: &Capture, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if cap
            .output
            .lock()
            .unwrap()
            .iter()
            .any(|ev| ev.session_id == session_id)
        {
            return;
        }
        if Instant::now() > deadline {
            panic!("session output never arrived for {session_id}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn join_forwarder_for_test(mgr: &SessionManager, session_id: &str) {
    let forwarder = mgr.session_state(session_id).and_then(|state| {
        let mut state = state.lock().unwrap();
        state
            .handle
            .as_mut()
            .and_then(|handle| handle.forwarder.take())
    });
    if let Some(forwarder) = forwarder {
        forwarder.join().unwrap();
    }
}

fn has_arg_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

fn pool_with_schema() -> Arc<DbPool> {
    let tmp = tempfile::tempdir().unwrap();
    // Leak the tempdir so the DB file outlives this fn; fine in tests.
    let path = tmp.path().join("c6.db");
    std::mem::forget(tmp);
    Arc::new(db::open_pool(&path).unwrap())
}

fn insert_crew_role(pool: &DbPool, mission_id: &str, role_id: &str) -> String {
    // Satisfy the FKs the `sessions` INSERT needs (crew, global role,
    // slot, mission) and return the slot id so the caller can build a
    // matching `Slot` to hand to `spawn`. Post-crew-slots, membership
    // lives on `slots` and roles no longer carry `role`.
    let conn = pool.get().unwrap();
    let now = Utc::now().to_rfc3339();
    let slot_id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c', 'c', ?1, ?1)",
        params![now],
    )
    .unwrap();
    crate::test_support::insert_test_role(&conn, role_id, "t", "shell", "/bin/sh");
    crate::test_support::insert_test_slot(&conn, &slot_id, "c", role_id, "t", 0, true);
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES (?1, 'c', 't', 'running', ?2)",
        params![mission_id, now],
    )
    .unwrap();
    slot_id
}

fn install_test_session_handle(manager: &SessionManager, session_id: &str) {
    manager
        .session_state_or_insert(session_id)
        .lock()
        .unwrap()
        .handle = Some(SessionHandle {
        #[cfg(windows)]
        pending_first_turn: None,
        id: session_id.into(),
        mission_id: Some("mission-observed-input".into()),
        role_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: session_id.into(),
        },
        codex_capture: None,
        forwarder: None,
        stop: Arc::new(AtomicBool::new(false)),
    });
}

/// Seed the crew/role/slot/mission rows for a mission spawn and keep
/// the role row in step with `role`, so a later resume (which
/// re-reads the row) rebuilds the same runtime and args.
fn seed_mission_rows(pool: &DbPool, role: &Role) -> (Mission, crate::model::Slot) {
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let slot_id = insert_crew_role(pool, &mission_base.id, &role.id);
    {
        let conn = pool.get().unwrap();
        update_role_row(&conn, role);
    }
    let mut slot = slot_for(role);
    slot.id = slot_id;
    (mission_base, slot)
}

fn assert_chat_has_no_permission_flags(args: &[String]) {
    for flag in [
        "--permission-mode",
        "--dangerously-skip-permissions",
        "--ask-for-approval",
        "--sandbox",
        "--allow-tool",
        "--yolo",
        "--allow-all",
        "--allow-all-tools",
        "--allow-all-paths",
        "--allow-all-urls",
    ] {
        assert!(
            !args
                .iter()
                .any(|arg| arg == flag || arg.starts_with(&format!("{flag}="))),
            "{args:?}"
        );
    }
}

/// Mission + role + slot rows for a single-slot crew, ready for
/// `register_mission_session`.
fn single_slot_mission(pool: &DbPool) -> (Mission, Role, crate::model::Slot) {
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
    let slot_id = insert_crew_role(pool, &mission_base.id, &role.id);
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission_base
    };
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    (mission, role, slot)
}

/// Poll the sessions row until the forwarder demotes it from
/// `running` — `resume()` refuses rows that still look live.
/// The forwarder flips the row to stopped before it emits activity and
/// forgets the runtime handle, so waiting on the row alone can observe a
/// session the manager still treats as live.
fn wait_for_session_exit(mgr: &SessionManager, pool: &DbPool, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        let handle_released = mgr
            .session_state(session_id)
            .is_none_or(|state| state.lock().unwrap().handle.is_none());
        if status != "running" && handle_released {
            return;
        }
        if Instant::now() > deadline {
            panic!("session {session_id} never finished exiting (status={status}, handle_released={handle_released})");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
#[test]
fn headless_fork_descendant() {
    let Some(pid_file) = std::env::var_os("RUNNER_FORK_TEST_PID") else {
        return;
    };
    std::fs::write(pid_file, std::process::id().to_string()).unwrap();
    // The inherited stderr pipe stays open without emitting libtest progress as Codex JSON.
    thread::sleep(Duration::from_secs(30));
}

#[cfg(windows)]
#[test]
fn windows_batch_prompt_probe() {
    if std::env::var_os("RUNNER_BATCH_PROMPT_EXE").is_none() {
        return;
    }
    use std::io::Write;
    let input = crate::session::process::RawConsoleInput::open().unwrap();
    let early = input.read_for(Duration::from_millis(200)).unwrap();
    assert!(early.is_empty(), "input before readiness: {early:?}");
    let ready = std::env::var("RUNNER_BATCH_PROMPT_MODE").unwrap() == "ready";
    if ready {
        print!("\x1b[?20");
        std::io::stdout().flush().unwrap();
        let early = input.read_for(Duration::from_millis(100)).unwrap();
        assert!(
            early.is_empty(),
            "input before complete readiness signal: {early:?}"
        );
        print!("04h");
        std::io::stdout().flush().unwrap();
    }
    let bytes = input.read_for(Duration::from_secs(5)).unwrap();
    let expected = if ready {
        b"\x1b[200~first line\nsecond line\x1b[201~\r".as_slice()
    } else {
        b"first line\nsecond line\r".as_slice()
    };
    assert_eq!(bytes, expected, "PTY first-turn bytes: {bytes:?}");
    println!("BATCH_INPUT_FIRST=first line");
    println!("BATCH_INPUT_SECOND=second line");
}

fn slot_respawn_fixture(runtime: &str, lead: bool) -> (Arc<DbPool>, tempfile::TempDir, String) {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let mut role = role("test-agent", &[]);
    role.runtime = runtime.into();
    let (mission, slot) = seed_mission_rows(&pool, &role);
    let conn = pool.get().unwrap();
    role.command = "test-agent".into();
    role.system_prompt = Some("SLOT_BRIEF".into());
    update_role_row(&conn, &role);
    conn.execute(
        "UPDATE crews SET system_prompt_addendum = 'TEAM_RULES' WHERE id = ?1",
        params![mission.crew_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE slots SET lead = ?1, slot_handle = 'slot' WHERE id = ?2",
        params![lead, slot.id],
    )
    .unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running("slot-session".into());
    row.mission_id = Some(mission.id.clone());
    row.slot_id = Some(slot.id);
    row.role_id = Some(role.id);
    row.status = crate::model::SessionStatus::Stopped;
    row.cwd = Some(app_data.path().to_string_lossy().into_owned());
    row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
    crate::repo::session::insert(&conn, &row).unwrap();
    let log = open_mission_event_log(app_data.path(), &mission.crew_id, &mission.id).unwrap();
    log.append(runner_core::model::EventDraft::signal(
        mission.crew_id,
        mission.id,
        "human",
        runner_core::model::SignalType::new("mission_goal"),
        serde_json::json!({"text": "LATEST_GOAL"}),
    ))
    .unwrap();
    drop(conn);
    (pool, app_data, row.id)
}

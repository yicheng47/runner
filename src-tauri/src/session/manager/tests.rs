use super::*;

// These tests don't touch Tauri — they hit the PTY layer directly. We
// build a minimal `Runner` row, skip the DB (the SessionManager writes
// to DB on spawn), and cover: spawn-echo-readback, inject-stdin-roundtrip,
// and exit-emits-correct-status. For DB coverage we use the app's
// file-backed pool helper.

use crate::db;
use crate::model::{MissionStatus, Runner};
use crate::session::runtime::{
    OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
    SpawnSpec,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// Test stand-in that captures every call so assertions can read
/// back what the manager handed to the runtime layer (env vars,
/// argv, byte writes, key names, resize dimensions). Lets
/// tests that depend on runtime-side behavior — DB writes after
/// spawn, output buffer machinery, kill semantics, first-prompt
/// scheduling, agent_session_key resume preservation — run
/// without forking a real PTY.
#[derive(Default)]
struct FakeRuntime {
    spawns: std::sync::Mutex<Vec<FakeSpawn>>,
    inputs: std::sync::Mutex<Vec<FakeInput>>,
    stops: std::sync::Mutex<Vec<String>>,
    resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
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

    fn push_status(&self, i: usize, state: RunnerStatus) {
        let spawns = self.spawns.lock().unwrap();
        if let Some(tx) = spawns.get(i).and_then(|s| s.tx.as_ref()) {
            let _ = tx.send(RuntimeOutput::StatusTransition {
                state,
                source: "forwarder",
            });
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
        Ok((rt_session, OutputStream::new(rx, stop)))
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        self.stops.lock().unwrap().push(session.session_id.clone());
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
    SessionManager::new(
        crate::shell_path::LoginShellEnv {
            path: shell,
            vars: Default::default(),
        },
        fake,
    )
}

/// Test emitter that just records every event. Replaces the Tauri
/// `AppHandle` in unit tests — no runtime dependency.
#[derive(Default)]
struct Capture {
    output: Mutex<Vec<OutputEvent>>,
    exit: Mutex<Vec<ExitEvent>>,
    status: Mutex<Vec<SessionActivityEvent>>,
    activity: Mutex<Vec<RunnerActivityEvent>>,
}
impl SessionEvents for Capture {
    fn output(&self, ev: &OutputEvent) {
        self.output.lock().unwrap().push(ev.clone());
    }
    fn exit(&self, ev: &ExitEvent) {
        self.exit.lock().unwrap().push(ev.clone());
    }
    fn status(&self, ev: &SessionActivityEvent) {
        self.status.lock().unwrap().push(ev.clone());
    }
    fn runner_activity(&self, ev: &RunnerActivityEvent) {
        self.activity.lock().unwrap().push(ev.clone());
    }
}

fn runner(command: &str, args: &[&str]) -> Runner {
    Runner {
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

fn slot_for(runner: &Runner) -> crate::model::Slot {
    crate::model::Slot {
        id: ulid::Ulid::new().to_string(),
        crew_id: "c".into(),
        runner_id: runner.id.clone(),
        slot_handle: runner.handle.clone(),
        position: 0,
        lead: true,
        added_at: Utc::now(),
    }
}

fn mission() -> Mission {
    Mission {
        id: ulid::Ulid::new().to_string(),
        crew_id: "crew-ignored-in-tests".into(),
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
    let deadline = Instant::now() + Duration::from_secs(2);
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

fn insert_crew_runner(pool: &DbPool, mission_id: &str, runner_id: &str) -> String {
    // Satisfy the FKs the `sessions` INSERT needs (crew, global runner,
    // slot, mission) and return the slot id so the caller can build a
    // matching `Slot` to hand to `spawn`. Post-crew-slots, membership
    // lives on `slots` and runners no longer carry `role`.
    let conn = pool.get().unwrap();
    let now = Utc::now().to_rfc3339();
    let slot_id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c', 'c', ?1, ?1)",
        params![now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO runners
                (id, handle, display_name, runtime, command,
                 args_json, working_dir, system_prompt, env_json,
                 created_at, updated_at)
             VALUES (?1, 't', 'T', 'shell', '/bin/sh',
                     NULL, NULL, NULL, NULL, ?2, ?2)",
        params![runner_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, 'c', ?2, 't', 0, 1, ?3)",
        params![slot_id, runner_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES (?1, 'c', 't', 'running', ?2)",
        params![mission_id, now],
    )
    .unwrap();
    slot_id
}

// `compose_path` moved to `session::launch::compose_path` as
// part of the Step 9 cutover; equivalent coverage lives in
// `session::launch::tests::compose_path_*`.

#[test]
fn concurrent_missions_on_same_crew_keep_session_state_isolated() {
    // Per #55 the per-crew "at most one live mission" guard was
    // lifted. The contract that makes that safe is mission-id
    // namespacing: `sessions.mission_id` is a foreign key,
    // `kill_all_for_mission` filters on `mission_id`, the runner
    // CLI shim path is keyed by mission_id, etc. This test pins
    // the session-isolation half of that contract: spawn one
    // session per mission against the same crew + same runner
    // template, assert both alive concurrently, then assert
    // `kill_all_for_mission(A)` reaps A's session and leaves B's
    // alone.
    let pool = pool_with_schema();
    let runner_id = ulid::Ulid::new().to_string();
    let crew_id = "c-concurrent".to_string();
    let slot_id = ulid::Ulid::new().to_string();
    let mission_a = ulid::Ulid::new().to_string();
    let mission_b = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES (?1, 'c', ?2, ?2)",
            params![crew_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'concurrent', 'C', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead, added_at)
                 VALUES (?1, ?2, ?3, 'concurrent', 0, 1, ?4)",
            params![slot_id, crew_id, runner_id, now],
        )
        .unwrap();
        for mid in [&mission_a, &mission_b] {
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                     VALUES (?1, ?2, 't', 'running', ?3)",
                params![mid, crew_id, now],
            )
            .unwrap();
        }
    }

    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id.clone();
    runner.handle = "concurrent".into();
    let mut slot = slot_for(&runner);
    slot.id = slot_id.clone();
    slot.crew_id = crew_id.clone();

    let mission_row_a = Mission {
        id: mission_a.clone(),
        crew_id: crew_id.clone(),
        ..mission()
    };
    let mission_row_b = Mission {
        id: mission_b.clone(),
        crew_id: crew_id.clone(),
        ..mission()
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned_a = mgr
        .spawn(
            &mission_row_a,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let spawned_b = mgr
        .spawn(
            &mission_row_b,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_ne!(
        spawned_a.id, spawned_b.id,
        "two missions on the same crew must produce distinct session ids",
    );

    // Both sessions live in the SessionManager's map at this point
    // — /bin/cat reads stdin until EOF, so neither has exited yet.
    {
        assert!(
            mgr.session_state(&spawned_a.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "session A must be live"
        );
        assert!(
            mgr.session_state(&spawned_b.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "session B must be live"
        );
    }

    // Reap mission A's sessions only. The filter on mission_id must
    // leave B untouched.
    mgr.kill_all_for_mission(&mission_a).unwrap();

    // After kill_all_for_mission, A's reader thread joins via
    // SessionManager::kill (which awaits the join), so A's row is
    // already terminal in the DB. B is still running.
    let status_a: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned_a.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(status_a, "running", "mission A's session must be reaped");

    {
        assert!(
            mgr.session_state(&spawned_a.id).is_none_or(|state| state
                .lock()
                .unwrap()
                .handle
                .is_none()),
            "mission A's live handle must be cleared",
        );
        assert!(
            mgr.session_state(&spawned_b.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "mission B's session must survive kill_all_for_mission(A)",
        );
    }
    let status_b: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned_b.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status_b, "running",
        "mission B's session row must still be running",
    );

    // Cleanup so the test's PTY child doesn't outlive the test.
    mgr.kill(&spawned_b.id).unwrap();
}

#[test]
fn spawn_marks_session_stopped_after_runtime_channel_closes() {
    // Spawn a mission session through FakeRuntime, then close
    // the runtime's output channel to simulate a clean pane exit.
    // The forwarder thread should query status (FakeRuntime
    // returns exit_code=0 by default), flip the DB row to
    // 'stopped', and emit ExitEvent with success=true.
    let pool = pool_with_schema();
    let mission = mission();
    let mut runner = runner("/bin/sh", &["-c", "echo hi"]);
    insert_crew_runner(&pool, &mission.id, &runner.id);
    runner.id = {
        let conn = pool.get().unwrap();
        let id: String = conn
            .query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
            .unwrap();
        id
    };
    let fresh_mission_id = {
        let conn = pool.get().unwrap();
        let id: String = conn
            .query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap();
        id
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let slot = slot_for(&runner);
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();
    // pid is no longer pre-known on spawn return — the runtime
    // surfaces it lazily via status() once the manager needs it.
    assert!(spawned.pid.is_none());
    assert_eq!(fake.spawn_count(), 1);

    // Simulate a clean pane exit.
    fake.close_spawn(0);

    // Poll the DB until the forwarder thread has marked the session stopped.
    let deadline = Instant::now() + Duration::from_secs(2);
    let final_status = loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned.id],
                |r| r.get(0),
            )
            .unwrap();
        if status != "running" {
            break status;
        }
        if Instant::now() > deadline {
            panic!("session never exited");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(final_status, "stopped");

    // Exit event should have fired with success=true.
    let exits = cap.exit.lock().unwrap();
    assert_eq!(exits.len(), 1, "expected 1 exit event, got {}", exits.len());
    assert!(exits[0].success);
}

#[test]
fn inject_stdin_roundtrip_routes_through_runtime() {
    // After the Step 9 cutover, inject_stdin no longer writes to
    // a master PTY — it routes through `runtime.send_bytes`
    // (literal byte stream) or `runtime.send_key("Enter")` (the
    // bare `\r` carve-out). FakeRuntime captures both; assert
    // the byte payload landed in `bytes_writes`, then bare `\r`
    // routed as a key press, then kill flips the row.
    let pool = pool_with_schema();
    let mission = mission();
    let mut runner = runner("/bin/cat", &[]);
    insert_crew_runner(&pool, &mission.id, &runner.id);
    runner.id = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let fresh_mission_id = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let slot = slot_for(&runner);
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    mgr.inject_stdin(&spawned.id, b"hello\n").unwrap();
    mgr.inject_stdin(&spawned.id, b"\r").unwrap();

    let writes = fake.bytes_writes();
    assert!(
        writes.iter().any(|(_, bytes)| bytes == b"hello\n"),
        "send_bytes should have captured hello\\n; got = {writes:?}",
    );
    let keys = fake.keys();
    assert!(
        keys.iter().any(|(_, k)| k == "Enter"),
        "bare \\r should route as send_key(Enter); got = {keys:?}",
    );

    mgr.kill(&spawned.id).unwrap();

    // After kill, forwarder thread exits and flips the row.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned.id],
                |r| r.get(0),
            )
            .unwrap();
        if status != "running" {
            break;
        }
        if Instant::now() > deadline {
            panic!("session never exited after kill");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn inject_stdin_on_unknown_session_errors_cleanly() {
    let mgr = SessionManager::new(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let err = mgr.inject_stdin("nope", b"x").unwrap_err();
    assert!(format!("{err}").contains("session not found"));
}

// `await_pty_output` was deleted in the Step 9 cutover. Tests
// that previously observed echoed bytes from /bin/cat through
// a portable-pty master now assert on FakeRuntime's captured
// pastes / keys / bytes_writes directly — faster and free of
// shell-timing flakes.

// Pre-#88 `codex_direct_chat_injects_persona_without_preamble`
// and `claude_code_direct_chat_injects_persona_without_preamble`
// asserted the off-bus invariant from #51 over the post-spawn
// paste path. Plan 0007 moved first-turn delivery to spawn-time
// positional argv; the same invariant is now exercised by
// `direct_chat_persona_lands_as_trailing_positional_argv_without_worker_preamble`
// below, and `compose_direct_first_turn` is unit-tested in
// `router::prompt`.

#[test]
fn direct_chat_persona_lands_as_trailing_positional_argv_without_worker_preamble() {
    // Plan 0007: when `spawn_direct` receives a non-empty
    // `first_turn`, the body must (a) land as the trailing
    // positional argv on the SpawnSpec, (b) suppress the
    // post-spawn paste fallback so the agent doesn't receive
    // the persona twice, and (c) preserve the off-bus
    // invariant from #51 — direct chats must NOT carry the
    // worker coordination preamble (the bundled `runner` CLI
    // isn't on PATH for direct chats; the preamble's verbs
    // would mislead the agent).
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'cc-argv', 'CC', 'claude-code', '/bin/sh',
                         ?3, NULL, ?4, NULL, ?2, ?2)",
            params![runner_id, now, r#"["-c","cat"]"#, "DIRECT_PERSONA"],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/sh", &["-c", "cat"]);
    runner.id = runner_id;
    runner.handle = "cc-argv".into();
    runner.runtime = "claude-code".into();
    runner.system_prompt = Some("DIRECT_PERSONA".into());

    // Compose via the same helper `session_start_direct` uses.
    let body = crate::router::prompt::compose_direct_first_turn(runner.system_prompt.as_deref())
        .expect("non-empty persona");
    assert!(
        !body.contains("in a crew coordinated by the bundled"),
        "compose_direct_first_turn must NOT include the worker preamble (off-bus invariant)",
    );

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            Some(body.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let trailing = spec.args.last().map(String::as_str).unwrap_or("");
    assert!(
        trailing.contains("DIRECT_PERSONA"),
        "first_turn body must land as the trailing positional argv; got args = {:?}",
        spec.args
    );
    assert!(
        !trailing.contains("in a crew coordinated by the bundled"),
        "direct chat must NOT ship the worker coordination preamble in argv: {trailing:?}",
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must suppress the post-spawn byte injection fallback; got writes = {:?}",
        fake.bytes_writes()
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_worker_preamble_lands_as_trailing_positional_argv_with_brief() {
    // Regression guard for #45 + #88 combined: a non-lead worker
    // must still receive the WORKER_COORDINATION_PREAMBLE plus
    // its brief as the first user turn, but now via the
    // spawn-time positional argv path rather than post-spawn
    // paste. Argv delivery must also suppress the paste
    // fallback so the worker doesn't get double-delivered.
    use crate::router::prompt::compose_worker_first_turn;

    let pool = pool_with_schema();
    let mission = mission();
    let mut runner = runner("/bin/sh", &["-c", "cat"]);
    runner.runtime = "claude-code".into();
    runner.handle = "worker-argv".into();
    runner.system_prompt = Some("WORKER_BRIEF".into());

    let slot_id = insert_crew_runner(&pool, &mission.id, &runner.id);
    {
        let conn = pool.get().unwrap();
        conn.execute("UPDATE slots SET lead = 0 WHERE id = ?1", params![slot_id])
            .unwrap();
        conn.execute(
            "UPDATE runners
                    SET runtime = ?2, handle = ?3, system_prompt = ?4
                  WHERE id = ?1",
            params![
                runner.id,
                runner.runtime,
                runner.handle,
                runner.system_prompt
            ],
        )
        .unwrap();
    }
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.lead = false;

    let body = compose_worker_first_turn(runner.system_prompt.as_deref(), None);
    // Composer ships the on-bus preamble + the brief.
    assert!(body.contains("in a crew coordinated by the bundled"));
    assert!(body.contains("WORKER_BRIEF"));

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            Some(body.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let trailing = spec.args.last().map(String::as_str).unwrap_or("");
    assert_eq!(
            trailing, body,
            "worker first-turn body must land as the trailing positional argv; got args.last() = {trailing:?}"
        );
    assert!(
        trailing.contains("in a crew coordinated by the bundled"),
        "worker argv must ship the coordination preamble (on-bus invariant)"
    );
    assert!(
        trailing.contains("WORKER_BRIEF"),
        "worker argv must ship the brief"
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must suppress the post-spawn byte injection fallback; got = {:?}",
        fake.bytes_writes()
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn codex_mission_spawn_grants_event_log_dir_to_sandbox() {
    // Codex's workspace-write sandbox cannot append to Runner's
    // app-data mission log unless we grant the mission directory.
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let mut runner = runner(
        "codex",
        &[
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write",
        ],
    );
    runner.runtime = "codex".into();
    runner.handle = "codex-worker".into();
    let slot_id = insert_crew_runner(&pool, &mission_base.id, &runner.id);
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission_base
    };
    let mut slot = slot_for(&runner);
    slot.id = slot_id;

    let app_data = tempfile::tempdir().unwrap();
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &mission.crew_id, &mission.id);
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first_turn = "mission first turn".to_string();
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            capture(),
            Some(first_turn.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let mission_dir_arg = mission_dir.to_string_lossy().to_string();
    let marker = crate::session::codex_capture::prompt_marker(&spawned.id);
    assert!(
        has_arg_pair(&spec.args, "--add-dir", &mission_dir_arg),
        "codex mission spawn must grant mission dir with --add-dir; args = {:?}",
        spec.args,
    );
    assert!(
        spec.args
            .iter()
            .any(|arg| arg.contains(&first_turn) && arg.contains(&marker)),
        "codex mission first turn and capture marker must ride argv; args = {:?}",
        spec.args,
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must not schedule byte injection; got {:?}",
        fake.bytes_writes(),
    );
    assert!(
        fake.keys().is_empty(),
        "argv delivery must not schedule submit key injection; got {:?}",
        fake.keys(),
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_registration_preserves_initial_terminal_size() {
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let runner = runner("/bin/cat", &[]);
    let slot_id = insert_crew_runner(&pool, &mission_base.id, &runner.id);
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission_base
    };
    let mut slot = slot_for(&runner);
    slot.id = slot_id;

    let mgr = mgr_with_fake(None, fake_runtime());
    let pending = mgr
        .register_mission_session(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            Some((132, 41)),
        )
        .unwrap();

    assert_eq!(pending.spec.initial_size, Some((132, 41)));
}

#[test]
fn mission_spawn_cwd_prefers_mission_over_runner_working_dir() {
    // Regression guard for #101: the per-mission cwd typed into the
    // Start-mission modal must beat the runner template's
    // `working_dir` default. Before the fix the runner override
    // silently won, so StartMissionModal's helper text ("Each
    // runner's PTY starts in this directory") was a lie.
    //
    // Exercises the resolver at the spawn site by inspecting the
    // SpawnSpec FakeRuntime captures. The contended both-set case
    // is the load-bearing one; the others lock in the fallback
    // chain so a future refactor can't quietly drop a branch.
    fn resolved_spawn_cwd(mission_cwd: Option<&str>, runner_cwd: Option<&str>) -> Option<PathBuf> {
        let pool = pool_with_schema();
        let mission_base = mission();
        let mut runner = runner("/bin/sh", &["-c", "cat"]);
        runner.working_dir = runner_cwd.map(|s| s.to_string());
        let slot_id = insert_crew_runner(&pool, &mission_base.id, &runner.id);
        let mission = Mission {
            cwd: mission_cwd.map(|s| s.to_string()),
            ..mission_base
        };
        let mut slot = slot_for(&runner);
        slot.id = slot_id;

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
                None,
            )
            .unwrap();
        let cwd = fake.last_spawn_spec().expect("spawn was called").cwd;
        mgr.kill(&spawned.id).unwrap();
        cwd
    }

    // The contended case: both set, mission wins. This is the bug.
    assert_eq!(
        resolved_spawn_cwd(Some("/mission-dir"), Some("/runner-dir")),
        Some(PathBuf::from("/mission-dir")),
        "mission.cwd must beat runner.working_dir when both are set",
    );
    // Mission only: mission flows through.
    assert_eq!(
        resolved_spawn_cwd(Some("/mission-only"), None),
        Some(PathBuf::from("/mission-only")),
    );
    // Runner only: runner is the fallback.
    assert_eq!(
        resolved_spawn_cwd(None, Some("/runner-only")),
        Some(PathBuf::from("/runner-only")),
    );
    // Neither set: inherit parent (None).
    assert_eq!(resolved_spawn_cwd(None, None), None);
}

// Pre-#88 `mission_spawn_injects_preamble_for_non_lead_worker`
// is superseded by
// `mission_spawn_worker_preamble_lands_as_trailing_positional_argv_with_brief`
// above; the on-bus invariant from #45 is now exercised over
// the argv delivery path, and persistence-layer validation
// (`MAX_SYSTEM_PROMPT_BYTES` / `MAX_MISSION_GOAL_BYTES`)
// prevents the body from exceeding the runtime's argv slot.

#[test]
fn codex_resume_skips_first_prompt_injection() {
    // On a codex resume the agent already has its system context
    // — replaying the brief would either be a no-op (codex
    // resume doesn't replay first turns) or, worse, push a fresh
    // user turn against the existing conversation. Verify the
    // resume path leaves stdin untouched: spawn /bin/cat with
    // codex runtime + a populated `agent_session_key` (so
    // `resume_plan` chooses the resuming branch), wait briefly,
    // and assert no echo arrived. Pairs with
    // `codex_fresh_spawn_injects_brief_via_stdin` — same setup,
    // opposite expectation, locking in the resume guard.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    let session_id = ulid::Ulid::new().to_string();
    let sibling_session_id = ulid::Ulid::new().to_string();
    let prior_key = uuid::Uuid::new_v4().to_string();
    let sibling_key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'codex-resumer', 'CR', 'codex', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at,
                     agent_session_key)
                 VALUES (?1, NULL, ?2, '/tmp', 'stopped', ?3, ?4)",
            params![session_id, runner_id, now, prior_key],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at,
                     agent_session_key)
                 VALUES (?1, NULL, ?2, '/tmp', 'stopped', ?3, ?4)",
            params![sibling_session_id, runner_id, now, sibling_key],
        )
        .unwrap();
    }
    // Update the in-memory runner row to mirror the DB so resume()
    // reads what we just inserted.
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "UPDATE runners SET system_prompt = ?2 WHERE id = ?1",
            params![runner_id, "CODEX_BRIEF_TOKEN_RESUME"],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let resumed = mgr
        .resume(
            &session_id,
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let spec = fake
        .last_spawn_spec()
        .expect("codex resume should spawn through FakeRuntime");
    assert_eq!(
        spec.args,
        vec!["resume".to_string(), prior_key.clone()],
        "codex resume must bind argv to the resumed row's own agent_session_key",
    );
    assert!(
        !spec.args.contains(&sibling_key),
        "codex resume must not use a sibling row's agent_session_key"
    );

    // FIRST_PROMPT_DELAY = ZERO under cfg(test); a would-be
    // injection would already be visible in fake.bytes_writes() by
    // the time resume() returns. The contract: codex resume
    // MUST NOT write anything containing the brief.
    let written: String = fake
        .bytes_writes()
        .iter()
        .map(|(_, p)| String::from_utf8_lossy(p).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !written.contains("CODEX_BRIEF_TOKEN_RESUME"),
        "codex resume must NOT write the brief; got = {written:?}"
    );

    mgr.kill(&resumed.id).unwrap();
}

#[test]
fn spawn_failure_after_spawn_command_reaps_the_child() {
    // Force the `sessions` INSERT to fail by dropping the table after the
    // pool is built. Without the post-spawn cleanup, the child would keep
    // running after `spawn` returns Err because nothing knows about it.
    let pool = pool_with_schema();
    let mission = mission();
    let mut runner = runner("/bin/cat", &[]);
    insert_crew_runner(&pool, &mission.id, &runner.id);
    runner.id = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };

    // Break the schema so the next INSERT fails.
    pool.get()
        .unwrap()
        .execute("DROP TABLE sessions", [])
        .unwrap();

    let mgr = SessionManager::new(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let slot = slot_for(&runner);
    let err = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap_err();
    // The error must surface the DB failure, not a spawn failure.
    assert!(
        format!("{err}").contains("sessions") || format!("{err}").contains("no such table"),
        "unexpected error: {err}"
    );
    // No live session left behind.
    assert!(mgr.sessions.lock().unwrap().values().all(|state| state
        .lock()
        .unwrap()
        .handle
        .is_none()));
}

#[test]
fn kill_blocks_until_session_row_is_terminal() {
    // mission_stop relies on this contract: kill must return only
    // after the forwarder thread has updated the DB row. With
    // FakeRuntime, `runtime.stop` drops the mpsc Sender so the
    // forwarder sees Disconnected and reconciles immediately;
    // `kill` joins on it before returning.
    let pool = pool_with_schema();
    let mission = mission();
    let mut runner = runner("/bin/cat", &[]);
    insert_crew_runner(&pool, &mission.id, &runner.id);
    runner.id = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let slot = slot_for(&runner);
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    mgr.kill(&spawned.id).unwrap();

    let conn = pool.get().unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        status != "running",
        "kill returned while session still running: {status}"
    );
    // The killed flag caused the forwarder to classify as `stopped`
    // even though FakeRuntime returns exit_code=0.
    assert_eq!(status, "stopped");
    // The runtime should have observed at least one stop call
    // — two is normal (kill calls stop directly; the
    // forwarder also calls stop on its way out as
    // belt-and-suspenders cleanup once the channel closes).
    assert!(!fake.stops.lock().unwrap().is_empty());
}

#[test]
fn spawn_direct_writes_session_with_null_mission_id_and_emits_activity() {
    // C8.5: a "Chat now" session lives outside any mission. Verify the
    // sessions row has mission_id IS NULL, the session lands in the
    // live state, and the runner_activity emission fires on spawn.
    let pool = pool_with_schema();
    // We don't go through `insert_crew_runner` here because direct
    // chat doesn't need a crew or mission — only a runner row.
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'directrunner', 'D', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/sh", &["-c", "echo direct"]);
    runner.id = runner_id.clone();
    runner.handle = "directrunner".into();

    let cap = capture();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            cap.clone(),
            None,
        )
        .unwrap();
    assert_eq!(spawned.mission_id, None);
    assert_eq!(spawned.runner_id, Some(runner_id.clone()));

    // Direct chat must NOT have a mission-side shim or
    // bundled-bin in its SpawnSpec — the off-bus invariant.
    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert!(!spec.mission, "spawn_direct must spawn with mission=false");
    assert!(spec.shim_dir.is_none(), "direct chat must not have a shim");
    assert!(
        spec.bundled_bin_dir.is_none(),
        "direct chat must not have the bundled bin on PATH",
    );

    // Simulate clean exit so the activity emission cycle
    // completes (spawn-time emit then reap-time emit).
    fake.close_spawn(0);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT status, mission_id FROM sessions WHERE id = ?1",
                params![&spawned.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            row.1, None,
            "direct session must persist with NULL mission_id"
        );
        if row.0 != "running" {
            break;
        }
        if Instant::now() > deadline {
            panic!("direct session never exited");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Last activity emission after reap should show zero
    // active sessions for this runner.
    let activity = cap.activity.lock().unwrap();
    assert!(!activity.is_empty(), "runner_activity must fire");
    let last = activity.last().unwrap();
    assert_eq!(last.runner_id, runner_id);
    assert_eq!(
        last.active_sessions, 0,
        "after reap, active_sessions for this runner must be 0"
    );
}

#[test]
fn direct_chat_status_transition_emits_session_status_busy() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'directbusy', 'Direct Busy', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    runner.handle = "directbusy".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();

    fake.push_status(0, RunnerStatus::Busy);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Busy);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_chat_status_transition_emits_session_status_idle() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'directidle', 'Direct Idle', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    runner.handle = "directidle".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();

    fake.push_status(0, RunnerStatus::Idle);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Idle);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_status_transition_appends_runner_status_without_session_status_event() {
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let runner = runner("/bin/cat", &[]);
    let slot_id = insert_crew_runner(&pool, &mission_base.id, &runner.id);
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission_base
    };
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.crew_id = mission.crew_id.clone();

    let app_data = tempfile::tempdir().unwrap();
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &mission.crew_id, &mission.id);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let spawned = mgr
        .spawn(
            &mission,
            &runner,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();

    fake.push_status(0, RunnerStatus::Busy);
    fake.close_spawn(0);
    join_forwarder_for_test(&mgr, &spawned.id);

    let log = EventLog::open(&mission_dir).unwrap();
    let event = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .find(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "runner_status")
        })
        .expect("runner_status event should be appended before the forwarder exits");

    assert_eq!(event.from, runner.handle);
    assert_eq!(event.payload["state"], "busy");
    assert_eq!(event.payload["source"], "forwarder");
    assert!(
        cap.status.lock().unwrap().is_empty(),
        "mission sessions must not emit live session/status events",
    );
}

#[test]
fn login_shell_proxy_env_reaches_spawn_with_runner_env_taking_precedence() {
    // Issue #152: GUI-launched Runner.app inherits launchd's
    // stripped env, so HTTPS_PROXY / NO_PROXY from the user's
    // shell rc files never reaches PTY children and claude /
    // codex login fails behind a corporate VPN / ClashX.
    //
    // The captured login-shell env on `SessionManager` should:
    //   - land in every spawn's env so children see the same
    //     proxy vars Terminal.app's children see;
    //   - lose to an explicit runner.env override on the same
    //     key, because the runner row is the more specific
    //     configuration surface.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'proxied', 'P', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/sh", &["-c", "true"]);
    runner.id = runner_id;
    runner.handle = "proxied".into();
    // The runner row overrides HTTPS_PROXY but leaves
    // NO_PROXY / lowercase variants untouched, so we expect
    // those to come straight from the login-shell snapshot.
    runner
        .env
        .insert("HTTPS_PROXY".into(), "http://runner-override:9999".into());

    let fake = fake_runtime();
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("HTTPS_PROXY".into(), "http://login-shell:7890".into());
    vars.insert("https_proxy".into(), "http://login-shell:7890".into());
    vars.insert("NO_PROXY".into(), "localhost,127.0.0.1,*.byted.org".into());
    let mgr = SessionManager::new(
        crate::shell_path::LoginShellEnv { path: None, vars },
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    mgr.spawn_direct(
        &runner,
        Some("/tmp"),
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
        None,
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_eq!(
        spec.env.get("HTTPS_PROXY").map(String::as_str),
        Some("http://runner-override:9999"),
        "runner.env must override the login-shell capture",
    );
    assert_eq!(
        spec.env.get("https_proxy").map(String::as_str),
        Some("http://login-shell:7890"),
        "lowercase variant must flow through unchanged",
    );
    assert_eq!(
        spec.env.get("NO_PROXY").map(String::as_str),
        Some("localhost,127.0.0.1,*.byted.org"),
        "NO_PROXY (with wildcard) must flow through unchanged",
    );
}

#[test]
fn output_snapshot_replays_live_session_and_clears_after_forget() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'buffered', 'Buffered', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    runner.handle = "buffered".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    // Push fake output through the runtime → forwarder
    // chain. The forwarder records it into the manager's
    // output buffer; output_snapshot reads it back.
    fake.push_output(0, b"hello snapshot");
    let deadline = Instant::now() + Duration::from_secs(2);
    let snapshot = loop {
        let snapshot = mgr.output_snapshot(&spawned.id);
        if !snapshot.is_empty() {
            break snapshot;
        }
        if Instant::now() > deadline {
            panic!("session output snapshot never captured live output");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    assert_eq!(snapshot[0].seq, 1);
    assert!(
        snapshot.iter().all(|ev| ev.session_id == spawned.id),
        "snapshot must only include chunks for the requested session"
    );

    mgr.kill(&spawned.id).unwrap();
    // After kill the buffer is intentionally preserved so a
    // remount can replay the dead session's scrollback. Explicit
    // cleanup is via `purge_session_buffers`.
    assert!(
        !mgr.output_snapshot(&spawned.id).is_empty(),
        "kill must keep the output buffer for snapshot replay"
    );
    mgr.purge_session_buffers(&spawned.id);
    assert!(
        mgr.output_snapshot(&spawned.id).is_empty(),
        "purge_session_buffers must drop the buffer"
    );
}

#[test]
fn resume_reuses_row_and_preserves_agent_session_key() {
    // Multi-chat-per-runner contract: a direct chat IS a
    // sessions row. spawn_direct creates the row and the
    // claude-code adapter persists a UUID under
    // `agent_session_key`. After exit, resume respawns the
    // *same* row (same id, same agent_session_key column
    // populated) and flips status back to running. See
    // docs/impls/archive/0003-direct-chats.md.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'resumer', 'R', 'claude-code', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/sh", &["-c", "echo first"]);
    runner.id = runner_id.clone();
    runner.handle = "resumer".into();
    runner.runtime = "claude-code".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let session_id = spawned.id.clone();

    // Force the spawn to "exit" so the forwarder marks the
    // row stopped; resume() refuses a row that's still
    // running.
    fake.close_spawn(0);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![&session_id],
                |r| r.get(0),
            )
            .unwrap();
        if status != "running" {
            break;
        }
        if Instant::now() > deadline {
            panic!("first spawn never exited");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // The claude-code adapter persisted a UUID — capture it.
    let key_before: Option<String> = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT agent_session_key FROM sessions WHERE id = ?1",
            params![&session_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        key_before.is_some(),
        "claude-code spawn must persist an agent_session_key for later resume",
    );

    // Resume: same id, same row.
    let resumed = mgr
        .resume(
            &session_id,
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert_eq!(resumed.id, session_id, "resume must reuse the row id");

    // After resume the status is running again with the
    // agent_session_key still populated. We don't pin the
    // UUID value — the resume_plan logic + missing-
    // conversation-file fallback can rotate it; the
    // manager-level invariant is "row id is preserved and
    // the key column stays populated."
    let key_after: Option<String> = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT agent_session_key FROM sessions WHERE id = ?1",
            params![&session_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        key_after.is_some(),
        "resume must keep agent_session_key populated; got NULL",
    );

    // Only one row survives: resume must not have INSERTed a
    // duplicate.
    let count: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE runner_id = ?1",
            params![runner_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "resume must update in place, not insert");

    mgr.kill(&session_id).unwrap();
}

#[test]
fn resume_refuses_running_and_archived_rows() {
    // Mission rows are no longer rejected — see
    // resume_mission_session_stamps_slot_handle_env. This test
    // covers the gates that remain.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'r', 'R', 'shell', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        // Already-running direct session.
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at)
                 VALUES ('running-sid', NULL, ?1, 'running', ?2)",
            params![runner_id, now],
        )
        .unwrap();
        // Archived direct session.
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at, archived_at)
                 VALUES ('archived-sid', NULL, ?1, 'stopped', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mgr = SessionManager::new(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    for (sid, needle) in [
        ("running-sid", "already running"),
        ("archived-sid", "archived"),
    ] {
        let err = mgr
            .resume(
                sid,
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains(needle),
            "resume({sid}) should reject with `{needle}`, got `{msg}`"
        );
    }
}

#[test]
fn resume_mission_session_stamps_slot_handle_env() {
    // Mission resume must look up the slot for the session and
    // use slot.slot_handle as RUNNER_HANDLE, not runner.handle.
    // After the Step 9 cutover the manager hands env to the
    // runtime via SpawnSpec.env; FakeRuntime captures the spec
    // and we assert RUNNER_HANDLE == slot_handle directly.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c-mr', 'c', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'template-handle', 'R', 'shell', '/bin/sh',
                         '[\"-c\", \"echo HANDLE=$RUNNER_HANDLE && exit\"]',
                         ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead, added_at)
                 VALUES (?1, 'c-mr', ?2, 'architect-slot', 0, 1, ?3)",
            params![slot_id, runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                    (id, crew_id, title, status, started_at)
                 VALUES (?1, 'c-mr', 't', 'running', ?2)",
            params![mission_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, status, started_at)
                 VALUES ('mr-sid', ?1, ?2, ?3, 'stopped', ?4)",
            params![mission_id, runner_id, slot_id, now],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .resume(
            "mr-sid",
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    // Returned identity is the slot's, not the template's.
    assert_eq!(spawned.handle, "architect-slot");
    assert_eq!(spawned.mission_id.as_deref(), Some(mission_id.as_str()));

    // The SpawnSpec the manager built for the runtime must
    // carry RUNNER_HANDLE = slot_handle (not the template
    // handle), plus the other mission-bus env vars.
    let spec = fake
        .last_spawn_spec()
        .expect("resume should have called spawn");
    assert_eq!(
        spec.env.get("RUNNER_HANDLE").map(String::as_str),
        Some("architect-slot"),
        "RUNNER_HANDLE must be the slot_handle, got env = {:?}",
        spec.env,
    );
    assert_eq!(
        spec.env.get("RUNNER_CREW_ID").map(String::as_str),
        Some("c-mr"),
    );
    assert_eq!(
        spec.env.get("RUNNER_MISSION_ID").map(String::as_str),
        Some(mission_id.as_str()),
    );
    assert!(
        spec.shim_dir.is_some(),
        "mission resume must install the per-slot shim",
    );
    assert!(
        spec.bundled_bin_dir.is_some(),
        "mission resume must put the bundled CLI on PATH",
    );

    mgr.kill("mr-sid").unwrap();
}

#[test]
fn codex_mission_resume_grants_event_log_dir_to_sandbox() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c-codex-resume', 'c', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'codex-template', 'Codex', 'codex', 'codex',
                         '[\"--ask-for-approval\",\"on-request\",\"--sandbox\",\"workspace-write\"]',
                         ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        conn.execute(
            "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead, added_at)
                 VALUES (?1, 'c-codex-resume', ?2, 'impl', 0, 1, ?3)",
            params![slot_id, runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                    (id, crew_id, title, status, started_at)
                 VALUES (?1, 'c-codex-resume', 't', 'running', ?2)",
            params![mission_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, status, started_at)
                 VALUES ('codex-resume-sid', ?1, ?2, ?3, 'stopped', ?4)",
            params![mission_id, runner_id, slot_id, now],
        )
        .unwrap();
    }

    let app_data = tempfile::tempdir().unwrap();
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), "c-codex-resume", &mission_id);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .resume(
            "codex-resume-sid",
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let spec = fake
        .last_spawn_spec()
        .expect("resume should have called spawn");
    let mission_dir_arg = mission_dir.to_string_lossy().to_string();
    assert!(
        has_arg_pair(&spec.args, "--add-dir", &mission_dir_arg),
        "codex mission resume must grant mission dir with --add-dir; args = {:?}",
        spec.args,
    );

    mgr.kill(&spawned.id).unwrap();
}

// The verify-and-retry first-prompt readback tests
// (`first_prompt_landed_first_try`, `*_after_retry`,
// `*_gives_up_after_max_attempts`,
// `continue_resume_rejects_stale_placeholder`) lived here
// before docs/impls/archive/0011 retired the readback verify path. The
// post-spawn "continue" auto-paste on resume that
// also lived here has been removed — Resume just respawns the
// PTY with no stdin injection, so the helper that synthesized
// a FakeRuntime SessionHandle for those tests is gone too.

#[test]
fn forwarder_status_emit_does_not_block_under_event_log_contention() {
    // Issue #124 / @reviewer P1: the forwarder consumer drains
    // terminal output, exit-event reap, AND `runner_status`
    // emission through the same thread. If `try_append_runner_status`
    // ever blocked on the event-log flock, a stuck mission log
    // would freeze terminal output too — the user would see a
    // hang the moment a second CLI writer took the lock.
    // Construct a real ForwarderEmitCtx against a tempdir,
    // steal the flock from another "process" (a parallel fd
    // holding LOCK_EX), and assert that
    // `try_append_runner_status` returns `Contended` within a
    // hard 100ms bound.
    use fs2::FileExt;
    use std::fs::OpenOptions;
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let blocker = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(event_log.path())
        .unwrap();
    blocker.lock_exclusive().unwrap();

    let ctx = ForwarderEmitCtx {
        crew_id: "test-crew".into(),
        mission_id: "test-mission".into(),
        handle: "tester".into(),
        event_log: Arc::clone(&event_log),
    };

    let start = Instant::now();
    let outcome = ctx.try_append_runner_status(RunnerStatus::Idle, "forwarder");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(100),
        "try_append_runner_status must not block; took {elapsed:?}",
    );
    assert!(
        matches!(outcome, AppendOutcome::Contended),
        "expected Contended outcome under lock contention",
    );

    // Streak-threshold table: the consumer logs at 1 / 10 / 100 /
    // 1000 / 10_000 / 20_000 / … Anything between those values
    // should be silent so a steady failure doesn't spam stderr.
    assert!(drop_streak_is_loggable(1));
    assert!(drop_streak_is_loggable(10));
    assert!(drop_streak_is_loggable(100));
    assert!(drop_streak_is_loggable(1000));
    assert!(drop_streak_is_loggable(10_000));
    assert!(drop_streak_is_loggable(20_000));
    assert!(!drop_streak_is_loggable(2));
    assert!(!drop_streak_is_loggable(50));
    assert!(!drop_streak_is_loggable(999));
    assert!(!drop_streak_is_loggable(10_001));
    assert!(!drop_streak_is_loggable(15_000));

    // Release the blocker and confirm the same call now succeeds.
    // Proves the test setup isn't accidentally getting Contended
    // for the wrong reason.
    blocker.unlock().unwrap();
    let outcome = ctx.try_append_runner_status(RunnerStatus::Busy, "forwarder");
    assert!(matches!(outcome, AppendOutcome::Ok));
}

#[test]
fn scan_alt_screen_detects_modern_and_legacy_escapes() {
    // No escape → no transition reported.
    assert_eq!(scan_alt_screen_transition(b"hello world"), None);

    // Modern combined pair (the one claude-code / codex emit).
    assert_eq!(
        scan_alt_screen_transition(b"prelude\x1b[?1049hbody"),
        Some(true)
    );
    assert_eq!(
        scan_alt_screen_transition(b"\x1b[?1049lcleanup"),
        Some(false)
    );

    // Legacy 47h/l pair still covered (older TUIs).
    assert_eq!(scan_alt_screen_transition(b"\x1b[?47h"), Some(true));
    assert_eq!(scan_alt_screen_transition(b"\x1b[?47l"), Some(false));

    // Latest match wins — enter followed by exit resolves to
    // main-screen, not alt.
    assert_eq!(
        scan_alt_screen_transition(b"\x1b[?1049henter middle \x1b[?1049lexit"),
        Some(false)
    );
    // …and exit followed by enter resolves to alt.
    assert_eq!(
        scan_alt_screen_transition(b"\x1b[?1049l\x1b[?1049h"),
        Some(true)
    );
}

#[test]
fn scan_bracketed_paste_detects_enable_and_disable_escapes() {
    assert_eq!(scan_bracketed_paste_transition(b"hello world"), None);
    assert_eq!(
        scan_bracketed_paste_transition(b"ready\x1b[?2004h"),
        Some(true)
    );
    assert_eq!(
        scan_bracketed_paste_transition(b"\x1b[?2004lprompt"),
        Some(false)
    );
    assert_eq!(
        scan_bracketed_paste_transition(b"\x1b[?2004hon\x1b[?2004loff"),
        Some(false)
    );
    assert_eq!(
        scan_bracketed_paste_transition(b"\x1b[?2004l\x1b[?2004h"),
        Some(true)
    );
}

#[test]
fn output_snapshot_prepends_alt_screen_enter_when_session_in_alt_screen() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = SessionManager::new(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    let runner = runner("/bin/sh", &["-c", "true"]);
    {
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'r', 'r', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner.id, now],
        )
        .unwrap();
    }
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    // Drive a chunk that enters alt-screen + some content, then
    // a chunk of pure content (no transition) so we exercise the
    // "scan no-ops on chunks without escapes" branch too.
    fake.push_output(0, b"\x1b[?1049hinitial banner");
    fake.push_output(0, b"more painted content");
    // Give the forwarder a beat to drain both chunks.
    std::thread::sleep(Duration::from_millis(50));

    let snapshot = mgr.output_snapshot(&spawned.id);
    assert!(!snapshot.is_empty(), "snapshot should contain chunks");
    // First event is the synthetic alt-screen enter, seq=0.
    assert_eq!(snapshot[0].seq, 0);
    assert_eq!(
        BASE64.decode(&snapshot[0].data).unwrap(),
        b"\x1b[?1049h",
        "synthetic prepend must be a single bare enter-alt-screen escape"
    );

    // After an exit-alt-screen chunk, the synthetic prepend
    // disappears and the snapshot starts at the first real event.
    fake.push_output(0, b"\x1b[?1049lback to main");
    std::thread::sleep(Duration::from_millis(50));
    let snapshot2 = mgr.output_snapshot(&spawned.id);
    assert_ne!(
        snapshot2.first().map(|e| e.seq),
        Some(0),
        "main-screen sessions must not prepend the alt-screen enter"
    );
}

#[test]
fn output_snapshot_prepends_bracketed_paste_enable_when_session_has_it_enabled() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = SessionManager::new(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    let runner = runner("/bin/sh", &["-c", "true"]);
    {
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'r', 'r', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner.id, now],
        )
        .unwrap();
    }
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    fake.push_output(0, b"\x1b[?2004hprompt ready");
    std::thread::sleep(Duration::from_millis(50));

    let snapshot = mgr.output_snapshot(&spawned.id);
    assert!(!snapshot.is_empty(), "snapshot should contain chunks");
    assert_eq!(snapshot[0].seq, 0);
    assert_eq!(
        BASE64.decode(&snapshot[0].data).unwrap(),
        b"\x1b[?2004h",
        "synthetic prepend must restore bracketed-paste mode"
    );

    fake.push_output(0, b"\x1b[?2004lplain prompt");
    std::thread::sleep(Duration::from_millis(50));
    let snapshot2 = mgr.output_snapshot(&spawned.id);
    assert_ne!(
        snapshot2.first().map(|e| e.seq),
        Some(0),
        "sessions with bracketed paste disabled must not prepend the enable escape"
    );
}

#[test]
fn output_snapshot_combines_alt_screen_and_bracketed_paste_prefixes() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = SessionManager::new(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    let runner = runner("/bin/sh", &["-c", "true"]);
    {
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'r', 'r', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner.id, now],
        )
        .unwrap();
    }
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    fake.push_output(0, b"\x1b[?1049h\x1b[?2004hpainted prompt");
    std::thread::sleep(Duration::from_millis(50));

    let snapshot = mgr.output_snapshot(&spawned.id);
    assert!(!snapshot.is_empty(), "snapshot should contain chunks");
    assert_eq!(snapshot[0].seq, 0);
    assert_eq!(
        BASE64.decode(&snapshot[0].data).unwrap(),
        b"\x1b[?1049h\x1b[?2004h",
        "synthetic prepend must restore every tracked terminal mode"
    );
}

// ---- claude-code launch gate (issue #171) -----------------------------

#[test]
fn compute_gate_wait_returns_zero_when_no_prior_spawn() {
    // First claude through the gate — `last` is None, so the
    // caller pays nothing. This is the property that makes
    // single direct chats / cold mission starts feel instant.
    let now = Instant::now();
    assert_eq!(
        compute_gate_wait(None, now, Duration::from_millis(1500)),
        Duration::ZERO
    );
}

#[test]
fn compute_gate_wait_returns_remaining_grace_when_prior_recent() {
    // Mid-grace case: a prior claude spawned 400ms ago and the
    // grace is 1500ms → caller waits the remaining 1100ms.
    let now = Instant::now();
    let last = now - Duration::from_millis(400);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::from_millis(1100)
    );
}

#[test]
fn compute_gate_wait_returns_zero_when_grace_already_elapsed() {
    // Prior spawn is older than the grace window → no wait. This
    // is what keeps single chats opened minutes apart from
    // paying any tax for a long-stale prior spawn.
    let now = Instant::now();
    let last = now - Duration::from_millis(5000);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::ZERO
    );
}

#[test]
fn compute_gate_wait_handles_clock_skew_without_panic() {
    // `last` being slightly in the future (Instant arithmetic
    // shouldn't underflow). saturating_duration_since clamps to
    // zero, so we treat a "future" last the same as "just now"
    // and return the full grace. Defensive only — Instant is
    // monotonic on every platform we target, so this shouldn't
    // happen in practice.
    let now = Instant::now();
    let last = now + Duration::from_millis(100);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::from_millis(1500)
    );
}

#[test]
fn enter_claude_launch_gate_records_timestamp_only_for_claude_code() {
    // Non-claude runtimes must not touch `last_spawn_at` —
    // otherwise a codex spawn would unnecessarily delay a
    // subsequent claude. Sanity-check that the runtime-string
    // discriminator is wired correctly.
    let mgr = mgr_with_fake(None, fake_runtime());
    assert!(mgr.claude_launch_gate.lock().unwrap().is_none());

    // Shell / codex / empty string: state stays None.
    mgr.enter_claude_launch_gate("s1", "shell");
    mgr.enter_claude_launch_gate("s2", "codex");
    mgr.enter_claude_launch_gate("s3", "");
    assert!(
        mgr.claude_launch_gate.lock().unwrap().is_none(),
        "non-claude runtimes must not advance the gate"
    );

    // claude-code stamps the field.
    mgr.enter_claude_launch_gate("s4", "claude-code");
    assert!(
        mgr.claude_launch_gate.lock().unwrap().is_some(),
        "claude-code spawn must advance the gate"
    );
}

#[test]
fn enter_claude_launch_gate_first_claude_does_not_sleep() {
    // First claude-code spawn through the gate (no prior) must
    // return nearly immediately — the deadline-based design's
    // whole point. Even at the production GRACE (1500ms), a
    // cold start should take << 100ms here.
    let mgr = mgr_with_fake(None, fake_runtime());
    let started = Instant::now();
    mgr.enter_claude_launch_gate("first", "claude-code");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "first claude must not wait — actual elapsed {}ms",
        elapsed.as_millis()
    );
}

// Regression for the resize buffer purge (impl 0020 dogfooding): the
// runtime lookup must resolve runner-backed sessions too, where
// `sessions.agent_runtime` is NULL and the runtime lives on the runner
// row — the common "Chat now" / mission path. A shell runner must NOT
// purge (no SIGWINCH repaint would rebuild its history).
#[test]
fn runtime_clears_on_resize_resolves_runner_backed_runtimes() {
    let pool = db::open_in_memory().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let conn = pool.get().unwrap();
    for (runner_id, runtime) in [("r-codex", "codex"), ("r-shell", "shell")] {
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, ?1, ?1, ?2, '/bin/cat',
                         NULL, NULL, NULL, NULL, ?3, ?3)",
            params![runner_id, runtime, now],
        )
        .unwrap();
    }
    // Runner-backed rows: agent_runtime stays NULL.
    conn.execute(
        "INSERT INTO sessions (id, mission_id, runner_id, cwd, status, started_at)
             VALUES ('s-codex-runner', NULL, 'r-codex', '/tmp', 'running', ?1)",
        params![now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, mission_id, runner_id, cwd, status, started_at)
             VALUES ('s-shell-runner', NULL, 'r-shell', '/tmp', 'running', ?1)",
        params![now],
    )
    .unwrap();
    // Runtime-only chat: no runner row, agent_runtime column set.
    conn.execute(
        "INSERT INTO sessions
                (id, mission_id, runner_id, cwd, status, started_at, agent_runtime)
             VALUES ('s-claude-runtime', NULL, NULL, '/tmp', 'running', ?1, 'claude-code')",
        params![now],
    )
    .unwrap();
    drop(conn);

    assert!(super::output::runtime_clears_on_resize(
        "s-codex-runner",
        &pool
    ));
    assert!(super::output::runtime_clears_on_resize(
        "s-claude-runtime",
        &pool
    ));
    assert!(!super::output::runtime_clears_on_resize(
        "s-shell-runner",
        &pool
    ));
    assert!(!super::output::runtime_clears_on_resize("s-missing", &pool));
}

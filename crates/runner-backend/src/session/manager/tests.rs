use super::*;

// These tests don't touch the GPUI frontend — they hit the PTY layer directly. We
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
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

#[derive(Debug, PartialEq, Eq)]
enum ForwardedEvent {
    Output(u64, Vec<u8>),
    Status(SessionActivityState, String),
}

#[derive(Default)]
struct ForwarderCapture(Mutex<Vec<ForwardedEvent>>);

impl SessionEvents for ForwarderCapture {
    fn output(&self, ev: &OutputEvent) {
        self.0
            .lock()
            .unwrap()
            .push(ForwardedEvent::Output(ev.seq, ev.bytes.clone()));
    }

    fn status(&self, ev: &SessionActivityEvent) {
        self.0
            .lock()
            .unwrap()
            .push(ForwardedEvent::Status(ev.state, ev.source.clone()));
    }

    fn exit(&self, _: &ExitEvent) {}
}

fn forward_queued_output(items: Vec<RuntimeOutput>) -> Vec<ForwardedEvent> {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: "burst-test".into(),
            ..Default::default()
        })
        .unwrap();
    for item in items {
        match item {
            RuntimeOutput::Stream(bytes) => fake.push_output(0, &bytes),
            RuntimeOutput::StatusTransition { state, .. } => fake.push_status(0, state),
        }
    }
    fake.close_spawn(0);
    let capture = Arc::new(ForwarderCapture::default());
    mgr.start_forwarder_thread(
        rt_session.session_id.clone(),
        None,
        rt_session,
        output,
        pool_with_schema(),
        capture.clone(),
        runner("fake", &[]),
        false,
        false,
        None,
    )
    .join()
    .unwrap();
    let events = std::mem::take(&mut *capture.0.lock().unwrap());
    events
}

#[test]
fn forwarder_coalesces_queued_stream_chunks_into_one_output_event() {
    let chunks = [
        vec![b'a'; 8 * 1024],
        vec![b'b'; 8 * 1024],
        vec![b'c'; 8 * 1024],
    ];
    let events = forward_queued_output(
        chunks
            .iter()
            .map(|bytes| RuntimeOutput::Stream(bytes.to_vec()))
            .collect(),
    );
    assert_eq!(events, vec![ForwardedEvent::Output(1, chunks.concat())]);
    eprintln!(
        "forwarder burst: {} queued 8 KiB chunks -> {} output event",
        chunks.len(),
        events.len()
    );
}

#[test]
fn forwarder_preserves_status_transition_between_stream_chunks() {
    let events = forward_queued_output(vec![
        RuntimeOutput::Stream(b"before".to_vec()),
        RuntimeOutput::StatusTransition {
            state: RunnerStatus::Idle,
            source: "forwarder",
        },
        RuntimeOutput::Stream(b"after".to_vec()),
    ]);
    assert_eq!(
        events,
        vec![
            ForwardedEvent::Output(1, b"before".to_vec()),
            ForwardedEvent::Status(SessionActivityState::Idle, "forwarder".into()),
            ForwardedEvent::Output(2, b"after".to_vec()),
        ]
    );
}

#[test]
fn forwarder_caps_bursts_without_losing_the_next_chunk() {
    for first_len in [1024 * 1024, 1024 * 1024 - 1] {
        let first = vec![b'a'; first_len];
        let events = forward_queued_output(vec![
            RuntimeOutput::Stream(first.clone()),
            RuntimeOutput::Stream(b"bc".to_vec()),
            RuntimeOutput::Stream(b"de".to_vec()),
        ]);
        assert_eq!(
            events,
            vec![
                ForwardedEvent::Output(1, first),
                ForwardedEvent::Output(2, b"bcde".to_vec()),
            ]
        );
    }
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
    activity: Mutex<Vec<RunnerActivityEvent>>,
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

fn assert_effective_command(command: &str, catalog_name: &str) {
    assert_eq!(
        std::path::Path::new(command)
            .file_name()
            .and_then(|name| name.to_str()),
        Some(catalog_name),
        "expected {catalog_name} or an absolute path ending in {catalog_name}, got {command}",
    );
}

fn slot_for(runner: &Runner) -> crate::model::Slot {
    crate::model::Slot {
        id: ulid::Ulid::new().to_string(),
        crew_id: "c".into(),
        runner_id: runner.id.clone(),
        slot_handle: runner.handle.clone(),
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

#[cfg(unix)]
fn fork_materializer(stdout: &str, exit_code: i32) -> (tempfile::TempDir, String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("fork-materializer");
    let capture_path = dir.path().join("capture.txt");
    std::fs::write(
        &command,
        format!(
            "#!/bin/sh\n{{\n  printf 'cwd=%s\\n' \"$PWD\"\n  printf 'env=%s\\n' \"$FORK_TEST_ENV\"\n  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n}} > \"{}\"\nprintf '%s\\n' '{}'\nexit {exit_code}\n",
            capture_path.display(),
            stdout,
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    (dir, command.to_string_lossy().into_owned(), capture_path)
}

#[cfg(unix)]
fn codex_fork_materializer(
    source_key: &str,
    fork_key: &str,
    create_rollout: bool,
) -> (tempfile::TempDir, String, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("codex-fork-materializer");
    let capture_path = dir.path().join("capture.txt");
    let codex_home = dir.path().join("codex-home");
    let now = chrono::Local::now();
    let rollout_dir = codex_home
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_path = rollout_dir.join(format!("rollout-test-{fork_key}.jsonl"));
    let event = format!(r#"{{"type":"thread.started","thread_id":"{fork_key}"}}"#);
    let session_meta = serde_json::json!({
        "type": "session_meta",
        "payload": {"id": fork_key, "forked_from_id": source_key},
    });
    let create_rollout = if create_rollout {
        format!(
            "printf '%s\\n' '{}' > \"{}\"\n",
            session_meta,
            rollout_path.display()
        )
    } else {
        String::new()
    };
    std::fs::write(
        &command,
        format!(
            "#!/bin/sh\n{{\n  printf 'cwd=%s\\n' \"$PWD\"\n  printf 'env=%s\\n' \"$FORK_TEST_ENV\"\n  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n}} > \"{}\"\nprintf '%s\\n' '{}'\n{create_rollout}sleep 10\n",
            capture_path.display(),
            event,
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    (
        dir,
        command.to_string_lossy().into_owned(),
        capture_path,
        codex_home,
    )
}

#[cfg(unix)]
struct RepairingCapture {
    pool: Arc<DbPool>,
    updated: Mutex<Vec<SessionUpdatedEvent>>,
}

#[cfg(unix)]
impl SessionEvents for RepairingCapture {
    fn output(&self, _ev: &OutputEvent) {}

    fn exit(&self, _ev: &ExitEvent) {}

    fn updated(&self, ev: &SessionUpdatedEvent) {
        self.updated.lock().unwrap().push(ev.clone());
        let mut conn = self.pool.get().unwrap();
        crate::repo::node::list_with_repair(&mut conn).unwrap();
    }
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

fn wait_for_output_event(cap: &Capture, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
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
fn mission_slot_exit_reaps_live_siblings_and_keeps_mission_running() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_id, &runner_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let sibling = mgr
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

    fake.close_spawn(0);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let live_sessions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions
                  WHERE mission_id = ?1 AND status = 'running'",
                params![mission_id],
                |row| row.get(0),
            )
            .unwrap();
        if live_sessions == 0 {
            break;
        }
        if Instant::now() > deadline {
            panic!("mission siblings were not reaped");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        fake.stops.lock().unwrap().contains(&sibling.id),
        "the surviving sibling must be stopped",
    );
    for session_id in [&first.id, &sibling.id] {
        assert!(
            mgr.session_state(session_id).is_none_or(|state| state
                .lock()
                .unwrap()
                .handle
                .is_none()),
            "session {session_id} must not retain a live handle",
        );
    }
    let mission_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM missions WHERE id = ?1",
            params![mission_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mission_status, "running");
}

#[test]
fn mission_slot_exit_cancels_pending_sibling_spawns() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_id, &runner_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.spawn(
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
    let cancel = mgr.register_pending_mission_cancel(&mission_id);

    fake.close_spawn(0);

    let deadline = Instant::now() + Duration::from_secs(2);
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        if Instant::now() > deadline {
            panic!("pending sibling spawns were not cancelled");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    mgr.drop_pending_mission_cancel(&mission_id, &cancel);
}

#[test]
fn intentional_mission_kill_does_not_reap_siblings_from_exit_epilogue() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_id, &runner_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let sibling = mgr
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

    // mission_stop kills sessions one at a time through this path. The
    // first intentional exit must not recursively start another sweep.
    mgr.kill(&first.id).unwrap();

    assert!(
        mgr.session_state(&sibling.id)
            .is_some_and(|state| state.lock().unwrap().handle.is_some()),
        "the sibling must stay live until mission_stop reaches it",
    );
    assert!(
        !fake.stops.lock().unwrap().contains(&sibling.id),
        "the first intentional exit must not stop its sibling",
    );
    let sibling_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![sibling.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sibling_status, "running");
    let first_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_status, "stopped");

    mgr.kill_all_for_mission(&mission_id).unwrap();
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
    let project = {
        let conn = pool.get().unwrap();
        crate::repo::project::create(&conn, "Runner", "/tmp/runner").unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        project_id: Some(project.id.clone()),
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
    let stored_project: Option<String> = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT project_id FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_project, Some(project.id));

    // Exit event should have fired with success=true.
    let exits = cap.exit.lock().unwrap();
    assert_eq!(exits.len(), 1, "expected 1 exit event, got {}", exits.len());
    assert!(exits[0].success);
    drop(exits);

    let mission_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM missions WHERE id = ?1",
            params![mission.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mission_status, "running");
    assert!(
        fake.stops
            .lock()
            .unwrap()
            .iter()
            .all(|session_id| session_id == &spawned.id),
        "a single-slot exit must not start a mission sweep",
    );
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
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let err = mgr.inject_stdin("nope", b"x").unwrap_err();
    assert!(format!("{err}").contains("session not found"));
}

#[test]
fn local_input_byte_classes_remain_the_unobserved_fallback() {
    use super::output::{classify_local_input, update_local_input_state, LocalInputClass};

    assert_eq!(
        classify_local_input(b"x"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input("界".as_bytes()),
        Some(LocalInputClass::SetPending)
    );
    for protocol in [
        b"\x1b[A".as_slice(),
        b"\x1b]10;rgb:dcdc/dcdc/e0e0\x1b\\",
        b"\x1b]11;rgb:1515/1616/1b1b\x1b\\",
    ] {
        assert_eq!(
            classify_local_input(protocol),
            Some(LocalInputClass::ActivityOnly),
            "terminal protocol traffic must not mark local input pending"
        );
    }
    assert_eq!(
        classify_local_input(b"\x1b[200~pasted text\x1b[201~"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input(b"\x16"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input(b"\r"),
        Some(LocalInputClass::ClearPending)
    );
    assert_eq!(
        classify_local_input(b"\x03"),
        Some(LocalInputClass::ClearPending)
    );

    let now = Instant::now();
    let mut state = SessionState::default();
    assert!(state.observed_input.is_none());
    update_local_input_state(&mut state, classify_local_input(b"draft"), now);
    assert!(state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));

    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.local_input_pending);
    assert!(state.last_local_input_at.is_none());

    update_local_input_state(&mut state, classify_local_input(b"\x1b[D"), now);
    assert!(!state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));

    state.local_input_pending = true;
    update_local_input_state(&mut state, classify_local_input(b"\x03"), now);
    assert!(!state.local_input_pending);
    assert!(state.last_local_input_at.is_none());
}

#[test]
fn enter_while_observed_idle_is_activity_only() {
    use super::output::{classify_local_input, update_local_input_state};

    let now = Instant::now();
    let mut state = SessionState {
        observed_input: Some(ObservedInput {
            state: InputState::Idle,
            since: now,
        }),
        ..SessionState::default()
    };
    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));
}

fn install_test_session_handle(manager: &SessionManager, session_id: &str) {
    manager
        .session_state_or_insert(session_id)
        .lock()
        .unwrap()
        .handle = Some(SessionHandle {
        id: session_id.into(),
        mission_id: Some("mission-observed-input".into()),
        runner_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: session_id.into(),
        },
        codex_capture: None,
        forwarder: None,
        stop: Arc::new(AtomicBool::new(false)),
    });
}

#[test]
fn observed_input_tier_precedes_the_byte_latch_and_hidden_drafts_park() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let session_id = "observed-input";
    install_test_session_handle(&manager, session_id);
    {
        let state = manager.session_state(session_id).unwrap();
        let mut state = state.lock().unwrap();
        state.local_input_pending = true;
        state.last_local_input_at = None;
    }
    assert_eq!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::PendingInput,
        "None must retain the byte latch exactly as the fallback tier"
    );

    let now = Instant::now();
    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Idle,
            since: now,
            composing: false,
            composer_visible: true,
        },
    );
    let token = match manager.reserve_delivery(session_id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("observed Idle must override a stale byte latch, got {other:?}"),
    };
    manager.finish_delivery(session_id, token);

    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Drafting,
            since: now,
            composing: false,
            composer_visible: false,
        },
    );
    assert_eq!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::PendingInput
    );
    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Submitted,
            since: Instant::now(),
            composing: false,
            composer_visible: true,
        },
    );
    assert!(matches!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::RecentlyTyping(_)
    ));

    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Submitted,
            since: Instant::now() - RECENT_LOCAL_INPUT_WINDOW,
            composing: false,
            composer_visible: true,
        },
    );
    manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .last_local_input_at = Some(Instant::now());
    assert!(matches!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::RecentlyTyping(_)
    ));
}

#[derive(Default)]
struct DeliveryEventCapture(Mutex<Vec<router::SessionDeliveryEvent>>);

impl router::SessionDeliveryListener for DeliveryEventCapture {
    fn session_delivery_event(&self, _session_id: &str, event: router::SessionDeliveryEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn observed_drafting_to_idle_emits_input_cleared() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let session_id = "observed-clear";
    install_test_session_handle(&manager, session_id);
    let capture = Arc::new(DeliveryEventCapture::default());
    let listener: Arc<dyn router::SessionDeliveryListener> = capture.clone();
    manager.register_delivery_listener(session_id, Arc::downgrade(&listener));
    let observation = |state| InputObservation {
        state,
        since: Instant::now(),
        composing: false,
        composer_visible: true,
    };
    manager.report_input_state(session_id, observation(InputState::Drafting));
    manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .last_local_input_at = Some(Instant::now());
    manager.report_input_state(session_id, observation(InputState::Idle));
    assert_eq!(
        capture.0.lock().unwrap().as_slice(),
        &[router::SessionDeliveryEvent::InputCleared]
    );
    assert!(manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .last_local_input_at
        .is_none());

    capture.0.lock().unwrap().clear();
    manager.report_input_state(session_id, observation(InputState::Drafting));
    manager.report_input_state(session_id, observation(InputState::Submitted));
    manager.report_input_state(session_id, observation(InputState::Idle));
    assert_eq!(
        capture.0.lock().unwrap().as_slice(),
        &[router::SessionDeliveryEvent::InputCleared],
        "a submitted draft must release the same parked outbox once the grid is empty"
    );
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

#[cfg(unix)]
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
            None,
            None,
            None,
            None,
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
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "argv first-turn delivery must arm the initial busy episode",
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "taking the argv completion arm must consume it",
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
fn trae_first_turn_gets_capture_prompt_marker() {
    let (first_turn, marker) = SessionManager::codex_capture_prompt_marker(
        "trae",
        "session-id",
        Some("first turn".to_string()),
    );
    let marker = marker.expect("trae must use the codex-lineage capture marker");
    assert_eq!(
        marker,
        crate::session::codex_capture::prompt_marker("session-id")
    );
    let expected = format!("first turn\n\n{marker}");
    assert_eq!(first_turn.as_deref(), Some(expected.as_str()));
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
            "caller-supplied",
        )
        .unwrap();

    assert_eq!(pending.spec.initial_size, Some((132, 41)));
}

#[test]
fn hinted_mission_start_forks_slots_at_the_hint() {
    // #367 across the whole seam: the size the resolver derives from the
    // frontend grid hint (mission_fork_size — exactly what mission_start
    // feeds register when the caller passes no size) must reach the PTY
    // fork itself. FakeRuntime records the SpawnSpec actually forked.
    let (size, source) = crate::ops::mission::mission_fork_size(None, Some((161, 45)));
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

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            size,
            source,
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((161, 45))
    );
    mgr.kill(&session_id).unwrap();
}

/// Mission + runner + slot rows for a single-slot crew, ready for
/// `register_mission_session`.
fn single_slot_mission(pool: &DbPool) -> (Mission, Runner, crate::model::Slot) {
    let mission_base = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let runner = runner("/bin/cat", &[]);
    let slot_id = insert_crew_runner(pool, &mission_base.id, &runner.id);
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
    (mission, runner, slot)
}

#[test]
fn mission_fork_uses_a_size_pushed_before_the_pty_existed() {
    // The two-phase spawn leaves the row visible for the whole gate wait
    // before any PTY exists. A terminal that measures itself in that
    // window pushes through `resize`, which can only persist the size;
    // the fork must honor it over the hint captured at registration, or
    // the PTY comes up wider than the grid the terminal already moved to
    // and every full-width row wraps by a cell.
    let pool = pool_with_schema();
    let (mission, runner, slot) = single_slot_mission(&pool);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            Some((113, 38)),
            "mission-hint",
        )
        .unwrap();
    let session_id = pending.session_id.clone();

    mgr.resize(&session_id, 112, 38, &pool).unwrap();
    assert!(
        fake.resizes.lock().unwrap().is_empty(),
        "no PTY yet: the push can only be persisted"
    );

    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((112, 38))
    );
    assert!(fake.resizes.lock().unwrap().is_empty());
    mgr.kill(&session_id).unwrap();
}

#[test]
fn mission_fork_applies_a_size_pushed_mid_fork() {
    // Narrower window, same drop: a push between the fork and the handle
    // install. The post-install re-read applies it to the new PTY.
    let pool = pool_with_schema();
    let (mission, runner, slot) = single_slot_mission(&pool);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            Some((113, 38)),
            "mission-hint",
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    {
        let mgr = Arc::clone(&mgr);
        let pool = Arc::clone(&pool);
        let session_id = session_id.clone();
        *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
            mgr.resize(&session_id, 112, 38, &pool).unwrap();
        }));
    }

    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((113, 38)),
        "the push came after the fork"
    );
    assert_eq!(
        fake.resizes.lock().unwrap().as_slice(),
        &[(session_id.clone(), 112, 38)]
    );
    *fake.spawn_hook.lock().unwrap() = None;
    mgr.kill(&session_id).unwrap();
}

#[test]
fn unhinted_mission_start_still_forks_at_default() {
    // Same seam with no caller size and no recorded hint: the fork still
    // happens, at DEFAULT_PTY_SIZE — the pre-#367 behavior.
    let (size, source) = crate::ops::mission::mission_fork_size(None, None);
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

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            size,
            source,
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some(DEFAULT_PTY_SIZE)
    );
    mgr.kill(&session_id).unwrap();
}

#[test]
fn mission_registration_defaults_to_80x24_when_unsized() {
    // The last rung of the #367 chain: no caller size and no recorded
    // grid hint must still fork — at DEFAULT_PTY_SIZE, as before.
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
            None,
            "DEFAULT_PTY_SIZE",
        )
        .unwrap();

    assert_eq!(pending.spec.initial_size, Some(DEFAULT_PTY_SIZE));
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

#[cfg(unix)]
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

    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
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
fn kill_all_for_mission_attempts_every_session_and_aggregates_failures() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_id, &runner_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let second = mgr
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
    fake.fail_stop_for(&first.id);

    let error = mgr.kill_all_for_mission(&mission_id).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(&first.id), "unexpected error: {message}");
    assert!(
        fake.stops.lock().unwrap().contains(&second.id),
        "sweep stopped before attempting the second session"
    );
    assert!(
        mgr.session_state(&second.id)
            .is_none_or(|state| state.lock().unwrap().handle.is_none()),
        "successful sessions must still be torn down"
    );
    assert!(
        mgr.session_state(&first.id)
            .is_some_and(|state| state.lock().unwrap().handle.is_some()),
        "failed session must remain retryable"
    );

    fake.allow_stop_for(&first.id);
    mgr.kill(&first.id).unwrap();
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
    let project = {
        let conn = pool.get().unwrap();
        crate::repo::project::create(&conn, "Runner", "/tmp").unwrap()
    };

    let cap = capture();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            Some(&project.id),
            Some(&project.cwd),
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
    let (stored_project_id, stored_cwd): (Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT project_id, cwd FROM sessions WHERE id = ?1",
            params![&spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(stored_cwd.as_deref(), Some(project.cwd.as_str()));

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
fn runner_activity_event_direct_session_id_ignores_slot_bound_orphans() {
    let pool = pool_with_schema();
    let now = Utc::now();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'directevent', 'Direct Event', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now.to_rfc3339()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, status, started_at)
                 VALUES ('slot-orphan-newer', NULL, ?1, 'slot-old', 'running', ?2)",
            params![
                runner_id,
                (now + chrono::Duration::seconds(10)).to_rfc3339()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, status, started_at)
                 VALUES ('direct-valid-older', NULL, ?1, NULL, 'running', ?2)",
            params![runner_id, now.to_rfc3339()],
        )
        .unwrap();
    }

    let mut r = runner("/bin/cat", &[]);
    r.id = runner_id;
    r.handle = "directevent".into();
    let cap = capture();
    emit_runner_activity(&pool, &r, cap.as_ref());

    let activity = cap.activity.lock().unwrap();
    let ev = activity.last().expect("runner/activity event emitted");
    assert_eq!(
        ev.direct_session_id.as_deref(),
        Some("direct-valid-older"),
        "slot-bound orphan must not be emitted as a direct chat"
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
    assert!(mgr.activity_snapshot().is_empty());
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();

    let seeded = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(seeded.source, "spawn");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy)
    );
    cap.status.lock().unwrap().clear();

    fake.push_status(0, RunnerStatus::Idle);
    wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);
    cap.status.lock().unwrap().clear();
    fake.push_status(0, RunnerStatus::Busy);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Busy);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
    assert!(!mgr.activity_snapshot().contains_key(&spawned.id));
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
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();
    assert!(
        mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .mission_status_sink
            .is_none(),
        "direct chats must not carry a mission status sink",
    );

    fake.push_status(0, RunnerStatus::Idle);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Idle);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_chat_typing_stays_idle_until_submit() {
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
                 VALUES (?1, 'directtyping', 'Direct Typing', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }

    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    runner.handle = "directtyping".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
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
    wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);
    cap.status.lock().unwrap().clear();

    let token = match mgr.reserve_delivery(&spawned.id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected delivery reservation, got {other:?}"),
    };
    let first_mgr = Arc::clone(&mgr);
    let first_cap = Arc::clone(&cap);
    let first_session_id = spawned.id.clone();
    let first = std::thread::spawn(move || {
        first_mgr
            .inject_direct_stdin(&first_session_id, b"h", first_cap.as_ref())
            .unwrap();
    });
    let wait_for_tickets = |expected| {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let gate = mgr
                .session_state(&spawned.id)
                .unwrap()
                .lock()
                .unwrap()
                .delivery_gate
                .clone();
            if gate.state.lock().unwrap().next_ticket == expected {
                break;
            }
            assert!(Instant::now() < deadline, "input ticket was not issued");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait_for_tickets(1);

    let second_mgr = Arc::clone(&mgr);
    let second_cap = Arc::clone(&cap);
    let second_session_id = spawned.id.clone();
    let second = std::thread::spawn(move || {
        second_mgr
            .inject_direct_stdin(&second_session_id, b"e", second_cap.as_ref())
            .unwrap();
    });
    wait_for_tickets(2);
    assert!(
        !first.is_finished() && !second.is_finished(),
        "local input must wait behind the reserved body/Enter chord"
    );
    assert!(mgr.inject_reserved(&spawned.id, token, b"[inbox]").unwrap());
    assert!(mgr.inject_reserved(&spawned.id, token, b"\r").unwrap());
    mgr.finish_delivery(&spawned.id, token);
    first.join().unwrap();
    second.join().unwrap();
    let writes = fake.bytes_writes();
    assert!(writes.ends_with(&[
        (spawned.id.clone(), b"[inbox]".to_vec()),
        (spawned.id.clone(), b"h".to_vec()),
        (spawned.id.clone(), b"e".to_vec()),
    ]));
    assert!(!mgr.input_quiescent(&spawned.id));
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "typing without submit must not arm completion",
    );
    fake.push_status(0, RunnerStatus::Busy);
    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"typing-echo-drained");
    wait_for_output_event(&cap, &spawned.id);

    assert!(cap.status.lock().unwrap().is_empty());
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle)
    );

    mgr.inject_direct_stdin(&spawned.id, b"\r", cap.as_ref())
        .unwrap();
    let submitted = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(submitted.source, "input-submit");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy)
    );
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "xterm Enter submit must arm completion",
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "taking the submit completion arm must consume it",
    );

    mgr.inject_paste(&spawned.id, b"pasted prompt").unwrap();
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "paste-then-Enter delivery must arm completion",
    );

    let stale_token = match mgr.reserve_delivery(&spawned.id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected delivery reservation, got {other:?}"),
    };
    mgr.kill(&spawned.id).unwrap();
    assert!(!mgr
        .inject_reserved(&spawned.id, stale_token, b"must not reach respawn")
        .unwrap());
    mgr.finish_delivery(&spawned.id, stale_token);
}

#[test]
fn direct_input_gate_timeout_is_bounded_and_does_not_pin_the_queue() {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let session_id = "direct-timeout";
    let state = mgr.session_state_or_insert(session_id);
    let gate = state.lock().unwrap().delivery_gate.clone();
    state.lock().unwrap().handle = Some(SessionHandle {
        id: session_id.into(),
        mission_id: None,
        runner_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: session_id.into(),
        },
        codex_capture: None,
        forwarder: None,
        stop: Arc::new(AtomicBool::new(false)),
    });
    gate.state.lock().unwrap().in_flight = true;

    let budget = Duration::from_millis(20);
    let started = Instant::now();
    let error = mgr
        .inject_direct_stdin_with_wait_timeout(session_id, b"x", cap.as_ref(), budget)
        .unwrap_err();
    assert!(matches!(
        error,
        Error::DirectInputTimeout {
            ref session_id,
            timeout_ms: 20,
        } if session_id == "direct-timeout"
    ));
    assert!(
        started.elapsed() <= ci_scaled_budget(Duration::from_secs(1)),
        "gate timeout exceeded its bounded test budget"
    );

    mgr.finish_delivery(session_id, 0);
    mgr.inject_direct_stdin_with_wait_timeout(session_id, b"after-timeout", cap.as_ref(), budget)
        .unwrap();
    assert_eq!(
        fake.bytes_writes(),
        vec![(session_id.to_string(), b"after-timeout".to_vec())]
    );
}

#[test]
fn mission_status_transition_appends_once_without_session_status_event() {
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
    fake.push_status(0, RunnerStatus::Busy);
    fake.close_spawn(0);
    join_forwarder_for_test(&mgr, &spawned.id);

    let log = EventLog::open(&mission_dir).unwrap();
    let events: Vec<_> = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "runner_status")
        })
        .collect();
    assert_eq!(
        events.len(),
        1,
        "unchanged mission states must not append duplicate rows",
    );
    let event = &events[0];

    assert_eq!(event.from, runner.handle);
    assert_eq!(event.payload["state"], "busy");
    assert_eq!(event.payload["source"], "forwarder");
    assert!(
        cap.status.lock().unwrap().is_empty(),
        "mission sessions must not emit live session/status events",
    );
}

#[test]
fn mission_typing_stays_idle_until_submit() {
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

    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"initial-idle-synced");
    wait_for_output_event(&cap, &spawned.id);
    cap.output.lock().unwrap().clear();

    let log = EventLog::open(&mission_dir).unwrap();
    let read_statuses = || {
        log.read_from(0)
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .filter(|event| {
                event
                    .signal_type
                    .as_ref()
                    .is_some_and(|ty| ty.as_str() == "runner_status")
            })
            .collect::<Vec<_>>()
    };
    let initial_statuses = read_statuses();
    assert_eq!(initial_statuses.len(), 1);
    assert_eq!(initial_statuses[0].payload["state"], "idle");
    assert_eq!(initial_statuses[0].payload["source"], "forwarder");

    mgr.inject_direct_stdin(&spawned.id, b"x", cap.as_ref())
        .unwrap();
    fake.push_status(0, RunnerStatus::Busy);
    fake.push_status(0, RunnerStatus::Idle);
    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"typing-echo-drained");
    wait_for_output_event(&cap, &spawned.id);

    assert_eq!(
        read_statuses().len(),
        1,
        "suppressed echo-busy and the unchanged idle transition must append nothing",
    );
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );
    assert!(
        !mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .suppress_local_input_busy,
        "the idle transition must clear local-input suppression",
    );

    use fs2::FileExt;
    use std::fs::OpenOptions;
    let blocker = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(log.path())
        .unwrap();
    blocker.lock_exclusive().unwrap();

    let submit_mgr = Arc::clone(&mgr);
    let submit_cap = Arc::clone(&cap);
    let submit_session_id = spawned.id.clone();
    let (submit_done_tx, submit_done_rx) = std::sync::mpsc::channel();
    let submit = std::thread::spawn(move || {
        let result = submit_mgr
            .inject_direct_stdin(&submit_session_id, b"\r", submit_cap.as_ref())
            .map_err(|error| error.to_string());
        submit_done_tx.send(result).unwrap();
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    while !fake
        .keys()
        .iter()
        .any(|(session_id, key)| session_id == &spawned.id && key == "Enter")
    {
        assert!(
            Instant::now() <= deadline,
            "submit never reached the PTY while the event log was contended",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        submit_done_rx.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    blocker.unlock().unwrap();
    submit_done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    submit.join().unwrap();

    let statuses = read_statuses();
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[1].payload["state"], "busy");
    assert_eq!(statuses[1].payload["source"], "input-submit");
    assert_eq!(
        statuses
            .iter()
            .filter(|event| event.payload["source"] == "input-submit")
            .count(),
        1,
    );
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy),
    );
    assert!(
        cap.status.lock().unwrap().is_empty(),
        "mission typing and submit must not emit live session/status events",
    );

    mgr.kill(&spawned.id).unwrap();
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
    let mgr = manager_with_runtime(
        crate::shell_path::LoginShellEnv { path: None, vars },
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    mgr.spawn_direct(
        &runner,
        None,
        None,
        None,
        None,
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
fn utf8_locale_fallback_applies_only_when_no_locale_present() {
    use super::spawn::ensure_utf8_locale;

    let mut env = std::collections::BTreeMap::new();
    ensure_utf8_locale(&mut env, false);
    assert_eq!(
        env.get("LC_CTYPE").map(String::as_str),
        Some("UTF-8"),
        "no locale anywhere must fall back to LC_CTYPE=UTF-8",
    );

    let mut env = std::collections::BTreeMap::new();
    ensure_utf8_locale(&mut env, true);
    assert!(env.is_empty(), "an inherited process locale must win");

    for var in ["LANG", "LC_ALL", "LC_CTYPE"] {
        let mut env = std::collections::BTreeMap::new();
        env.insert(var.to_string(), "zh_CN.GB18030".to_string());
        ensure_utf8_locale(&mut env, false);
        assert_eq!(env.len(), 1, "a configured locale must not be augmented");
        assert_eq!(env.get(var).map(String::as_str), Some("zh_CN.GB18030"));
    }
}

#[test]
fn spawn_env_respects_configured_locale_and_falls_back_to_utf8() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let localized_id = ulid::Ulid::new().to_string();
    let bare_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        for (id, handle) in [(&localized_id, "localized"), (&bare_id, "bare")] {
            conn.execute(
                "INSERT INTO runners
                        (id, handle, display_name, runtime, command,
                         args_json, working_dir, system_prompt, env_json,
                         created_at, updated_at)
                     VALUES (?1, ?2, 'L', 'shell', '/bin/sh',
                             NULL, NULL, NULL, NULL, ?3, ?3)",
                params![id, handle, now],
            )
            .unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = manager_with_runtime(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );

    let mut localized = runner("/bin/sh", &["-c", "true"]);
    localized.id = localized_id;
    localized.handle = "localized".into();
    localized.env.insert("LC_ALL".into(), "zh_CN.UTF-8".into());
    mgr.spawn_direct(
        &localized,
        None,
        None,
        None,
        None,
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
        spec.env.get("LC_ALL").map(String::as_str),
        Some("zh_CN.UTF-8"),
        "runner.env locale must flow through",
    );
    assert!(
        !spec.env.contains_key("LC_CTYPE"),
        "a runner-configured locale must suppress the fallback",
    );

    let mut bare = runner("/bin/sh", &["-c", "true"]);
    bare.id = bare_id;
    bare.handle = "bare".into();
    mgr.spawn_direct(
        &bare,
        None,
        None,
        None,
        None,
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
    let process_has_locale = ["LANG", "LC_ALL", "LC_CTYPE"]
        .iter()
        .any(|var| std::env::var_os(var).is_some());
    if process_has_locale {
        assert!(
            !spec.env.contains_key("LC_CTYPE"),
            "children inherit the process locale — no fallback expected",
        );
    } else {
        assert_eq!(
            spec.env.get("LC_CTYPE").map(String::as_str),
            Some("UTF-8"),
            "locale-free spawn must get the UTF-8 fallback",
        );
    }
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
    let cap = capture();
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
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

    assert!(!mgr.activity_snapshot().contains_key(&session_id));
    cap.status.lock().unwrap().clear();

    // Resume: same id, same row.
    let resumed = mgr
        .resume(
            &session_id,
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
        )
        .unwrap();
    assert_eq!(resumed.id, session_id, "resume must reuse the row id");
    let seeded = wait_for_session_status_event(&cap, &session_id, SessionActivityState::Busy);
    assert_eq!(seeded.source, "resume");
    assert_eq!(
        mgr.activity_snapshot().get(&session_id),
        Some(&SessionActivityState::Busy)
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&session_id)),
        "resume busy seeding must not arm completion",
    );

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

/// Poll the sessions row until the forwarder demotes it from
/// `running` — `resume()` refuses rows that still look live.
fn wait_for_db_stop(pool: &DbPool, session_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        if status != "running" {
            return;
        }
        if Instant::now() > deadline {
            panic!("session {session_id} never left running");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn resume_applies_a_size_pushed_mid_fork() {
    // The resume window has the same shape as the mission fork: the row
    // exists, `resuming` is set, and there is no handle until the new
    // PTY is installed. A push in that window is persisted only; the
    // post-install re-read must apply it.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'midfork', 'MidFork', 'codex', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/sh", &[]);
    runner.id = runner_id;
    runner.handle = "midfork".into();
    runner.runtime = "codex".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            events.clone(),
            None,
        )
        .unwrap();
    let session_id = spawned.id.clone();
    fake.close_spawn(0);
    wait_for_db_stop(&pool, &session_id);
    {
        let mgr = Arc::clone(&mgr);
        let pool = Arc::clone(&pool);
        let session_id = session_id.clone();
        *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
            mgr.resize(&session_id, 112, 38, &pool).unwrap();
        }));
    }

    mgr.resume(
        &session_id,
        Some(113),
        Some(38),
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        events.clone(),
    )
    .unwrap();

    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((113, 38)),
        "the caller's size still forks the PTY"
    );
    assert_eq!(
        fake.resizes.lock().unwrap().as_slice(),
        &[(session_id.clone(), 112, 38)]
    );
    *fake.spawn_hook.lock().unwrap() = None;
    mgr.kill(&session_id).unwrap();
}

#[test]
fn first_spawn_without_dims_uses_and_persists_default_size() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'defaultsize', 'DefaultSize', 'shell', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/sh", &[]);
    runner.id = runner_id;
    runner.handle = "defaultsize".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some(DEFAULT_PTY_SIZE)
    );
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, DEFAULT_PTY_SIZE);

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn resume_size_resolution_prefers_explicit_then_persisted_after_manager_restart() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'persistedsize', 'PersistedSize', 'shell', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/sh", &[]);
    runner.id = runner_id;
    runner.handle = "persistedsize".into();

    let first_fake = fake_runtime();
    let first_mgr = mgr_with_fake(None, Arc::clone(&first_fake));
    let spawned = first_mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            Some(120),
            Some(30),
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    first_fake.close_spawn(0);
    wait_for_db_stop(&pool, &spawned.id);
    first_mgr.resize(&spawned.id, 132, 41, &pool).unwrap();
    first_mgr.settle_pending_resize_now(&spawned.id);
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        persisted,
        (132, 41),
        "a stopped pane must persist its measured dimensions"
    );
    assert!(
        first_fake.resizes.lock().unwrap().is_empty(),
        "a stopped pane must not call the runtime resize path"
    );
    drop(first_mgr);
    drop(first_fake);

    let resumed_fake = fake_runtime();
    let resumed_mgr = mgr_with_fake(None, Arc::clone(&resumed_fake));
    resumed_mgr
        .resume(
            &spawned.id,
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    assert_eq!(
        resumed_fake.last_spawn_spec().unwrap().initial_size,
        Some((132, 41)),
        "unsized resume must use the DB size without manager memory"
    );
    resumed_mgr.kill(&spawned.id).unwrap();
    wait_for_db_stop(&pool, &spawned.id);

    resumed_mgr
        .resume(
            &spawned.id,
            Some(144),
            Some(50),
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert_eq!(
        resumed_fake.last_spawn_spec().unwrap().initial_size,
        Some((144, 50)),
        "explicit resume size must win over the persisted size"
    );
    resumed_mgr.kill(&spawned.id).unwrap();
}

#[test]
fn resize_unknown_session_defers_validation_off_the_caller_thread() {
    let pool = pool_with_schema();
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());

    mgr.set_resize_settle_ms(3_600_000);
    mgr.resize("missing-session", 120, 30, &pool).unwrap();

    assert!(
        mgr.session_state("missing-session").is_some(),
        "the caller only records the measurement in manager memory"
    );
    mgr.settle_pending_resize_now("missing-session");
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
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
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
fn launch_resume_never_falls_back_to_a_fresh_chat_spawn() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'codex-runner', 'Codex', 'codex', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, runner_id, status, started_at)
                 VALUES ('launch-sid', ?1, 'stopped', ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());

    let error = mgr
        .resume_on_launch(
            "launch-sid",
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("cannot resume"));
    let status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = 'launch-sid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "stopped");
}

#[test]
fn launch_resume_keeps_missing_cwd_as_a_chat_error() {
    let pool = pool_with_schema();
    let root = tempfile::tempdir().unwrap();
    let missing_cwd = root.path().join("deleted-chat-cwd");
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'codex-runner', 'Codex', 'codex', '/bin/sh', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, runner_id, status, started_at, cwd, agent_session_key)
                 VALUES ('chat-missing-cwd', ?1, 'stopped', ?2, ?3,
                         '00000000-0000-0000-0000-000000000001')",
            params![runner_id, now, missing_cwd.to_string_lossy()],
        )
        .unwrap();
    }
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let error = mgr
        .resume_on_launch(
            "chat-missing-cwd",
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "working directory does not exist: {}",
            missing_cwd.to_string_lossy()
        )
    );
    assert_eq!(fake.spawn_count(), 0);
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
    let missing_cwd_root = tempfile::tempdir().unwrap();
    let missing_cwd = missing_cwd_root.path().join("deleted-mission-cwd");
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
                    (id, mission_id, runner_id, slot_id, status, started_at, cwd)
                 VALUES ('codex-resume-sid', ?1, ?2, ?3, 'stopped', ?4, ?5)",
            params![
                mission_id,
                runner_id,
                slot_id,
                now,
                missing_cwd.to_string_lossy()
            ],
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
    assert_eq!(spec.cwd.as_deref(), Some(missing_cwd.as_path()));
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
fn synthetic_wake_busy_updates_activity_and_allows_final_idle() {
    let pool = pool_with_schema();
    let mission = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let runner = runner("/bin/cat", &[]);
    let slot_id = insert_crew_runner(&pool, &mission.id, &runner.id);
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

    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"initial-idle-synced");
    wait_for_output_event(&cap, &spawned.id);
    cap.output.lock().unwrap().clear();

    let log = EventLog::open(&mission_dir).unwrap();
    mgr.synthesize_wake_busy(
        &spawned.id,
        EventDraft::signal(
            mission.crew_id.clone(),
            mission.id.clone(),
            runner.handle.clone(),
            SignalType::new("runner_status"),
            serde_json::json!({ "state": "busy" }),
        ),
    )
    .unwrap();

    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy),
        "synthetic busy must update the session-side dedup key",
    );
    let after_busy: Vec<_> = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "runner_status")
        })
        .collect();
    assert_eq!(after_busy.len(), 2);
    assert_eq!(after_busy[1].payload["state"], "busy");

    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"final-idle-drained");
    wait_for_output_event(&cap, &spawned.id);

    let statuses: Vec<_> = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "runner_status")
        })
        .collect();
    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses[2].payload["state"], "idle");
    assert_eq!(statuses[2].payload["source"], "forwarder");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn suppressed_busy_then_agent_output_and_quiet_appends_final_idle() {
    let pool = pool_with_schema();
    let mission = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let runner = runner("/bin/cat", &[]);
    let slot_id = insert_crew_runner(&pool, &mission.id, &runner.id);
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

    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"initial-idle-synced");
    wait_for_output_event(&cap, &spawned.id);
    cap.output.lock().unwrap().clear();

    mgr.inject_direct_stdin(&spawned.id, b"router nudge", cap.as_ref())
        .unwrap();
    assert!(
        mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .suppress_local_input_busy,
        "unsubmitted nudge body must open the suppressed-busy window",
    );

    let log = EventLog::open(&mission_dir).unwrap();
    mgr.synthesize_wake_busy(
        &spawned.id,
        EventDraft::signal(
            mission.crew_id.clone(),
            mission.id.clone(),
            runner.handle.clone(),
            SignalType::new("runner_status"),
            serde_json::json!({ "state": "busy" }),
        ),
    )
    .unwrap();

    fake.push_status(0, RunnerStatus::Busy);
    fake.push_output(0, b"agent output");
    fake.push_status(0, RunnerStatus::Idle);
    fake.push_output(0, b"quiet-transition-drained");

    let deadline = Instant::now() + Duration::from_secs(2);
    let statuses = loop {
        let statuses: Vec<_> = log
            .read_from(0)
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .filter(|event| {
                event
                    .signal_type
                    .as_ref()
                    .is_some_and(|ty| ty.as_str() == "runner_status")
            })
            .collect();
        if statuses.last().is_some_and(|event| {
            event.payload.get("state").and_then(|state| state.as_str()) == Some("idle")
        }) && statuses.len() >= 3
        {
            break statuses;
        }
        assert!(
            Instant::now() < deadline,
            "final idle was not appended after the suppressed busy"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        statuses
            .iter()
            .map(|event| event.payload["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["idle", "busy", "idle"],
        "the suppressed forwarder busy must not swallow the paired idle",
    );
    assert_eq!(statuses[2].payload["source"], "forwarder");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn forwarder_status_emit_stays_bounded_under_event_log_contention() {
    // Issue #124 / @reviewer P1: the forwarder consumer drains
    // terminal output, exit-event reap, AND `runner_status`
    // emission through the same thread. If `try_append_runner_status`
    // ever blocked on the event-log flock, a stuck mission log
    // would freeze terminal output too — the user would see a
    // hang the moment a second CLI writer took the lock.
    // Construct a real ForwarderEmitCtx against a tempdir,
    // steal the flock from another "process" (a parallel fd
    // holding LOCK_EX), and assert that
    // `try_append_runner_status` exhausts its bounded retries and
    // returns `Contended` within a hard 100ms bound.
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
        elapsed < ci_scaled_budget(Duration::from_millis(100)),
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
fn forwarder_status_emit_retries_brief_event_log_contention() {
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
    assert!(matches!(
        event_log.try_append(ctx.runner_status_draft(RunnerStatus::Idle, "forwarder")),
        Err(TryAppendError::Contended),
    ));

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let retry_ctx = ctx.clone();
    let append = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        retry_ctx.try_append_runner_status(RunnerStatus::Idle, "forwarder")
    });
    started_rx.recv().unwrap();
    // Keep the unlock well inside the ~35ms retry budget so a loaded CI
    // machine can't slip it past the last attempt.
    std::thread::sleep(Duration::from_millis(5));
    blocker.unlock().unwrap();

    assert!(matches!(append.join().unwrap(), AppendOutcome::Ok));
    let statuses: Vec<_> = event_log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "runner_status")
        })
        .collect();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].payload["state"], "idle");
}

fn hold_event_log_lock(
    event_log: &EventLog,
) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    use fs2::FileExt;
    use std::fs::OpenOptions;

    let path = event_log.path().to_path_buf();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .unwrap();
        file.lock_exclusive().unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        file.unlock().unwrap();
    });
    ready_rx.recv().unwrap();
    (release_tx, blocker)
}

fn manager_with_contended_wake_sink(event_log: Arc<EventLog>) -> Arc<SessionManager> {
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let state = mgr.session_state_or_insert("session");
    state.lock().unwrap().mission_status_sink = Some(ForwarderEmitCtx {
        crew_id: "crew".into(),
        mission_id: "mission".into(),
        handle: "runner".into(),
        event_log,
    });
    mgr
}

fn wake_busy_draft() -> EventDraft {
    EventDraft::signal(
        "crew",
        "mission",
        "runner",
        SignalType::new("runner_status"),
        serde_json::json!({ "state": "busy", "source": "router-wake" }),
    )
}

#[test]
fn contended_wake_append_does_not_block_output_ingestion() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    let (release, blocker) = hold_event_log_lock(&event_log);

    let session = mgr.session_state("session").unwrap();
    let session_guard = session.lock().unwrap();
    let (wake_started_tx, wake_started_rx) = std::sync::mpsc::channel();
    let wake_mgr = Arc::clone(&mgr);
    let wake = std::thread::spawn(move || {
        wake_started_tx.send(()).unwrap();
        wake_mgr.synthesize_wake_busy("session", wake_busy_draft())
    });
    wake_started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(1));
    drop(session_guard);
    std::thread::sleep(Duration::from_millis(2));

    let (output_started_tx, output_started_rx) = std::sync::mpsc::channel();
    let (output_done_tx, output_done_rx) = std::sync::mpsc::channel();
    let output_mgr = Arc::clone(&mgr);
    let output = std::thread::spawn(move || {
        output_started_tx.send(()).unwrap();
        let start = Instant::now();
        let mut event = None;
        for _ in 0..300 {
            event = Some(output_mgr.record_output("session", Some("mission"), b"output"));
        }
        output_done_tx.send((start.elapsed(), event)).unwrap();
    });
    output_started_rx.recv().unwrap();
    let observed = output_done_rx.recv_timeout(Duration::from_millis(20));

    release.send(()).unwrap();
    blocker.join().unwrap();
    let wake_result = wake.join().unwrap();
    output.join().unwrap();

    let (elapsed, event) = observed.expect(
        "record_output must finish while the wake append is still parked on the event-log lock",
    );
    assert!(
        elapsed < Duration::from_millis(20),
        "record_output must stay well under the wake retry budget; took {elapsed:?}",
    );
    assert_eq!(event.unwrap().seq, 300);
    wake_result.unwrap();
}

#[test]
fn synthetic_wake_does_not_overwrite_a_newer_forwarder_transition() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    let (release, blocker) = hold_event_log_lock(&event_log);

    let session = mgr.session_state("session").unwrap();
    let session_guard = session.lock().unwrap();
    let (wake_started_tx, wake_started_rx) = std::sync::mpsc::channel();
    let wake_mgr = Arc::clone(&mgr);
    let wake = std::thread::spawn(move || {
        wake_started_tx.send(()).unwrap();
        wake_mgr.synthesize_wake_busy("session", wake_busy_draft())
    });
    wake_started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(1));
    drop(session_guard);
    std::thread::sleep(Duration::from_millis(2));

    let (transition_started_tx, transition_started_rx) = std::sync::mpsc::channel();
    let (transition_done_tx, transition_done_rx) = std::sync::mpsc::channel();
    let transition_mgr = Arc::clone(&mgr);
    let transition = std::thread::spawn(move || {
        transition_started_tx.send(()).unwrap();
        let busy_changed = transition_mgr.note_forwarder_transition(
            "session",
            SessionActivityState::Busy,
            "forwarder",
        );
        let idle_changed = transition_mgr.note_forwarder_transition(
            "session",
            SessionActivityState::Idle,
            "forwarder",
        );
        transition_done_tx
            .send((busy_changed, idle_changed))
            .unwrap();
    });
    transition_started_rx.recv().unwrap();
    let transition_before_unlock = transition_done_rx.recv_timeout(Duration::from_millis(20));

    release.send(()).unwrap();
    blocker.join().unwrap();
    let wake_result = wake.join().unwrap();
    transition.join().unwrap();

    assert_eq!(
        transition_before_unlock.expect(
            "newer forwarder transitions must acquire the session lock while append is parked",
        ),
        (true, true),
    );
    wake_result.unwrap();
    assert_eq!(
        mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
        "the completed wake append must not replace the newer idle transition",
    );
}

#[test]
fn failed_synthetic_wake_append_preserves_activity_and_error_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    let (release, blocker) = hold_event_log_lock(&event_log);

    let error = mgr
        .synthesize_wake_busy("session", wake_busy_draft())
        .unwrap_err();
    assert_eq!(error.to_string(), "event log busy");
    assert_eq!(
        mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
    );

    release.send(()).unwrap();
    blocker.join().unwrap();

    let missing_dir = tempfile::tempdir().unwrap();
    let missing_log = Arc::new(EventLog::open(missing_dir.path()).unwrap());
    let missing_mgr = manager_with_contended_wake_sink(missing_log);
    missing_mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    missing_dir.close().unwrap();

    let error = missing_mgr
        .synthesize_wake_busy("session", wake_busy_draft())
        .unwrap_err();
    assert_ne!(error.to_string(), "event log busy");
    assert_eq!(
        missing_mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
    );
}

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
        elapsed < ci_scaled_budget(Duration::from_millis(100)),
        "first claude must not wait — actual elapsed {}ms",
        elapsed.as_millis()
    );
}

#[test]
fn spawn_argv_injects_claude_fullscreen_for_fresh_and_resume_only() {
    let compose = |runtime: &str, plan: router::runtime::ResumePlan| {
        let mut runner = runner("/bin/cat", &["--debug"]);
        runner.runtime = runtime.into();
        let mut spec = SpawnSpec {
            session_id: "settings-argv".into(),
            cwd: None,
            command: runner.command.clone(),
            args: runner.args.clone(),
            env: BTreeMap::new(),
            mission: false,
            shim_dir: None,
            bundled_bin_dir: None,
            shell_path: None,
            initial_size: Some((80, 24)),
        };
        SessionManager::apply_runtime_args(
            &mut spec,
            &runner,
            &plan,
            Path::new("/tmp/runner-app-data"),
            Some("first turn"),
            None,
        );
        spec.args
    };

    let fresh = compose(
        "claude-code",
        router::runtime::resume_plan("claude-code", None),
    );
    let settings = fresh
        .windows(2)
        .find(|pair| pair[0] == "--settings")
        .map(|pair| serde_json::from_str::<serde_json::Value>(&pair[1]).unwrap())
        .expect("Claude spawn should carry --settings");
    assert_eq!(settings["tui"], "fullscreen");
    assert!(settings["hooks"]["SessionStart"].is_array());
    assert_eq!(fresh.last().map(String::as_str), Some("first turn"));

    let prior = uuid::Uuid::new_v4().to_string();
    let resumed = compose(
        "claude-code",
        router::runtime::resume_plan("claude-code", Some(&prior)),
    );
    assert!(resumed.windows(2).any(|pair| pair[0] == "--settings"));

    let codex = compose("codex", router::runtime::resume_plan("codex", None));
    assert!(!codex.iter().any(|arg| arg == "--settings"));
}

fn spawn_claude_for_resize(
    handle: &str,
) -> (
    Arc<db::DbPool>,
    Arc<FakeRuntime>,
    Arc<SessionManager>,
    String,
) {
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
                 VALUES (?1, ?2, 'Debounce', 'claude-code', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?3, ?3)",
            params![runner_id, handle, now],
        )
        .unwrap();
    }
    let mut runner = runner("/bin/cat", &[]);
    runner.id = runner_id;
    runner.handle = handle.into();
    runner.runtime = "claude-code".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            Some(120),
            Some(30),
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    (pool, fake, mgr, spawned.id)
}

#[test]
fn resize_storm_ioctls_every_push_and_persists_once() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("storm");
    mgr.set_resize_settle_ms(3_600_000);
    {
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE resize_writes (count INTEGER NOT NULL);
             INSERT INTO resize_writes VALUES (0);
             CREATE TRIGGER count_resize_writes
             AFTER UPDATE OF last_cols, last_rows ON sessions
             BEGIN UPDATE resize_writes SET count = count + 1; END;",
        )
        .unwrap();
    }

    for cols in [78u16, 76, 209, 76, 210] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let expected = vec![
        (id.clone(), 78, 30),
        (id.clone(), 76, 30),
        (id.clone(), 209, 30),
        (id.clone(), 76, 30),
        (id.clone(), 210, 30),
    ];
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    let before: (u16, u16, i64) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT s.last_cols, s.last_rows, w.count
               FROM sessions s CROSS JOIN resize_writes w
              WHERE s.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(before, (120, 30, 0));

    mgr.settle_pending_resize_now(&id);
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    let settled: (u16, u16, i64) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT s.last_cols, s.last_rows, w.count
               FROM sessions s CROSS JOIN resize_writes w
              WHERE s.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(settled, (210, 30, 1));

    mgr.kill(&id).unwrap();
}

#[test]
fn round_trip_resize_storm_preserves_the_immediate_width_chain() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("roundtrip");
    mgr.set_resize_settle_ms(3_600_000);

    for cols in [150u16, 209, 150, 120] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let expected = vec![
        (id.clone(), 150, 30),
        (id.clone(), 209, 30),
        (id.clone(), 150, 30),
        (id.clone(), 120, 30),
    ];
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    mgr.settle_pending_resize_now(&id);
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);

    mgr.kill(&id).unwrap();
}

#[test]
fn stale_resize_settle_does_not_persist_after_respawn() {
    let (pool, _fake, mgr, id) = spawn_claude_for_resize("stalegeneration");
    mgr.set_resize_settle_ms(3_600_000);

    mgr.resize(&id, 100, 30, &pool).unwrap();
    let stale_generation = mgr
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .pending_resize
        .as_ref()
        .unwrap()
        .generation;
    mgr.kill(&id).unwrap();
    mgr.resume(
        &id,
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    mgr.resize(&id, 110, 30, &pool).unwrap();
    let state = mgr.session_state(&id).unwrap();
    let current_generation = state
        .lock()
        .unwrap()
        .pending_resize
        .as_ref()
        .unwrap()
        .generation;
    assert_ne!(stale_generation, current_generation);

    mgr.settle_pending_resize_generation_now(&id, stale_generation);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .pending_resize
            .as_ref()
            .map(|pending| pending.generation),
        Some(current_generation)
    );
    let before: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, (120, 30));

    mgr.settle_pending_resize_generation_now(&id, current_generation);
    let settled: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(settled, (110, 30));

    mgr.kill(&id).unwrap();
}

#[test]
fn settle_during_inflight_kill_abandons_persistence() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("killwindow");
    mgr.set_resize_settle_ms(3_600_000);

    mgr.resize(&id, 100, 30, &pool).unwrap();
    let (stop_entered, stop_release) = fake.arm_stop_gate();
    let kill = {
        let mgr = Arc::clone(&mgr);
        let id = id.clone();
        std::thread::spawn(move || mgr.kill(&id))
    };
    stop_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("kill never reached runtime.stop");
    mgr.settle_pending_resize_now(&id);
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, (120, 30));
    assert_eq!(
        fake.resizes.lock().unwrap().clone(),
        vec![(id.clone(), 100, 30)]
    );

    stop_release.send(()).unwrap();
    kill.join().unwrap().unwrap();
}

#[test]
fn resize_settle_thread_persists_without_extra_ioctls() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("stormthread");
    mgr.set_resize_settle_ms(25);

    for cols in [78u16, 209, 210] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let persisted: (u16, u16) = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        if persisted == (210, 30) {
            break;
        }
        if Instant::now() > deadline {
            panic!("settle thread never persisted the storm");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fake.resizes.lock().unwrap().clone(),
        vec![
            (id.clone(), 78, 30),
            (id.clone(), 209, 30),
            (id.clone(), 210, 30),
        ]
    );

    mgr.kill(&id).unwrap();
}

#[test]
fn runtime_direct_runner_applies_model_and_effort() {
    let configured =
        runtime_direct_runner("codex", None, Some(" gpt-5.6-sol "), Some(" max ")).unwrap();
    assert_eq!(configured.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(configured.effort.as_deref(), Some("max"));

    let defaults = runtime_direct_runner("codex", None, Some(" "), Some("")).unwrap();
    assert_eq!(defaults.model, None);
    assert_eq!(defaults.effort, None);
}

#[cfg(unix)]
#[test]
fn shell_runtime_spawns_and_resumes_as_plain_login_shell() {
    let pool = pool_with_schema();
    let project = crate::repo::project::create(&pool.get().unwrap(), "Project", "/tmp").unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(
        Some("/usr/local/bin:/usr/bin:/bin".into()),
        Arc::clone(&fake),
    );
    let shell = runtime_direct_runner("shell", Some("/bin/zsh"), None, None).unwrap();

    assert_eq!(shell.args, ["-l"]);
    assert!(shell.system_prompt.is_none());
    assert!(shell.env.is_empty());
    assert!(shell.model.is_none());
    assert!(shell.effort.is_none());

    let spawned = mgr
        .spawn_runtime_direct(
            &shell,
            Some(&project.id),
            Some("/tmp"),
            Some(132),
            Some(41),
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().expect("shell should spawn");
    assert_eq!(spec.command, "/bin/zsh");
    assert_eq!(spec.args, ["-l"]);
    assert_eq!(spec.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    assert_eq!(spec.initial_size, Some((132, 41)));
    assert_eq!(
        spec.shell_path.as_deref(),
        Some("/usr/local/bin:/usr/bin:/bin")
    );
    assert!(spec.shim_dir.is_none());
    assert!(spec.bundled_bin_dir.is_none());
    assert!(spec.env.keys().all(|key| !key.starts_with("RUNNER_")));

    let stored = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(stored.cwd.as_deref(), Some("/tmp"));
    assert_eq!(stored.agent_runtime.as_deref(), Some("shell"));
    assert_eq!(stored.agent_command.as_deref(), Some("/bin/zsh"));
    assert!(stored.agent_session_key.is_none());

    mgr.kill(&spawned.id).unwrap();
    mgr.resume_on_launch(
        &spawned.id,
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("shell should resume");
    assert_eq!(resumed.command, "/bin/zsh");
    assert_eq!(resumed.args, ["-l"]);
    assert_eq!(resumed.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    assert!(resumed.env.keys().all(|key| !key.starts_with("RUNNER_")));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn shell_resume_uses_nearest_existing_cwd_and_feeds_notice_first() {
    let pool = pool_with_schema();
    let root = tempfile::tempdir().unwrap();
    let project_cwd = root.path().join("project");
    let existing_ancestor = root.path().join("worktrees").join("feature");
    std::fs::create_dir_all(&project_cwd).unwrap();
    std::fs::create_dir_all(&existing_ancestor).unwrap();
    let missing_cwd = existing_ancestor.join("deleted").join("nested");
    let project = crate::repo::project::create(
        &pool.get().unwrap(),
        "Project",
        &project_cwd.to_string_lossy(),
    )
    .unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, project_id, status, started_at, cwd,
                 agent_runtime, agent_command, resume_on_launch)
             VALUES ('shell-missing-cwd', ?1, 'stopped', ?2, ?3,
                     'shell', '/bin/zsh', 1)",
            params![
                project.id,
                Utc::now().to_rfc3339(),
                missing_cwd.to_string_lossy()
            ],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let fake_for_hook = Arc::clone(&fake);
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        fake_for_hook.push_output(0, b"shell startup\r\n");
    }));
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    mgr.resume_on_launch(
        "shell-missing-cwd",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        events.clone(),
    )
    .unwrap();

    let spawned = fake.last_spawn_spec().expect("shell should relaunch");
    assert_eq!(spawned.cwd.as_deref(), Some(existing_ancestor.as_path()));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if events.output.lock().unwrap().len() >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "shell startup output was not forwarded"
        );
        std::thread::yield_now();
    }
    let output = events.output.lock().unwrap();
    assert_eq!(output[0].seq, 1);
    assert_eq!(
        output[0].bytes,
        format!(
            "\x1b[33mrunner: {} no longer exists\r\n        opened {} instead\x1b[0m\r\n",
            missing_cwd.to_string_lossy(),
            existing_ancestor.to_string_lossy(),
        )
        .into_bytes()
    );
    assert_eq!(output[1].bytes, b"shell startup\r\n");
    drop(output);

    mgr.kill("shell-missing-cwd").unwrap();
}

#[test]
fn runtime_direct_spawn_persists_model_and_effort() {
    let pool = pool_with_schema();
    let configured =
        runtime_direct_runner("codex", Some("/bin/sh"), Some("gpt-5.6-sol"), Some("max")).unwrap();
    let mgr = mgr_with_fake(None, fake_runtime());
    let spawned = mgr
        .spawn_runtime_direct(
            &configured,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let stored: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_command, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(stored.0.as_deref(), Some("codex"));
    assert_eq!(stored.1.as_deref(), Some("/bin/sh"));
    assert_eq!(stored.2.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.3.as_deref(), Some("max"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn pinned_direct_spawn_records_override_model_and_effort() {
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
                 VALUES (?1, 'pin-me', 'Pin', 'codex', '/bin/sh',
                         '[]', NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut r = runner("/bin/sh", &[]);
    r.id = runner_id;
    r.runtime = "codex".into();
    r.model = Some("runner-model".into());
    r.effort = Some("runner-effort".into());
    let mgr = mgr_with_fake(None, fake_runtime());
    let spawned = mgr
        .spawn_direct(
            &r,
            Some("codex"),
            Some("gpt-5.6-sol"),
            Some("ultra"),
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let stored: (Option<String>, Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored.0.as_deref(), Some("codex"));
    assert_eq!(stored.1.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.2.as_deref(), Some("ultra"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn unpinned_direct_spawn_persists_options_without_pinning_runtime() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'options-only', 'Options', 'codex', '/bin/sh',
                         '[]', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
    }
    let mut r = runner("/bin/sh", &[]);
    r.id = runner_id;
    r.runtime = "codex".into();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &r,
            None,
            Some("gpt-5.6-sol"),
            Some("ultra"),
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let stored: (Option<String>, Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored.0, None, "options-only direct chats must not pin");
    assert_eq!(stored.1.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.2.as_deref(), Some("ultra"));

    mgr.kill(&spawned.id).unwrap();
    mgr.resume(
        &spawned.id,
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("resume should spawn");
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "gpt-5.6-sol"));
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=ultra"));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn runtime_override_helper_distinguishes_absent_matching_and_differing() {
    let mut r = runner("codex-custom", &["--custom"]);
    r.runtime = "codex".into();

    // Absent / blank: no rebuild, no pin.
    for value in [None, Some("  ")] {
        let res = resolve_runtime_override(&r, value, None, None).unwrap();
        assert!(res.effective.is_none());
        assert!(!res.pinned, "absent/blank override must not pin");
    }

    // Matching: no rebuild (spawn stays byte-identical), but pinned —
    // the session row must record the engine so a later runner-
    // template edit can't re-engine its resume.
    let matching = resolve_runtime_override(&r, Some("codex"), None, None).unwrap();
    assert!(matching.effective.is_none());
    assert!(matching.pinned, "explicit matching override must pin");

    // Differing: rebuild + pin for every other catalog runtime.
    for runtime in ["claude-code", "trae"] {
        let differing = resolve_runtime_override(&r, Some(runtime), None, None).unwrap();
        assert_eq!(
            differing.effective.as_ref().map(|r| r.runtime.as_str()),
            Some(runtime),
        );
        assert!(differing.pinned);
    }
}

#[test]
fn runtime_override_helper_resets_engine_fields_and_keeps_persona() {
    let mut r = runner("codex-custom", &["--custom-flag"]);
    r.runtime = "codex".into();
    r.model = Some("gpt-5-codex".into());
    r.effort = Some("high".into());
    r.system_prompt = Some("persona".into());
    r.working_dir = Some("/work".into());
    r.env.insert("FOO".into(), "bar".into());

    let effective = resolve_runtime_override(&r, Some("claude-code"), None, None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective runner");
    // Engine fields reset to registry defaults.
    assert_eq!(effective.runtime, "claude-code");
    assert_eq!(effective.command, "claude");
    assert_eq!(
        effective.args,
        router::runtime::apply_permission_mode(
            "claude-code",
            &[],
            crate::ops::runner::default_permission_mode(),
        ),
        "override args must be the registry default permission-mode pair",
    );
    assert!(!effective.args.contains(&"--custom-flag".to_string()));
    assert_eq!(effective.model, None);
    assert_eq!(effective.effort, None);
    // Persona fields carry over.
    assert_eq!(effective.system_prompt.as_deref(), Some("persona"));
    assert_eq!(effective.working_dir.as_deref(), Some("/work"));
    assert_eq!(effective.env.get("FOO").map(String::as_str), Some("bar"));
    assert_eq!(effective.id, r.id);
    assert_eq!(effective.handle, r.handle);
}

#[test]
fn runtime_override_helper_applies_slot_model_to_selected_runtime() {
    let mut r = runner("codex-custom", &["--custom"]);
    r.runtime = "codex".into();
    r.model = Some("runner-model".into());

    let differing = resolve_runtime_override(&r, Some("trae"), Some("trae-slot-model"), None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective runner");
    assert_eq!(differing.runtime, "trae");
    assert_eq!(differing.model.as_deref(), Some("trae-slot-model"));

    let matching = resolve_runtime_override(&r, Some("codex"), Some("codex-slot-model"), None)
        .unwrap()
        .effective
        .expect("a model override must rebuild even for a matching runtime");
    assert_eq!(matching.runtime, "codex");
    assert_eq!(matching.model.as_deref(), Some("codex-slot-model"));
    assert_eq!(matching.args, r.args);

    let unpinned = resolve_runtime_override(&r, None, Some("codex-slot-model"), None).unwrap();
    let effective = unpinned
        .effective
        .expect("a model-only override must rebuild the runner config");
    assert_eq!(effective.runtime, "codex");
    assert_eq!(effective.model.as_deref(), Some("codex-slot-model"));
    assert_eq!(effective.effort, r.effort);
    assert!(!unpinned.pinned, "model-only overrides must not pin");
}

#[test]
fn runtime_override_helper_applies_effort_to_selected_runtime() {
    let mut r = runner("codex-custom", &["--custom"]);
    r.runtime = "codex".into();
    r.model = Some("runner-model".into());
    r.effort = Some("runner-effort".into());

    let differing = resolve_runtime_override(&r, Some("claude-code"), Some("fable"), Some("max"))
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective runner");
    assert_eq!(differing.runtime, "claude-code");
    assert_eq!(differing.model.as_deref(), Some("fable"));
    assert_eq!(differing.effort.as_deref(), Some("max"));

    let cleared = resolve_runtime_override(&r, Some("claude-code"), None, None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective runner");
    assert_eq!(cleared.model, None);
    assert_eq!(cleared.effort, None);

    let matching = resolve_runtime_override(&r, Some("codex"), None, Some("xhigh"))
        .unwrap()
        .effective
        .expect("an effort override must rebuild even for a matching runtime");
    assert_eq!(matching.model.as_deref(), Some("runner-model"));
    assert_eq!(matching.effort.as_deref(), Some("xhigh"));

    let unpinned = resolve_runtime_override(&r, None, None, Some("high")).unwrap();
    let effective = unpinned
        .effective
        .expect("an effort-only override must rebuild the runner config");
    assert_eq!(effective.runtime, "codex");
    assert_eq!(effective.model.as_deref(), Some("runner-model"));
    assert_eq!(effective.effort.as_deref(), Some("high"));
    assert!(!unpinned.pinned, "effort-only overrides must not pin");

    let blank = resolve_runtime_override(&r, Some("codex"), Some("  "), Some("")).unwrap();
    assert!(blank.effective.is_none());
    assert!(blank.pinned);
}

#[test]
fn runtime_override_helper_rejects_unknown_runtime() {
    let r = runner("/bin/sh", &[]);
    let err = resolve_runtime_override(&r, Some("aider-future"), None, None).unwrap_err();
    assert!(err.to_string().contains("unknown runtime"), "got: {err}",);
}

#[test]
fn mission_spawn_with_slot_override_uses_registry_engine_and_records_runtime() {
    let pool = pool_with_schema();
    let mission_row = mission();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_row.id, &runner_id);

    // Runner row is a codex engine with custom flags + pinned
    // model/effort; the slot overrides to claude-code and selects
    // its own model and effort.
    let mut runner = runner("codex-custom", &["--custom-flag"]);
    runner.id = runner_id.clone();
    runner.runtime = "codex".into();
    runner.model = Some("gpt-5-codex".into());
    runner.effort = Some("high".into());
    runner.env.insert("FOO".into(), "bar".into());
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.runtime_override = Some("claude-code".into());
    slot.model_override = Some("opus".into());
    slot.effort_override = Some("max".into());

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission_row,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_effective_command(&spec.command, "claude");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()),
        "runner args are engine flags and must not carry across runtimes: {:?}",
        spec.args,
    );
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "opus"));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--effort" && w[1] == "max"));
    assert!(
        spec.args
            .windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "auto"),
        "override args must be the registry default permission-mode pair: {:?}",
        spec.args,
    );
    assert!(
        spec.args.contains(&"--session-id".to_string()),
        "resume plan must be computed for the effective runtime: {:?}",
        spec.args,
    );
    assert_eq!(
        spec.env.get("FOO").map(String::as_str),
        Some("bar"),
        "persona env must carry over",
    );

    // Session row records the effective runtime for respawn/resume.
    let (agent_runtime, agent_command, agent_model, agent_effort): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_command, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(agent_runtime.as_deref(), Some("claude-code"));
    assert_effective_command(agent_command.as_deref().unwrap(), "claude");
    assert_eq!(agent_model.as_deref(), Some("opus"));
    assert_eq!(agent_effort.as_deref(), Some("max"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_with_model_only_slot_override_uses_runner_runtime_without_pinning() {
    let pool = pool_with_schema();
    let mission_row = mission();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_row.id, &runner_id);

    let mut runner = runner("codex-custom", &["--custom-flag"]);
    runner.id = runner_id;
    runner.runtime = "codex".into();
    runner.model = Some("runner-model".into());
    runner.effort = Some("high".into());
    pool.get()
        .unwrap()
        .execute(
            "UPDATE runners
                SET runtime = 'codex', command = 'codex-custom',
                    args_json = '[\"--custom-flag\"]',
                    model = 'runner-model', effort = 'high'
              WHERE id = ?1",
            params![runner.id],
        )
        .unwrap();
    let mut slot = slot_for(&runner);
    slot.id = slot_id;
    slot.model_override = Some("slot-model".into());

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission_row,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_eq!(spec.command, "codex-custom");
    assert!(spec.args.contains(&"--custom-flag".to_string()));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "slot-model"));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"));

    let (agent_runtime, agent_model, agent_effort): (
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(agent_runtime, None, "model-only overrides must not pin");
    assert_eq!(agent_model.as_deref(), Some("slot-model"));
    assert_eq!(agent_effort.as_deref(), Some("high"));

    mgr.kill(&spawned.id).unwrap();
    mgr.resume(
        &spawned.id,
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("resume should spawn");
    assert_eq!(resumed.command, "codex-custom");
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "slot-model"));
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_with_matching_override_keeps_args_and_pins_runtime() {
    // An override naming the runner's own runtime must spawn
    // byte-identically to no override (same command, same args) but
    // still record the effective runtime on the row: the slot is
    // explicitly pinned, so a later edit to the runner template's
    // runtime must not re-engine this session's resume.
    let pool = pool_with_schema();
    let mission_row = mission();
    let runner_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_runner(&pool, &mission_row.id, &runner_id);

    // "codex" is a registry runtime — the only kind the slot write
    // validator can actually store as an override.
    let mut runner = runner("codex-custom", &["--custom-flag"]);
    runner.id = runner_id.clone();
    runner.runtime = "codex".into();
    let mut slot = slot_for(&runner);
    slot.id = slot_id;

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    // Baseline: no override.
    slot.runtime_override = None;
    let baseline = mgr
        .spawn(
            &mission_row,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let baseline_spec = fake.last_spawn_spec().expect("baseline spawn");

    // Matching override.
    slot.runtime_override = Some("codex".into());
    let pinned = mgr
        .spawn(
            &mission_row,
            &runner,
            &slot,
            std::path::Path::new("/tmp"),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let pinned_spec = fake.last_spawn_spec().expect("pinned spawn");

    assert_eq!(pinned_spec.command, baseline_spec.command);
    assert_eq!(
        pinned_spec.args, baseline_spec.args,
        "matching override must spawn byte-identical args",
    );
    assert_eq!(pinned_spec.command, "codex-custom");

    let runtime_for = |id: &str| -> (Option<String>, Option<String>) {
        pool.get()
            .unwrap()
            .query_row(
                "SELECT agent_runtime, agent_command FROM sessions WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(
        runtime_for(&baseline.id),
        (None, None),
        "no override must not record agent_runtime",
    );
    assert_eq!(
        runtime_for(&pinned.id),
        (Some("codex".into()), Some("codex-custom".into())),
        "matching override must pin the effective runtime on the row",
    );

    mgr.kill(&baseline.id).unwrap();
    mgr.kill(&pinned.id).unwrap();
}

#[test]
fn resume_keeps_pinned_runtime_after_runner_template_edit() {
    // The scenario the pin exists for: a session spawned with an
    // explicit override matching the runner's then-runtime ("codex"),
    // recorded on the row. The user later edits the runner template
    // to claude-code. Resume must respawn this session on codex —
    // registry defaults — not on the template's new runtime, which
    // would hand the codex-native session key to the wrong CLI.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'tester', 'T', 'codex', 'codex-custom',
                         '[\"--custom-flag\"]', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at,
                     agent_runtime, agent_command)
                 VALUES ('pin-sid', NULL, ?1, '/tmp', 'stopped', ?2,
                         'codex', 'codex-custom')",
            params![runner_id, now],
        )
        .unwrap();
        // The runner template moves on to a different engine.
        conn.execute(
            "UPDATE runners SET runtime = 'claude-code', command = 'claude-custom'
              WHERE id = ?1",
            params![runner_id],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.resume(
        "pin-sid",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("resume should have spawned");
    assert_effective_command(&spec.command, "codex");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()) && spec.command != "claude-custom",
        "neither the template's new engine nor its old flags may leak in: {:?}",
        spec.args,
    );

    mgr.kill("pin-sid").unwrap();
}

#[test]
fn direct_spawn_with_override_uses_registry_engine_and_records_runtime() {
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let mut runner = runner("codex-custom", &["--custom-flag"]);
    runner.runtime = "codex".into();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command, created_at, updated_at)
                 VALUES (?1, 'tester', 'T', 'codex', 'codex-custom', ?2, ?2)",
            params![runner.id, now],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &runner,
            Some("claude-code"),
            None,
            None,
            None,
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
    assert_effective_command(&spec.command, "claude");
    assert!(!spec.args.contains(&"--custom-flag".to_string()));

    let (row_runner_id, agent_runtime, agent_command): (
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT runner_id, agent_runtime, agent_command FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row_runner_id.as_deref(),
        Some(runner.id.as_str()),
        "overridden chats stay runner-backed",
    );
    assert_eq!(agent_runtime.as_deref(), Some("claude-code"));
    assert_effective_command(agent_command.as_deref().unwrap(), "claude");

    mgr.kill(&spawned.id).unwrap();
}

#[cfg(unix)]
#[test]
fn claude_direct_chat_fork_spawns_tui_directly_with_copied_row() {
    let pool = pool_with_schema();
    let runner_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let command = "claude-custom".to_string();
    let project_id = {
        let conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "Runner", "/tmp").unwrap();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, args_json,
                 env_json, working_dir, system_prompt, created_at, updated_at)
             VALUES (?1, 'forker', 'Forker', 'claude-code', ?2,
                     '[\"--runner-flag\"]', ?3,
                     '/tmp', 'Forker persona', ?4, ?4)",
            params![
                runner_id,
                command,
                serde_json::json!({"FORK_TEST_ENV": "same-env"}).to_string(),
                now.to_rfc3339(),
            ],
        )
        .unwrap();
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.project_id = Some(project.id.clone());
        row.runner_id = Some(runner_id.clone());
        row.cwd = Some("/tmp".into());
        row.started_at = Some(now);
        row.agent_session_key = Some(source_key.clone());
        row.agent_model = Some("opus".into());
        row.agent_effort = Some("max".into());
        row.title = Some("Source".into());
        row.last_cols = Some(90);
        row.last_rows = Some(30);
        crate::repo::session::insert(&conn, &row).unwrap();
        project.id
    };
    let source_before = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    let spawned = mgr
        .spawn_fork(
            &source_id,
            Some("Source (fork)".into()),
            Some(120),
            Some(40),
            Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&events) as Arc<dyn SessionEvents>,
        )
        .unwrap();
    assert_eq!(
        *events.fork_started.lock().unwrap(),
        [SessionForkStartedEvent {
            source_session_id: source_id.clone(),
            session_id: spawned.id.clone(),
        }]
    );

    let fork = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_ne!(fork.id, source_id);
    assert_eq!(fork.project_id.as_deref(), Some(project_id.as_str()));
    assert_eq!(fork.runner_id.as_deref(), Some(runner_id.as_str()));
    assert_eq!(fork.cwd.as_deref(), Some("/tmp"));
    assert_eq!(fork.agent_runtime, source_before.agent_runtime);
    assert_eq!(fork.agent_command, source_before.agent_command);
    assert_eq!(fork.agent_model, source_before.agent_model);
    assert_eq!(fork.agent_effort, source_before.agent_effort);
    assert_eq!(fork.title.as_deref(), Some("Source (fork)"));
    assert_eq!(fork.last_cols, Some(120));
    assert_eq!(fork.last_rows, Some(40));
    assert!(fork.agent_session_key.is_some());
    assert_ne!(fork.agent_session_key, source_before.agent_session_key);

    let assigned_key = fork.agent_session_key.as_deref().unwrap();
    let spec = fake.last_spawn_spec().expect("fork should spawn the TUI");
    assert_eq!(spec.command, command);
    assert_eq!(
        &spec.args[..11],
        [
            "--runner-flag",
            "--resume",
            source_key.as_str(),
            "--fork-session",
            "--session-id",
            assigned_key,
            "--model",
            "opus",
            "--effort",
            "max",
            "--settings",
        ]
    );
    let settings: serde_json::Value = serde_json::from_str(&spec.args[11]).unwrap();
    assert_eq!(settings["tui"], "fullscreen");
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(spec.cwd.as_deref(), Some(Path::new("/tmp")));
    assert_eq!(
        spec.env.get("FORK_TEST_ENV").map(String::as_str),
        Some("same-env")
    );
    assert!(!spec.args.contains(&"-p".to_string()));
    assert!(!spec.args.iter().any(|arg| arg.contains("forked from")));
    assert!(!spec.args.iter().any(|arg| arg.contains("Forker persona")));
    assert_eq!(spec.initial_size, Some((120, 40)));
    assert_eq!(fake.spawn_count(), 1);

    let source_after = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();
    assert_eq!(source_after, source_before);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
#[cfg(unix)]
fn codex_direct_chat_fork_captures_headless_key_then_resumes_without_watcher() {
    let pool = pool_with_schema();
    let runner_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let fork_key = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let (_materializer, command, capture_path, codex_home) =
        codex_fork_materializer(&source_key, &fork_key, true);
    {
        let conn = pool.get().unwrap();
        let env_json = serde_json::json!({
            "CODEX_HOME": codex_home,
            "FORK_TEST_ENV": "same-env",
        })
        .to_string();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command, args_json,
                     env_json, working_dir, system_prompt, created_at, updated_at)
                 VALUES (?1, 'codex-forker', 'Codex Forker', 'codex', ?2,
                         '[\"--ask-for-approval\",\"never\",\"--sandbox\",\"workspace-write\"]',
                         ?3, '/tmp', 'Codex persona', ?4, ?4)",
            params![runner_id, command, env_json, now.to_rfc3339()],
        )
        .unwrap();
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.status = crate::model::SessionStatus::Stopped;
        row.runner_id = Some(runner_id);
        row.cwd = Some("/tmp".into());
        row.started_at = Some(now);
        row.agent_session_key = Some(source_key.clone());
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some(command.clone());
        row.agent_model = Some("gpt-5.6-sol".into());
        row.agent_effort = Some("xhigh".into());
        row.title = Some("Codex".into());
        crate::repo::session::insert(&conn, &row).unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let started = Instant::now();
    let spawned = mgr
        .spawn_fork(
            &source_id,
            Some("Codex (fork)".into()),
            None,
            None,
            Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert!(
        started.elapsed() < ci_scaled_budget(Duration::from_secs(4)),
        "Codex fork waited for the materializer reply instead of the rollout file"
    );

    let materialize = std::fs::read_to_string(capture_path).unwrap();
    let expected_cwd = std::fs::canonicalize("/tmp").unwrap();
    assert!(materialize.contains(&format!("cwd={}", expected_cwd.display())));
    assert!(materialize.contains(&format!("arg=exec\narg=fork\narg={source_key}\narg=--json")));
    assert!(materialize.contains("arg=--json\narg=--skip-git-repo-check"));
    assert!(!materialize.contains("arg=-c"));
    assert!(!materialize.contains("gpt-5.6-luna"));
    assert!(materialize.contains("env=same-env"));
    assert!(materialize.contains("arg=This chat was forked from 'Codex'."));
    assert!(!materialize.contains("Codex persona"));
    assert!(!materialize.contains("arg=--ask-for-approval"));
    assert!(!materialize.contains("arg=--sandbox"));

    let spec = fake.last_spawn_spec().expect("resume should spawn the TUI");
    assert_eq!(&spec.args[..2], &["resume", fork_key.as_str()]);
    assert!(!spec.args.contains(&"fork".to_string()));
    assert!(mgr.codex_capture_context(&spawned.id).is_none());
    let fork = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(fork.agent_runtime.as_deref(), Some("codex"));
    assert_eq!(fork.agent_command.as_deref(), Some(command.as_str()));
    assert_eq!(fork.agent_model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(fork.agent_effort.as_deref(), Some("xhigh"));
    assert_eq!(fork.agent_session_key.as_deref(), Some(fork_key.as_str()));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
#[cfg(unix)]
fn fork_materialization_missing_thread_event_removes_row_and_tab() {
    let pool = pool_with_schema();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let (_materializer, command, _capture_path) =
        fork_materializer(r#"{"type":"turn.started"}"#, 0);
    let source_before = {
        let conn = pool.get().unwrap();
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.cwd = Some("/tmp".into());
        row.started_at = Some(Utc::now());
        row.agent_session_key = Some(source_key);
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some(command);
        crate::repo::session::insert(&conn, &row).unwrap();
        row
    };
    let events = Arc::new(RepairingCapture {
        pool: Arc::clone(&pool),
        updated: Mutex::new(Vec::new()),
    });
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let error = mgr
        .spawn_fork(
            &source_id,
            Some("Bad fork".into()),
            None,
            None,
            Path::new("/tmp"),
            Arc::clone(&pool),
            events.clone(),
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("first event is not thread.started"));
    assert_eq!(fake.spawn_count(), 0);

    let updates = events.updated.lock().unwrap();
    assert_eq!(updates.len(), 2);
    let fork_id = &updates[0].session_id;
    assert_eq!(updates[1].session_id, *fork_id);
    assert!(crate::repo::session::get_row(&pool.get().unwrap(), fork_id)
        .unwrap()
        .is_none());
    let nodes = crate::repo::node::list(&pool.get().unwrap()).unwrap();
    assert!(!nodes.iter().any(|node| node
        .layout
        .as_deref()
        .is_some_and(|layout| layout.contains(fork_id))));
    let source_after = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();
    assert_eq!(source_after, source_before);
}

#[test]
#[cfg(unix)]
fn headless_fork_rejects_nonzero_exit_and_kills_timed_out_process_group() {
    use std::os::unix::fs::PermissionsExt;

    let source_key = uuid::Uuid::new_v4().to_string();
    let fork_key = uuid::Uuid::new_v4().to_string();
    let event = format!(r#"{{"type":"thread.started","thread_id":"{fork_key}"}}"#);
    let plan = router::runtime::fork_plan("codex", &source_key, "fork note").unwrap();
    let (_materializer, command, _capture_path) = fork_materializer(&event, 7);
    let codex_home = tempfile::tempdir().unwrap();
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.path().to_string_lossy().into_owned(),
    );
    let spec = SpawnSpec {
        session_id: ulid::Ulid::new().to_string(),
        cwd: Some(PathBuf::from("/tmp")),
        command,
        args: Vec::new(),
        env,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let error = super::spawn::run_headless_fork(&spec, &plan, Duration::from_secs(10)).unwrap_err();
    assert!(error.to_string().contains("exited with"), "{error}");

    let fork_key = uuid::Uuid::new_v4().to_string();
    let (_materializer, command, _capture_path, codex_home) =
        codex_fork_materializer(&source_key, &fork_key, false);
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.to_string_lossy().into_owned(),
    );
    let missing_rollout_spec = SpawnSpec {
        session_id: ulid::Ulid::new().to_string(),
        cwd: Some(PathBuf::from("/tmp")),
        command,
        args: Vec::new(),
        env,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let codex_plan = router::runtime::fork_plan("codex", &source_key, "Source").unwrap();
    let started = Instant::now();
    let error =
        super::spawn::run_headless_fork(&missing_rollout_spec, &codex_plan, Duration::from_secs(3))
            .unwrap_err();
    assert!(error.to_string().contains("rollout for thread"), "{error}");
    assert!(started.elapsed() < ci_scaled_budget(Duration::from_secs(4)));

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("slow-materializer");
    std::fs::write(&command, "#!/bin/sh\nsleep 10\n").unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    let mut timed_spec = spec;
    timed_spec.command = command.to_string_lossy().into_owned();
    let started = Instant::now();
    let error = super::spawn::run_headless_fork(&timed_spec, &plan, Duration::from_millis(250))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < ci_scaled_budget(Duration::from_secs(2)));
}

#[test]
fn fork_refuses_ineligible_source_rows() {
    let pool = pool_with_schema();
    let now = Utc::now();
    let key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('fork-crew', 'Fork Crew', ?1, ?1)",
            params![now.to_rfc3339()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('fork-mission', 'fork-crew', 'Fork mission', 'running', ?1)",
            params![now.to_rfc3339()],
        )
        .unwrap();
        for (id, runtime, agent_key) in [
            ("fork-mission-source", "claude-code", Some(key.as_str())),
            ("fork-archived-source", "claude-code", Some(key.as_str())),
            ("fork-unkeyed-source", "claude-code", None),
            ("fork-trae-source", "trae", Some(key.as_str())),
            ("fork-shell-source", "shell", Some(key.as_str())),
        ] {
            let mut row = crate::repo::session::SessionRowDb::new_running(id.into());
            row.cwd = Some("/tmp".into());
            row.started_at = Some(now);
            row.agent_runtime = Some(runtime.into());
            row.agent_command =
                (!matches!(runtime, "trae" | "shell")).then(|| format!("{runtime}-custom"));
            row.agent_session_key = agent_key.map(str::to_owned);
            if id == "fork-mission-source" {
                row.mission_id = Some("fork-mission".into());
            }
            if id == "fork-archived-source" {
                row.archived_at = Some(now);
            }
            crate::repo::session::insert(&conn, &row).unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    for (source, message) in [
        ("missing-fork-source", "session not found"),
        ("fork-mission-source", "only direct chats"),
        ("fork-archived-source", "is archived"),
        ("fork-unkeyed-source", "no captured agent session key"),
        ("fork-trae-source", "does not support native fork"),
        ("fork-shell-source", "does not support native fork"),
    ] {
        let error = mgr
            .spawn_fork(
                source,
                None,
                None,
                None,
                Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap_err();
        assert!(
            error.to_string().contains(message),
            "unexpected error for {source}: {error}"
        );
    }
    assert_eq!(fake.spawn_count(), 0);
}

#[test]
fn resume_respawns_recorded_override_runtime() {
    // A stopped runner-backed session that recorded an effective
    // runtime must resume on that engine — not the runner row's —
    // with registry defaults instead of the runner's engine flags.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let runner_id = ulid::Ulid::new().to_string();
    let key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'tester', 'T', 'codex', 'codex-custom',
                         '[\"--custom-flag\"]', ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at,
                     agent_session_key, agent_runtime, agent_command)
                 VALUES ('ovr-sid', NULL, ?1, '/tmp', 'stopped', ?2,
                         ?3, 'claude-code', 'claude')",
            params![runner_id, now, key],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.resume(
        "ovr-sid",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("resume should have spawned");
    assert_effective_command(&spec.command, "claude");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()),
        "runner engine flags must not leak into an overridden resume: {:?}",
        spec.args,
    );
    assert!(
        spec.args
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == key),
        "resume must hand the prior agent_session_key to the effective runtime: {:?}",
        spec.args,
    );

    mgr.kill("ovr-sid").unwrap();
}

#[test]
#[cfg(unix)]
fn catalog_default_runner_uses_detected_command_while_custom_command_stays_untouched() {
    use std::os::unix::fs::PermissionsExt;

    let pool = pool_with_schema();
    let bin = tempfile::tempdir().unwrap();
    let detected = bin.path().join("codex");
    std::fs::write(&detected, "#!/bin/sh\n").unwrap();
    let mut permissions = std::fs::metadata(&detected).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&detected, permissions).unwrap();

    let now = Utc::now().to_rfc3339();
    let default_id = ulid::Ulid::new().to_string();
    let custom_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        for (id, handle, command) in [
            (&default_id, "default-runtime", "codex"),
            (&custom_id, "custom-runtime", "codex-wrapper"),
        ] {
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command, created_at, updated_at)
                 VALUES (?1, ?2, ?2, 'codex', ?3, ?4, ?4)",
                params![id, handle, command, now],
            )
            .unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(Some(bin.path().display().to_string()), Arc::clone(&fake));
    let mut default_runner = runner("codex", &[]);
    default_runner.id = default_id;
    default_runner.handle = "default-runtime".into();
    default_runner.runtime = "codex".into();
    let default_session = mgr
        .spawn_direct(
            &default_runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        detected.display().to_string()
    );

    let mut custom_runner = runner("codex-wrapper", &[]);
    custom_runner.id = custom_id;
    custom_runner.handle = "custom-runtime".into();
    custom_runner.runtime = "codex".into();
    let custom_session = mgr
        .spawn_direct(
            &custom_runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(fake.last_spawn_spec().unwrap().command, "codex-wrapper");

    mgr.shell_env.write().unwrap().path = Some("/swapped/bin".into());
    let swapped_session = mgr
        .spawn_direct(
            &custom_runner,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().shell_path.as_deref(),
        Some("/swapped/bin")
    );

    mgr.kill(&default_session.id).unwrap();
    mgr.kill(&custom_session.id).unwrap();
    mgr.kill(&swapped_session.id).unwrap();
}

#[test]
#[cfg(unix)]
fn runtime_only_resume_keeps_live_recorded_path_and_reresolves_dead_path() {
    use std::os::unix::fs::PermissionsExt;

    let pool = pool_with_schema();
    let recorded_dir = tempfile::tempdir().unwrap();
    let detected_dir = tempfile::tempdir().unwrap();
    let make_executable = |path: &Path| {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    };
    let recorded = recorded_dir.path().join("codex-recorded");
    let detected = detected_dir.path().join("codex");
    make_executable(&recorded);
    make_executable(&detected);

    let now = Utc::now().to_rfc3339();
    {
        let conn = pool.get().unwrap();
        for (id, command) in [
            ("runtime-live-path", recorded.display().to_string()),
            ("runtime-dead-path", "/definitely/missing/codex".to_string()),
        ] {
            conn.execute(
                "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command,
                     agent_model, agent_effort)
                 VALUES (?1, 'stopped', ?2, 'codex', ?3,
                         'gpt-5.6-sol', 'max')",
                params![id, now, command],
            )
            .unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(
        Some(detected_dir.path().display().to_string()),
        Arc::clone(&fake),
    );
    mgr.resume(
        "runtime-live-path",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        recorded.display().to_string()
    );
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .windows(2)
        .any(|args| args[0] == "--model" && args[1] == "gpt-5.6-sol"));
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .windows(2)
        .any(|args| args[0] == "-c" && args[1] == "model_reasoning_effort=max"));

    mgr.resume(
        "runtime-dead-path",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        detected.display().to_string()
    );

    mgr.kill("runtime-live-path").unwrap();
    mgr.kill("runtime-dead-path").unwrap();
}

#[cfg(windows)]
#[test]
fn headless_fork_timeout_kills_batch_descendant_and_closes_pipes() {
    if std::env::var_os("RUNNER_FORK_TEST_PID").is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let descendant_pid = dir.path().join("descendant.pid");
    let command = dir.path().join("materializer.cmd");
    std::fs::write(
        &command,
        "@echo off\r\n\"%RUNNER_FORK_TEST_EXE%\" --exact session::manager::tests::headless_fork_descendant --nocapture >nul\r\n",
    ).unwrap();
    let spec = SpawnSpec {
        session_id: ulid::Ulid::new().to_string(),
        command: command.to_string_lossy().into_owned(),
        args: Vec::new(),
        env: BTreeMap::from([
            (
                "CODEX_HOME".into(),
                dir.path().to_string_lossy().into_owned(),
            ),
            (
                "RUNNER_FORK_TEST_PID".into(),
                descendant_pid.to_string_lossy().into_owned(),
            ),
            (
                "RUNNER_FORK_TEST_EXE".into(),
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]),
        cwd: Some(dir.path().to_path_buf()),
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let plan = router::runtime::ForkPlan::Headless {
        args: Vec::new(),
        source_key: uuid::Uuid::new_v4().to_string(),
    };
    let started = Instant::now();
    let error = super::spawn::run_headless_fork(&spec, &plan, Duration::from_secs(5)).unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(15));
    let pid: i32 = std::fs::read_to_string(descendant_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!crate::session::process::process_exists(pid));
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
fn windows_batch_first_turn_is_pasted_after_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let batch = dir.path().join("prompt reader.cmd");
    std::fs::write(&batch,
        "@echo off\r\n\"%RUNNER_BATCH_PROMPT_EXE%\" --exact session::manager::tests::windows_batch_prompt_probe --nocapture\r\n").unwrap();
    let mut runner = runner(batch.to_str().unwrap(), &[]);
    runner.runtime = "claude-code".into();
    runner.env.insert(
        "RUNNER_BATCH_PROMPT_EXE".into(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    let pool = pool_with_schema();
    insert_crew_runner(&pool, "batch-prompt", &runner.id);
    let events = capture();
    let mgr = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned = mgr
        .spawn_direct(
            &runner,
            None,
            None,
            None,
            None,
            Some(dir.path().to_str().unwrap()),
            None,
            None,
            dir.path(),
            Arc::clone(&pool),
            events.clone(),
            Some("first line\nsecond line".into()),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let output = loop {
        let bytes = events
            .output
            .lock()
            .unwrap()
            .iter()
            .flat_map(|event| event.bytes.iter().copied())
            .collect::<Vec<_>>();
        let output = String::from_utf8_lossy(&bytes).into_owned();
        if output.contains("BATCH_INPUT_FIRST=first line")
            && output.contains("BATCH_INPUT_SECOND=second line")
            || Instant::now() >= deadline
        {
            break output;
        }
        thread::sleep(Duration::from_millis(20));
    };
    mgr.kill(&spawned.id).unwrap();
    assert!(output.contains("BATCH_INPUT_FIRST=first line"), "{output}");
    assert!(
        output.contains("BATCH_INPUT_SECOND=second line"),
        "{output}"
    );
}

#[cfg(windows)]
#[test]
fn windows_batch_prompt_probe() {
    if std::env::var_os("RUNNER_BATCH_PROMPT_EXE").is_none() {
        return;
    }
    // Cooked console reads consume LF; agent TUIs read raw input instead.
    let expected = b"first line\nsecond line\r";
    let bytes = crate::session::process::read_raw_console_input(expected.len()).unwrap();
    assert_eq!(bytes, expected);
    let (first, second) = std::str::from_utf8(&bytes)
        .unwrap()
        .split_once('\n')
        .unwrap();
    println!("BATCH_INPUT_FIRST={first}");
    println!(
        "BATCH_INPUT_SECOND={}",
        second.trim_end_matches(['\r', '\n'])
    );
}

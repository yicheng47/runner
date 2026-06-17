// In-process `SessionRuntime` implementation over `portable-pty`.
// Replaces the tmux runtime in `session::tmux_runtime` per impl 0011.
//
// One PtyRuntime instance owns a HashMap of session_id → SessionHandle.
// Per session: PTY master fd, child handle, writer mutex, killer
// handle, and a small slab of atomics for resize / exit bookkeeping.
//
// Reader threads pump raw PTY bytes into the existing
// `RuntimeOutput::Stream` channel that `SessionManager`'s forwarder
// consumes. There is no host-side terminal model; xterm.js on the
// frontend is the only emulator. See plan §"Why no headless emulator"
// for the trade-offs.
//
// `resume(...)` is intentionally defensive: under the in-process model
// the manager's `reattach_running_sessions` is short-circuited at app
// start (DB cleanup demotes any prior `running` rows to `stopped`), so
// this trait method is never reached on the live path. If it does fire,
// erroring loud is better than returning a half-initialized stream.
//
// The `#[cfg(unix)]` gate is applied at the parent `session/mod.rs`
// when this module is registered, so no inner attribute is needed
// here.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

use super::launch;
use super::runtime::{
    OutputStream, RunnerStatus, RuntimeError, RuntimeOutput, RuntimeResult, RuntimeSession,
    SessionRuntime, SessionStatus, SpawnSpec,
};

const RUNTIME_LABEL: &str = "native-pty";
const READ_BUF: usize = 8 * 1024;
const DEFAULT_IDLE_THRESHOLD: Duration = Duration::from_millis(750);
const IDLE_MONITOR_POLL: Duration = Duration::from_millis(50);

/// Public constructor. Holds no external state — the runtime is purely
/// in-memory and the per-session resources tear down with their handles.
pub struct PtyRuntime {
    sessions: Mutex<HashMap<String, Arc<SessionHandle>>>,
}

impl PtyRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for PtyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

struct SessionHandle {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// `Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>` — the reader
    /// thread `try_wait`s on this once it observes EOF so the manager
    /// gets a real exit code. `Option` so we can `.take()` the value
    /// out for the final `wait()` without leaving a half-locked entry.
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    /// Last observed exit code. Reader thread writes after `try_wait`
    /// returns `Some(status)`. `i32::MIN` is the "unset" sentinel —
    /// real exit codes fit in `u8`-ish, and `try_wait`'s `None` ==
    /// "still alive" maps to the sentinel staying put.
    exit_code: AtomicI32,
    /// `false` once the reader thread breaks. `status()` reads this
    /// for the trait's `SessionStatus.alive`.
    alive: AtomicBool,
    pid: Option<i32>,
    command: String,
}

const EXIT_UNSET: i32 = i32::MIN;

impl SessionRuntime for PtyRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        // Compose PATH the same way the tmux runtime does so direct-chat
        // sessions get identical shell-resolution behavior across the
        // cutover. `launch::compose_path` is the canonical place for
        // shim_dir / bundled_bin_dir / shell_path / HOME / inherited
        // PATH precedence rules.
        let inherited_path = std::env::var("PATH").ok();
        let home_path: Option<PathBuf> = std::env::var_os("HOME").map(PathBuf::from);
        let composed_path = launch::compose_path(
            spec.shim_dir.as_deref(),
            spec.bundled_bin_dir.as_deref(),
            spec.shell_path.as_deref(),
            home_path.as_deref(),
            inherited_path.as_deref(),
        );

        let (cols, rows) = spec.initial_size.unwrap_or((80, 24));
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| RuntimeError::Msg(format!("openpty: {e}")))?;

        // Build the child command. portable-pty wraps the std
        // CommandBuilder, so env / cwd / args go through the same APIs
        // we'd use for std::process::Command — minus the inherited-env
        // surprises (CommandBuilder clears inherited env unless we
        // explicitly env_clear()).
        let mut cmd = CommandBuilder::new(&spec.command);
        cmd.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }

        // Reserved env names (PATH) come from the composed result, not
        // from spec.env — same precedence as the tmux launch script.
        for (k, v) in &spec.env {
            if launch::is_reserved_env_name(k) {
                continue;
            }
            if !launch::is_valid_env_name(k) {
                return Err(RuntimeError::Msg(format!(
                    "invalid env var name {k:?}: must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
            cmd.env(k, v);
        }
        cmd.env("PATH", &composed_path);
        // COLUMNS/LINES so Node-based TUIs pick up the initial grid
        // before SIGWINCH lands — same hint the tmux launcher injects.
        cmd.env("COLUMNS", cols.to_string());
        cmd.env("LINES", rows.to_string());

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| RuntimeError::Msg(format!("spawn_command: {e}")))?;
        // Drop the slave handle: the child holds its own end of the
        // PTY, and keeping the parent-side slave open would prevent us
        // from observing EOF on the master when the child exits.
        drop(pair.slave);

        let pid = child.process_id().map(|p| p as i32);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| RuntimeError::Msg(format!("try_clone_reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| RuntimeError::Msg(format!("take_writer: {e}")))?;
        let killer = child.clone_killer();

        let (tx, rx) = mpsc::channel::<RuntimeOutput>();
        let stop = Arc::new(AtomicBool::new(false));

        let child_slot: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>> =
            Arc::new(Mutex::new(Some(child)));

        let handle = Arc::new(SessionHandle {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            child: Arc::clone(&child_slot),
            exit_code: AtomicI32::new(EXIT_UNSET),
            alive: AtomicBool::new(true),
            pid,
            command: format_command_summary(&spec.command, &spec.args),
        });

        self.sessions
            .lock()
            .expect("PtyRuntime.sessions poisoned")
            .insert(spec.session_id.clone(), Arc::clone(&handle));

        let stop_for_reader = Arc::clone(&stop);
        let handle_for_reader = Arc::clone(&handle);
        let session_id_for_reader = spec.session_id.clone();
        thread::Builder::new()
            .name(format!("pty-reader-{}", spec.session_id))
            .spawn(move || {
                reader_thread(
                    reader,
                    tx,
                    stop_for_reader,
                    handle_for_reader,
                    session_id_for_reader,
                );
            })
            .map_err(|e| RuntimeError::Msg(format!("spawn reader thread: {e}")))?;

        let rt_session = RuntimeSession {
            runtime: RUNTIME_LABEL.to_string(),
            socket: String::new(),
            session_name: spec.session_id.clone(),
            window: "main".to_string(),
            pane: spec.session_id,
        };
        let stream = OutputStream::new(rx, stop);
        Ok((rt_session, stream))
    }

    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream> {
        // Under the in-process model the manager's reattach pass is
        // short-circuited at startup, so this is only reachable via a
        // legacy code path. Failing loud beats handing back a stream
        // that drops the bytes a still-alive session is producing.
        Err(RuntimeError::Msg(format!(
            "pty runtime: resume() is not used in-process; \
             session {} should be respawned via SessionManager::resume",
            session.session_name
        )))
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        let handle = lookup(self, &session.session_name)?;
        let mut killer = handle.killer.lock().expect("killer poisoned");
        killer
            .kill()
            .map_err(|e| RuntimeError::Msg(format!("ChildKiller::kill: {e}")))?;
        Ok(())
    }

    fn paste(&self, session: &RuntimeSession, payload: &[u8]) -> RuntimeResult<()> {
        // xterm.js handles bracketed-paste wrapping on the user-input
        // path. Rust-side callers (manager's `inject_paste`) that want
        // bracketing also prefix it themselves before calling — see
        // `manager::inject_paste`. The runtime just writes the bytes.
        write_to(self, &session.session_name, payload)
    }

    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        write_to(self, &session.session_name, bytes)
    }

    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        let bytes = translate_key(key)?;
        write_to(self, &session.session_name, &bytes)
    }

    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()> {
        let handle = lookup(self, &session.session_name)?;
        let master = handle.master.lock().expect("master poisoned");
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| RuntimeError::Msg(format!("MasterPty::resize: {e}")))?;
        Ok(())
    }

    fn status(&self, session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        let handle = match self
            .sessions
            .lock()
            .expect("PtyRuntime.sessions poisoned")
            .get(&session.session_name)
        {
            Some(h) => Arc::clone(h),
            None => return Ok(None),
        };
        let exit_code = match handle.exit_code.load(Ordering::Acquire) {
            EXIT_UNSET => None,
            v => Some(v),
        };
        Ok(Some(SessionStatus {
            alive: handle.alive.load(Ordering::Acquire),
            exit_code,
            pid: handle.pid,
            command: Some(handle.command.clone()),
        }))
    }
}

// --- Reader thread ------------------------------------------------------

struct IdleDetector {
    last_byte: Instant,
    current: RunnerStatus,
    threshold: Duration,
}

impl IdleDetector {
    fn new(threshold: Duration) -> Self {
        Self::new_at(threshold, Instant::now())
    }

    fn new_at(threshold: Duration, now: Instant) -> Self {
        Self {
            last_byte: now,
            current: RunnerStatus::Busy,
            threshold,
        }
    }

    fn on_bytes(&mut self, n: usize) -> Option<RunnerStatus> {
        self.on_bytes_at(n, Instant::now())
    }

    fn on_bytes_at(&mut self, n: usize, now: Instant) -> Option<RunnerStatus> {
        if n == 0 {
            return None;
        }
        self.last_byte = now;
        if self.current == RunnerStatus::Idle {
            self.current = RunnerStatus::Busy;
            Some(RunnerStatus::Busy)
        } else {
            None
        }
    }

    fn tick(&mut self) -> Option<RunnerStatus> {
        self.tick_at(Instant::now())
    }

    fn tick_at(&mut self, now: Instant) -> Option<RunnerStatus> {
        if self.current == RunnerStatus::Busy
            && now.duration_since(self.last_byte) >= self.threshold
        {
            self.current = RunnerStatus::Idle;
            Some(RunnerStatus::Idle)
        } else {
            None
        }
    }
}

fn idle_monitor_thread(
    detector: Arc<Mutex<IdleDetector>>,
    tx: mpsc::Sender<RuntimeOutput>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::Acquire) || done.load(Ordering::Acquire) {
            break;
        }
        let transition = {
            let mut detector = detector.lock().expect("idle detector poisoned");
            detector.tick()
        };
        if let Some(state) = transition {
            if tx
                .send(RuntimeOutput::StatusTransition {
                    state,
                    source: "forwarder",
                })
                .is_err()
            {
                break;
            }
        }
        thread::sleep(IDLE_MONITOR_POLL);
    }
}

fn reader_thread(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<RuntimeOutput>,
    stop: Arc<AtomicBool>,
    handle: Arc<SessionHandle>,
    session_id: String,
) {
    let detector = Arc::new(Mutex::new(IdleDetector::new(DEFAULT_IDLE_THRESHOLD)));
    let monitor_done = Arc::new(AtomicBool::new(false));
    let monitor = thread::Builder::new()
        .name(format!("pty-idle-{session_id}"))
        .spawn({
            let detector = Arc::clone(&detector);
            let tx = tx.clone();
            let stop = Arc::clone(&stop);
            let done = Arc::clone(&monitor_done);
            move || idle_monitor_thread(detector, tx, stop, done)
        });
    let monitor = match monitor {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("spawn idle monitor thread for {session_id}: {e}");
            None
        }
    };

    let mut buf = vec![0u8; READ_BUF];
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let transition = {
                    let mut detector = detector.lock().expect("idle detector poisoned");
                    detector.on_bytes(n)
                };
                if let Some(state) = transition {
                    if tx
                        .send(RuntimeOutput::StatusTransition {
                            state,
                            source: "forwarder",
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                if tx.send(RuntimeOutput::Stream(buf[..n].to_vec())).is_err() {
                    // Receiver dropped (manager's forwarder gone).
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    monitor_done.store(true, Ordering::Release);
    if let Some(monitor) = monitor {
        let _ = monitor.join();
    }

    handle.alive.store(false, Ordering::Release);

    // Reap the child to capture its exit code. Take the value out so
    // the std::process::Child equivalent is consumed and not double-
    // waited later.
    let mut child_slot = handle.child.lock().expect("child slot poisoned");
    if let Some(mut child) = child_slot.take() {
        match child.wait() {
            Ok(status) => {
                let code = status
                    .exit_code()
                    .try_into()
                    .unwrap_or(EXIT_UNSET.saturating_add(1));
                handle.exit_code.store(code, Ordering::Release);
            }
            Err(_) => {
                // wait() failure leaves exit_code as EXIT_UNSET;
                // status() reports None and the manager treats it as
                // "crashed".
            }
        }
    }
    // tx dropped on scope exit → OutputStream::recv_timeout sees
    // Disconnected on the manager side, which is how the existing
    // forwarder thread knows to wind down.
}

// --- Helpers ------------------------------------------------------------

fn lookup(runtime: &PtyRuntime, session_id: &str) -> RuntimeResult<Arc<SessionHandle>> {
    runtime
        .sessions
        .lock()
        .expect("PtyRuntime.sessions poisoned")
        .get(session_id)
        .cloned()
        .ok_or_else(|| RuntimeError::Msg(format!("unknown session: {session_id}")))
}

fn write_to(runtime: &PtyRuntime, session_id: &str, bytes: &[u8]) -> RuntimeResult<()> {
    let handle = lookup(runtime, session_id)?;
    let mut writer = handle.writer.lock().expect("writer poisoned");
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

/// Map a symbolic key name to the byte sequence the child PTY expects.
/// Conservative table — covers the only production caller (`"Enter"`
/// from manager.rs) plus the doc-string examples on `SessionRuntime::send_key`
/// (`"Escape"`, `"C-c"`, `"Up"`). xterm.js translates user typing
/// client-side before sending it through `send_bytes`, so this isn't
/// the place to keep the full `KeyboardEvent.key` space.
fn translate_key(key: &str) -> RuntimeResult<Vec<u8>> {
    // C-<letter> chord: Ctrl + letter → control byte.
    if let Some(rest) = key.strip_prefix("C-") {
        if rest.len() == 1 {
            let ch = rest.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                let upper = ch.to_ascii_uppercase() as u8;
                let ctrl = upper - b'A' + 1;
                return Ok(vec![ctrl]);
            }
        }
        return Err(RuntimeError::Msg(format!("unsupported chord: {key:?}")));
    }
    let bytes: &[u8] = match key {
        "Enter" => b"\r",
        "Escape" => b"\x1b",
        "Tab" => b"\t",
        "BSpace" | "Backspace" => b"\x7f",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" => b"\x1b[5~",
        "PageDown" => b"\x1b[6~",
        "Space" => b" ",
        other => {
            return Err(RuntimeError::Msg(format!(
                "unsupported key name: {other:?}"
            )));
        }
    };
    Ok(bytes.to_vec())
}

fn format_command_summary(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

// --- Public lifecycle helpers used by lib.rs / manager startup --------

/// Demote any rows still marked `running` in the DB to `stopped`. Run
/// once at app startup before `SessionManager` accepts work, so the
/// sidebar reflects "agent died with prior app process" reality
/// without trying to reattach to a PTY that doesn't exist anymore.
///
/// Matches the legacy reattach pass's behavior on a dead pane: status
/// flips to `stopped`, `stopped_at` records the wall-clock at cleanup
/// time, `pid` is left as the prior value for diagnostics. Mission
/// rows are demoted alongside direct chats — the router will need
/// re-mount on next user interaction, same as today's tmux flow when
/// reattach fails.
pub fn cleanup_stale_running_rows_on_startup(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> rusqlite::Result<usize> {
    let conn = pool
        .get()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let now = chrono::Utc::now().to_rfc3339();
    let updated = conn.execute(
        "UPDATE sessions
            SET status = 'stopped',
                stopped_at = COALESCE(stopped_at, ?1)
            WHERE status = 'running'",
        rusqlite::params![now],
    )?;
    if updated > 0 {
        log::info!(
            "pty-runtime startup cleanup: demoted {updated} stale running session(s) to stopped"
        );
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(session_id: &str, command: &str, args: &[&str]) -> SpawnSpec {
        let env: BTreeMap<String, String> = BTreeMap::new();
        SpawnSpec {
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
            Some(RunnerStatus::Idle)
        );
        assert_eq!(
            detector.tick_at(start + threshold + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn idle_detector_wakes_on_first_byte_after_idle() {
        let threshold = Duration::from_millis(750);
        let start = Instant::now();
        let mut detector = IdleDetector::new_at(threshold, start);

        assert_eq!(
            detector.tick_at(start + threshold),
            Some(RunnerStatus::Idle)
        );
        assert_eq!(
            detector.on_bytes_at(1, start + threshold + Duration::from_millis(1)),
            Some(RunnerStatus::Busy)
        );
        assert_eq!(
            detector.on_bytes_at(1, start + threshold + Duration::from_millis(2)),
            None
        );
    }

    #[test]
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
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(RuntimeOutput::Stream(bytes)) => collected.extend_from_slice(&bytes),
                Ok(RuntimeOutput::Replay(_)) => panic!("pty runtime must not emit Replay"),
                Ok(RuntimeOutput::StatusTransition { .. }) => {}
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
    fn spawn_emits_idle_after_silence_and_busy_on_more_output() {
        let rt = PtyRuntime::new();
        let (sess, stream) = rt
            .spawn(spec(
                "test-idle-detector",
                "/bin/sh",
                &["-c", "printf first; sleep 1; printf second; sleep 0.1"],
            ))
            .unwrap();

        let mut statuses = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            match stream.recv_timeout(Duration::from_millis(100)) {
                Ok(RuntimeOutput::StatusTransition { state, source }) => {
                    assert_eq!(source, "forwarder");
                    statuses.push(state);
                    if statuses
                        .windows(2)
                        .any(|w| w == [RunnerStatus::Idle, RunnerStatus::Busy])
                    {
                        break;
                    }
                }
                Ok(RuntimeOutput::Stream(_)) => {}
                Ok(RuntimeOutput::Replay(_)) => panic!("pty runtime must not emit Replay"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            statuses
                .windows(2)
                .any(|w| w == [RunnerStatus::Idle, RunnerStatus::Busy]),
            "expected idle then busy transition, got {statuses:?}"
        );
        let _ = rt.stop(&sess);
    }

    #[test]
    fn spawn_exit_seven_records_exit_code() {
        let rt = PtyRuntime::new();
        let (sess, stream) = rt
            .spawn(spec("test-exit", "/bin/sh", &["-c", "exit 7"]))
            .unwrap();
        // Drain until EOF — the reader thread breaks on EOF and the
        // sender drops, so recv eventually returns Disconnected.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match stream.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
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
            socket: String::new(),
            session_name: "nonexistent".into(),
            window: "main".into(),
            pane: "nonexistent".into(),
        };
        assert!(rt.status(&phantom).unwrap().is_none());
    }

    #[test]
    fn resize_succeeds_on_live_session() {
        let rt = PtyRuntime::new();
        let (sess, _stream) = rt
            .spawn(spec("test-resize", "/bin/sh", &["-c", "sleep 5"]))
            .unwrap();
        rt.resize(&sess, 120, 40).expect("resize should succeed");
        rt.stop(&sess).unwrap();
    }

    #[test]
    fn resume_returns_error_under_pty_runtime() {
        let rt = PtyRuntime::new();
        let phantom = RuntimeSession {
            runtime: RUNTIME_LABEL.into(),
            socket: String::new(),
            session_name: "phantom".into(),
            window: "main".into(),
            pane: "phantom".into(),
        };
        assert!(matches!(rt.resume(&phantom), Err(RuntimeError::Msg(_))));
    }
}

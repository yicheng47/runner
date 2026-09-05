// In-process `SessionRuntime` implementation over `portable-pty`.
//
// One PtyRuntime instance owns a HashMap of session_id → SessionHandle.
// Per session: PTY master fd, child handle, writer mutex, killer
// handle, and a small slab of atomics for resize / exit bookkeeping.
//
// Reader threads pump raw PTY bytes into the existing
// `RuntimeOutput::Stream` channel that `SessionManager`'s forwarder
// consumes. The GPUI frontend owns the only terminal model; the backend
// does not run a second headless emulator.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, ExitStatus, MasterPty, PtySize};

#[cfg(unix)]
use portable_pty::ChildKiller;

use super::launch;
use super::process::{kill_process, process_exists, ProcessTree};
#[cfg(unix)]
use super::process::{process_command_line, reap_descendants, wait_for_process_exit_until};
use super::runtime::{
    OutputStream, RunnerStatus, RuntimeError, RuntimeOutput, RuntimeResult, RuntimeSession,
    SessionRuntime, SessionStatus, SpawnSpec,
};

const RUNTIME_LABEL: &str = "native-pty";
// Typical ConPTY redraws can fit in one read; the forwarder coalesces larger bursts.
const READ_BUF: usize = 64 * 1024;
/// Silence before a session reads as Idle. The agent TUIs animate a spinner
/// while a turn runs, so genuine work rarely goes quiet; the 2 s window is
/// for the moments it does (a tool call that pauses the spinner), because
/// the router treats Idle as a turn boundary and delivers into it.
const DEFAULT_IDLE_THRESHOLD: Duration = Duration::from_secs(2);
const IDLE_MONITOR_POLL: Duration = Duration::from_millis(50);
/// How long a runtime gets to exit on its own after SIGHUP before its
/// process group is SIGKILLed. Sized to claude-code's own shutdown failsafe
/// (SIGTERM to each tool shell, SIGKILL backstop at 1.5 s, SessionEnd hooks,
/// force-exit at 5 s) so a stop never cuts that cleanup short; codex has no
/// SIGHUP handler and exits at once, so the window costs nothing there.
#[cfg(all(unix, not(test)))]
const HUP_GRACE: Duration = Duration::from_secs(6);
#[cfg(all(unix, test))]
const HUP_GRACE: Duration = Duration::from_millis(250);
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_millis(250);
#[cfg(windows)]
const WINDOWS_KILL_GRACE: Duration = Duration::from_secs(2);
pub(super) const STOP_POLL: Duration = Duration::from_millis(10);
#[cfg(unix)]
const ORPHAN_SWEEP_CONFIRM: Duration = Duration::from_secs(1);
/// Window right after a `resize` (SIGWINCH) during which repaint bytes
/// from the child's TUI are not treated as fresh activity. Without this,
/// resizing the window while a session is idle flips it to Busy for a
/// full idle threshold — the agent redraws its prompt, and silence-based
/// detection can't tell a repaint from real output. Kept under
/// `DEFAULT_IDLE_THRESHOLD` so genuine work started right after a resize
/// still surfaces promptly.
const RESIZE_GRACE: Duration = Duration::from_millis(500);
// PTYs can boot before their GPUI terminal pane is ready to answer startup
// probes. Answer them here so Codex does not cache fallback colors first.
const TERMINAL_QUERY_STARTUP_BUDGET: usize = 8 * 1024;
const TERMINAL_QUERY_TAIL: usize = 16;
const DEFAULT_OSC10_FG_REPLY: &[u8] = b"\x1b]10;rgb:dcdc/dcdc/e0e0\x1b\\";
const DEFAULT_OSC11_BG_REPLY: &[u8] = b"\x1b]11;rgb:1515/1616/1b1b\x1b\\";
const DSR_CURSOR_POS_REPLY: &[u8] = b"\x1b[1;1R";
const DA1_XTERM_REPLY: &[u8] = b"\x1b[?1;2c";

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
    #[cfg(unix)]
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
    /// Timestamp of the last `resize` call. The reader thread consults it
    /// to ignore SIGWINCH repaint bursts for `RESIZE_GRACE` (see
    /// `IdleDetector`), so resizing an idle session doesn't read as Busy.
    last_resize: Mutex<Option<Instant>>,
    pid: Option<i32>,
    process_tree: Option<ProcessTree>,
    command: String,
}

const EXIT_UNSET: i32 = i32::MIN;

impl SessionRuntime for PtyRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        // `launch::compose_path` is the canonical place for shim_dir /
        // bundled_bin_dir / shell_path / HOME / inherited PATH
        // precedence rules.
        let inherited_path = std::env::var("PATH").ok();
        let home_path: Option<PathBuf> = runner_core::app_paths::home_dir();
        let composed_path = launch::compose_path(
            spec.shim_dir.as_deref(),
            spec.bundled_bin_dir.as_deref(),
            spec.shell_path.as_deref(),
            home_path.as_deref(),
            inherited_path.as_deref(),
        );

        let (cols, rows) = spec
            .initial_size
            .expect("SessionManager must resolve the initial PTY size");
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
        // from spec.env.
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
        // before SIGWINCH lands.
        cmd.env("COLUMNS", cols.to_string());
        cmd.env("LINES", rows.to_string());

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| RuntimeError::Msg(format!("spawn_command: {e}")))?;
        // Drop the slave handle: the child holds its own end of the
        // PTY, and keeping the parent-side slave open would prevent us
        // from observing EOF on the master when the child exits.
        drop(pair.slave);

        let pid = child.process_id().map(|p| p as i32);
        let mut process_tree = match child.process_id().map(ProcessTree::adopt).transpose() {
            Ok(tree) => tree,
            Err(error) => {
                #[cfg(unix)]
                let _ = child.kill();
                #[cfg(windows)]
                if let Some(pid) = pid {
                    kill_process(pid).map_err(|kill_error| {
                        RuntimeError::Msg(format!(
                            "adopt process tree: {error}; terminate child: {kill_error}"
                        ))
                    })?;
                }
                let _ = child.wait();
                return Err(RuntimeError::Msg(format!("adopt process tree: {error}")));
            }
        };
        if let Some(tree) = process_tree.as_mut() {
            tree.attach_pty(pair.master.as_ref());
        }
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| RuntimeError::Msg(format!("try_clone_reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| RuntimeError::Msg(format!("take_writer: {e}")))?;
        #[cfg(unix)]
        let killer = child.clone_killer();

        let (tx, rx) = mpsc::channel::<RuntimeOutput>();
        let stop = Arc::new(AtomicBool::new(false));

        let child_slot: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>> =
            Arc::new(Mutex::new(Some(child)));

        let handle = Arc::new(SessionHandle {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            #[cfg(unix)]
            killer: Mutex::new(killer),
            child: Arc::clone(&child_slot),
            exit_code: AtomicI32::new(EXIT_UNSET),
            alive: AtomicBool::new(true),
            last_resize: Mutex::new(None),
            pid,
            process_tree,
            command: format_command_summary(&spec.command, &spec.args),
        });

        self.sessions
            .lock()
            .expect("PtyRuntime.sessions poisoned")
            .insert(spec.session_id.clone(), Arc::clone(&handle));

        let stop_for_reader = Arc::clone(&stop);
        let handle_for_reader = Arc::clone(&handle);
        let session_id_for_reader = spec.session_id.clone();
        let query_responder = TerminalQueryResponder::default();
        thread::Builder::new()
            .name(format!("pty-reader-{}", spec.session_id))
            .spawn(move || {
                reader_thread(
                    reader,
                    tx,
                    stop_for_reader,
                    handle_for_reader,
                    session_id_for_reader,
                    query_responder,
                );
            })
            .map_err(|e| RuntimeError::Msg(format!("spawn reader thread: {e}")))?;

        let rt_session = RuntimeSession {
            runtime: RUNTIME_LABEL.to_string(),
            session_id: spec.session_id,
        };
        let stream = OutputStream::new(rx, stop);
        Ok((rt_session, stream))
    }

    #[cfg(unix)]
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        let handle = lookup(self, &session.session_id)?;
        let mut killer = handle.killer.lock().expect("killer poisoned");
        let child = handle.child.lock().expect("child slot poisoned").take();
        // Snapshot before SIGHUP: once the agent is gone its children are
        // reparented to launchd and nothing else can tell they were its.
        let descendants: Vec<i32> = handle
            .process_tree
            .as_ref()
            .map(ProcessTree::snapshot_descendants)
            .unwrap_or_default()
            .into_iter()
            .map(|pid| pid as i32)
            .collect();

        let result = match child {
            Some(mut child) => {
                let result = stop_and_reap_child(
                    &session.session_id,
                    handle.process_tree.as_ref(),
                    killer.as_mut(),
                    child.as_mut(),
                );
                match result {
                    Ok(status) => {
                        record_exit_status(&handle, status);
                        Ok(())
                    }
                    Err(error) => {
                        handle
                            .child
                            .lock()
                            .expect("child slot poisoned")
                            .replace(child);
                        Err(error)
                    }
                }
            }
            None => stop_child_owned_by_reader(
                &session.session_id,
                handle.pid,
                killer.as_mut(),
                &handle,
            ),
        };
        if result.is_ok() {
            reap_descendants(&session.session_id, &descendants);
        }
        result
    }

    #[cfg(windows)]
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        let handle = lookup(self, &session.session_id)?;
        let child = handle.child.lock().expect("child slot poisoned").take();
        match child {
            Some(mut child) => {
                match stop_and_reap_child(
                    &session.session_id,
                    handle.process_tree.as_ref(),
                    child.as_mut(),
                ) {
                    Ok(status) => {
                        record_exit_status(&handle, status);
                        Ok(())
                    }
                    Err(error) => {
                        handle
                            .child
                            .lock()
                            .expect("child slot poisoned")
                            .replace(child);
                        Err(error)
                    }
                }
            }
            None => stop_child_owned_by_reader(&session.session_id, &handle),
        }
    }

    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        write_to(self, &session.session_id, bytes)
    }

    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        let bytes = translate_key(key)?;
        write_to(self, &session.session_id, &bytes)
    }

    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()> {
        let handle = lookup(self, &session.session_id)?;
        {
            let master = handle.master.lock().expect("master poisoned");
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| RuntimeError::Msg(format!("MasterPty::resize: {e}")))?;
        }
        // Open the grace window only after the SIGWINCH is delivered, so the
        // repaint burst it triggers doesn't read as fresh activity.
        *handle.last_resize.lock().expect("last_resize poisoned") = Some(Instant::now());
        Ok(())
    }

    fn status(&self, session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        let handle = match self
            .sessions
            .lock()
            .expect("PtyRuntime.sessions poisoned")
            .get(&session.session_id)
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

    fn has_foreground_process(&self, session: &RuntimeSession) -> RuntimeResult<Option<bool>> {
        let handle = match self
            .sessions
            .lock()
            .expect("PtyRuntime.sessions poisoned")
            .get(&session.session_id)
        {
            Some(handle) => Arc::clone(handle),
            None => return Ok(Some(false)),
        };
        // Keep foreground queries serialized with other PTY master operations.
        let _master = handle.master.lock().expect("SessionHandle.master poisoned");
        Ok(handle
            .process_tree
            .as_ref()
            .and_then(|tree| tree.has_other_processes().ok()))
    }
}

#[cfg(unix)]
fn stop_and_reap_child(
    session_id: &str,
    process_tree: Option<&ProcessTree>,
    killer: &mut dyn ChildKiller,
    child: &mut dyn Child,
) -> RuntimeResult<ExitStatus> {
    if poll_until(STOP_POLL, Duration::ZERO, || {
        child
            .try_wait()
            .map_err(|e| RuntimeError::Msg(format!("try_wait {session_id}: {e}")))
    })?
    .is_some()
    {
        return child
            .wait()
            .map_err(|e| RuntimeError::Msg(format!("wait {session_id}: {e}")));
    }

    let hup_sent = Instant::now();
    let hup_error = killer.kill().err();
    if poll_until(STOP_POLL, HUP_GRACE, || {
        child
            .try_wait()
            .map_err(|e| RuntimeError::Msg(format!("try_wait {session_id}: {e}")))
    })?
    .is_some()
    {
        log::info!(
            "session {session_id} exited {:?} after SIGHUP",
            hup_sent.elapsed()
        );
        return child
            .wait()
            .map_err(|e| RuntimeError::Msg(format!("wait {session_id}: {e}")));
    }

    let process_tree = process_tree.ok_or_else(|| {
        RuntimeError::Msg(format!(
            "session {session_id} survived SIGHUP but has no pid for SIGKILL"
        ))
    })?;
    log::warn!(
        "session {session_id} survived SIGHUP for {HUP_GRACE:?}; SIGKILL to its process group"
    );
    let kill_error = process_tree.terminate().err();
    if poll_until(STOP_POLL, KILL_GRACE, || {
        child
            .try_wait()
            .map_err(|e| RuntimeError::Msg(format!("try_wait {session_id}: {e}")))
    })?
    .is_some()
    {
        return child
            .wait()
            .map_err(|e| RuntimeError::Msg(format!("wait {session_id}: {e}")));
    }

    Err(RuntimeError::Msg(format!(
        "session {session_id} is still alive after SIGHUP and SIGKILL{}{}",
        hup_error
            .map(|e| format!("; SIGHUP error: {e}"))
            .unwrap_or_default(),
        kill_error
            .map(|e| format!("; SIGKILL error: {e}"))
            .unwrap_or_default(),
    )))
}

#[cfg(unix)]
fn stop_child_owned_by_reader(
    session_id: &str,
    pid: Option<i32>,
    killer: &mut dyn ChildKiller,
    handle: &SessionHandle,
) -> RuntimeResult<()> {
    if poll_until(STOP_POLL, Duration::ZERO, || {
        Ok(reader_reaped_child(handle, pid).then_some(()))
    })?
    .is_some()
    {
        return Ok(());
    }

    let hup_sent = Instant::now();
    let hup_error = killer.kill().err();
    if poll_until(STOP_POLL, HUP_GRACE, || {
        Ok(reader_reaped_child(handle, pid).then_some(()))
    })?
    .is_some()
    {
        log::info!(
            "session {session_id} exited {:?} after SIGHUP",
            hup_sent.elapsed()
        );
        return Ok(());
    }

    let pid = pid.ok_or_else(|| {
        RuntimeError::Msg(format!(
            "session {session_id} has no child handle or pid and was not reaped"
        ))
    })?;
    log::warn!(
        "session {session_id} survived SIGHUP for {HUP_GRACE:?}; SIGKILL to its process group"
    );
    let kill_error = handle
        .process_tree
        .as_ref()
        .ok_or_else(|| {
            RuntimeError::Msg(format!(
                "session {session_id} has no process tree for SIGKILL"
            ))
        })?
        .terminate()
        .err();
    if poll_until(STOP_POLL, KILL_GRACE, || {
        Ok(reader_reaped_child(handle, Some(pid)).then_some(()))
    })?
    .is_some()
    {
        return Ok(());
    }

    Err(RuntimeError::Msg(format!(
        "session {session_id} was not reaped after SIGHUP and SIGKILL{}{}",
        hup_error
            .map(|e| format!("; SIGHUP error: {e}"))
            .unwrap_or_default(),
        kill_error
            .map(|e| format!("; SIGKILL error: {e}"))
            .unwrap_or_default(),
    )))
}

#[cfg(windows)]
fn terminate_session_job(
    session_id: &str,
    process_tree: Option<&ProcessTree>,
) -> RuntimeResult<()> {
    process_tree
        .ok_or_else(|| RuntimeError::Msg(format!("session {session_id} has no process job")))?
        .terminate()
        .map_err(|e| RuntimeError::Msg(format!("terminate job {session_id}: {e}")))
}

#[cfg(windows)]
fn stop_and_reap_child(
    session_id: &str,
    process_tree: Option<&ProcessTree>,
    child: &mut dyn Child,
) -> RuntimeResult<ExitStatus> {
    terminate_session_job(session_id, process_tree)?;
    poll_until(STOP_POLL, WINDOWS_KILL_GRACE, || {
        child
            .try_wait()
            .map_err(|e| RuntimeError::Msg(format!("try_wait {session_id}: {e}")))
    })?
    .ok_or_else(|| RuntimeError::Msg(format!("session {session_id} survived job termination")))
}

#[cfg(windows)]
fn stop_child_owned_by_reader(session_id: &str, handle: &SessionHandle) -> RuntimeResult<()> {
    terminate_session_job(session_id, handle.process_tree.as_ref())?;
    poll_until(STOP_POLL, WINDOWS_KILL_GRACE, || {
        Ok(reader_reaped_child(handle, handle.pid).then_some(()))
    })?
    .ok_or_else(|| {
        RuntimeError::Msg(format!(
            "session {session_id} was not reaped after job termination"
        ))
    })
}

pub(super) fn poll_until<T>(
    poll_interval: Duration,
    timeout: Duration,
    mut poll: impl FnMut() -> RuntimeResult<Option<T>>,
) -> RuntimeResult<Option<T>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll()? {
            return Ok(Some(value));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        thread::sleep(poll_interval.min(deadline.saturating_duration_since(now)));
    }
}

fn reaped_exit_status(handle: &SessionHandle) -> Option<i32> {
    match handle.exit_code.load(Ordering::Acquire) {
        EXIT_UNSET => None,
        code => Some(code),
    }
}

fn reader_reaped_child(handle: &SessionHandle, pid: Option<i32>) -> bool {
    reaped_exit_status(handle).is_some() || pid.is_some_and(|pid| !process_exists(pid))
}

fn record_exit_status(handle: &SessionHandle, status: ExitStatus) {
    let code = status
        .exit_code()
        .try_into()
        .unwrap_or(EXIT_UNSET.saturating_add(1));
    handle.exit_code.store(code, Ordering::Release);
    handle.alive.store(false, Ordering::Release);
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

    /// Byte arrival during a resize grace window: keep the idle timer
    /// fresh (a genuinely busy session that was also resized must not
    /// idle early) but never flip Idle→Busy, so a SIGWINCH repaint on an
    /// idle session stays idle.
    fn on_bytes_quiet_at(&mut self, n: usize, now: Instant) {
        if n == 0 {
            return;
        }
        self.last_byte = now;
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
    mut query_responder: TerminalQueryResponder,
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
                answer_terminal_queries(&mut query_responder, &buf[..n], &handle, &session_id);
                let in_resize_grace = handle
                    .last_resize
                    .lock()
                    .expect("last_resize poisoned")
                    .is_some_and(|t| t.elapsed() < RESIZE_GRACE);
                let transition = {
                    let mut detector = detector.lock().expect("idle detector poisoned");
                    if in_resize_grace {
                        detector.on_bytes_quiet_at(n, Instant::now());
                        None
                    } else {
                        detector.on_bytes(n)
                    }
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
            Ok(status) => record_exit_status(&handle, status),
            Err(error) => {
                // wait() failure leaves exit_code as EXIT_UNSET;
                // status() reports None and the manager treats it as
                // "crashed".
                log::warn!("wait failed while reaping session {session_id}: {error}");
            }
        }
    }
    // tx dropped on scope exit → OutputStream::recv_timeout sees
    // Disconnected on the manager side, which is how the existing
    // forwarder thread knows to wind down.
}

// --- Helpers ------------------------------------------------------------

#[derive(Default)]
struct TerminalQueryResponder {
    tail: Vec<u8>,
    observed: usize,
}

impl TerminalQueryResponder {
    fn observe(&mut self, chunk: &[u8]) -> Vec<&'static [u8]> {
        if chunk.is_empty() {
            return Vec::new();
        }
        if self.observed >= TERMINAL_QUERY_STARTUP_BUDGET {
            self.observed = self.observed.saturating_add(chunk.len());
            self.tail.clear();
            return Vec::new();
        }

        let remaining = TERMINAL_QUERY_STARTUP_BUDGET - self.observed;
        let scan_len = chunk.len().min(remaining);
        let scan_chunk = &chunk[..scan_len];
        let old_len = self.tail.len();
        let mut combined = Vec::with_capacity(old_len + scan_chunk.len());
        combined.extend_from_slice(&self.tail);
        combined.extend_from_slice(scan_chunk);

        let mut matches: Vec<(usize, &'static [u8])> = Vec::new();
        for (needle, response) in terminal_query_patterns() {
            for pos in find_subsequence_positions(&combined, needle) {
                if pos + needle.len() > old_len {
                    matches.push((pos, response));
                }
            }
        }
        matches.sort_by_key(|(pos, _)| *pos);

        let tail_start = combined.len().saturating_sub(TERMINAL_QUERY_TAIL);
        self.tail.clear();
        self.tail.extend_from_slice(&combined[tail_start..]);
        self.observed = self.observed.saturating_add(chunk.len());

        matches.into_iter().map(|(_, response)| response).collect()
    }
}

fn terminal_query_patterns() -> &'static [(&'static [u8], &'static [u8])] {
    &[
        (b"\x1b]10;?\x1b\\", DEFAULT_OSC10_FG_REPLY),
        (b"\x1b]10;?\x07", DEFAULT_OSC10_FG_REPLY),
        (b"\x1b]11;?\x1b\\", DEFAULT_OSC11_BG_REPLY),
        (b"\x1b]11;?\x07", DEFAULT_OSC11_BG_REPLY),
        (b"\x1b[6n", DSR_CURSOR_POS_REPLY),
        (b"\x1b[c", DA1_XTERM_REPLY),
        (b"\x1b[0c", DA1_XTERM_REPLY),
    ]
}

fn find_subsequence_positions(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(idx, window)| (window == needle).then_some(idx))
        .collect()
}

fn answer_terminal_queries(
    responder: &mut TerminalQueryResponder,
    bytes: &[u8],
    handle: &SessionHandle,
    session_id: &str,
) {
    let responses = responder.observe(bytes);
    if responses.is_empty() {
        return;
    }

    let mut writer = handle.writer.lock().expect("writer poisoned");
    for response in responses {
        if let Err(e) = writer.write_all(response) {
            log::warn!("terminal query response write failed for {session_id}: {e}");
            return;
        }
    }
    if let Err(e) = writer.flush() {
        log::warn!("terminal query response flush failed for {session_id}: {e}");
    }
}

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
/// (`"Escape"`, `"C-c"`, `"Up"`). The GPUI terminal translates ordinary
/// user typing before sending it through `send_bytes`, so this table only
/// covers named keys injected by the backend.
fn translate_key(key: &str) -> RuntimeResult<Vec<u8>> {
    // C-<letter> chord: Ctrl + letter → control byte.
    if let Some(rest) = key.strip_prefix("C-") {
        if rest.len() == 1 {
            let ch = rest
                .chars()
                .next()
                .expect("single-byte control chord has one character");
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
/// sidebar reflects "agent died with prior app process" reality.
///
/// Status flips to `stopped`, `stopped_at` records the wall-clock at
/// cleanup time, and `pid` is left as the prior value for diagnostics.
/// Mission rows are demoted alongside direct chats; users can resume
/// them by spawning a fresh PTY through `SessionManager::resume`.
pub fn cleanup_stale_running_rows_on_startup(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> rusqlite::Result<usize> {
    let conn = pool
        .get()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let updated = crate::repo::session::cleanup_stale_running(&conn, chrono::Utc::now())?;
    if updated > 0 {
        log::info!(
            "pty-runtime startup cleanup: demoted {updated} stale running session(s) to stopped"
        );
    }
    Ok(updated)
}

#[cfg(unix)]
pub fn cleanup_orphan_processes_on_startup(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> crate::error::Result<usize> {
    let conn = pool.get()?;
    let candidates = {
        let mut stmt = conn.prepare(
            "SELECT s.id,
                    s.pid,
                    COALESCE(s.agent_runtime, r.runtime),
                    COALESCE(s.agent_command, r.command)
               FROM sessions s
               LEFT JOIN runners r ON r.id = s.runner_id
              WHERE s.status != 'running'
                AND s.pid IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut signaled = Vec::new();
    for (session_id, raw_pid, runtime, command) in candidates {
        let Ok(pid) = i32::try_from(raw_pid) else {
            log::warn!(
                "startup orphan sweep: session={session_id} has invalid pid={raw_pid}; skipping"
            );
            clear_recorded_pid(&conn, &session_id, raw_pid);
            continue;
        };
        if !process_exists(pid) {
            clear_recorded_pid(&conn, &session_id, raw_pid);
            continue;
        }
        let command_line = match process_command_line(pid) {
            Ok(Some(command_line)) => command_line,
            Ok(None) => {
                clear_recorded_pid(&conn, &session_id, raw_pid);
                continue;
            }
            Err(error) => {
                log::warn!(
                    "startup orphan sweep: session={session_id} pid={pid} command lookup failed: {error}"
                );
                continue;
            }
        };
        if !command_line_matches_recorded_agent(
            &command_line,
            runtime.as_deref(),
            command.as_deref(),
        ) {
            log::warn!(
                "startup orphan sweep: session={session_id} pid={pid} command mismatch; not signaling"
            );
            clear_recorded_pid(&conn, &session_id, raw_pid);
            continue;
        }

        let signal_error = kill_process(pid).err();
        signaled.push((session_id, raw_pid, pid, signal_error));
    }

    let deadline = Instant::now() + ORPHAN_SWEEP_CONFIRM;
    let mut reaped = 0;
    for (session_id, raw_pid, pid, signal_error) in signaled {
        if wait_for_process_exit_until(pid, deadline) {
            reaped += 1;
            clear_recorded_pid(&conn, &session_id, raw_pid);
            log::info!("startup orphan sweep: reaped session={session_id} pid={pid}");
        } else {
            log::warn!(
                "startup orphan sweep: session={session_id} pid={pid} survived SIGKILL{}",
                signal_error
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(reaped)
}

// Job handles closed with the previous app; only stale database pids need clearing.
#[cfg(windows)]
pub fn cleanup_orphan_processes_on_startup(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> crate::error::Result<usize> {
    pool.get()?.execute(
        "UPDATE sessions SET pid = NULL WHERE status != 'running' AND pid IS NOT NULL",
        [],
    )?;
    Ok(0)
}

#[cfg(unix)]
fn clear_recorded_pid(conn: &rusqlite::Connection, session_id: &str, expected_pid: i64) {
    if let Err(error) = conn.execute(
        "UPDATE sessions
            SET pid = NULL
          WHERE id = ?1
            AND pid = ?2
            AND status != 'running'",
        rusqlite::params![session_id, expected_pid],
    ) {
        log::warn!(
            "startup orphan sweep: session={session_id} pid={expected_pid} clear failed: {error}"
        );
    }
}

#[cfg(unix)]
fn command_line_matches_recorded_agent(
    command_line: &str,
    runtime: Option<&str>,
    command: Option<&str>,
) -> bool {
    let expected = command
        .filter(|command| !command.trim().is_empty())
        .or_else(|| {
            runtime.and_then(|runtime| {
                crate::router::runtime::runtime_definition(runtime)
                    .map(|definition| definition.command)
            })
        });
    let Some(expected) = expected.map(str::trim) else {
        return false;
    };

    if command_line == expected
        || command_line
            .strip_prefix(expected)
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
    {
        return true;
    }

    let Some(actual_executable) = command_line.split_ascii_whitespace().next() else {
        return false;
    };
    let expected_name = std::path::Path::new(expected).file_name();
    let actual_name = std::path::Path::new(actual_executable).file_name();
    expected_name.is_some() && expected_name == actual_name
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::process::distinct_foreground_process;
    use super::*;
    use std::collections::BTreeMap;

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
    fn terminal_query_responder_answers_codex_startup_handshake() {
        let mut responder = TerminalQueryResponder::default();
        let responses =
            responder.observe(b"\x1b[?2004h\x1b[6n\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[c");

        assert_eq!(
            responses,
            vec![
                DSR_CURSOR_POS_REPLY,
                DEFAULT_OSC10_FG_REPLY,
                DEFAULT_OSC11_BG_REPLY,
                DA1_XTERM_REPLY,
            ]
        );
    }

    #[test]
    fn terminal_query_responder_handles_fragmented_osc_without_duplicates() {
        let mut responder = TerminalQueryResponder::default();

        assert!(responder.observe(b"\x1b]1").is_empty());
        assert_eq!(
            responder.observe(b"1;?\x1b\\trail"),
            vec![DEFAULT_OSC11_BG_REPLY]
        );
        assert!(responder.observe(b" more output").is_empty());
    }

    #[test]
    fn terminal_query_responder_supports_bel_terminated_colors_and_da_zero() {
        let mut responder = TerminalQueryResponder::default();
        let responses = responder.observe(b"\x1b]10;?\x07\x1b]11;?\x07\x1b[0c");

        assert_eq!(
            responses,
            vec![
                DEFAULT_OSC10_FG_REPLY,
                DEFAULT_OSC11_BG_REPLY,
                DA1_XTERM_REPLY,
            ]
        );
    }

    #[test]
    fn terminal_query_responder_only_answers_startup_window() {
        let mut responder = TerminalQueryResponder::default();
        let filler = vec![b'x'; TERMINAL_QUERY_STARTUP_BUDGET];

        assert!(responder.observe(&filler).is_empty());
        assert!(responder.observe(b"\x1b]11;?\x1b\\").is_empty());
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
    fn resize_grace_bytes_do_not_wake_idle_detector() {
        // Models the reader loop's grace branch: an idle session that
        // emits a SIGWINCH repaint burst stays idle, and its idle timer
        // is kept fresh so it doesn't immediately re-flip on the next tick.
        let threshold = Duration::from_millis(750);
        let start = Instant::now();
        let mut detector = IdleDetector::new_at(threshold, start);

        assert_eq!(
            detector.tick_at(start + threshold),
            Some(RunnerStatus::Idle)
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
            Some(RunnerStatus::Busy)
        );
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
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(RuntimeOutput::Stream(bytes)) => collected.extend_from_slice(&bytes),
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
    #[cfg(unix)]
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
        let mut attempts = 0;
        let result = poll_until(Duration::from_millis(1), Duration::from_millis(20), || {
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
                "/bin/zsh",
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
            match stream.recv_timeout(Duration::from_millis(50)) {
                Ok(RuntimeOutput::Stream(bytes)) => output.extend_from_slice(&bytes),
                Ok(RuntimeOutput::StatusTransition { .. }) | Err(_) => {}
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
            match stream.recv_timeout(Duration::from_millis(50)) {
                Ok(RuntimeOutput::Stream(bytes)) => output.extend_from_slice(&bytes),
                Ok(RuntimeOutput::StatusTransition { .. }) => {}
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
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Err(mpsc::RecvTimeoutError::Disconnected) =
                stream.recv_timeout(Duration::from_millis(100))
            {
                break;
            }
        }
        let status = rt.status(&session).unwrap().unwrap();
        assert!(!status.alive);
        assert_eq!(status.exit_code, Some(7));
    }

    #[cfg(windows)]
    #[test]
    fn spawn_emits_idle_after_silence_and_busy_on_more_output_windows() {
        let rt = PtyRuntime::new();
        let (session, stream) = rt.spawn(spec(
            "idle-windows", "cmd", &["/d", "/c", "echo first & timeout /t 2 /nobreak >nul & timeout /t 2 /nobreak >nul & echo second"],
        )).unwrap();
        let mut statuses = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(12);
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
                Ok(RuntimeOutput::Stream(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        rt.stop(&session).unwrap();
        assert!(
            statuses
                .windows(2)
                .any(|w| w == [RunnerStatus::Idle, RunnerStatus::Busy]),
            "got {statuses:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn foreground_process_tracks_job_and_stop_reaps_child_windows() {
        let rt = PtyRuntime::new();
        let (session, _stream) = rt.spawn(spec("job-stop", "cmd", &["/d", "/q"])).unwrap();
        assert_eq!(rt.has_foreground_process(&session).unwrap(), Some(false));
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
            "INSERT INTO sessions (id, status, pid, agent_runtime, agent_command) VALUES ('stale', 'running', ?1, 'test', 'runner-backend.exe')",
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
}

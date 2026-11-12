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

use super::hook_feed::HookWatcher;
use super::launch;
#[cfg(any(unix, test))]
use super::process::process_exists;
use super::process::{kill_process, ProcessTree};
#[cfg(unix)]
use super::process::{process_command_line, reap_descendants, wait_for_process_exit_until};
use super::runtime::{
    OutputStream, RuntimeError, RuntimeOutput, RuntimeResult, RuntimeSession, SessionActivityState,
    SessionRuntime, SessionStatus, SpawnSpec,
};
use crate::session::state::{CTRL_C_INTERRUPT, ESCAPE_INTERRUPT};

const RUNTIME_LABEL: &str = "native-pty";
// Typical ConPTY redraws can fit in one read; the forwarder coalesces larger bursts.
const READ_BUF: usize = 64 * 1024;
/// Silence before a session reads as Idle. The agent TUIs animate a spinner
/// while a turn runs, so genuine work rarely goes quiet; the 2 s window is
/// for the moments it does (a tool call that pauses the spinner), because
/// the router treats Idle as a turn boundary and delivers into it.
pub(crate) const DEFAULT_IDLE_THRESHOLD: Duration = Duration::from_secs(2);
// An optional native bridge may never start despite its receiver being allocated.
const HOOK_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
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

/// Public constructor. Holds no external state — the runtime is purely
/// in-memory and the per-session resources tear down with their handles.
pub struct PtyRuntime {
    hooks: Arc<super::hook_queue::HookRoutes>,
    sessions: Sessions,
    readers: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl PtyRuntime {
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(super::hook_queue::HookRoutes::default()),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            readers: Mutex::new(Vec::new()),
        }
    }
}

impl Default for PtyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

type Sessions = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>;

struct SessionHandle {
    /// `None` once the session has ended, which closes the PTY.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
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
    idle_detector: Arc<Mutex<IdleDetector>>,
    pid: Option<i32>,
    process_tree: Option<ProcessTree>,
    command: String,
}

const EXIT_UNSET: i32 = i32::MIN;

impl SessionRuntime for PtyRuntime {
    fn admit_hook(
        &self,
        report: runner_core::protocol::HookReport,
    ) -> crate::error::Result<super::hook_queue::Admission> {
        self.hooks.admit(report)
    }
    fn drain_workers(&self) {
        let workers = std::mem::take(&mut *self.readers.lock().unwrap());
        for worker in workers {
            let _ = worker.join();
        }
    }
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
        #[cfg(windows)]
        {
            static LOG_CONPTY: std::sync::Once = std::sync::Once::new();
            LOG_CONPTY.call_once(|| {
                let sideloaded = std::env::current_exe()
                    .ok()
                    .is_some_and(|exe| exe.with_file_name("conpty.dll").is_file());
                if sideloaded {
                    log::info!(
                        "PTY host: sideloaded ConPTY (conpty.dll present beside executable)"
                    );
                } else {
                    log::info!("PTY host: inbox conhost (conpty.dll absent beside executable)");
                }
            });
        }
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| RuntimeError::Msg(format!("openpty: {e}")))?;

        // `CommandBuilder::new` starts from portable-pty's base environment:
        // this process's variables and, on Windows, the system and user
        // variables in the registry. `spec.env` then overrides it by name.
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
        // Shells keep a symlinked cwd's own path only when PWD names it;
        // otherwise they print the resolved path.
        #[cfg(unix)]
        if let Some(cwd) = &spec.cwd {
            cmd.env("PWD", cwd);
        }
        // COLUMNS/LINES so Node-based TUIs pick up the initial grid
        // before SIGWINCH lands.
        cmd.env("COLUMNS", cols.to_string());
        cmd.env("LINES", rows.to_string());

        #[cfg(windows)]
        launch::adapt_windows_batch_command(&mut cmd)
            .map_err(|error| RuntimeError::Msg(error.to_string()))?;

        let mut hook_status = spec
            .agent_runtime
            .and_then(|runtime| crate::runtimes::adapter(runtime).status_hooks())
            .and_then(|hooks| {
                if let Some(generation) = spec.env.get(runner_core::protocol::hook::GENERATION_ENV)
                {
                    let receiver = self.hooks.register(
                        spec.agent_runtime.unwrap(),
                        spec.session_id.clone(),
                        generation.clone(),
                    );
                    hooks.start_receiver(&spec, receiver)
                } else {
                    None
                }
            });
        let status_hooks_enabled = crate::runtimes::status_hooks_enabled(&spec.env);
        if !status_hooks_enabled {
            hook_status = hook_status.map(|watcher| {
                Box::new(super::hook_feed::IdentityWatcher(watcher)) as Box<dyn HookWatcher>
            });
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| RuntimeError::Msg(format!("spawn_command: {e}")))?;
        // Drop the slave handle: the child holds its own end of the
        // PTY, and keeping the parent-side slave open would prevent us
        // from observing EOF on the master when the child exits.
        drop(pair.slave);

        let pid = child.process_id().map(|p| p as i32);
        if let (Some(watcher), Some(pid)) = (hook_status.as_mut(), child.process_id()) {
            watcher.spawned(pid);
        }
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

        let terminal_adapter = spec.agent_runtime.and_then(|runtime| {
            crate::runtimes::adapter(runtime).terminal_adapter(spec.pending_turn)
        });
        let mut idle_detector =
            IdleDetector::with_adapter_at(DEFAULT_IDLE_THRESHOLD, terminal_adapter, Instant::now());
        if spec.agent_runtime == Some(crate::model::Runtime::Shell) {
            idle_detector.read_shell_marks();
        }
        idle_detector.awaiting_first_hook = spec.mission && hook_status.is_some();
        if hook_status.is_none() || !status_hooks_enabled {
            if spec.mission {
                let _ = tx.send(RuntimeOutput::StatusBridgeFailed);
            }
            idle_detector.hooks_unavailable();
        }
        let handle = Arc::new(SessionHandle {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            #[cfg(unix)]
            killer: Mutex::new(killer),
            child: Arc::clone(&child_slot),
            exit_code: AtomicI32::new(EXIT_UNSET),
            alive: AtomicBool::new(true),
            last_resize: Mutex::new(None),
            idle_detector: Arc::new(Mutex::new(idle_detector)),
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
        let entry = Arc::new(SessionEntry {
            sessions: Arc::clone(&self.sessions),
            session_id: spec.session_id.clone(),
            handle: Arc::clone(&handle),
        });
        let entry_for_reader = Arc::clone(&entry);
        let session_id_for_reader = spec.session_id.clone();
        let worker = thread::Builder::new()
            .name(format!("pty-reader-{}", spec.session_id))
            .spawn(move || {
                reader_thread(
                    reader,
                    tx,
                    stop_for_reader,
                    handle_for_reader,
                    entry_for_reader,
                    session_id_for_reader,
                    hook_status,
                );
            })
            .map_err(|e| RuntimeError::Msg(format!("spawn reader thread: {e}")))?;
        let mut readers = self.readers.lock().unwrap();
        readers.retain(|reader| !reader.is_finished());
        readers.push(worker);

        let rt_session = RuntimeSession {
            runtime: RUNTIME_LABEL.to_string(),
            session_id: spec.session_id,
        };
        let stream = OutputStream::new(rx, stop).holding(entry);
        Ok((rt_session, stream))
    }

    #[cfg(unix)]
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        self.hooks.retire(&session.session_id);
        // An untracked session's reader has reaped its child and its output
        // stream is gone, so it has already stopped.
        let Ok(handle) = lookup(self, &session.session_id) else {
            return Ok(());
        };
        let mut killer = handle.killer.lock().expect("killer poisoned");
        let child = handle.child.lock().expect("child slot poisoned").take();
        // Snapshot before SIGHUP: once the agent is gone its children are
        // reparented outside its tree and ancestry no longer identifies them.
        let descendants = handle
            .process_tree
            .as_ref()
            .map(ProcessTree::snapshot_descendants)
            .unwrap_or_default();

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
        self.hooks.retire(&session.session_id);
        // An untracked session's reader has reaped its child and its output
        // stream is gone, so it has already stopped.
        let Ok(handle) = lookup(self, &session.session_id) else {
            return Ok(());
        };
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
            let Some(master) = master.as_ref() else {
                return Ok(());
            };
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
        let master = handle.master.lock().expect("SessionHandle.master poisoned");
        if master.is_none() {
            return Ok(Some(false));
        }
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
fn terminate_session_job<'a>(
    session_id: &str,
    process_tree: Option<&'a ProcessTree>,
) -> RuntimeResult<&'a ProcessTree> {
    let tree = process_tree
        .ok_or_else(|| RuntimeError::Msg(format!("session {session_id} has no process job")))?;
    tree.terminate()
        .map_err(|e| RuntimeError::Msg(format!("terminate job {session_id}: {e}")))?;
    Ok(tree)
}

#[cfg(windows)]
fn stop_and_reap_child(
    session_id: &str,
    process_tree: Option<&ProcessTree>,
    child: &mut dyn Child,
) -> RuntimeResult<ExitStatus> {
    let tree = terminate_session_job(session_id, process_tree)?;
    poll_until(STOP_POLL, WINDOWS_KILL_GRACE, || {
        if !tree
            .root_has_exited()
            .map_err(|e| RuntimeError::Msg(format!("wait {session_id}: {e}")))?
        {
            return Ok(None);
        }
        child
            .try_wait()
            .map_err(|e| RuntimeError::Msg(format!("try_wait {session_id}: {e}")))
    })?
    .ok_or_else(|| RuntimeError::Msg(format!("session {session_id} survived job termination")))
}

#[cfg(windows)]
fn stop_child_owned_by_reader(session_id: &str, handle: &SessionHandle) -> RuntimeResult<()> {
    let tree = terminate_session_job(session_id, handle.process_tree.as_ref())?;
    poll_until(STOP_POLL, WINDOWS_KILL_GRACE, || {
        tree.root_has_exited()
            .map(|exited| exited.then_some(()))
            .map_err(|e| RuntimeError::Msg(format!("wait {session_id}: {e}")))
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

#[cfg(unix)]
fn reaped_exit_status(handle: &SessionHandle) -> Option<i32> {
    match handle.exit_code.load(Ordering::Acquire) {
        EXIT_UNSET => None,
        code => Some(code),
    }
}

#[cfg(unix)]
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

pub(crate) struct IdleDetector {
    first_output: Option<Instant>,
    awaiting_first_hook: bool,
    last_byte: Instant,
    current: SessionActivityState,
    threshold: Duration,
    adapter: Option<Box<dyn crate::runtimes::TerminalAdapter>>,
    input_transition: bool,
    pending_cancel: u8,
    /// A terminal shell's command marks. Once a D shows the shell sends them,
    /// they alone set its activity: output, silence and resizes no longer do.
    shell_marks: Option<runner_terminal::program_status::ShellMarks>,
    follows_shell_marks: bool,
}

impl IdleDetector {
    pub(crate) fn with_adapter_at(
        threshold: Duration,
        adapter: Option<Box<dyn crate::runtimes::TerminalAdapter>>,
        now: Instant,
    ) -> Self {
        let input_transition = adapter
            .as_ref()
            .is_some_and(|adapter| adapter.held_activity() == Some(SessionActivityState::Busy));
        Self {
            first_output: None,
            awaiting_first_hook: false,
            last_byte: now,
            current: SessionActivityState::Busy,
            threshold,
            adapter,
            input_transition,
            pending_cancel: 0,
            shell_marks: None,
            follows_shell_marks: false,
        }
    }
    pub(crate) fn read_shell_marks(&mut self) {
        self.shell_marks = Some(Default::default());
    }
    #[cfg(test)]
    pub(crate) fn new_at(threshold: Duration, now: Instant) -> Self {
        Self::with_adapter_at(threshold, None, now)
    }
    fn on_output(&mut self, bytes: &[u8], quiet: bool) -> Option<crate::runtimes::TerminalEvent> {
        self.on_output_event_at(bytes, quiet, Instant::now())
    }
    pub(crate) fn on_output_event_at(
        &mut self,
        bytes: &[u8],
        quiet: bool,
        now: Instant,
    ) -> Option<crate::runtimes::TerminalEvent> {
        use crate::runtimes::TerminalEvent;
        if !bytes.is_empty() {
            self.first_output.get_or_insert(now);
        }
        if let Some(marks) = self.shell_marks.as_mut() {
            use runner_terminal::program_status::ShellMark;
            let previous = self.current;
            for mark in marks.scan(bytes) {
                match mark {
                    ShellMark::CommandFinished => {
                        self.follows_shell_marks = true;
                        self.current = SessionActivityState::Idle;
                    }
                    // An agent's transcript zones send C too, so C alone
                    // cannot show that a shell without the integration has it.
                    ShellMark::CommandStarted if self.follows_shell_marks => {
                        self.current = SessionActivityState::Busy;
                    }
                    ShellMark::CommandStarted => {}
                }
            }
            if self.follows_shell_marks {
                return (self.current != previous).then_some(TerminalEvent::Activity(self.current));
            }
        }
        let (event, activity_bytes) = if let Some(adapter) = self.adapter.as_mut() {
            let event = adapter.on_output(bytes);
            (event, adapter.output_activity_bytes(bytes))
        } else {
            (None, bytes.len())
        };
        if let Some(event) = event {
            if matches!(event, TerminalEvent::Ready(_)) {
                self.last_byte = now;
                return self.tick_event_at(now);
            }
            let state = event.state()?;
            if activity_bytes > 0 {
                self.last_byte = now;
            }
            if self.current != state {
                self.current = state;
                return Some(event);
            }
            return None;
        }
        if quiet {
            self.on_bytes_quiet_at(activity_bytes, now);
            None
        } else {
            self.on_bytes_at(activity_bytes, now)
                .map(TerminalEvent::Activity)
        }
    }
    pub(crate) fn take_attention(&mut self) -> Option<crate::runtimes::TerminalEvent> {
        self.adapter
            .as_mut()?
            .take_attention()
            .map(crate::runtimes::TerminalEvent::CodexAttention)
    }
    #[cfg(test)]
    fn on_output_state(&mut self, bytes: &[u8], quiet: bool) -> Option<SessionActivityState> {
        self.on_output(bytes, quiet).and_then(|event| event.state())
    }
    #[cfg(test)]
    fn tick_state(&mut self) -> Option<SessionActivityState> {
        self.tick().and_then(|event| event.state())
    }
    #[cfg(test)]
    fn on_output_at(
        &mut self,
        bytes: &[u8],
        quiet: bool,
        now: Instant,
    ) -> Option<SessionActivityState> {
        self.on_output_event_at(bytes, quiet, now)
            .and_then(|event| event.state())
    }
    fn on_input(&mut self, bytes: &[u8]) {
        self.on_input_at(bytes, Instant::now());
    }
    pub(crate) fn on_input_at(&mut self, bytes: &[u8], now: Instant) {
        if let Some(kind) = interrupt_key(bytes) {
            self.pending_cancel |= kind;
        }
        if let Some(adapter) = self.adapter.as_mut() {
            let input = adapter.on_input(bytes);
            if let Some(state) = input.state {
                self.current = state;
            }
            if input.refresh {
                self.last_byte = now;
            }
            self.input_transition |= input.announce;
        }
    }
    pub(crate) fn hook_startup_timed_out_at(&self, now: Instant) -> bool {
        self.awaiting_first_hook
            && self
                .first_output
                .is_some_and(|first| now.duration_since(first) >= HOOK_STARTUP_TIMEOUT)
            && now.duration_since(self.last_byte) >= self.threshold
    }

    pub(crate) fn hooks_unavailable(&mut self) {
        self.awaiting_first_hook = false;
        if let Some(adapter) = self.adapter.as_mut() {
            adapter.hooks_unavailable();
        }
    }
    pub(crate) fn accept_event(&mut self, event: &super::state::agent::AgentEvent) -> bool {
        self.awaiting_first_hook = false;
        if self
            .adapter
            .as_mut()
            .is_some_and(|adapter| !adapter.accept_event(event))
        {
            return false;
        }
        self.input_transition = false;
        true
    }
    fn on_bytes_at(&mut self, n: usize, now: Instant) -> Option<SessionActivityState> {
        if n == 0 {
            return None;
        }
        self.last_byte = now;
        if self.current == SessionActivityState::Idle {
            self.current = SessionActivityState::Busy;
            Some(SessionActivityState::Busy)
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

    fn tick(&mut self) -> Option<crate::runtimes::TerminalEvent> {
        self.tick_event_at(Instant::now())
    }
    pub(crate) fn tick_event_at(&mut self, now: Instant) -> Option<crate::runtimes::TerminalEvent> {
        use crate::runtimes::TerminalEvent;
        if let Some(state) = self
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.held_activity())
        {
            let input_transition = std::mem::take(&mut self.input_transition);
            if self.current != state || input_transition {
                self.current = state;
                return Some(TerminalEvent::Ready(state));
            }
            return None;
        }
        if std::mem::take(&mut self.input_transition) {
            return Some(TerminalEvent::Activity(self.current));
        }
        if !self.follows_shell_marks
            && self.current == SessionActivityState::Busy
            && now.duration_since(self.last_byte) >= self.threshold
        {
            self.current = SessionActivityState::Idle;
            Some(TerminalEvent::Activity(SessionActivityState::Idle))
        } else {
            None
        }
    }
    #[cfg(test)]
    fn tick_at(&mut self, now: Instant) -> Option<SessionActivityState> {
        self.tick_event_at(now).and_then(|event| event.state())
    }
}

fn idle_monitor_thread(
    detector: Arc<Mutex<IdleDetector>>,
    tx: mpsc::Sender<RuntimeOutput>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    mut hook_status: Option<Box<dyn HookWatcher>>,
    #[cfg(windows)] handle: Arc<SessionHandle>,
) {
    loop {
        let stopped = stop.load(Ordering::Acquire) || done.load(Ordering::Acquire);
        #[cfg(windows)]
        let stopped = stopped
            || handle
                .process_tree
                .as_ref()
                .is_some_and(|tree| tree.root_has_exited().unwrap_or(false));
        if stopped {
            break;
        }
        if let Some(watcher) = hook_status.as_mut() {
            let result = watcher.drain_events(
                {
                    let mut detector = detector.lock().expect("idle detector poisoned");
                    std::mem::take(&mut detector.pending_cancel)
                },
                &mut |event| {
                    if !detector
                        .lock()
                        .expect("idle detector poisoned")
                        .accept_event(&event)
                    {
                        return Default::default();
                    }
                    // The manager can write input while publishing; release the detector before waiting.
                    if event.needs_feedback() {
                        let (reply, receipt) = mpsc::channel();
                        if tx
                            .send(RuntimeOutput::AgentEvent {
                                event,
                                feedback: Some(reply),
                            })
                            .is_err()
                        {
                            return Default::default();
                        }
                        loop {
                            match receipt.recv_timeout(IDLE_MONITOR_POLL) {
                                Ok(feedback) => return feedback,
                                Err(mpsc::RecvTimeoutError::Disconnected) => {
                                    return Default::default()
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => {
                                    if stop.load(Ordering::Acquire) || done.load(Ordering::Acquire)
                                    {
                                        return Default::default();
                                    }
                                }
                            }
                        }
                    }
                    let _ = tx.send(RuntimeOutput::AgentEvent {
                        event,
                        feedback: None,
                    });
                    Default::default()
                },
                &mut |id| {
                    if !id.is_empty()
                        || (!stop.load(Ordering::Acquire) && !done.load(Ordering::Acquire))
                    {
                        let _ = tx.send(RuntimeOutput::ConversationStart(id));
                    }
                },
            );
            let failed = match result {
                Ok(()) => {
                    let mut detector = detector.lock().expect("idle detector poisoned");
                    let expired = detector.hook_startup_timed_out_at(Instant::now());
                    // A resumed agent at rest reports nothing until its next turn, so keep
                    // draining: its first report hands status back to the hooks.
                    if expired {
                        log::warn!(
                            "read agent status: hook bridge produced no status within {HOOK_STARTUP_TIMEOUT:?} of terminal output; using output fallback until it reports"
                        );
                        detector.hooks_unavailable();
                    }
                    expired
                }
                Err(error) => {
                    log::warn!("read agent status: {error}");
                    detector
                        .lock()
                        .expect("idle detector poisoned")
                        .hooks_unavailable();
                    hook_status = None;
                    true
                }
            };
            if failed {
                let _ = tx.send(RuntimeOutput::StatusBridgeFailed);
            }
        }
        let transition = {
            let mut detector = detector.lock().expect("idle detector poisoned");
            detector.tick()
        };
        if let Some(state) = transition {
            if tx.send(RuntimeOutput::TerminalEvent(state)).is_err() {
                break;
            }
        }
        thread::sleep(IDLE_MONITOR_POLL);
    }
    #[cfg(windows)]
    {
        // ConPTY retains its output pipe until the master closes; the reader must keep draining.
        let master = handle.master.lock().expect("master poisoned").take();
        drop(master);
    }
}

fn reader_thread(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<RuntimeOutput>,
    stop: Arc<AtomicBool>,
    handle: Arc<SessionHandle>,
    entry: Arc<SessionEntry>,
    session_id: String,
    hook_status: Option<Box<dyn HookWatcher>>,
) {
    let detector = Arc::clone(&handle.idle_detector);
    let monitor_done = Arc::new(AtomicBool::new(false));
    let monitor = thread::Builder::new()
        .name(format!("pty-idle-{session_id}"))
        .spawn({
            let detector = Arc::clone(&detector);
            let tx = tx.clone();
            let stop = Arc::clone(&stop);
            let done = Arc::clone(&monitor_done);
            #[cfg(windows)]
            let handle = Arc::clone(&handle);
            move || {
                idle_monitor_thread(
                    detector,
                    tx,
                    stop,
                    done,
                    hook_status,
                    #[cfg(windows)]
                    handle,
                )
            }
        });
    let monitor = match monitor {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("spawn idle monitor thread for {session_id}: {e}");
            None
        }
    };

    let mut buf = vec![0u8; READ_BUF];
    'read_output: loop {
        #[cfg(not(windows))]
        if stop.load(Ordering::Acquire) {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let in_resize_grace = handle
                    .last_resize
                    .lock()
                    .expect("last_resize poisoned")
                    .is_some_and(|t| t.elapsed() < RESIZE_GRACE);
                let (transition, attention) = {
                    let mut detector = detector.lock().expect("idle detector poisoned");
                    let transition = detector.on_output(&buf[..n], in_resize_grace);
                    (transition, detector.take_attention())
                };
                for event in attention.into_iter().chain(transition) {
                    if tx.send(RuntimeOutput::TerminalEvent(event)).is_err() && !cfg!(windows) {
                        break 'read_output;
                    }
                }
                if tx.send(RuntimeOutput::Stream(buf[..n].to_vec())).is_err() && !cfg!(windows) {
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
    drop(child_slot);
    release_pty(&handle);
    drop(entry);
    // tx dropped on scope exit → OutputStream::recv_timeout sees
    // Disconnected on the manager side, which is how the existing
    // forwarder thread knows to wind down.
}

// --- Helpers ------------------------------------------------------------

fn release_pty(handle: &SessionHandle) {
    drop(handle.writer.lock().expect("writer poisoned").take());
    drop(handle.master.lock().expect("master poisoned").take());
}

/// Shared by a session's reader thread and its output stream. The session
/// stays in the map, so the manager can still read its exit status, until
/// both are gone.
struct SessionEntry {
    sessions: Sessions,
    session_id: String,
    handle: Arc<SessionHandle>,
}

impl Drop for SessionEntry {
    fn drop(&mut self) {
        let mut sessions = self.sessions.lock().expect("PtyRuntime.sessions poisoned");
        // A respawn under the same id has replaced the entry and keeps it.
        if sessions
            .get(&self.session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.handle))
        {
            sessions.remove(&self.session_id);
        }
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
    let writer = writer
        .as_mut()
        .ok_or_else(|| RuntimeError::Msg(format!("session {session_id} has exited")))?;
    writer.write_all(bytes)?;
    writer.flush()?;
    handle
        .idle_detector
        .lock()
        .expect("idle detector poisoned")
        .on_input(bytes);
    Ok(())
}

pub(crate) fn interrupt_key(bytes: &[u8]) -> Option<u8> {
    match bytes {
        b"\x03" | b"\x1b[99;5u" => Some(CTRL_C_INTERRUPT),
        b"\x1b" | b"\x1b[27u" => Some(ESCAPE_INTERRUPT),
        _ => None,
    }
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

#[test]
fn command_summary_hides_shared_configuration_values() {
    let args = [
        "--settings",
        "secret settings",
        "--mcp-config",
        "secret mcp",
        "--plugin-dir",
        "private plugin path",
        "--plugin-dir=another private path",
        "--resume",
        "session-id",
    ]
    .map(str::to_owned);
    assert_eq!(
        format_command_summary("claude", &args),
        "claude --settings <configuration> --mcp-config <configuration> --plugin-dir <configuration> <configuration> --resume session-id"
    );
}

fn format_command_summary(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        let mut hidden = false;
        let args: Vec<_> = args
            .iter()
            .map(|arg| {
                if hidden {
                    hidden = false;
                    return "<configuration>";
                }
                hidden = matches!(arg.as_str(), "--settings" | "--mcp-config" | "--plugin-dir");
                if arg.starts_with("--settings=")
                    || arg.starts_with("--mcp-config=")
                    || arg.starts_with("--plugin-dir=")
                {
                    "<configuration>"
                } else {
                    arg
                }
            })
            .collect();
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
    let candidates = crate::repo::session::list_orphan_process_candidates(&conn)?;

    let mut signaled = Vec::new();
    for candidate in candidates {
        let session_id = candidate.session_id;
        let raw_pid = candidate.pid;
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
            candidate.runtime.as_deref(),
            candidate.command.as_deref(),
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
    let conn = pool.get()?;
    crate::repo::session::clear_stale_pids(&conn)?;
    Ok(0)
}

#[cfg(unix)]
fn clear_recorded_pid(conn: &rusqlite::Connection, session_id: &str, expected_pid: i64) {
    if let Err(error) = crate::repo::session::clear_recorded_pid(conn, session_id, expected_pid) {
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
                crate::runtimes::for_key(runtime)
                    .catalog()
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
mod tests;

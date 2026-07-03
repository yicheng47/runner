// Internal runtime abstraction for the session layer. The trait is the
// seam between the manager and whoever owns the terminal process. The
// current implementation is the in-process portable-pty runtime; a
// future platform-specific implementation can slot in without
// rewriting commands/frontend.
//
// Intentionally small. Add methods only when a caller needs them;
// don't pre-shape an API that's purely speculative.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Everything `spawn` needs that doesn't already live on a `Session`
/// row. The runtime never reads the DB — callers (mission/session
/// commands) gather the inputs and hand them in.
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    /// ULID of the `sessions` row that will own this runtime session.
    /// The runtime uses this for deterministic naming
    /// (`runner-<session_id>`), and persists nothing else about the
    /// row.
    pub session_id: String,
    /// Working directory the agent process should start in. None ⇒
    /// inherit the runtime's process cwd (rare in practice).
    pub cwd: Option<PathBuf>,
    /// The agent CLI command name (`claude`, `codex`, etc.) and its
    /// argv. PATH resolution happens inside the launch-script wrapper
    /// (Step 4), not here.
    pub command: String,
    pub args: Vec<String>,
    /// Composed environment for the agent process. The runtime layer
    /// passes this through unchanged; PATH composition / mission-bus
    /// env vars are decided by the caller.
    pub env: BTreeMap<String, String>,
    /// `true` for mission sessions (which get the bundled runner CLI
    /// plus mission-bus env). `false` for direct chats (off-bus
    /// invariant — see PR #51). The runtime uses this only to log /
    /// emit, not to make spawn decisions.
    pub mission: bool,
    /// Per-(mission, slot) shim directory containing a `runner` shim
    /// that bakes the mission-bus env vars and execs the bundled CLI.
    /// `None` for direct chats (off-bus invariant).
    pub shim_dir: Option<PathBuf>,
    /// `<app_data>/bin/` containing the bundled `runner` CLI. `None`
    /// for direct chats (also off-bus invariant — direct chats must
    /// not have the bundled CLI on PATH).
    pub bundled_bin_dir: Option<PathBuf>,
    /// Best-effort login-shell PATH from
    /// `shell_path::resolve_login_shell_env`, captured once by the
    /// manager at app start. `None` if the resolver
    /// failed/timed out — the launch script's fallback CLI dirs
    /// (`~/.local/bin`, `/opt/homebrew/bin`, etc.) cover the common
    /// cases regardless.
    pub shell_path: Option<String>,
    /// Initial pane size (cols, rows) — `xterm.js` reports its
    /// foreground grid on direct-chat spawn so the pane lays out at
    /// the right size before the first paint. `None` falls back to
    /// the runner config's `default-size`.
    pub initial_size: Option<(u16, u16)>,
    /// Where to run the agent (Windows+WSL fork): `Some("native")` runs
    /// the command directly on the Windows host; `Some("wsl")` / `None`
    /// wraps it in `wsl.exe`. The native runtime ignores this; the
    /// Windows shaper dispatches on it. See `session::wsl`.
    pub exec_target: Option<String>,
}

/// What `spawn` returns: the runtime-side identity persisted on the
/// `sessions` row. Under the in-process PTY runtime this is just the
/// runtime discriminator plus the session id used to look up the
/// live handle while the app process is running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSession {
    /// Discriminator for the runtime that produced this row.
    /// Currently `"native-pty"` for the portable-pty implementation.
    pub runtime: String,
    /// Session row id and key into the runtime's live-handle map.
    pub session_id: String,
}

/// Liveness snapshot of a runtime session. Returned by
/// `SessionRuntime::status` so the manager can reconcile the DB row
/// against what the runtime knows: a live child stays
/// `running`; a dead child with a captured exit code becomes
/// `stopped` (status 0) or `crashed` (non-zero); a missing session
/// (the runtime returns `Ok(None)`) is treated as
/// terminal-unavailable.
#[derive(Debug, Clone, Default)]
pub struct SessionStatus {
    /// `true` while the agent process is still attached to the PTY.
    /// Once the agent exits and the reader observes EOF, this flips
    /// to `false`.
    pub alive: bool,
    /// Exit code captured from the child process once the agent has
    /// exited. Only populated when `alive == false`.
    pub exit_code: Option<i32>,
    /// Process id of the child process when available.
    pub pid: Option<i32>,
    /// Name of the spawned command. Useful for diagnostics.
    pub command: Option<String>,
}

/// Latest-known availability of a runtime session, as inferred by
/// the forwarder from PTY-byte activity (issue #124). The router
/// projects this into a per-handle availability map; the workspace
/// rail dot reads off the same projection. Lives here (rather than
/// in `router/`) because the forwarder is the authoritative source
/// — the router consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStatus {
    Busy,
    Idle,
}

/// One unit of output produced by a runtime session. Raw stream bytes
/// are appended to xterm.js; `StatusTransition` is the forwarder's
/// busy/idle signal (issue #124) and never reaches xterm.js; the
/// SessionManager consumer routes it to the event log.
#[derive(Debug, Clone)]
pub enum RuntimeOutput {
    /// Live PTY bytes the agent wrote since the last `Stream` chunk.
    /// xterm.js **appends**.
    Stream(Vec<u8>),
    /// Forwarder-inferred busy/idle transition. `source` is
    /// `"forwarder"` for these synthetic events (the CLI's
    /// `runner status` verb emits `source: "agent"` directly into
    /// the log without going through this channel). Static-str
    /// because both producers' values are known at compile time.
    StatusTransition {
        state: RunnerStatus,
        source: &'static str,
    },
}

/// Receiver half of a runtime session's output channel. Returned
/// from `spawn`; the runtime is the sender. Backed by a
/// blocking `std::sync::mpsc` because the manager already runs
/// reader threads off `std::thread`; matching that avoids dragging
/// in a tokio runtime for the session layer.
///
/// Wrapped in a struct (rather than the bare `Receiver`) so a
/// `Drop` impl can flip a stop flag the runtime's forwarder thread
/// polls — without that, dropping the receiver while no bytes are
/// arriving leaves the forwarder blocked forever in `read()`,
/// which leaks one OS thread per detach. The wrapper trades the
/// `Receiver` API for explicit `recv_timeout`.
pub struct OutputStream {
    inner: std::sync::mpsc::Receiver<RuntimeOutput>,
    /// Set to true when this `OutputStream` is dropped. The
    /// runtime's reader thread polls this flag every tick and exits
    /// when it flips, releasing the PTY reader and `Sender` half of
    /// the channel.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OutputStream {
    /// Construct from a `Receiver` plus the stop flag the runtime
    /// already gave to its forwarder thread. Internal — only the
    /// runtime impl wires this; manager code drops in via
    /// `recv_timeout`.
    pub(crate) fn new(
        inner: std::sync::mpsc::Receiver<RuntimeOutput>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self { inner, stop }
    }

    /// Mirrors `Receiver::recv_timeout`. Used by the integration
    /// tests; the manager will reach for it once Step 9 wires the
    /// runtime in.
    pub fn recv_timeout(
        &self,
        dur: std::time::Duration,
    ) -> Result<RuntimeOutput, std::sync::mpsc::RecvTimeoutError> {
        self.inner.recv_timeout(dur)
    }

    /// Clone of the cancellation flag. Set this from outside the
    /// consumer thread to break it out of `recv_timeout` on the
    /// next tick, regardless of whether the channel has
    /// disconnected. Used by `SessionManager::kill` so kill
    /// doesn't hang waiting on the reader thread to observe EOF and
    /// drop its sender. That path is normally fast but can stall
    /// under load, and `kill` must not block the calling Tauri
    /// command indefinitely.
    pub fn stop_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.stop)
    }
}

impl Drop for OutputStream {
    fn drop(&mut self) {
        // Tell the forwarder to wake up on its next poll tick and
        // exit. The Sender half held by the forwarder thread also
        // gets `send`-Err on the next attempted send, but that
        // path only fires when bytes actually flow; the explicit
        // flag handles the "no bytes arriving, manager detached"
        // case so the thread doesn't sleep forever.
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Typed runtime errors. These bubble up through
/// `crate::error::Error` via the `From` impl so command code can `?`
/// across the boundary. v1 keeps the surface narrow: I/O failures
/// (from the master fd / writer half) and free-form `Msg(...)` for
/// every other condition the runtime wants to name.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Msg(String),
}

impl From<RuntimeError> for crate::error::Error {
    fn from(e: RuntimeError) -> Self {
        match e {
            RuntimeError::Io(err) => crate::error::Error::Io(err),
            other => crate::error::Error::Msg(other.to_string()),
        }
    }
}

pub type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

/// The session runtime trait. Frontend / Tauri commands never touch
/// this — they go through `SessionManager`, which in turn delegates
/// to a `dyn SessionRuntime` for the per-session PTY work.
pub trait SessionRuntime: Send + Sync {
    /// Start a fresh session. Returns the runtime-side ids to
    /// persist on the `sessions` row, plus the output channel the
    /// runtime will write `RuntimeOutput::Stream` into.
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)>;

    /// Best-effort terminate. The runtime is responsible for
    /// triggering whatever exit-status capture the manager needs;
    /// this method only signals.
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()>;

    /// Literal byte stream from xterm.js passthrough — the user is
    /// typing directly into the foreground terminal, or the manager
    /// is preserving the old paste path by writing prompt bytes
    /// unchanged before sending Enter.
    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()>;

    /// Named key. Examples: `"Enter"`, `"C-c"`, `"Up"`,
    /// `"Escape"`. The runtime translates names to PTY byte
    /// sequences. Caller is responsible for using a supported key
    /// name.
    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()>;

    /// Frontend resize event. The runtime is expected to debounce
    /// internally if multiple `resize` calls land back-to-back.
    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()>;

    /// Liveness probe used by the manager's exit reconciliation.
    /// `Ok(None)` means the runtime can't find the session — treat
    /// as terminal-unavailable. `Ok(Some(_))` means the PTY child is
    /// known; the caller branches on `SessionStatus.alive` and
    /// `exit_code`. Errors are reserved for transport failures.
    fn status(&self, session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>>;
}

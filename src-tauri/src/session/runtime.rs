#![allow(dead_code)] // Wired into SessionManager in Step 5+; foundation now.

// Internal runtime abstraction for the session layer (impl plan
// docs/impls/0004-tmux-session-runtime.md, Step 1). The trait is the
// seam between the command layer and whoever owns the terminal —
// `TmuxRuntime` for v1 (Step 5+); a future `NativePtyRuntime` slots
// in here for Windows or for the no-dependency mode without
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
#[derive(Debug, Clone)]
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
}

/// What `spawn`/`resume` returns: the runtime-side identifiers we
/// persist on the `sessions` row so a future process can reattach.
/// Everything is opaque to the caller; the runtime owns the schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSession {
    /// Discriminator for the runtime that produced this row.
    /// Currently always `"tmux"`. A future `NativePtyRuntime` would
    /// emit `"native-pty"`.
    pub runtime: String,
    /// `-L` label (tmux) — distinct sockets let the dev / test
    /// harness coexist with the production server. Persisted so
    /// reattach knows which socket to talk to.
    pub socket: String,
    /// `-s` session name. Always `runner-<SpawnSpec.session_id>` for
    /// the tmux runtime.
    pub session_name: String,
    /// Window id within the tmux session. The first window is
    /// `main`; we don't yet create others, but the column is here
    /// so the schema doesn't need to grow when we do.
    pub window: String,
    /// Pane id (e.g. `%3`). Pane ids survive index reshuffles —
    /// always persist these, never `:0.0`-style indexes.
    pub pane: String,
}

/// One unit of output produced by a runtime session. The manager
/// forwards these to xterm.js with **distinct semantics** for each
/// variant — collapsing them back into a single byte stream is the
/// duplicated-cells bug the plan calls out (Step 6: snapshot ≠
/// stream).
#[derive(Debug, Clone)]
pub enum RuntimeOutput {
    /// Attach-time snapshot. xterm.js **resets** its buffer to this
    /// content. Includes alternate-screen handling — for a
    /// Claude/Codex pane in alternate-screen mode, this is the
    /// current TUI render, not the pre-TUI scrollback.
    Replay(Vec<u8>),
    /// Live PTY bytes the agent wrote since the last `Stream`
    /// chunk. xterm.js **appends**. Sourced from `pipe-pane`.
    Stream(Vec<u8>),
}

/// Receiver half of a runtime session's output channel. Returned
/// from `spawn` / `resume`; the runtime is the sender. Backed by a
/// blocking `std::sync::mpsc` because the manager already runs
/// reader threads off `std::thread`; matching that avoids dragging
/// in a tokio runtime for the session layer.
///
/// Wrapped in a struct (rather than the bare `Receiver`) so a
/// `Drop` impl can flip a stop flag the runtime's forwarder thread
/// polls — without that, dropping the receiver while no bytes are
/// arriving leaves the forwarder blocked forever in `read()`,
/// which leaks one OS thread per detach. The wrapper trades the
/// `Receiver` API for explicit `recv_timeout` / `try_recv`
/// methods; callers that want the full receiver surface can take
/// `&self.inner` via the `as_receiver` accessor.
pub struct OutputStream {
    inner: std::sync::mpsc::Receiver<RuntimeOutput>,
    /// Set to true when this `OutputStream` is dropped. The
    /// runtime's forwarder thread polls this flag every tick and
    /// exits when it flips, releasing its FIFO read fd and
    /// `Sender` half of the channel.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OutputStream {
    /// Construct from a `Receiver` plus the stop flag the runtime
    /// already gave to its forwarder thread. Internal — only the
    /// runtime impl wires this; manager code drops in via
    /// `recv_timeout` / `try_recv`.
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

    /// Mirrors `Receiver::try_recv`.
    pub fn try_recv(&self) -> Result<RuntimeOutput, std::sync::mpsc::TryRecvError> {
        self.inner.try_recv()
    }

    /// Borrow the inner receiver for callers that need the full
    /// surface (iterator chains, select-style multiplexing).
    pub fn as_receiver(&self) -> &std::sync::mpsc::Receiver<RuntimeOutput> {
        &self.inner
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
/// across the boundary, but the variants stay typed at this layer
/// so the manager can branch on `TmuxNotFound` (show install hint)
/// vs. `TmuxFailed` (treat as transient) vs. `TmuxRequiresUnix`
/// (refuse to construct the runtime on Windows).
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// We're on Windows and tmux can't run. The Windows path is the
    /// future native-pty runtime; v1 ships macOS + Linux only.
    #[error("tmux runtime is not available on Windows; native-pty runtime is not yet shipped")]
    TmuxRequiresUnix,

    /// tmux binary not found in any of the searched locations. The
    /// list is included so the error surface explains where we
    /// looked.
    #[error(
        "tmux not found in any of {searched:?}; install tmux or set RUNNER_TMUX=/path/to/tmux"
    )]
    TmuxNotFound { searched: Vec<PathBuf> },

    /// A tmux subprocess returned non-zero. Captures stderr so the
    /// surfaced error includes whatever tmux printed.
    #[error("tmux {command} failed (exit {status}): {stderr}")]
    TmuxFailed {
        command: String,
        status: i32,
        stderr: String,
    },

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

/// The session runtime trait. `TmuxRuntime` (Step 5) is the only
/// implementer right now. Frontend / Tauri commands never touch
/// this — they go through `SessionManager`, which in turn delegates
/// to a `dyn SessionRuntime` for the per-pane work.
///
/// Output / input are split into distinct shapes by intent so
/// callers can't accidentally collapse a snapshot into the live
/// stream (Step 6 anti-pattern) or send a literal byte payload as a
/// paste (Step 7 anti-pattern).
pub trait SessionRuntime: Send + Sync {
    /// Start a fresh session. Returns the runtime-side ids to
    /// persist on the `sessions` row, plus the output channel the
    /// runtime will write `RuntimeOutput::Replay` (once) and
    /// `RuntimeOutput::Stream` (indefinitely) into.
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)>;

    /// Re-establish liveness for a session that already has runtime
    /// metadata persisted (app restart, route switch, etc.). Errors
    /// out if the underlying pane is gone — the manager treats that
    /// as `terminal-unavailable` and marks the session stopped.
    /// Returns a fresh output channel: a `Replay` snapshot of the
    /// pane's current state arrives first, then live `Stream`
    /// events resume.
    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream>;

    /// Best-effort terminate. The runtime is responsible for
    /// triggering whatever exit-status capture the manager needs;
    /// this method only signals.
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()>;

    /// Multi-line prompt paste. Runtime applies bracketed-paste
    /// (`paste-buffer -p -r -d`); LF stays literal so the agent
    /// sees one paste, not one submit per line. The runtime does
    /// **not** submit — the manager follows up with `send_key`
    /// when it wants the agent to act, so the timing is explicit
    /// (Step 7's `paste_after` readiness wait lives on the
    /// manager).
    fn paste(&self, session: &RuntimeSession, payload: &[u8]) -> RuntimeResult<()>;

    /// Literal byte stream from xterm.js passthrough — the user is
    /// typing directly into the foreground terminal. Runtime uses
    /// `send-keys -l -- <bytes>` so the bytes arrive as keystrokes
    /// without bracketed-paste markers.
    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()>;

    /// Named key. Examples: `"Enter"`, `"C-c"`, `"Up"`,
    /// `"Escape"`. Runtime uses `send-keys -t=<pane> <key>` (no
    /// `-l`, no `--`, so tmux's key-name lookup runs). Caller is
    /// responsible for using a name tmux understands; the runtime
    /// validates the name shape but does not enumerate every
    /// possible key.
    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()>;

    /// Frontend resize event. The runtime is expected to debounce
    /// internally if multiple `resize` calls land back-to-back.
    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()>;
}

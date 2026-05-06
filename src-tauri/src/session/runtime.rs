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

/// Cursor for incremental output replay. Opaque to callers — the
/// runtime decides what counts as a position. For tmux this is a
/// pane history offset; for a future runtime it might be a byte
/// count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptureCursor {
    pub history_lines: u64,
}

/// One increment of captured output, plus the cursor to ask for the
/// next one with.
#[derive(Debug)]
pub struct CaptureChunk {
    pub bytes: Vec<u8>,
    pub cursor: CaptureCursor,
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
pub trait SessionRuntime: Send + Sync {
    /// Start a fresh session. Returns the runtime-side ids to
    /// persist on the `sessions` row.
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<RuntimeSession>;

    /// Re-establish liveness for a session that already has runtime
    /// metadata persisted (app restart, route switch, etc.). Errors
    /// out if the underlying pane is gone — the manager treats that
    /// as `terminal-unavailable` and marks the session stopped.
    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<()>;

    /// Best-effort terminate. The runtime is responsible for
    /// triggering whatever exit-status capture the manager needs;
    /// this method only signals.
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()>;

    /// Inject bytes into the running pane. Today the trait is
    /// single-shape: paste-style multi-line prompts get
    /// `paste-buffer -p -r -d` semantics (bracketed paste, LF stays
    /// literal); literal byte streams (xterm.js passthrough) get
    /// `send-keys -l --`. The runtime decides which based on
    /// content / call site. Specialized methods can grow on this
    /// trait if the manager ever needs explicit control.
    fn send_input(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()>;

    /// Pull the next chunk of output past `cursor`. The runtime
    /// decides whether this is a `pipe-pane` drain, a
    /// `capture-pane` snapshot, or something else entirely; the
    /// manager just forwards `CaptureChunk.bytes` as
    /// `session/output`.
    fn capture_since(
        &self,
        session: &RuntimeSession,
        cursor: CaptureCursor,
    ) -> RuntimeResult<CaptureChunk>;

    /// Frontend resize event. The runtime is expected to debounce
    /// internally if multiple `resize` calls land back-to-back.
    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()>;
}

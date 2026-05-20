// Session layer — spawns and controls each runner's local CLI process
// via the `SessionRuntime` trait. v1 implementation:
//   * `pty_runtime::PtyRuntime` (in-process, docs/impls/0011)
//
// The `manager` submodule owns DB persistence, output buffering, and the
// kill / resume state machine; it delegates every PTY-side operation to
// the runtime. The tmux-backed runtime that powered earlier versions
// has been retired — see docs/impls/0011-pty-host-terminal-runtime.md
// for the rationale.

pub mod codex_capture;
pub mod launch;
pub mod manager;
#[cfg(unix)]
pub mod pty_runtime;
pub mod runtime;

pub use manager::{CompleteSpawnOutcome, PendingMissionSpawn, SessionManager};

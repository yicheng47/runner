// Session layer — spawns and controls each runner's local CLI process
// via the `SessionRuntime` trait. Two implementations:
//   * `tmux_runtime::TmuxRuntime` (legacy, docs/impls/0004)
//   * `pty_runtime::PtyRuntime` (in-process, docs/impls/0011)
// Selection at app startup via `RUNNER_SESSION_RUNTIME=tmux|pty`.
//
// The `manager` submodule owns DB persistence, output buffering, and the
// kill / resume state machine; it delegates every PTY-side operation to
// the runtime.

pub mod codex_capture;
pub mod launch;
pub mod manager;
#[cfg(unix)]
pub mod pty_runtime;
pub mod runtime;
pub mod tmux;
#[cfg(unix)]
pub mod tmux_runtime;

pub use manager::SessionManager;

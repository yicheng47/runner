// Session layer — spawns and controls each runner's local CLI process
// via a tmux pane (the `SessionRuntime` trait → `TmuxRuntime`).
//
// The `manager` submodule owns DB persistence, output buffering, and the
// kill / resume state machine; it delegates every PTY-side operation to
// the runtime. `tmux` + `tmux_runtime` + `launch` together form the
// runtime layer (docs/impls/0004-tmux-session-runtime.md).

pub mod codex_capture;
pub mod launch;
pub mod manager;
pub mod runtime;
pub mod tmux;
#[cfg(unix)]
pub mod tmux_runtime;

pub use manager::SessionManager;

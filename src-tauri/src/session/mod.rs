// Session runtime — spawns and controls each runner's local CLI process
// via a pseudo-terminal (portable-pty). See docs/impls/0001-v0-mvp.md §C6.
//
// The `manager` submodule owns the per-process PTY machinery. The app wires
// it into AppState and calls into it from mission/session Tauri commands.
//
// `runtime` and `tmux` are the foundation for the tmux migration spec'd in
// docs/impls/0004-tmux-session-runtime.md. They aren't yet wired into
// `manager` — that lands in Step 5+ and is gated on the runtime metadata
// schema migration.

pub mod codex_capture;
pub mod launch;
pub mod manager;
pub mod runtime;
pub mod tmux;

pub use manager::SessionManager;

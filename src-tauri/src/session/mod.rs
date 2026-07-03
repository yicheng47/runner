// Session layer — spawns and controls each runner's local CLI process
// via the `SessionRuntime` trait. v1 implementation:
//   * `pty_runtime::PtyRuntime` (in-process, docs/impls/archive/0011)
//
// The `manager` submodule owns DB persistence, output buffering, and the
// kill / resume state machine; it delegates every PTY-side operation to
// the runtime. The tmux-backed runtime that powered earlier versions
// has been retired — see docs/impls/archive/0011-pty-host-terminal-runtime.md
// for the rationale.

pub mod codex_capture;
pub mod launch;
pub mod manager;
// portable-pty's ConPTY backend compiles on Windows, so the runtime is
// no longer unix-gated. On Windows the runtime is constructed with a
// WSL command shaper (see `wsl`) that wraps each spawn in `wsl.exe`.
pub mod pty_runtime;
pub mod runtime;
#[cfg(windows)]
pub mod wsl;

pub use manager::{CompleteSpawnOutcome, PendingMissionSpawn, SessionManager};

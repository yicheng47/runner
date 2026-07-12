// Tauri command handlers exposed to the frontend.
//
// Each submodule splits into pure-SQL functions (unit-testable against an
// in-memory pool) plus thin `#[tauri::command]` wrappers that pull a
// connection from the r2d2 pool and delegate. See docs/impls/archive/0001-v0-mvp.md §C2.

pub mod app;
pub mod crew;
pub mod folder;
pub mod mcp;
pub mod mission;
pub mod runner;
pub mod runtime;
pub mod session;
pub mod slot;
pub mod tab;
pub mod window;

// Application operations — the bodies behind every frontend command.
//
// Each submodule splits into pure-SQL functions (unit-testable against an
// in-memory pool) plus state-level functions over `AppCore` that the thin
// `#[tauri::command]` wrappers in src-tauri (and the MCP tools) delegate
// to. See docs/impls/archive/0001-v0-mvp.md §C2 and impl 0031 Phase 2.

pub mod crew;
pub mod folder;
pub mod mcp;
pub mod mission;
pub mod project;
pub mod runner;
pub mod runtime;
pub mod session;
pub mod slot;
pub mod tab;

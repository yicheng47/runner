// Tauri command handlers exposed to the frontend.
//
// Since impl 0031 Phase 2 the bodies live in `runner_app::ops`; every
// wrapper here is a one-line delegation that keeps the invoke contract
// (command name, argument names, return shape) stable. Only `app` and
// `window` contain real logic — they drive Tauri-specific surfaces
// (webview windows, the opener plugin) that have no core equivalent.

pub mod app;
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
pub mod window;

// Runner's UI-agnostic application core (impl 0031 Phase 2).
//
// Everything a frontend needs to run Runner lives here: the SQLite layer,
// the PTY session manager, the per-mission event bus + signal router, the
// MCP server, and the command bodies (`ops`). Frontends — the Tauri app
// today, the native GPUI binary in Phase 3 — are thin adapters: they build
// an `AppCore`, subscribe to its event channel, and delegate their command
// surface to `ops::*`.

pub mod cli_install;
pub mod db;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod mcp;
pub mod model;
pub mod ops;
pub mod repo;
pub mod router;
pub mod session;
pub mod shell_path;
pub mod windows;

use std::path::PathBuf;
use std::sync::Arc;

use events::EventChannel;
use session::manager::CoreSessionEvents;

/// Shared application state. One instance per process, cheap to clone
/// (every field is an `Arc` or small value) — the Tauri layer stores it in
/// `app.manage`, the MCP handler clones it per connection.
#[derive(Clone)]
pub struct AppCore {
    pub db: Arc<db::DbPool>,
    /// Root of the app's per-user data tree — `$APPDATA/runner/` on real
    /// installs, a tempdir in tests. Mission commands resolve event-log paths
    /// relative to this via `runner_core::event_log::path`.
    pub app_data_dir: PathBuf,
    /// Live per-mission session manager. Created at app
    /// start, shared across all frontends and the per-session
    /// forwarder threads it spawns.
    pub sessions: Arc<session::SessionManager>,
    /// Live per-mission event-bus watchers. Mounted by `mission_start` once
    /// the opening events are durable; unmounted by `mission_stop` and on
    /// any rollback path.
    pub buses: Arc<event_bus::BusRegistry>,
    /// Live per-mission signal routers. Mounted alongside the bus so the
    /// router observes the bootstrap `mission_goal` event during initial
    /// replay and pushes the launch prompt into the lead's stdin.
    pub routers: Arc<router::RouterRegistry>,
    /// MCP server lifecycle handle (impl 0013). Unix socket listener
    /// that external clients connect to via the `runner-mcp` bridge.
    pub mcp: Arc<mcp::McpHandle>,
    /// Cross-window coordination map (impl 0018). Tracks which subject
    /// (mission / direct chat) each window is looking at + when it
    /// was last focused, so exactly one window owns a duplicated subject's
    /// PTY.
    pub windows: Arc<windows::WindowRegistry>,
    /// Broadcast channel every app-observable event flows through. The
    /// frontend subscribes and forwards to its own event surface (the
    /// Tauri layer re-emits to the webview verbatim).
    pub events: EventChannel,
    /// The application's user-facing version (the Tauri crate's
    /// `CARGO_PKG_VERSION`, which the release bump updates). Advertised by
    /// the MCP server's `ServerInfo`.
    pub app_version: String,
}

impl AppCore {
    /// Session-event sink for spawn/resume/inject call sites. Holds the
    /// manager as `Weak` because instances get stored inside the manager's
    /// own session state (codex capture context) — a strong ref would cycle.
    pub fn session_events(&self) -> CoreSessionEvents {
        CoreSessionEvents::new(
            Arc::clone(&self.db),
            Arc::downgrade(&self.sessions),
            Arc::clone(&self.windows),
            self.events.clone(),
        )
    }

    /// Broadcast the current window→subject map. Called after every window
    /// registry mutation so all windows converge on a consistent picture of
    /// who owns what. Broadcast, not targeted: each window filters by its
    /// own subject (spec decision 5).
    pub fn broadcast_focus_map(&self) {
        self.events
            .emit("window_focus_map", &self.windows.snapshot());
    }
}

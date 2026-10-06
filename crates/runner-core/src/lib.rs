// Runner shared core — types and event-log primitives used by both the GPUI
// app binary and the `runner` CLI.

pub mod app_paths;
pub mod cli_install;
pub mod command_install;
pub mod daemon_process;
pub mod error;
pub mod event_log;
pub mod logging;
pub mod model;
pub mod runtime;
pub use runtime::Runtime;

pub use error::{Error, Result};
pub use event_log::{EventLog, EVENTS_FILENAME};
pub use model::{Event, EventDraft, EventKind, SignalType, Timestamp, Ulid};

pub const RUNNER_SKILL_ROOTS: &[&str] = &[
    ".claude/skills",
    ".agents/skills",
    ".trae/skills",
    ".gemini/antigravity-cli/skills",
];
pub const RUNNER_SKILL_MARKER: &str = ".runner-managed";

pub const fn runner_skill_name(debug: bool) -> &'static str {
    if debug {
        "runner-dev"
    } else {
        "runner"
    }
}

pub mod protocol;

pub mod version;

// Runner shared core — types and event-log primitives used by both the GPUI
// app binary and the `runner` CLI.

pub mod app_paths;
pub mod error;
pub mod event_log;
pub mod model;

pub use error::{Error, Result};
pub use event_log::{EventLog, EVENTS_FILENAME};
pub use model::{Event, EventDraft, EventKind, SignalType, Timestamp, Ulid};

pub const RUNNER_SKILL_ROOTS: &[&str] = &[".claude/skills", ".agents/skills", ".trae/skills"];
pub const RUNNER_SKILL_MARKER: &str = ".runner-managed";

pub const fn runner_skill_name(debug: bool) -> &'static str {
    if debug {
        "runner-dev"
    } else {
        "runner"
    }
}

/// Socket tools exposed by Runner. The backend registry and the `runner`
/// command map both assert against this list so a new tool cannot ship
/// without a CLI path.
pub const RUNNER_TOOL_NAMES: &[&str] = &[
    "crew_list",
    "crew_get",
    "crew_create",
    "crew_update",
    "crew_delete",
    "role_list",
    "role_get",
    "role_get_by_handle",
    "role_create",
    "role_update",
    "role_delete",
    "slot_list",
    "slot_create",
    "slot_update",
    "slot_delete",
    "slot_set_lead",
    "slot_reorder",
    "project_list",
    "project_get",
    "project_create",
    "project_rename",
    "project_delete",
    "mission_list",
    "mission_get",
    "mission_list_summary",
    "mission_feed",
    "mission_status",
    "mission_start",
    "mission_stop",
    "mission_resume",
    "mission_archive",
    "mission_unarchive",
    "mission_pin",
    "mission_rename",
    "mission_set_project",
    "mission_post",
    "mission_signal",
    "session_list",
    "session_get",
    "session_stop",
    "session_archive",
    "session_start_direct",
    "session_resume",
    "session_restart",
];

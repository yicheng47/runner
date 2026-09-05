// Event log primitives — append-only NDJSON, monotonic ULIDs, path helpers.
// Consumed by the GPUI app's backend and by the standalone `runner` CLI.

pub mod log;
pub mod path;
pub mod ulid;

pub use log::{EventLog, LogEntry, SkipReport, TryAppendError};
pub use path::{crew_dir, events_path, mission_dir, EVENTS_FILENAME};
pub use ulid::UlidGen;

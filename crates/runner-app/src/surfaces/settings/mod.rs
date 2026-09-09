pub(crate) mod about;
pub(crate) mod agents;
pub(crate) mod archived;
pub(crate) mod diagnostics;
pub(crate) mod skills;
#[cfg(target_os = "macos")]
pub(crate) mod updates;
#[cfg(windows)]
#[path = "updates_windows.rs"]
pub(crate) mod updates;

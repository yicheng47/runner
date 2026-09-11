pub(crate) mod about;
pub(crate) mod agents;
pub(crate) mod archived;
pub(crate) mod diagnostics;
pub(crate) mod mcp;
pub(crate) mod skills;
pub(crate) mod theme_preview;
#[cfg(target_os = "macos")]
pub(crate) mod updates;
#[cfg(windows)]
#[path = "updates_windows.rs"]
pub(crate) mod updates;

/// Raised by a settings editor's Save, success or failure, so the app shell
/// can show it as a toast.
pub(crate) struct SaveNotice {
    pub message: String,
    pub tone: crate::toast::ToastTone,
}

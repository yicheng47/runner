pub(crate) mod about;
pub(crate) mod agents;
pub(crate) mod archived;
pub(crate) mod claude_accounts;
pub(crate) mod diagnostics;
pub(crate) mod mcp;
pub(crate) mod notifications;
pub(crate) mod remote;
pub(crate) mod skills;
pub(crate) mod theme_preview;
#[cfg(target_os = "macos")]
pub(crate) mod updates;
#[cfg(windows)]
#[path = "updates_windows.rs"]
pub(crate) mod updates;
#[cfg(target_os = "linux")]
#[path = "updates_linux.rs"]
pub(crate) mod updates;

/// Raised by a settings editor's Save, success or failure, so the app shell
/// can show it as a toast.
pub(crate) struct SaveNotice {
    pub message: runner_app::i18n::Text,
    pub tone: crate::toast::ToastTone,
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandPlatform {
    Unix,
    Windows,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandInstallInputs {
    pub home: PathBuf,
    pub login_path: String,
    pub system_path: String,
    pub sidecar: PathBuf,
    pub local_bin: PathBuf,
    pub system_bin: PathBuf,
    pub system_bin_writable: bool,
    pub debug: bool,
    pub platform: CommandPlatform,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandActionOutcome {
    Installed(PathBuf),
    AlreadyInstalled(PathBuf),
    Removed(PathBuf),
    NotInstalled,
    Foreign(PathBuf),
    NoTarget,
    Cancelled,
    Unsupported,
}

pub use crate::command_install::{runner_command_name, RunnerCommandState, RunnerCommandStatus};
pub fn force_escalated_command_install() -> bool {
    #[cfg(debug_assertions)]
    {
        force_escalated_setting(
            true,
            std::env::var("RUNNER_COMMAND_INSTALL_FORCE_ESCALATED")
                .ok()
                .as_deref(),
        )
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

pub const MCP_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};
pub fn force_escalated_setting(debug: bool, value: Option<&str>) -> bool {
    debug && value == Some("1")
}

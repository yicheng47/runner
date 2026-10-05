use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginShellEnv {
    pub path: Option<String>,
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOutcome {
    Ok,
    WindowsRegistry,
    WindowsRegistryError,
    Timeout,
    SpawnError,
    EmptyCapture,
    NoShell,
}

impl DiscoveryOutcome {
    pub fn is_success(self) -> bool {
        matches!(self, Self::Ok | Self::WindowsRegistry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryResult {
    pub shell: Option<String>,
    pub outcome: DiscoveryOutcome,
    pub duration_ms: u64,
    pub env: LoginShellEnv,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DiscoverySnapshot {
    pub checking: bool,
    pub result: Option<DiscoveryResult>,
    pub shell_env: LoginShellEnv,
}

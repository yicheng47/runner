use serde::{Deserialize, Serialize};

pub const DEFAULT_HEARTBEAT_ENDPOINT: &str = "https://runnersh.dev/api/heartbeat";
pub const PRIVACY_GUIDE_URL_EN: &str = "https://runnersh.dev/docs/guides/privacy/";
pub const PRIVACY_GUIDE_URL_ZH: &str = "https://runnersh.dev/zh-CN/docs/guides/privacy/";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatMeta {
    pub active_runtimes: Vec<String>,
    pub used_mission_7d: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub install_id: String,
    pub version: String,
    pub build_stamp: String,
    pub channel: String,
    pub os: String,
    pub arch: String,
    pub remote_enabled: bool,
    pub meta: HeartbeatMeta,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryStatus {
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub stored_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_override: Option<String>,
    #[serde(default)]
    pub dev_build: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    pub payload_preview: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sent_at: Option<String>,
}

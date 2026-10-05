use super::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub name: String,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentUsage {
    pub windows: Vec<UsageWindow>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnavailableReason {
    SignIn,
    KeychainDenied,
    KeychainUnavailable,
    ClaudeUnreachable,
    CodexNoAnswer,
    AntigravityNoAnswer,
    InvalidResponse,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub runtimes: std::collections::HashMap<Runtime, RuntimeUsage>,
    pub last_fetch_at: Option<DateTime<Utc>>,
    pub refreshing: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuntimeUsage {
    pub value: Option<AgentUsage>,
    pub error: Option<UnavailableReason>,
}

impl UsageSnapshot {
    pub fn get(&self, runtime: Runtime) -> Option<&AgentUsage> {
        self.runtimes.get(&runtime)?.value.as_ref()
    }
    pub fn error(&self, runtime: Runtime) -> Option<UnavailableReason> {
        self.runtimes.get(&runtime)?.error
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefreshReason {
    Launch,
    Schedule,
    Button,
    Open,
}

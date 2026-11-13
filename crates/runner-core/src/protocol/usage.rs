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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<UsageCredits>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCredits {
    pub balance: Option<String>,
    pub has_credits: bool,
    pub unlimited: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_accept_old_window_only_values_and_preserve_credit_precision() {
        let old = serde_json::json!({"windows": [], "updated_at": "2026-11-12T08:00:00Z"});
        let mut usage: AgentUsage = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(usage.credits, None);
        assert_eq!(serde_json::to_value(&usage).unwrap(), old);
        usage.credits = Some(UsageCredits {
            balance: Some("1250.750012345678901234".into()),
            has_credits: true,
            unlimited: false,
        });
        let snapshot = UsageSnapshot {
            runtimes: [(
                Runtime::Codex,
                RuntimeUsage {
                    value: Some(usage.clone()),
                    error: None,
                },
            )]
            .into(),
            ..UsageSnapshot::default()
        };
        let decoded: UsageSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(decoded.get(Runtime::Codex), Some(&usage));
    }
}

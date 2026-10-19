use super::{StopReason, DEFAULT_ORIGIN};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAccount {
    pub username: String,
    pub display_name: String,
}
impl From<super::Account> for RemoteAccount {
    fn from(value: super::Account) -> Self {
        Self {
            username: value.username,
            display_name: value.display_name,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteState {
    #[default]
    Off,
    Enrolling {
        user_code: String,
        verification_uri: String,
        verification_uri_complete: String,
        expires_at: DateTime<Utc>,
    },
    Confirming {
        account: RemoteAccount,
        expires_at: DateTime<Utc>,
    },
    Connecting {
        attempt: u32,
    },
    Connected {
        since: DateTime<Utc>,
    },
    Reconnecting {
        attempt: u32,
        next_retry_at: DateTime<Utc>,
    },
    Paused,
    Stopped {
        reason: StopReason,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteStatus {
    pub state: RemoteState,
    pub origin: String,
    pub account: Option<RemoteAccount>,
    pub host_name: Option<String>,
    pub host_id: Option<String>,
    pub fingerprint: Option<String>,
    pub last_error: Option<String>,
}
impl Default for RemoteStatus {
    fn default() -> Self {
        Self {
            state: RemoteState::Off,
            origin: DEFAULT_ORIGIN.into(),
            account: None,
            host_name: None,
            host_id: None,
            fingerprint: None,
            last_error: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RemoteLogout {
    Removed,
    StillListed { error: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteTrust {
    pub fingerprint: Option<String>,
    pub pin_set: bool,
    pub pin_locked: bool,
    pub devices: Vec<PairedDevice>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub fingerprint: String,
    pub name: String,
    pub public_key: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

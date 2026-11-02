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

/// What a paired device says it is. The value is self-reported in the companion `Hello`, so it only
/// chooses how a device is shown; it never decides what the device may do.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    #[default]
    Unknown,
    Phone,
    Computer,
}
impl DeviceKind {
    pub fn from_client(client: &str) -> Self {
        match client {
            "ios" => Self::Phone,
            "runner" => Self::Computer,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairedDevice {
    pub fingerprint: String,
    pub name: String,
    pub public_key: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub kind: DeviceKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_names_map_to_kinds_and_anything_else_is_unknown() {
        assert_eq!(DeviceKind::from_client("ios"), DeviceKind::Phone);
        assert_eq!(DeviceKind::from_client("runner"), DeviceKind::Computer);
        for client in ["", "iOS", "android", "test", "runner "] {
            assert_eq!(DeviceKind::from_client(client), DeviceKind::Unknown);
        }
    }

    #[test]
    fn a_device_saved_before_kinds_existed_loads_as_unknown() {
        let device: PairedDevice = serde_json::from_str(
            r#"{"fingerprint":"ab","name":"Phone","public_key":"k",
                "paired_at":"2026-10-01T00:00:00Z","last_seen_at":"2026-10-02T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(device.kind, DeviceKind::Unknown);
        assert!(!device.connected);
    }
}

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

mod client;
pub use client::*;
pub mod identity;
pub use identity::HostInfo;

pub const DEFAULT_ORIGIN: &str = "https://runner.yeelinks.com.cn";
pub const SUBPROTOCOL: &str = "runner-relay.v1";
pub const RPC_PATH: &str = "/api/runner_server.v1.HostEnrollmentService/";
pub const MAX_CONTROL_SIZE: usize = 64 * 1024;
pub const MAX_RELAY_SIZE: usize = crate::companion::channel::MAX_FRAME + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinError {
    Format,
}

impl PinError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Format => "The PIN must be exactly six digits.",
        }
    }
}

pub fn validate_pin_format(pin: &str) -> Result<(), PinError> {
    if pin.len() != 6 || !pin.bytes().all(|digit| digit.is_ascii_digit()) {
        return Err(PinError::Format);
    }
    Ok(())
}

pub fn is_weak_pin(pin: &str) -> bool {
    if validate_pin_format(pin).is_err() {
        return false;
    }
    let digits = pin.as_bytes();
    digits.iter().all(|digit| *digit == digits[0])
        || digits
            .windows(2)
            .all(|pair| pair[1] == b'0' + (pair[0] - b'0' + 1) % 10)
        || digits
            .windows(2)
            .all(|pair| pair[1] == b'0' + (pair[0] - b'0' + 9) % 10)
        || [
            "121212", "123123", "112233", "123321", "111222", "000111", "159753", "147258",
            "789456", "696969", "101010", "112358",
        ]
        .contains(&pin)
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(pub String);
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

pub struct HostKey {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}
impl HostKey {
    pub fn from_private(encoded: &str) -> Result<Self, &'static str> {
        use snow::resolvers::{CryptoResolver, DefaultResolver};
        let private = STANDARD.decode(encoded).map_err(|_| "Invalid host key")?;
        if private.len() != 32 {
            return Err("Invalid host key");
        }
        let mut dh = DefaultResolver
            .resolve_dh(&snow::params::DHChoice::Curve25519)
            .ok_or("Could not restore host key")?;
        dh.set(&private);
        Ok(Self {
            private,
            public: dh.pubkey().to_vec(),
        })
    }
    pub fn generate() -> Result<Self, snow::Error> {
        let params = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
        let key = snow::Builder::new(params).generate_keypair()?;
        Ok(Self {
            private: key.private,
            public: key.public,
        })
    }
}
impl fmt::Debug for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostKey([redacted])")
    }
}

pub fn fingerprint(public: &[u8]) -> String {
    Sha256::digest(public)[..10]
        .chunks(2)
        .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn encode_key(public: &[u8]) -> String {
    STANDARD.encode(public)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub username: String,
    pub display_name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartRequest {
    pub host: HostInfo,
    pub host_key: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartResponse {
    pub device_code: Secret,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i32,
    pub interval: i32,
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PollRequest {
    pub device_code: Secret,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PollResponse {
    pub state: EnrollmentState,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub host_id: String,
    #[serde(skip_serializing_if = "secret_empty")]
    pub credential: Secret,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub relay_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_expires_at: Option<String>,
}
fn secret_empty(s: &Secret) -> bool {
    s.0.is_empty()
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EnrollmentState {
    #[serde(rename = "ENROLLMENT_STATE_PENDING")]
    Pending,
    #[serde(rename = "ENROLLMENT_STATE_DENIED")]
    Denied,
    #[serde(rename = "ENROLLMENT_STATE_EXPIRED")]
    Expired,
    #[serde(rename = "ENROLLMENT_STATE_APPROVED")]
    Approved,
    #[default]
    #[serde(other, rename = "ENROLLMENT_STATE_UNSPECIFIED")]
    Unspecified,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Empty {}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcError {
    pub code: String,
    pub message: Secret,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Hello {
        #[serde(default)]
        host: HostInfo,
        #[serde(default)]
        features: Vec<String>,
    },
    Welcome {
        #[serde(default, rename = "hostId")]
        host_id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        account: Account,
    },
    #[serde(rename = "channelOpen")]
    ChannelOpen { channel: u32 },
    #[serde(rename = "channelClosed")]
    ChannelClosed { channel: u32 },
    #[serde(rename = "closeChannel")]
    CloseChannel { channel: u32 },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Removed,
    InUseElsewhere,
    UpdateRequired,
    Denied,
    Expired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Stop(StopReason),
    Backoff,
    After(u64),
    Restart,
}
pub fn close_action(code: u16) -> Retry {
    match code {
        4001 => Retry::Stop(StopReason::InUseElsewhere),
        4003 => Retry::Stop(StopReason::Removed),
        1012 => Retry::Restart,
        _ => Retry::Backoff,
    }
}
pub fn upgrade_action(status: u16, retry_after: Option<u64>) -> Retry {
    match status {
        400 | 403 => Retry::Stop(StopReason::UpdateRequired),
        401 => Retry::Stop(StopReason::Removed),
        429 => Retry::After(retry_after.unwrap_or(60)),
        _ => Retry::Backoff,
    }
}

pub fn redact(value: &str) -> String {
    let mut out = String::new();
    let mut remaining = value;
    while let Some(at) = remaining.find("rhc_") {
        out.push_str(&remaining[..at]);
        out.push_str("[redacted]");
        let token = &remaining[at + 4..];
        let end = token
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(token.len());
        remaining = &token[end..];
    }
    out.push_str(remaining);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn round_trip<T: serde::de::DeserializeOwned + Serialize>(name: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/remote/fixtures")
            .join(name);
        let data = std::fs::read(path).unwrap();
        let value: T = serde_json::from_slice(&data).unwrap();
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&data).unwrap(),
            "{name}"
        );
    }
    #[test]
    fn service_golden_fixtures() {
        round_trip::<StartRequest>("start_request.json");
        round_trip::<StartResponse>("start_response.json");
        round_trip::<PollRequest>("poll_request.json");
        for name in [
            "poll_approved.json",
            "poll_pending.json",
            "poll_denied.json",
            "poll_expired.json",
        ] {
            round_trip::<PollResponse>(name);
        }
        round_trip::<RpcError>("rpc_error.json");
        round_trip::<Empty>("signout.json");
        round_trip::<Control>("hello.json");
        round_trip::<Control>("welcome.json");
        for name in [
            "channel_open.json",
            "channel_closed.json",
            "close_channel.json",
        ] {
            round_trip::<Control>(name);
        }
    }
    #[test]
    fn identity_and_secrets() {
        let key: Vec<u8> = (0..32).collect();
        assert_eq!(fingerprint(&key), "630d cd29 66c4 3366 9112");
        assert_eq!(
            encode_key(&key),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
        );
        let a = HostKey::generate().unwrap();
        let b = HostKey::generate().unwrap();
        assert!(!format!("{a:?}").contains(&encode_key(&a.private)));
        assert!(valid_credential(&format!("rhc_{}", "x".repeat(43))));
        for bad in ["rhc_abc", "rhc_", "rhc_secret with spaces"] {
            assert!(!valid_credential(bad));
        }
        assert_eq!(a.private.len(), 32);
        assert_eq!(a.public.len(), 32);
        assert_ne!(a.public, b.public);
        assert_eq!(
            format!("{:?}", Secret("device-secret".into())),
            "[redacted]"
        );
        assert_eq!(
            redact("Bearer rhc_abc-123 and rhc_xyz!"),
            "Bearer [redacted] and [redacted]!"
        );
    }
    #[test]
    fn weak_pin_patterns_are_detected_without_rejecting_six_digit_pins() {
        for pin in [
            "000000", "111111", "222222", "333333", "444444", "555555", "666666", "777777",
            "888888", "999999", "012345", "123456", "456789", "890123", "654321", "987654",
            "210987", "121212", "123123", "112233", "123321", "111222", "000111", "159753",
            "147258", "789456", "696969", "101010", "112358",
        ] {
            assert!(is_weak_pin(pin));
            assert!(validate_pin_format(pin).is_ok());
        }
        for pin in ["001234", "019438", "903617"] {
            assert!(!is_weak_pin(pin));
            assert!(validate_pin_format(pin).is_ok());
        }
        for pin in ["", "12345", "1234567", "１２３４５６", "12345a", " 123456"] {
            assert!(!is_weak_pin(pin));
            assert_eq!(validate_pin_format(pin), Err(PinError::Format));
        }
    }

    #[test]
    fn future_fields_and_types_are_ignored() {
        assert!(matches!(
            serde_json::from_str::<Control>(r#"{"type":"future","secret":"ignored"}"#).unwrap(),
            Control::Unknown
        ));
        assert_eq!(
            serde_json::from_str::<PollResponse>("{}").unwrap().state,
            EnrollmentState::Unspecified
        );
    }
}

mod origin;
mod status;
pub use origin::{canonical_origin, same_origin, valid_relay_url};
pub use status::{
    PairedDevice, RemoteAccount, RemoteLogout, RemoteState, RemoteStatus, RemoteTrust,
};

pub fn valid_credential(value: &str) -> bool {
    value.strip_prefix("rhc_").is_some_and(|token| {
        token.len() == 43
            && token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    })
}

#[cfg(test)]
mod host_key_tests {
    use super::*;
    #[test]
    fn restores_public_key_and_rejects_malformed_private_keys() {
        let key = HostKey::generate().unwrap();
        let restored = HostKey::from_private(&encode_key(&key.private)).unwrap();
        assert_eq!(restored.public, key.public);
        assert_eq!(fingerprint(&restored.public), fingerprint(&key.public));
        assert!(HostKey::from_private("not base64").is_err());
        assert!(HostKey::from_private(&encode_key(&[1; 31])).is_err());
        assert!(!format!("{restored:?}").contains(&encode_key(&key.private)));
    }
}

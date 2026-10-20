use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

mod messages;
mod overview;
mod status;
pub use messages::*;
pub use overview::*;
pub use status::*;

pub const VERSION: u32 = 1;
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
pub const TERMINAL_CHUNK_SIZE: usize = 32 * 1024;
pub const DEFAULT_HISTORY: u32 = 1_000;

pub type Extensions = Map<String, Value>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalBytes(pub Vec<u8>);

impl Serialize for TerminalBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for TerminalBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        STANDARD
            .decode(String::deserialize(deserializer)?)
            .map(Self)
            .map_err(D::Error::custom)
    }
}

pub fn terminal_chunks(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.chunks(TERMINAL_CHUNK_SIZE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRange {
    pub min: u32,
    pub max: u32,
}

impl VersionRange {
    pub fn negotiate(self, offered: &[u32]) -> Result<u32, Box<ErrorMessage>> {
        offered
            .iter()
            .copied()
            .filter(|version| (self.min..=self.max).contains(version))
            .max()
            .ok_or_else(|| {
                Box::new(ErrorMessage {
                    id: None,
                    code: ErrorCode::UnsupportedVersion,
                    message: "No supported companion protocol version in common.".into(),
                    versions: Some(self),
                })
            })
    }
}

// Preserve future enum values on the wire while giving consumers an explicit fallback.
macro_rules! wire_enum {
    ($name:ident, $fallback:ident, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant,)+
            #[serde(untagged)]
            Unrecognized(String),
        }

        impl $name {
            pub fn effective(&self) -> Self {
                match self {
                    Self::Unrecognized(_) => Self::$fallback,
                    known => known.clone(),
                }
            }
        }
    };
}
pub(crate) use wire_enum;

wire_enum!(ErrorCode, Failed, {
    UnsupportedVersion => "unsupportedVersion", BadRequest => "badRequest",
    UnknownRequest => "unknownRequest", NotFound => "notFound",
    NotRunning => "notRunning", Failed => "failed",
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorMessage {
    pub id: Option<u64>,
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<VersionRange>,
}

#[cfg(test)]
mod tests;

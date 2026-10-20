mod cpace;
mod handshake;
mod records;

pub use cpace::Cpace;
pub use handshake::{connect, BrowserConnect, BrowserPair, HostPair, Identity, NoiseKeys};
pub use records::Records;
pub use snow::TransportState as Transport;

pub const MAX_FRAME: usize = 65_536;
pub const MAX_RECORD_BODY: usize = 65_518;
pub const INITIAL_CREDIT: usize = 512 * 1024;
pub const CREDIT_THRESHOLD: usize = 64 * 1024;
pub const PAIR_PROLOGUE: &[u8] = b"runner-pair-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Connect = 1,
    Accept,
    Pair,
    PairReply,
    PairFinish,
    Data,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    NoPin,
    PinLocked,
    WrongPin,
    UnknownDevice,
    Busy,
    BadHandshake,
}

impl Reject {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoPin => "noPin",
            Self::PinLocked => "pinLocked",
            Self::WrongPin => "wrongPin",
            Self::UnknownDevice => "unknownDevice",
            Self::Busy => "busy",
            Self::BadHandshake => "badHandshake",
        }
    }
    pub fn frame(self) -> Vec<u8> {
        frame(Kind::Reject, self.reason().as_bytes())
    }
}

pub fn frame(kind: Kind, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(kind as u8);
    out.extend_from_slice(body);
    out
}

pub fn body(bytes: &[u8], kind: Kind) -> Result<&[u8], Reject> {
    if bytes.first() != Some(&(kind as u8)) || bytes.len() > MAX_FRAME {
        return Err(Reject::BadHandshake);
    }
    Ok(&bytes[1..])
}

pub fn wrap(channel: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = channel.to_be_bytes().to_vec();
    out.extend_from_slice(bytes);
    out
}

pub fn unwrap(bytes: &[u8]) -> Option<(u32, &[u8])> {
    if !(4..=MAX_FRAME + 4).contains(&bytes.len()) {
        return None;
    }
    let channel = u32::from_be_bytes(bytes[..4].try_into().ok()?);
    (channel != 0).then_some((channel, &bytes[4..]))
}

#[cfg(test)]
mod tests;

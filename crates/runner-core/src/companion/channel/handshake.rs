use super::{body, frame, Cpace, Kind, Reject, PAIR_PROLOGUE};
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState, TransportState};

pub struct NoiseKeys<'a> {
    pub private: &'a [u8],
    pub fixed_ephemeral: Option<&'a [u8]>,
}
impl NoiseKeys<'_> {
    fn build(
        &self,
        pairing: bool,
        initiator: bool,
        remote: Option<&[u8]>,
    ) -> Result<HandshakeState, Reject> {
        let (pattern, prologue) = if pairing {
            ("Noise_XXpsk3_25519_ChaChaPoly_SHA256", PAIR_PROLOGUE)
        } else {
            (
                "Noise_IK_25519_ChaChaPoly_SHA256",
                b"runner-connect-v1".as_slice(),
            )
        };
        let mut builder = Builder::new(pattern.parse().map_err(|_| Reject::BadHandshake)?)
            .local_private_key(self.private)
            .prologue(prologue);
        if let Some(key) = self.fixed_ephemeral {
            builder = builder.fixed_ephemeral_key_for_testing_only(key);
        }
        if let Some(key) = remote {
            builder = builder.remote_public_key(key);
        }
        if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|_| Reject::BadHandshake)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub public_key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct PairPayload {
    name: String,
}

fn write(state: &mut HandshakeState, kind: Kind, payload: &[u8]) -> Result<Vec<u8>, Reject> {
    let mut out = vec![0; super::MAX_FRAME - 1];
    let n = state
        .write_message(payload, &mut out)
        .map_err(|_| Reject::BadHandshake)?;
    out.truncate(n);
    Ok(frame(kind, &out))
}

fn empty(state: &mut HandshakeState, bytes: &[u8]) -> Result<(), Reject> {
    let mut payload = [0; 0];
    state
        .read_message(bytes, &mut payload)
        .map_err(|_| Reject::BadHandshake)?;
    Ok(())
}

pub struct HostPair {
    noise: HandshakeState,
    psk: [u8; 32],
}
impl HostPair {
    pub fn start(
        keys: NoiseKeys<'_>,
        pin: &str,
        input: &[u8],
        scalar: [u8; 32],
    ) -> Result<(Self, Vec<u8>), Reject> {
        let input = body(input, Kind::Pair)?;
        if input.len() != 96 {
            return Err(Reject::BadHandshake);
        }
        let (sid, ya) = (&input[..16], input[16..48].try_into().unwrap());
        let cpace = Cpace::new(pin.as_bytes(), PAIR_PROLOGUE, sid, scalar)?;
        let isk = cpace.isk(&ya, sid, false, b"", b"")?;
        let mut noise = keys.build(true, false, None)?;
        empty(&mut noise, &input[48..])?;
        let reply = write(&mut noise, Kind::PairReply, &[])?;
        let mut reply_body = cpace.share.to_vec();
        reply_body.extend_from_slice(&reply[1..]);
        Ok((
            Self {
                noise,
                psk: isk[..32].try_into().unwrap(),
            },
            frame(Kind::PairReply, &reply_body),
        ))
    }

    pub fn finish(mut self, input: &[u8]) -> Result<(TransportState, Identity), Reject> {
        let input = body(input, Kind::PairFinish)?;
        if input.len() < 64 {
            return Err(Reject::BadHandshake);
        }
        self.noise
            .set_psk(3, &self.psk)
            .map_err(|_| Reject::BadHandshake)?;
        let mut payload = vec![0; input.len()];
        let n = self.noise.read_message(input, &mut payload).map_err(|e| {
            if e == snow::Error::Decrypt {
                Reject::WrongPin
            } else {
                Reject::BadHandshake
            }
        })?;
        let payload: PairPayload =
            serde_json::from_slice(&payload[..n]).map_err(|_| Reject::BadHandshake)?;
        if payload.name.is_empty() || payload.name.len() > 64 {
            return Err(Reject::BadHandshake);
        }
        let identity = Identity {
            name: payload.name.chars().filter(|c| !c.is_control()).collect(),
            public_key: self
                .noise
                .get_remote_static()
                .ok_or(Reject::BadHandshake)?
                .to_vec(),
        };
        Ok((
            self.noise
                .into_transport_mode()
                .map_err(|_| Reject::BadHandshake)?,
            identity,
        ))
    }
}

pub fn connect(
    keys: NoiseKeys<'_>,
    input: &[u8],
    allowed: impl FnOnce(&[u8]) -> bool,
) -> Result<(TransportState, Vec<u8>, Vec<u8>), Reject> {
    let input = body(input, Kind::Connect)?;
    if input.len() != 96 {
        return Err(Reject::BadHandshake);
    }
    let mut noise = keys.build(false, false, None)?;
    empty(&mut noise, input).map_err(|_| Reject::UnknownDevice)?;
    let device = noise
        .get_remote_static()
        .ok_or(Reject::UnknownDevice)?
        .to_vec();
    if !allowed(&device) {
        return Err(Reject::UnknownDevice);
    }
    let accept = write(&mut noise, Kind::Accept, &[])?;
    Ok((
        noise
            .into_transport_mode()
            .map_err(|_| Reject::BadHandshake)?,
        accept,
        device,
    ))
}

pub struct BrowserPair {
    noise: HandshakeState,
    cpace: Cpace,
    sid: [u8; 16],
}
impl BrowserPair {
    pub fn start(
        keys: NoiseKeys<'_>,
        pin: &str,
        sid: [u8; 16],
        scalar: [u8; 32],
    ) -> Result<(Self, Vec<u8>), Reject> {
        let mut noise = keys.build(true, true, None)?;
        let cpace = Cpace::new(pin.as_bytes(), PAIR_PROLOGUE, &sid, scalar)?;
        let first = write(&mut noise, Kind::Pair, &[])?;
        let mut payload = sid.to_vec();
        payload.extend_from_slice(&cpace.share);
        payload.extend_from_slice(&first[1..]);
        Ok((Self { noise, cpace, sid }, frame(Kind::Pair, &payload)))
    }
    pub fn finish(
        mut self,
        input: &[u8],
        name: &str,
    ) -> Result<(TransportState, Vec<u8>, Vec<u8>), Reject> {
        let input = body(input, Kind::PairReply)?;
        if input.len() != 128 {
            return Err(Reject::BadHandshake);
        }
        let isk = self
            .cpace
            .isk(&input[..32].try_into().unwrap(), &self.sid, true, b"", b"")?;
        empty(&mut self.noise, &input[32..])?;
        self.noise
            .set_psk(3, &isk[..32])
            .map_err(|_| Reject::BadHandshake)?;
        let payload = serde_json::to_vec(&PairPayload { name: name.into() })
            .map_err(|_| Reject::BadHandshake)?;
        let finish = write(&mut self.noise, Kind::PairFinish, &payload)?;
        let host_key = self
            .noise
            .get_remote_static()
            .ok_or(Reject::BadHandshake)?
            .to_vec();
        Ok((
            self.noise
                .into_transport_mode()
                .map_err(|_| Reject::BadHandshake)?,
            finish,
            host_key,
        ))
    }
}

pub struct BrowserConnect {
    noise: HandshakeState,
}
impl BrowserConnect {
    pub fn start(keys: NoiseKeys<'_>, host: &[u8]) -> Result<(Self, Vec<u8>), Reject> {
        let mut noise = keys.build(false, true, Some(host))?;
        let input = write(&mut noise, Kind::Connect, &[])?;
        Ok((Self { noise }, input))
    }
    pub fn accept(mut self, input: &[u8]) -> Result<TransportState, Reject> {
        let input = body(input, Kind::Accept)?;
        if input.len() != 48 {
            return Err(Reject::BadHandshake);
        }
        empty(&mut self.noise, input)?;
        self.noise
            .into_transport_mode()
            .map_err(|_| Reject::BadHandshake)
    }
}

macro_rules! redacted {
    ($($ty:ty),*) => { $(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(concat!(stringify!($ty), "([redacted])")) }
    })* };
}
redacted!(
    NoiseKeys<'_>,
    Identity,
    HostPair,
    BrowserPair,
    BrowserConnect
);

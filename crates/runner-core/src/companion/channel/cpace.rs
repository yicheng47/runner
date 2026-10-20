use curve25519_dalek::{
    ristretto::CompressedRistretto, ristretto::RistrettoPoint, scalar::Scalar, traits::IsIdentity,
};
use sha2::{Digest, Sha512};

use super::Reject;

const DSI: &[u8] = b"CPaceRistretto255";

fn lv(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        let mut len = field.len();
        loop {
            out.push((len & 127) as u8 | if len >= 128 { 128 } else { 0 });
            len >>= 7;
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(field);
    }
    out
}

fn generator(prs: &[u8], ci: &[u8], sid: &[u8]) -> RistrettoPoint {
    let padding = vec![0; 127usize.saturating_sub(lv(&[DSI, prs]).len())];
    RistrettoPoint::from_uniform_bytes(&Sha512::digest(lv(&[DSI, prs, &padding, ci, sid])).into())
}

pub struct Cpace {
    scalar: Scalar,
    pub share: [u8; 32],
}

impl Cpace {
    pub fn new(prs: &[u8], ci: &[u8], sid: &[u8], mut scalar: [u8; 32]) -> Result<Self, Reject> {
        scalar[31] &= 15;
        let scalar = Scalar::from_bytes_mod_order(scalar);
        if scalar == Scalar::ZERO {
            return Err(Reject::BadHandshake);
        }
        Ok(Self {
            share: (scalar * generator(prs, ci, sid)).compress().to_bytes(),
            scalar,
        })
    }

    pub fn isk(
        &self,
        peer: &[u8; 32],
        sid: &[u8],
        initiator: bool,
        ada: &[u8],
        adb: &[u8],
    ) -> Result<[u8; 64], Reject> {
        let peer_point = CompressedRistretto(*peer)
            .decompress()
            .ok_or(Reject::BadHandshake)?;
        let k = self.scalar * peer_point;
        if k.is_identity() {
            return Err(Reject::BadHandshake);
        }
        let (a, b) = if initiator {
            (&self.share, peer)
        } else {
            (peer, &self.share)
        };
        let mut input = lv(&[b"CPaceRistretto255_ISK", sid, &k.compress().to_bytes()]);
        input.extend(lv(&[a, ada, b, adb]));
        Ok(Sha512::digest(input).into())
    }
}

impl std::fmt::Debug for Cpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cpace([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::channel::tests::unhex;
    #[test]
    fn appendix_b3_initiator_responder() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/cpace-b3.json")).unwrap();
        let b = |key: &str| unhex(v[key].as_str().unwrap());
        let a = Cpace::new(&b("prs"), &b("ci"), &b("sid"), b("ya").try_into().unwrap()).unwrap();
        let z = Cpace::new(&b("prs"), &b("ci"), &b("sid"), b("yb").try_into().unwrap()).unwrap();
        assert_eq!(
            generator(&b("prs"), &b("ci"), &b("sid"))
                .compress()
                .as_bytes()
                .as_slice(),
            b("g")
        );
        assert_eq!(a.share.as_slice(), b("Ya"));
        assert_eq!(z.share.as_slice(), b("Yb"));
        assert_eq!(
            a.isk(&z.share, &b("sid"), true, b"ADa", b"ADb")
                .unwrap()
                .as_slice(),
            b("isk")
        );
        assert_eq!(
            z.isk(&a.share, &b("sid"), false, b"ADa", b"ADb")
                .unwrap()
                .as_slice(),
            b("isk")
        );
        let k = a.scalar * CompressedRistretto(z.share).decompress().unwrap();
        assert_eq!(k.compress().as_bytes().as_slice(), b("k"));
        let scalar = Scalar::from_bytes_mod_order(b("scalar").try_into().unwrap());
        let valid = CompressedRistretto(b("validPoint").try_into().unwrap())
            .decompress()
            .unwrap();
        assert_eq!(
            (scalar * valid).compress().as_bytes().as_slice(),
            b("validProduct")
        );
        for point in v["invalidPoints"].as_array().unwrap() {
            assert_eq!(
                a.isk(
                    &unhex(point.as_str().unwrap()).try_into().unwrap(),
                    &b("sid"),
                    true,
                    b"",
                    b""
                ),
                Err(Reject::BadHandshake)
            );
        }
        for point in [[0; 32], [255; 32]] {
            assert_eq!(
                a.isk(&point, &b("sid"), true, b"", b""),
                Err(Reject::BadHandshake)
            );
        }
        assert!(Cpace::new(b"123456", b"", b"", [0; 32]).is_err());
    }
}

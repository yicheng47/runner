use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use super::terminal::TerminalSnapshot;
use super::{ClientEvent, Request, Response};

pub const MAX_FRAME: usize = 64 * 1024 * 1024;
pub const TERMINAL_CHUNK: usize = 64 * 1024;
pub const HELLO: u8 = 1;
pub const WELCOME: u8 = 2;
pub const MISMATCH: u8 = 3;
pub const SHUTDOWN: u8 = 4;
pub const REQUEST: u8 = 5;
pub const RESPONSE: u8 = 6;
pub const EVENT: u8 = 7;
pub const ATTACH: u8 = 8;
pub const SNAPSHOT: u8 = 9;
pub const OUTPUT: u8 = 10;
pub const RESIZED: u8 = 11;
pub const RESYNC: u8 = 12;
pub const INPUT: u8 = 13;
pub const RESIZE: u8 = 14;
pub const TERMINAL_CLOSED: u8 = 15;
pub const ATTACH_ERROR: u8 = 16;
pub const DETACH: u8 = 17;
pub const SNAPSHOT_DATA: u8 = 18;
pub const REQUEST_ERROR: u8 = 19;
pub const PROTOCOL_MISMATCH: &str =
    "Runner CLI and daemon use different client protocols. Restart Runner and retry.";

#[derive(Serialize, Deserialize)]
pub struct RequestError {
    pub id: u64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub exe_sha256: String,
    pub client: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub exe_sha256: String,
    pub pid: u32,
    pub started_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shutdown {
    pub stop_sessions: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Call {
    pub id: u64,
    pub request: Request,
}
#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    pub response: Response,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ClientEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lagged: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}
impl Frame {
    pub fn json(kind: u8, value: &impl Serialize) -> io::Result<Self> {
        Ok(Self {
            kind,
            payload: serde_json::to_vec(value)?,
        })
    }
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> io::Result<T> {
        Ok(serde_json::from_slice(&self.payload)?)
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let len = self.payload.len() + 1;
        if len > MAX_FRAME {
            return Err(invalid("frame too large"));
        }
        let mut bytes = Vec::with_capacity(len + 4);
        bytes.extend_from_slice(&(len as u32).to_le_bytes());
        bytes.push(self.kind);
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(&self.encode()?)
    }
    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; 4];
        reader.read_exact(&mut header)?;
        let len = u32::from_le_bytes(header) as usize;
        if !(1..=MAX_FRAME).contains(&len) {
            return Err(invalid("invalid frame length"));
        }
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes)?;
        Ok(Self {
            kind: bytes[0],
            payload: bytes[1..].to_vec(),
        })
    }
}
pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub struct Binary<'a>(pub &'a [u8]);
impl<'a> Binary<'a> {
    pub fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(invalid("short terminal frame"));
        }
        let (head, tail) = self.0.split_at(count);
        self.0 = tail;
        Ok(head)
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn string(&mut self) -> io::Result<String> {
        let len = self.u16()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| invalid("invalid session id"))
    }
}
pub fn session_payload(id: &str) -> io::Result<Vec<u8>> {
    let len = u16::try_from(id.len()).map_err(|_| invalid("session id too long"))?;
    let mut payload = len.to_le_bytes().to_vec();
    payload.extend_from_slice(id.as_bytes());
    Ok(payload)
}
pub fn numbered(kind: u8, id: u64) -> Frame {
    Frame {
        kind,
        payload: id.to_le_bytes().to_vec(),
    }
}
pub fn snapshot_header(id: u64, subscriber: u64, snapshot: &TerminalSnapshot) -> Frame {
    let mut frame = numbered(SNAPSHOT, id);
    frame.payload.extend_from_slice(&subscriber.to_le_bytes());
    frame.payload.extend_from_slice(&snapshot.seq.to_le_bytes());
    frame
        .payload
        .extend_from_slice(&snapshot.cols.to_le_bytes());
    frame
        .payload
        .extend_from_slice(&snapshot.rows.to_le_bytes());
    frame
        .payload
        .extend_from_slice(&(snapshot.unfinished_len as u64).to_le_bytes());
    frame.payload.extend_from_slice(
        &snapshot
            .preceding_char
            .map(u32::from)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_handshake_ignores_new_fields_and_frames_round_trip() {
        let hello: Hello =
            serde_json::from_str(r#"{"exe_sha256":"hash","client":"app","future":true}"#).unwrap();
        let frame = Frame::json(HELLO, &hello).unwrap();
        let decoded = Frame::read(&mut frame.encode().unwrap().as_slice()).unwrap();
        assert_eq!(decoded.decode::<Hello>().unwrap().client, "app");
        assert!(Frame::read(&mut [0; 4].as_slice()).is_err());
    }
}

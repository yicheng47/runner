use super::{body, frame, Kind, Reject, CREDIT_THRESHOLD, INITIAL_CREDIT, MAX_RECORD_BODY};
use crate::companion::MAX_MESSAGE_SIZE;
use snow::TransportState;

pub struct Records {
    noise: TransportState,
    send_credit: usize,
    receive_credit: usize,
    owed: usize,
    flush_credit: bool,
    incoming: Vec<u8>,
    handling: Option<usize>,
    outgoing: Option<(Vec<u8>, usize)>,
}
impl Records {
    pub fn new(noise: TransportState) -> Self {
        Self {
            noise,
            send_credit: INITIAL_CREDIT,
            receive_credit: INITIAL_CREDIT,
            owed: 0,
            flush_credit: false,
            incoming: Vec::new(),
            handling: None,
            outgoing: None,
        }
    }

    pub fn queue(&mut self, bytes: Vec<u8>) -> Result<(), Reject> {
        if bytes.len() > MAX_MESSAGE_SIZE || self.outgoing.is_some() {
            return Err(Reject::BadHandshake);
        }
        self.outgoing = Some((bytes, 0));
        Ok(())
    }

    pub fn send_credit(&self) -> usize {
        self.send_credit
    }

    pub fn sending(&self) -> bool {
        self.outgoing.is_some()
    }

    pub fn queued_bytes(&self) -> usize {
        self.outgoing
            .as_ref()
            .map_or(0, |(bytes, offset)| bytes.len() - offset)
    }

    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, Reject> {
        let record = if self.owed >= CREDIT_THRESHOLD || (self.flush_credit && self.owed > 0) {
            let n = self.owed;
            self.owed = 0;
            self.flush_credit = false;
            self.receive_credit += n;
            let mut record = vec![2];
            record.extend_from_slice(&(n as u32).to_be_bytes());
            record
        } else if let Some((message, offset)) = &mut self.outgoing {
            let left = message.len() - *offset;
            if left > 0 && self.send_credit == 0 {
                return Ok(None);
            }
            let n = left.min(MAX_RECORD_BODY).min(self.send_credit);
            let end = n == left;
            let mut record = Vec::with_capacity(n + 1);
            record.push(u8::from(end));
            record.extend_from_slice(&message[*offset..*offset + n]);
            *offset += n;
            self.send_credit -= n;
            if end {
                self.outgoing = None;
            }
            record
        } else {
            return Ok(None);
        };
        let mut encrypted = vec![0; record.len() + 16];
        let n = self
            .noise
            .write_message(&record, &mut encrypted)
            .map_err(|_| Reject::BadHandshake)?;
        encrypted.truncate(n);
        Ok(Some(frame(Kind::Data, &encrypted)))
    }

    pub fn receive(&mut self, input: &[u8]) -> Result<Option<Vec<u8>>, Reject> {
        let input = body(input, Kind::Data)?;
        let mut record = vec![0; input.len()];
        let n = self
            .noise
            .read_message(input, &mut record)
            .map_err(|_| Reject::BadHandshake)?;
        if n == 0 {
            return Err(Reject::BadHandshake);
        }
        let bytes = &record[1..n];
        match record[0] {
            0 | 1 => {
                if self.handling.is_some()
                    || bytes.len() > self.receive_credit
                    || self.incoming.len() + bytes.len() > MAX_MESSAGE_SIZE
                {
                    return Err(Reject::BadHandshake);
                }
                self.receive_credit -= bytes.len();
                self.incoming.extend_from_slice(bytes);
                if record[0] == 0 {
                    self.owed += bytes.len();
                    Ok(None)
                } else {
                    self.handling = Some(bytes.len());
                    Ok(Some(std::mem::take(&mut self.incoming)))
                }
            }
            2 => {
                let bytes: [u8; 4] = bytes.try_into().map_err(|_| Reject::BadHandshake)?;
                let n = u32::from_be_bytes(bytes) as usize;
                if n == 0 || n > INITIAL_CREDIT - self.send_credit {
                    return Err(Reject::BadHandshake);
                }
                self.send_credit += n;
                Ok(None)
            }
            _ => Err(Reject::BadHandshake),
        }
    }

    pub fn handled(&mut self) {
        if let Some(n) = self.handling.take() {
            self.owed += n;
            self.flush_credit = self.owed > 0;
        }
    }
}

impl std::fmt::Debug for Records {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Records([redacted])")
    }
}

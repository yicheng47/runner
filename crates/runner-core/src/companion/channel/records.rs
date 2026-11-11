use super::{
    body, frame, Kind, Reject, CREDIT_THRESHOLD, INITIAL_CREDIT, MAX_IN_FLIGHT, MAX_RECORD_BODY,
    PACING_DELAY, PACING_WINDOW,
};
use crate::companion::MAX_MESSAGE_SIZE;
use snow::TransportState;
use std::{collections::VecDeque, time::Instant};

pub struct Records {
    noise: TransportState,
    send_credit: usize,
    receive_credit: usize,
    owed: usize,
    owed_cost: usize,
    in_flight: VecDeque<(usize, usize, Instant)>,
    in_flight_cost: usize,
    incoming: Vec<u8>,
    handling: Option<(usize, usize)>,
    outgoing: Option<(Vec<u8>, usize)>,
}
impl Records {
    pub fn new(noise: TransportState) -> Self {
        Self {
            noise,
            send_credit: INITIAL_CREDIT,
            receive_credit: INITIAL_CREDIT,
            owed: 0,
            owed_cost: 0,
            in_flight: VecDeque::new(),
            in_flight_cost: 0,
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

    pub fn output_paced(&self, now: Instant) -> bool {
        self.in_flight_cost > PACING_WINDOW
            && self
                .in_flight
                .front()
                .is_some_and(|(_, _, sent)| now.duration_since(*sent) >= PACING_DELAY)
    }

    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, Reject> {
        self.next_frame_at(Instant::now())
    }

    pub fn next_frame_at(&mut self, now: Instant) -> Result<Option<Vec<u8>>, Reject> {
        let record = if self.owed > 0
            && (self.owed >= CREDIT_THRESHOLD || self.owed_cost >= CREDIT_THRESHOLD)
        {
            let n = self.owed;
            self.owed = 0;
            self.owed_cost = 0;
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
            let cost = n + 1 + 16 + 1 + 64;
            if self.in_flight_cost + cost > MAX_IN_FLIGHT {
                return Ok(None);
            }
            self.in_flight.push_back((n, cost, now));
            self.in_flight_cost += cost;
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
        let cost = input.len() + 64;
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
                    self.owed_cost += cost;
                    Ok(None)
                } else {
                    self.handling = Some((bytes.len(), cost));
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
                let mut remaining = n;
                while let Some((bytes, cost, _)) = self.in_flight.front_mut() {
                    let credited = remaining.min(*bytes);
                    *bytes -= credited;
                    remaining -= credited;
                    if *bytes > 0 {
                        break;
                    }
                    self.in_flight_cost -= *cost;
                    self.in_flight.pop_front();
                }
                Ok(None)
            }
            _ => Err(Reject::BadHandshake),
        }
    }

    pub fn handled(&mut self) {
        if let Some((n, cost)) = self.handling.take() {
            self.owed += n;
            self.owed_cost += cost;
        }
    }
}

impl std::fmt::Debug for Records {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Records([redacted])")
    }
}

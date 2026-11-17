use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::session::runtime::{BoundInput, RuntimeSession, SessionRuntime};

/// A healthy agent drains its tty input within milliseconds, and even a large paste within a
/// second, so a write that has not returned after five seconds means the agent stopped reading.
/// The deadline bounds how long callers wait and how long input queues behind the write; the PTY
/// write itself runs on until the agent reads or the session ends.
pub(super) const INPUT_WRITE_DEADLINE: Duration = Duration::from_secs(5);
/// What a session may hold unwritten while it is busy. A single larger write is accepted only
/// while nothing else is unwritten, so a large paste is never refused outright.
pub(super) const INPUT_QUEUE_LIMIT: usize = 1024 * 1024;

/// The ordered PTY input of one session process (#1377). Callers queue bytes while they hold
/// their session locks and wait for the write after releasing them; only the writer thread
/// blocks in the PTY write. Dropping the queue with its handle fails whatever is still pending.
#[derive(Default)]
pub(super) struct InputQueue(OnceLock<Arc<InputWriter>>);

impl InputQueue {
    pub(super) fn writer(
        &self,
        runtime: &Arc<dyn SessionRuntime>,
        session: &RuntimeSession,
        deadline: Duration,
    ) -> Arc<InputWriter> {
        Arc::clone(
            self.0
                .get_or_init(|| InputWriter::start(runtime, session, deadline)),
        )
    }
}

impl Drop for InputQueue {
    fn drop(&mut self) {
        if let Some(writer) = self.0.get() {
            writer.close();
        }
    }
}

/// How a write that did not fail ended.
#[derive(Debug, PartialEq)]
pub(super) enum Written {
    /// The PTY took the bytes.
    Done,
    /// The write outlived its deadline and is still under way, because a blocking PTY write
    /// cannot be cancelled. Its bytes arrive if the agent reads again, so the caller must neither
    /// undo them nor send them again; the message tells the person so.
    Overdue(String),
}

pub(super) struct InputWriter {
    session_id: String,
    deadline: Duration,
    state: Mutex<WriterState>,
    changed: Condvar,
}

#[derive(Default)]
struct WriterState {
    queue: VecDeque<(Vec<u8>, Arc<Outcome>)>,
    queued_bytes: usize,
    in_flight: Option<InFlight>,
    /// The write in flight overran its deadline: everything queued behind it was dropped, and new
    /// input is refused until it returns.
    stalled: bool,
    closed: bool,
}

struct InFlight {
    started: Instant,
    bytes: usize,
    outcome: Arc<Outcome>,
}

#[derive(Default)]
struct Outcome(Mutex<Option<Result<Written>>>);

impl Outcome {
    fn settle(&self, result: Result<Written>) {
        self.0.lock().unwrap().get_or_insert(result);
    }
}

/// A queued write. `wait` returns once it is written, overdue or dropped.
#[must_use]
pub(super) struct PendingWrite {
    writer: Arc<InputWriter>,
    outcome: Arc<Outcome>,
}

enum Sink {
    Bound(BoundInput),
    ById(Arc<dyn SessionRuntime>, RuntimeSession),
}

impl Sink {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Bound(write) => write(bytes),
            Self::ById(runtime, session) if bytes == b"\r" => runtime.send_key(session, "Enter"),
            Self::ById(runtime, session) => runtime.send_bytes(session, bytes),
        }
        .map_err(Into::into)
    }
}

impl InputWriter {
    fn start(
        runtime: &Arc<dyn SessionRuntime>,
        session: &RuntimeSession,
        deadline: Duration,
    ) -> Arc<Self> {
        let sink = match runtime.bind_input(session) {
            Some(input) => Sink::Bound(input),
            None => Sink::ById(Arc::clone(runtime), session.clone()),
        };
        Self::spawn(&session.session_id, deadline, sink)
    }

    fn spawn(session_id: &str, deadline: Duration, sink: Sink) -> Arc<Self> {
        let writer = Arc::new(Self {
            session_id: session_id.to_owned(),
            deadline,
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        let worker = Arc::clone(&writer);
        thread::Builder::new()
            .name(format!("pty-input-{session_id}"))
            .spawn(move || worker.run(sink))
            .expect("spawn PTY input writer");
        writer
    }

    pub(super) fn push(self: &Arc<Self>, bytes: &[u8]) -> Result<PendingWrite> {
        let mut state = self.state.lock().unwrap();
        self.refuse_input(&mut state)?;
        let unwritten =
            state.queued_bytes + state.in_flight.as_ref().map_or(0, |write| write.bytes);
        let idle = state.queue.is_empty() && state.in_flight.is_none();
        if !idle && unwritten + bytes.len() > INPUT_QUEUE_LIMIT {
            return Err(Error::msg(format!(
                "input queue of session {} is full",
                self.session_id
            )));
        }
        Ok(self.enqueue(&mut state, bytes))
    }

    /// Queues the bytes only if nothing else is waiting to be written, so they reach the PTY
    /// while what the caller checked still holds. `None` when the writer is busy.
    pub(super) fn push_when_idle(self: &Arc<Self>, bytes: &[u8]) -> Result<Option<PendingWrite>> {
        let mut state = self.state.lock().unwrap();
        self.refuse_input(&mut state)?;
        if !state.queue.is_empty() || state.in_flight.is_some() {
            return Ok(None);
        }
        Ok(Some(self.enqueue(&mut state, bytes)))
    }

    fn refuse_input(&self, state: &mut WriterState) -> Result<()> {
        if state.closed {
            return Err(self.closed_error());
        }
        self.expire(state);
        if state.stalled {
            return Err(self.dropped_error());
        }
        Ok(())
    }

    fn enqueue(self: &Arc<Self>, state: &mut WriterState, bytes: &[u8]) -> PendingWrite {
        let outcome = Arc::new(Outcome::default());
        state
            .queue
            .push_back((bytes.to_vec(), Arc::clone(&outcome)));
        state.queued_bytes += bytes.len();
        self.changed.notify_all();
        PendingWrite {
            writer: Arc::clone(self),
            outcome,
        }
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        let state = &mut *state;
        for outcome in state.queue.drain(..).map(|(_, outcome)| outcome).chain(
            state
                .in_flight
                .as_ref()
                .map(|write| Arc::clone(&write.outcome)),
        ) {
            outcome.settle(Err(self.closed_error()));
        }
        state.queued_bytes = 0;
        self.changed.notify_all();
    }

    fn expire(&self, state: &mut WriterState) {
        let Some(in_flight) = &state.in_flight else {
            return;
        };
        if state.stalled || in_flight.started.elapsed() < self.deadline {
            return;
        }
        log::warn!(
            "session {} has not read its input for {:?}; dropping the input queued behind it",
            self.session_id,
            self.deadline
        );
        state.stalled = true;
        in_flight.outcome.settle(Ok(Written::Overdue(format!(
            "session {} stopped reading its input; this input is still waiting and arrives if \
             the session reads again",
            self.session_id
        ))));
        for (_, outcome) in state.queue.drain(..) {
            outcome.settle(Err(self.dropped_error()));
        }
        state.queued_bytes = 0;
        self.changed.notify_all();
    }

    fn run(self: Arc<Self>, mut sink: Sink) {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.closed {
                return;
            }
            let Some((bytes, outcome)) = state.queue.pop_front() else {
                state = self.changed.wait(state).unwrap();
                continue;
            };
            state.queued_bytes -= bytes.len();
            state.in_flight = Some(InFlight {
                started: Instant::now(),
                bytes: bytes.len(),
                outcome: Arc::clone(&outcome),
            });
            drop(state);
            let result = sink.write(&bytes);
            state = self.state.lock().unwrap();
            self.expire(&mut state);
            let write = state.in_flight.take().expect("the write in flight");
            if std::mem::take(&mut state.stalled) {
                log::info!(
                    "session {} read its input again after {:?}",
                    self.session_id,
                    write.started.elapsed()
                );
            }
            outcome.settle(result.map(|()| Written::Done));
            self.changed.notify_all();
        }
    }

    #[cfg(test)]
    pub(super) fn wait_until_queued(&self, queued: usize) {
        self.wait_until(|state| state.queue.len() >= queued);
    }

    #[cfg(test)]
    pub(super) fn wait_until_idle(&self) {
        self.wait_until(|state| state.queue.is_empty() && state.in_flight.is_none());
    }

    #[cfg(test)]
    fn wait_until(&self, done: impl Fn(&WriterState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut state = self.state.lock().unwrap();
        while !done(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "input writer never reached the expected state"
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn closed_error(&self) -> Error {
        Error::msg(format!(
            "session {} stopped before its input was written",
            self.session_id
        ))
    }

    fn dropped_error(&self) -> Error {
        Error::msg(format!(
            "session {} stopped reading its input; input not yet written was dropped",
            self.session_id
        ))
    }
}

impl PendingWrite {
    pub(super) fn wait(self) -> Result<Written> {
        let writer = &self.writer;
        let mut state = writer.state.lock().unwrap();
        loop {
            writer.expire(&mut state);
            if let Some(result) = self.outcome.0.lock().unwrap().take() {
                return result;
            }
            let remaining = state.in_flight.as_ref().map_or(writer.deadline, |write| {
                (write.started + writer.deadline).saturating_duration_since(Instant::now())
            });
            state = writer.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests;

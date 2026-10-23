use super::*;
use runner_core::protocol::{
    terminal::{
        TerminalController, TerminalFrame, TerminalSnapshot, TerminalSubscription,
        TERMINAL_QUEUE_CAPACITY,
    },
    ClientError,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Condvar, Mutex as StdMutex},
};

#[derive(Default)]
struct Queue {
    state: StdMutex<(std::collections::VecDeque<TerminalFrame>, usize, bool)>,
    ready: Condvar,
}
impl Queue {
    fn push(&self, frame: TerminalFrame) {
        let mut state = self.state.lock().unwrap();
        if state.2 {
            return;
        }
        let size = frame_size(&frame);
        if state.1 + size > c::MAX_MESSAGE_SIZE || matches!(frame, TerminalFrame::Resync) {
            state.0.clear();
            state.1 = 0;
            state.0.push_back(TerminalFrame::Resync);
            state.2 = true;
        } else {
            state.1 += size;
            state.0.push_back(frame);
        }
        self.ready.notify_all();
    }
    fn close(&self) {
        self.state.lock().unwrap().2 = true;
        self.ready.notify_all();
    }
}
struct Frames {
    queue: Arc<Queue>,
    cancel: Arc<dyn Fn() + Send + Sync>,
}
impl TerminalSubscription for Frames {
    fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
        let mut state = self.queue.state.lock().unwrap();
        loop {
            if let Some(frame) = state.0.pop_front() {
                state.1 = state.1.saturating_sub(frame_size(&frame));
                return Ok(frame);
            }
            if state.2 {
                return Err(ClientError::msg("Remote terminal closed"));
            }
            state = self.queue.ready.wait(state).unwrap();
        }
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.cancel.clone()
    }
}
impl Drop for Frames {
    fn drop(&mut self) {
        (self.cancel)();
    }
}
struct Stream {
    config: RemoteTerminal,
    request: u64,
    snapshot: Option<TerminalSnapshot>,
    seq: u64,
    controller: TerminalController,
    replay: VecDeque<TerminalFrame>,
    replay_bytes: usize,
    replay_valid: bool,
    assembling: Option<TerminalSnapshot>,
    complete: bool,
    requested: bool,
    closed: bool,
    listeners: BTreeMap<u64, Arc<Queue>>,
    waiting: Vec<oneshot::Sender<Result<TerminalAttachment>>>,
}
#[derive(Default)]
pub(super) struct Terminals {
    streams: BTreeMap<String, Stream>,
}
impl Terminals {
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
    pub fn attach(
        &mut self,
        config: RemoteTerminal,
        reply: oneshot::Sender<Result<TerminalAttachment>>,
        session: &mut route::Session,
        handle: &route::Handle,
    ) -> Result<()> {
        let target = config.session.clone();
        let stream = self
            .streams
            .entry(target.clone())
            .or_insert_with(|| Stream {
                config: config.clone(),
                request: 0,
                snapshot: None,
                seq: 0,
                controller: TerminalController::None,
                replay: VecDeque::new(),
                replay_bytes: 0,
                replay_valid: false,
                assembling: None,
                complete: false,
                requested: false,
                closed: false,
                listeners: BTreeMap::new(),
                waiting: Vec::new(),
            });
        stream.config = config;
        if stream.closed && !stream.replay_valid {
            let _ = reply.send(Err("Remote terminal closed".into()));
            return Ok(());
        }
        stream.waiting.push(reply);
        if stream.closed {
            stream.deliver(handle);
            return Ok(());
        }
        if stream.requested
            || stream.assembling.is_some()
            || (stream.replay_valid
                && (stream.config.take_control || stream.controller != TerminalController::None))
        {
            let viewport = c::ViewportRequest {
                id: handle.next_id(),
                session: target,
                cols: stream.config.cols,
                rows: stream.config.rows,
            };
            session.queue(if stream.config.take_control {
                c::ClientMessage::TakeControl(viewport)
            } else {
                c::ClientMessage::Viewport(viewport)
            })?;
            if !stream.requested && stream.assembling.is_none() {
                stream.deliver(handle);
            }
            return Ok(());
        }
        // Only attach can claim a free terminal without stealing its controller.
        stream.requested = true;
        stream.request = handle.next_id();
        stream.assembling = None;
        stream.complete = false;
        session.queue(c::ClientMessage::Attach(c::AttachRequest {
            id: stream.request,
            session: target,
            cols: stream.config.cols,
            rows: stream.config.rows,
            control: if stream.config.take_control {
                c::ControlMode::Take
            } else {
                c::ControlMode::IfFree
            },
            history: None,
        }))
    }
    pub fn reattach(&mut self, session: &mut route::Session, handle: &route::Handle) -> Result<()> {
        for (target, stream) in &mut self.streams {
            if stream.closed {
                continue;
            }
            stream.requested = true;
            stream.request = handle.next_id();
            stream.assembling = None;
            stream.complete = false;
            session.queue(c::ClientMessage::Attach(c::AttachRequest {
                id: stream.request,
                session: target.clone(),
                cols: stream.config.cols,
                rows: stream.config.rows,
                control: c::ControlMode::IfFree,
                history: None,
            }))?;
        }
        Ok(())
    }
    pub fn viewport(&mut self, message: &c::ClientMessage) {
        if let c::ClientMessage::Viewport(viewport) | c::ClientMessage::TakeControl(viewport) =
            message
        {
            if let Some(stream) = self.streams.get_mut(&viewport.session) {
                (stream.config.cols, stream.config.rows) = viewport.clamped_size();
            }
        }
    }
    pub fn remove(&mut self, target: &str, subscriber: u64) {
        if let Some(stream) = self.streams.get_mut(target) {
            stream.waiting.retain(|reply| !reply.is_closed());
            if let Some(queue) = stream.listeners.remove(&subscriber) {
                queue.close();
            }
            if stream.listeners.is_empty() && stream.waiting.is_empty() {
                self.streams.remove(target);
            }
        }
    }
    pub fn detach(
        &mut self,
        target: &str,
        subscriber: u64,
        session: &mut route::Session,
        handle: &route::Handle,
    ) -> Result<()> {
        self.remove(target, subscriber);
        if !self.streams.contains_key(target) {
            session.queue(c::ClientMessage::Detach(c::SessionRequest {
                id: handle.next_id(),
                session: target.into(),
            }))?;
        }
        Ok(())
    }
    pub fn refused(&mut self, request: u64) {
        let target = self
            .streams
            .iter()
            .find(|(_, s)| s.request == request)
            .map(|(t, _)| t.clone());
        if let Some(target) = target {
            if let Some(mut stream) = self.streams.remove(&target) {
                for reply in stream.waiting.drain(..) {
                    let _ = reply.send(Err(
                        "The remote computer refused terminal attachment.".into()
                    ));
                }
                for queue in stream.listeners.values() {
                    queue.close();
                }
            }
        }
    }
    pub fn disconnected(&mut self, error: &str) {
        for stream in self.streams.values_mut() {
            stream.assembling = None;
            stream.complete = false;
            if !stream.closed {
                stream.replay_valid = false;
            }
            for reply in stream.waiting.drain(..) {
                let _ = reply.send(Err(error.into()));
            }
        }
    }
    pub fn close(&mut self) {
        for stream in self.streams.values_mut() {
            for queue in stream.listeners.values() {
                queue.close();
            }
            for reply in stream.waiting.drain(..) {
                let _ = reply.send(Err("Remote channel closed".into()));
            }
        }
        self.streams.clear();
    }
    pub fn message(&mut self, message: c::HostMessage, handle: &route::Handle) -> Result<()> {
        match message {
            c::HostMessage::Snapshot(chunk) => {
                let Some(stream) = self.streams.get_mut(&chunk.session) else {
                    return Ok(());
                };
                if stream.assembling.is_none() {
                    stream.assembling = Some(TerminalSnapshot {
                        seq: chunk.seq,
                        cols: chunk.cols,
                        rows: chunk.rows,
                        controller: TerminalController::None,
                        bytes: Vec::new(),
                        unfinished_len: chunk.unfinished_len,
                        preceding_char: chunk.preceding_char,
                    });
                }
                let snapshot = stream.assembling.as_mut().unwrap();
                if snapshot.seq != chunk.seq
                    || snapshot.cols != chunk.cols
                    || snapshot.rows != chunk.rows
                    || snapshot.preceding_char != chunk.preceding_char
                    || snapshot.unfinished_len != chunk.unfinished_len
                    || snapshot.bytes.len() + chunk.data.0.len() > 32 * 1024 * 1024
                {
                    return Err("Invalid remote snapshot continuation".into());
                }
                snapshot.bytes.extend(chunk.data.0);
                if !chunk.more {
                    if snapshot.unfinished_len > snapshot.bytes.len() {
                        return Err("Invalid remote snapshot suffix".into());
                    }
                    stream.complete = true;
                }
            }
            c::HostMessage::Resized(resized) => {
                let Some(stream) = self.streams.get_mut(&resized.session) else {
                    return Ok(());
                };
                let controller = controller(resized.controller);
                if stream.complete {
                    let mut snapshot = stream.assembling.take().ok_or("Invalid remote snapshot")?;
                    if snapshot.seq != resized.seq
                        || snapshot.cols != resized.cols
                        || snapshot.rows != resized.rows
                    {
                        return Err("Remote snapshot size does not match".into());
                    }
                    snapshot.controller = controller.clone();
                    for queue in stream.listeners.values() {
                        queue.push(TerminalFrame::Resync);
                    }
                    stream.seq = snapshot.seq;
                    stream.controller = controller;
                    stream.snapshot = Some(snapshot);
                    stream.replay.clear();
                    stream.replay_bytes = 0;
                    stream.replay_valid = true;
                    stream.complete = false;
                    stream.requested = false;
                    stream.deliver(handle);
                } else if stream.assembling.is_some() {
                    return Err("Resize interrupted a remote snapshot".into());
                } else {
                    if resized.seq != stream.seq + 1 {
                        return Err("Remote terminal resize sequence changed".into());
                    }
                    stream.seq = resized.seq;
                    stream.controller = controller.clone();
                    stream.push(TerminalFrame::Resized {
                        seq: resized.seq,
                        cols: resized.cols,
                        rows: resized.rows,
                        controller,
                    });
                }
            }
            c::HostMessage::Output(output) => {
                let Some(stream) = self.streams.get_mut(&output.session) else {
                    return Ok(());
                };
                if stream.snapshot.is_none() {
                    return Err("Output arrived before a remote snapshot".into());
                }
                if stream.assembling.is_some() || output.seq != stream.seq + 1 {
                    return Err("Remote terminal output sequence changed".into());
                }
                stream.seq = output.seq;
                stream.push(TerminalFrame::Output {
                    seq: output.seq,
                    bytes: output.data.0,
                });
            }
            c::HostMessage::Closed(closed) => {
                if let Some(stream) = self.streams.get_mut(&closed.session) {
                    stream.closed = true;
                    for queue in stream.listeners.values() {
                        queue.close();
                    }
                    for reply in stream.waiting.drain(..) {
                        let _ = reply.send(Err("Remote terminal closed".into()));
                    }
                    if stream.listeners.is_empty() {
                        self.streams.remove(&closed.session);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn frame_size(frame: &TerminalFrame) -> usize {
    match frame {
        TerminalFrame::Output { bytes, .. } => bytes.len() + 64,
        _ => 64,
    }
}
impl Stream {
    fn push(&mut self, frame: TerminalFrame) {
        for queue in self.listeners.values() {
            queue.push(frame.clone());
        }
        if self.replay_valid {
            self.replay_bytes += frame_size(&frame);
            if self.replay_bytes > c::MAX_MESSAGE_SIZE
                || self.replay.len() == TERMINAL_QUEUE_CAPACITY
            {
                self.replay.clear();
                self.replay_bytes = 0;
                self.replay_valid = false;
            } else {
                self.replay.push_back(frame);
            }
        }
    }
    fn deliver(&mut self, handle: &route::Handle) {
        let snapshot = self.snapshot.as_ref().unwrap();
        for reply in self.waiting.drain(..) {
            if reply.is_closed() {
                continue;
            }
            let subscriber = handle.next_id();
            let queue = Arc::new(Queue::default());
            for frame in &self.replay {
                queue.push(frame.clone());
            }
            if self.closed {
                queue.close();
            }
            let cancel_queue = queue.clone();
            let tx = handle.tx.clone();
            let target = self.config.session.clone();
            let runtime = tokio::runtime::Handle::current();
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancel = Arc::new(move || {
                if cancelled.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    return;
                }
                cancel_queue.close();
                let tx = tx.clone();
                let target = target.clone();
                runtime.spawn(async move {
                    let _ = tx.send(route::Command::Detach(target, subscriber)).await;
                });
            });
            self.listeners.insert(subscriber, queue.clone());
            let _ = reply.send(Ok(TerminalAttachment {
                subscriber_id: subscriber,
                snapshot: snapshot.clone(),
                frames: Box::new(Frames { queue, cancel }),
            }));
        }
    }
}

fn controller(controller: c::Controller) -> TerminalController {
    match controller {
        c::Controller::You => TerminalController::Desktop,
        c::Controller::Desktop => TerminalController::Browser {
            channel: 0,
            name: "Host desktop".into(),
        },
        c::Controller::Browser { name } => TerminalController::Browser { channel: 0, name },
        _ => TerminalController::None,
    }
}

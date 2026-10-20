use super::{machine::Machine, trust::Trust, Clock};
use futures_util::FutureExt;
use runner_core::{
    companion::{
        self,
        channel::{self, HostPair, NoiseKeys, Records, Reject, Transport},
    },
    remote::{self, Control},
};
use std::{
    collections::{HashMap, VecDeque},
    panic::AssertUnwindSafe,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::{
    sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore},
    task::AbortHandle,
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::Message;

pub(super) enum Event {
    Handshake {
        channel: u32,
        instance: u64,
        frame: Vec<u8>,
        reply: oneshot::Sender<Decision>,
    },
    Send {
        channel: u32,
        instance: u64,
        frame: Vec<u8>,
    },
    Finished {
        channel: u32,
        instance: u64,
        reason: &'static str,
    },
}
pub(super) struct Completion {
    channel: u32,
    instance: u64,
    reply: oneshot::Sender<Decision>,
    result: Authorized,
    saved: bool,
    paired: Option<(remote::PairedDevice, Option<u64>)>,
    _permit: Option<OwnedSemaphorePermit>,
}
pub(super) enum Decision {
    Reply(Vec<u8>),
    Open {
        transport: Box<Transport>,
        reply: Option<Vec<u8>>,
    },
    Reject(Reject),
}
struct QueuedFrame {
    frame: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}
struct Entry {
    instance: u64,
    input: mpsc::UnboundedSender<QueuedFrame>,
    budget: Arc<Semaphore>,
    abort: AbortHandle,
    pair: Option<HostPair>,
    fingerprint: Option<String>,
    ready: bool,
    deadline: Instant,
}
pub(super) struct Channels {
    entries: HashMap<u32, Entry>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    pub writes: Arc<TrustWrites>,
}

pub(super) struct TrustWrites {
    issued: AtomicU64,
    saved: Mutex<u64>,
    slots: Arc<Semaphore>,
    // Authorization and save completions must outlive relay loss and remote-task restarts.
    unconfirmed: Mutex<HashMap<String, PendingDevice>>,
    tx: mpsc::Sender<Completion>,
    rx: tokio::sync::Mutex<mpsc::Receiver<Completion>>,
}
struct PendingDevice {
    instance: u64,
    count: usize,
    confirmed: bool,
}
impl Default for TrustWrites {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(16);
        Self {
            issued: AtomicU64::new(0),
            saved: Mutex::new(0),
            slots: Arc::new(Semaphore::new(16)),
            unconfirmed: Mutex::new(HashMap::new()),
            tx,
            rx: tokio::sync::Mutex::new(rx),
        }
    }
}
impl TrustWrites {
    fn job(
        self: &Arc<Self>,
        trust: Trust,
        root: std::path::PathBuf,
    ) -> tokio::task::JoinHandle<Result<(), &'static str>> {
        let generation = self.issued.fetch_add(1, Ordering::Relaxed) + 1;
        let writes = self.clone();
        tokio::task::spawn_blocking(move || writes.persist(generation, &trust, &root))
    }
    fn persist(&self, generation: u64, trust: &Trust, root: &Path) -> Result<(), &'static str> {
        let mut saved = self.saved.lock().map_err(|_| "Trust writer failed")?;
        // A delayed snapshot must never restore a device removed by a newer Revoke or Reset.
        if generation > *saved {
            trust.save(root)?;
            *saved = generation;
        }
        Ok(())
    }
    pub async fn save(
        self: &Arc<Self>,
        trust: Trust,
        root: std::path::PathBuf,
    ) -> Result<(), &'static str> {
        self.job(trust, root)
            .await
            .map_err(|_| "Trust writer failed")?
    }
    pub async fn next(&self) -> Completion {
        self.rx
            .lock()
            .await
            .recv()
            .await
            .expect("trust writer owns sender")
    }
    pub fn complete(
        self: &Arc<Self>,
        mut completion: Completion,
        routes: Option<&mut Channels>,
        machine: &mut Machine,
        root: &Path,
        events: &crate::events::EventChannel,
    ) {
        let Completion {
            channel, instance, ..
        } = completion;
        let present = match &completion.result {
            Ok((Some(_), _, fingerprint)) => machine
                .trust
                .devices
                .iter()
                .any(|d| d.fingerprint == *fingerprint),
            _ => true,
        };
        let mut current = true;
        if let Some((device, Some(token))) = &completion.paired {
            let mut unconfirmed = self.unconfirmed.lock().unwrap();
            let pending = unconfirmed
                .get_mut(&device.fingerprint)
                .filter(|p| p.instance == *token);
            current = pending.is_some();
            let mut rollback = false;
            let mut finished = false;
            if let Some(pending) = pending {
                pending.confirmed |= completion.saved && present;
                pending.count -= 1;
                finished = pending.count == 0;
                rollback = finished && !pending.confirmed;
            }
            if finished {
                unconfirmed.remove(&device.fingerprint);
            }
            if rollback {
                machine
                    .trust
                    .devices
                    .retain(|d| d.fingerprint != device.fingerprint);
                self.job(machine.trust.clone(), root.to_path_buf());
            }
        }
        if !completion.saved {
            log::warn!("remote channel {channel}: could not save trust");
        }
        if !completion.saved || !present || !current {
            completion.result = Err(Reject::BadHandshake);
        } else if let Some((device, _)) = &completion.paired {
            events.emit(
                "remote/paired",
                &serde_json::json!({"name":device.name,"fingerprint":device.fingerprint}),
            );
        }
        let Some(entry) = routes
            .and_then(|routes| routes.entries.get_mut(&channel))
            .filter(|entry| entry.instance == instance)
        else {
            return;
        };
        let decision = match completion.result {
            Ok((Some(transport), response, fingerprint)) => {
                entry.ready = true;
                log::info!("remote channel {channel} opened, device {fingerprint}");
                Decision::Open {
                    transport: Box::new(transport),
                    reply: response,
                }
            }
            Ok((None, Some(response), _)) => Decision::Reply(response),
            Ok(_) => Decision::Reject(Reject::BadHandshake),
            Err(reason) => {
                log::info!(
                    "remote channel {channel}: handshake rejected ({})",
                    reason.reason()
                );
                Decision::Reject(reason)
            }
        };
        let _ = completion.reply.send(decision);
    }
}
impl Drop for Channels {
    fn drop(&mut self) {
        for (channel, entry) in &self.entries {
            entry.abort.abort();
            log::info!(
                "remote channel {channel} closed (route closed), device {}",
                entry.fingerprint.as_deref().unwrap_or("unpaired")
            );
        }
    }
}
impl Channels {
    pub fn new(writes: Arc<TrustWrites>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            entries: HashMap::new(),
            tx,
            rx,
            writes,
        }
    }
    pub fn open(&mut self, id: u32, machine: &Machine) -> Result<(), Reject> {
        if id == 0 || self.entries.contains_key(&id) {
            return Err(Reject::BadHandshake);
        }
        if self.entries.len() == 16 {
            return Err(Reject::Busy);
        }
        let (input, rx) = mpsc::unbounded_channel();
        let budget = Arc::new(Semaphore::new(companion::MAX_MESSAGE_SIZE));
        let tx = self.tx.clone();
        let instance = self.writes.issued.fetch_add(1, Ordering::Relaxed) + 1;
        let deadline = Instant::now() + Duration::from_secs(30);
        let welcome = companion::Welcome {
            version: companion::VERSION,
            name: machine
                .status
                .host_name
                .clone()
                .unwrap_or_else(|| machine.host.hostname.clone()),
            runner_version: machine.host.runner_version.clone(),
            platform: machine.host.platform.clone(),
            features: Vec::new(),
        };
        let task = tokio::spawn(async move {
            let result = AssertUnwindSafe(serve(id, instance, rx, &tx, deadline, welcome))
                .catch_unwind()
                .await;
            let reason = match result {
                Ok(Ok(())) => "closed",
                Ok(Err(reason)) => reason,
                Err(_) => "task failed",
            };
            let _ = tx
                .send(Event::Finished {
                    channel: id,
                    instance,
                    reason,
                })
                .await;
        });
        self.entries.insert(
            id,
            Entry {
                instance,
                input,
                budget,
                abort: task.abort_handle(),
                pair: None,
                fingerprint: None,
                ready: false,
                deadline,
            },
        );
        Ok(())
    }
    pub fn closed(&mut self, id: u32) {
        if let Some(entry) = self.entries.remove(&id) {
            entry.abort.abort();
            log::info!(
                "remote channel {id} closed, device {}",
                entry.fingerprint.as_deref().unwrap_or("unpaired")
            );
        }
    }
    pub fn feed(&mut self, id: u32, frame: &[u8]) -> bool {
        let Some(entry) = self.entries.get(&id) else {
            return true;
        };
        let Ok(permit) = entry
            .budget
            .clone()
            .try_acquire_many_owned((frame.len() + 64) as u32)
        else {
            self.closed(id);
            return false;
        };
        if entry
            .input
            .send(QueuedFrame {
                frame: frame.to_vec(),
                _permit: permit,
            })
            .is_err()
        {
            self.closed(id);
            return false;
        }
        true
    }
    pub async fn next(&mut self) -> Event {
        self.rx
            .recv()
            .await
            .expect("channel event sender is owned by receiver")
    }

    pub fn sync(&self, machine: &mut Machine) {
        for device in &mut machine.trust.devices {
            device.connected = self.entries.values().any(|entry| {
                entry.ready && entry.fingerprint.as_ref() == Some(&device.fingerprint)
            });
        }
    }
    pub fn revoke(&mut self, fingerprint: &str) -> Vec<Message> {
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.fingerprint.as_deref() == Some(fingerprint))
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .map(|id| {
                self.closed(id);
                close_message(id)
            })
            .collect()
    }

    pub fn handle(
        &mut self,
        event: Event,
        machine: &mut Machine,
        root: &Path,
        clock: &Clock,
        events: &crate::events::EventChannel,
    ) -> Option<Message> {
        match event {
            Event::Send {
                channel,
                instance,
                frame,
            } => self
                .entries
                .get(&channel)
                .filter(|entry| entry.instance == instance)
                .map(|_| Message::Binary(channel::wrap(channel, &frame).into())),
            Event::Finished {
                channel,
                instance,
                reason,
            } => {
                if self.entries.get(&channel)?.instance != instance {
                    return None;
                }
                let entry = self.entries.remove(&channel)?;
                log::info!(
                    "remote channel {channel} closed ({reason}), device {}",
                    entry.fingerprint.as_deref().unwrap_or("unpaired")
                );
                Some(close_message(channel))
            }
            Event::Handshake {
                channel,
                instance,
                frame,
                reply,
            } => {
                let entry = self
                    .entries
                    .get_mut(&channel)
                    .filter(|entry| entry.instance == instance)?;
                let previous = machine.trust.clone();
                let mut paired = false;
                let mut unconfirmed = self.writes.unconfirmed.lock().unwrap();
                let result = authorize(entry, &frame, machine, clock, &mut paired, &unconfirmed);
                let mut pairing = None;
                if let Ok((Some(_), _, fingerprint)) = &result {
                    entry.fingerprint = Some(fingerprint.clone());
                    if paired {
                        let token = if !previous
                            .devices
                            .iter()
                            .any(|d| d.fingerprint == *fingerprint)
                        {
                            unconfirmed.insert(
                                fingerprint.clone(),
                                PendingDevice {
                                    instance,
                                    count: 1,
                                    confirmed: false,
                                },
                            );
                            Some(instance)
                        } else {
                            unconfirmed.get_mut(fingerprint).map(|pending| {
                                pending.count += 1;
                                pending.instance
                            })
                        };
                        let device = machine
                            .trust
                            .devices
                            .iter()
                            .find(|d| d.fingerprint == *fingerprint)
                            .unwrap();
                        pairing = Some((device.clone(), token));
                    }
                }
                drop(unconfirmed);
                let needs_save = matches!(&result, Ok((Some(_), _, _)))
                    || machine.trust.devices != previous.devices
                    || machine.trust.failed_pins != previous.failed_pins;
                let mut completion = Completion {
                    channel,
                    instance,
                    reply,
                    result,
                    saved: !needs_save,
                    paired: pairing,
                    _permit: None,
                };
                if needs_save {
                    if let Ok(permit) = self.writes.slots.clone().try_acquire_owned() {
                        completion._permit = Some(permit);
                        let job = self.writes.job(machine.trust.clone(), root.to_path_buf());
                        let tx = self.writes.tx.clone();
                        tokio::spawn(async move {
                            completion.saved = matches!(job.await, Ok(Ok(())));
                            let _ = tx.send(completion).await;
                        });
                        return None;
                    }
                }
                self.writes
                    .clone()
                    .complete(completion, Some(self), machine, root, events);
                None
            }
        }
    }
}

type Authorized = Result<(Option<Transport>, Option<Vec<u8>>, String), Reject>;
fn authorize(
    entry: &mut Entry,
    frame: &[u8],
    machine: &mut Machine,
    clock: &Clock,
    paired: &mut bool,
    unconfirmed: &HashMap<String, PendingDevice>,
) -> Authorized {
    if Instant::now() >= entry.deadline {
        return Err(Reject::BadHandshake);
    }
    if let Some(pair) = entry.pair.take() {
        if machine.trust.pin_locked {
            return Err(Reject::PinLocked);
        }
        match pair.finish(frame) {
            Ok((transport, device)) => {
                machine
                    .trust
                    .record_paired(&device.public_key, device.name, clock.now())
                    .map_err(|_| Reject::BadHandshake)?;
                *paired = true;
                Ok((
                    Some(transport),
                    None,
                    remote::fingerprint(&device.public_key),
                ))
            }
            Err(Reject::WrongPin) => {
                machine.trust.record_failed_pin();
                Err(if machine.trust.pin_locked {
                    Reject::PinLocked
                } else {
                    Reject::WrongPin
                })
            }
            Err(reason) => Err(reason),
        }
    } else {
        let identity = machine.identity.as_ref().ok_or(Reject::NoPin)?;
        let key = identity.host_key().map_err(|_| Reject::BadHandshake)?;
        let keys = NoiseKeys {
            private: &key.private,
            fixed_ephemeral: None,
        };
        match frame.first() {
            Some(3) => {
                let pin = identity.pin.as_ref().ok_or(Reject::NoPin)?;
                if machine.trust.pin_locked {
                    return Err(Reject::PinLocked);
                }
                let mut scalar: [u8; 32] = rand::random();
                scalar[31] &= 15;
                while scalar == [0; 32] {
                    scalar = rand::random();
                    scalar[31] &= 15;
                }
                let (pair, reply) = HostPair::start(keys, &pin.0, frame, scalar)?;
                entry.pair = Some(pair);
                Ok((None, Some(reply), String::new()))
            }
            Some(1) => {
                let (transport, accept, public) = channel::connect(keys, frame, |key| {
                    machine.trust.devices.iter().any(|d| {
                        d.public_key == remote::encode_key(key)
                            && unconfirmed.get(&d.fingerprint).is_none_or(|p| p.confirmed)
                    })
                })?;
                let fingerprint = remote::fingerprint(&public);
                if let Some(device) = machine
                    .trust
                    .devices
                    .iter_mut()
                    .find(|d| d.fingerprint == fingerprint)
                {
                    device.last_seen_at = clock.now();
                }
                Ok((Some(transport), Some(accept), fingerprint))
            }
            _ => Err(Reject::BadHandshake),
        }
    }
}

pub(super) fn close_message(channel: u32) -> Message {
    Message::Text(
        serde_json::to_string(&Control::CloseChannel { channel })
            .unwrap()
            .into(),
    )
}

pub(super) fn transition(
    channels: &mut Option<Channels>,
    machine: &mut Machine,
    effects: &[super::machine::Effect],
    revoke: Option<&str>,
) -> Vec<Message> {
    if effects.iter().any(|effect| {
        matches!(
            effect,
            super::machine::Effect::Close | super::machine::Effect::Connect
        )
    }) {
        *channels = None;
    }
    let frames = match (channels.as_mut(), revoke) {
        (Some(routes), Some(fingerprint)) => routes.revoke(fingerprint),
        _ => Vec::new(),
    };
    if let Some(routes) = channels {
        routes.sync(machine);
    } else {
        for device in &mut machine.trust.devices {
            device.connected = false;
        }
    }
    frames
}

async fn send(
    tx: &mpsc::Sender<Event>,
    channel: u32,
    instance: u64,
    frame: Vec<u8>,
) -> Result<(), &'static str> {
    tx.send(Event::Send {
        channel,
        instance,
        frame,
    })
    .await
    .map_err(|_| "route closed")
}

async fn serve(
    id: u32,
    instance: u64,
    mut input: mpsc::UnboundedReceiver<QueuedFrame>,
    tx: &mpsc::Sender<Event>,
    deadline: Instant,
    welcome: companion::Welcome,
) -> Result<(), &'static str> {
    let handshake = async {
        loop {
            let frame = input.recv().await.ok_or("route closed")?.frame;
            let (reply, rx) = oneshot::channel();
            tx.send(Event::Handshake {
                channel: id,
                instance,
                frame,
                reply,
            })
            .await
            .map_err(|_| "route closed")?;
            match rx.await.map_err(|_| "route closed")? {
                Decision::Reply(frame) => send(tx, id, instance, frame).await?,
                Decision::Open { transport, reply } => {
                    if let Some(frame) = reply {
                        send(tx, id, instance, frame).await?;
                    }
                    return Ok::<_, &'static str>(Some(*transport));
                }
                Decision::Reject(reason) => {
                    send(tx, id, instance, reason.frame()).await?;
                    return Ok(None);
                }
            }
        }
    };
    let transport = match tokio::time::timeout_at(deadline, handshake).await {
        Ok(result) => result?,
        Err(_) => {
            send(tx, id, instance, Reject::BadHandshake.frame()).await?;
            return Err("handshake deadline");
        }
    };
    let Some(transport) = transport else {
        return Ok(());
    };
    let mut records = Records::new(transport);
    let mut hello = false;
    let mut pending = VecDeque::new();
    let mut queued = 0;
    let mut ending = false;
    loop {
        if !records.sending() {
            if let Some(message) = pending.pop_front() {
                queued -= Vec::<u8>::len(&message);
                records.queue(message).map_err(|_| "message too large")?;
            }
        }
        if let Some(frame) = records.next_frame().map_err(|_| "invalid record")? {
            send(tx, id, instance, frame).await?;
            continue;
        }
        if ending && !records.sending() && pending.is_empty() {
            return Ok(());
        }
        let frame = input.recv().await.ok_or("route closed")?.frame;
        if let Some(bytes) = records.receive(&frame).map_err(|_| "invalid record")? {
            let response = if !hello {
                let companion::ClientMessage::Hello(greeting) =
                    companion::ClientMessage::decode(&bytes).map_err(|_| "hello required")?
                else {
                    return Err("hello required");
                };
                hello = true;
                match (companion::VersionRange { min: 1, max: 1 }).negotiate(&greeting.versions) {
                    Ok(_) => companion::HostMessage::Welcome(welcome.clone()),
                    Err(error) => {
                        ending = true;
                        companion::HostMessage::Error(*error)
                    }
                }
            } else {
                match companion::ClientMessage::decode(&bytes) {
                    Err(error) => companion::HostMessage::Error(*error),
                    Ok(companion::ClientMessage::Hello(_)) => return Err("unexpected hello"),
                    Ok(_) => {
                        let envelope: serde_json::Value = serde_json::from_slice(&bytes)
                            .map_err(|_| "invalid companion message")?;
                        companion::HostMessage::Error(companion::ErrorMessage {
                            id: envelope.get("id").and_then(|v| v.as_u64()),
                            code: companion::ErrorCode::UnknownRequest,
                            message: "The host does not serve that request.".into(),
                            versions: None,
                        })
                    }
                }
            };
            let bytes = serde_json::to_vec(&response).map_err(|_| "companion encoding failed")?;
            queued += bytes.len();
            if queued + records.queued_bytes() > companion::MAX_MESSAGE_SIZE {
                return Err("reply queue full");
            }
            pending.push_back(bytes);
            records.handled();
        }
    }
}

#[cfg(test)]
mod tests;

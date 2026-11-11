mod channels;
mod client;
mod machine;
mod persistence;
pub mod store;
mod transport;
mod trust;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use machine::{Command, Effect, Input, Machine};
use persistence::Saved;
use runner_core::remote::{self, RemoteLogout, RemoteStatus, RemoteTrust};
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};
#[cfg(test)]
use store::Secrets;
use store::{Identity, OsStore, SecretStore};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_tungstenite::tungstenite::{
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};
use tokio_util::sync::CancellationToken;

struct Envelope {
    command: Command,
    reply: oneshot::Sender<Result<Reply, String>>,
}
struct Inbox {
    commands: Mutex<mpsc::Receiver<Envelope>>,
    trust_writes: std::sync::OnceLock<Arc<channels::TrustWrites>>,
    companion: Arc<std::sync::OnceLock<Arc<dyn crate::companion::core::Core>>>,
}
pub enum Reply {
    Status(Box<RemoteStatus>),
    Logout(RemoteLogout),
    Trust(RemoteTrust),
}
#[derive(Clone)]
pub struct RemoteHandle {
    client: Option<client::Client>,
    companion: Arc<std::sync::OnceLock<Arc<dyn crate::companion::core::Core>>>,
    tx: mpsc::Sender<Envelope>,
    status: watch::Receiver<RemoteStatus>,
    trust: watch::Receiver<RemoteTrust>,
    cancel: CancellationToken,
    stopped: watch::Receiver<bool>,
    #[cfg(test)]
    restarts: watch::Receiver<u32>,
}
impl Default for RemoteHandle {
    fn default() -> Self {
        let (tx, _) = mpsc::channel(16);
        let (_, status) = watch::channel(RemoteStatus::default());
        let (_, stopped) = watch::channel(true);
        Self {
            client: None,
            companion: Default::default(),
            tx,
            status,
            trust: watch::channel(RemoteTrust::default()).1,
            cancel: CancellationToken::new(),
            stopped,
            #[cfg(test)]
            restarts: watch::channel(0).1,
        }
    }
}
impl RemoteHandle {
    fn client(&self) -> Result<&client::Client, String> {
        self.client
            .as_ref()
            .ok_or_else(|| "Remote client is unavailable".into())
    }
    pub async fn computers(&self) -> Result<Vec<remote::Computer>, String> {
        self.client()?.computers().await
    }
    pub async fn pair(&self, computer: String, pin: remote::Secret) -> Result<(), String> {
        self.client()?.pair(computer, pin).await
    }
    pub async fn forget(&self, computer: String) -> Result<(), String> {
        self.client()?.forget(computer).await
    }
    pub async fn overview(
        &self,
        computer: String,
    ) -> Result<runner_core::companion::Overview, String> {
        self.client()?.overview(computer).await
    }
    pub async fn companion_request(
        &self,
        computer: String,
        request: runner_core::companion::ClientMessage,
    ) -> Result<runner_core::remote::RemoteResult, String> {
        self.client()?.request(computer, request).await
    }
    pub async fn follow(&self, computer: String, owner: u64, follow: bool) -> Result<(), String> {
        self.client()?.follow(computer, owner, follow).await
    }
    pub async fn attach_remote(
        &self,
        config: remote::RemoteTerminal,
    ) -> Result<runner_core::protocol::terminal::TerminalAttachment, String> {
        self.client()?.attach(config).await
    }
    pub async fn remote_input(
        &self,
        computer: String,
        session: String,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        self.client()?.input(computer, session, bytes).await
    }
    pub async fn remote_resize(
        &self,
        computer: String,
        session: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        self.client()?
            .queue_viewport(computer, session, cols, rows)
            .await
    }
    pub async fn remote_viewport(
        &self,
        computer: String,
        session: String,
        cols: u16,
        rows: u16,
        take: bool,
    ) -> Result<(), String> {
        self.client()?
            .viewport(computer, session, cols, rows, take)
            .await
    }
    pub fn set_companion_core(&self, core: Arc<dyn crate::companion::core::Core>) {
        let _ = self.companion.set(core);
    }
    pub fn status(&self) -> RemoteStatus {
        self.status.borrow().clone()
    }
    pub fn trust(&self) -> RemoteTrust {
        self.trust.borrow().clone()
    }
    pub async fn set_pin(&self, pin: remote::Secret) -> Result<RemoteTrust, String> {
        trust::validate_pin(&pin.0)?;
        self.trust_command(Command::SetPin(pin)).await
    }
    pub async fn unlock_pin(&self) -> Result<RemoteTrust, String> {
        self.trust_command(Command::UnlockPin).await
    }
    pub async fn revoke_device(&self, fingerprint: String) -> Result<RemoteTrust, String> {
        self.trust_command(Command::RevokeDevice(fingerprint)).await
    }
    pub async fn record_failed_pin(&self) -> Result<RemoteTrust, String> {
        self.trust_command(Command::RecordFailedPin).await
    }
    pub async fn record_paired(
        &self,
        public_key: Vec<u8>,
        name: String,
    ) -> Result<RemoteTrust, String> {
        self.trust_command(Command::RecordPaired { public_key, name })
            .await
    }
    async fn trust_command(&self, command: Command) -> Result<RemoteTrust, String> {
        match self.request(command).await? {
            Reply::Trust(trust) => Ok(trust),
            _ => Err("Invalid remote reply".into()),
        }
    }
    pub async fn start(
        &self,
        origin: Option<String>,
        confirm_account: bool,
    ) -> Result<RemoteStatus, String> {
        self.start_owned(origin, confirm_account, None).await
    }
    pub(crate) async fn start_owned(
        &self,
        origin: Option<String>,
        confirm_account: bool,
        owner: Option<u64>,
    ) -> Result<RemoteStatus, String> {
        self.status_command(Command::Start {
            origin,
            confirm_account,
            owner,
        })
        .await
    }
    pub async fn confirm(&self, allow: bool) -> Result<RemoteStatus, String> {
        self.status_command(Command::Confirm(allow)).await
    }
    pub async fn cancel(&self) -> Result<RemoteStatus, String> {
        self.status_command(Command::Cancel).await
    }
    pub async fn pause(&self, paused: bool) -> Result<RemoteStatus, String> {
        self.status_command(Command::Pause(paused)).await
    }
    pub async fn retry(&self) -> Result<RemoteStatus, String> {
        self.status_command(Command::Retry).await
    }
    pub async fn logout(&self) -> Result<RemoteLogout, String> {
        match self.request(Command::Logout).await? {
            Reply::Logout(r) => Ok(r),
            _ => Err("Invalid remote reply".into()),
        }
    }
    pub async fn reset_identity(&self) -> Result<RemoteTrust, String> {
        let identity = Identity::generate()?;
        self.client()?.reset().await?;
        self.trust_command(Command::ResetIdentity(identity)).await
    }
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let mut stopped = self.stopped.clone();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !*stopped.borrow_and_update() {
                if stopped.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
    pub fn owner_gone(&self, owner: u64) {
        if let Some(client) = self.client.clone() {
            tokio::spawn(async move {
                client.owner_gone(owner).await;
            });
        }
        let (reply, _) = oneshot::channel();
        let envelope = Envelope {
            command: Command::OwnerGone(owner),
            reply,
        };
        if let Err(mpsc::error::TrySendError::Full(envelope)) = self.tx.try_send(envelope) {
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(envelope).await;
            });
        }
    }
    async fn status_command(&self, command: Command) -> Result<RemoteStatus, String> {
        match self.request(command).await? {
            Reply::Status(s) => Ok(*s),
            _ => Err("Invalid remote reply".into()),
        }
    }
    async fn request(&self, command: Command) -> Result<Reply, String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Envelope { command, reply })
            .await
            .map_err(|_| "Remote access task is unavailable")?;
        rx.await
            .map_err(|_| "Remote access task restarted; try again".to_owned())?
    }
}

pub fn spawn(root: PathBuf, events: crate::events::EventChannel, isolated: bool) -> RemoteHandle {
    let store: Arc<dyn SecretStore> = if isolated {
        Arc::new(store::MemoryStore::default())
    } else {
        Arc::new(OsStore::new(&root, cfg!(debug_assertions)))
    };
    spawn_with_store(root, store, events)
}
fn spawn_with_store(
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    events: crate::events::EventChannel,
) -> RemoteHandle {
    spawn_with_clock(root, store, events, Clock::default())
}
fn spawn_with_clock(
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    events: crate::events::EventChannel,
    clock: Clock,
) -> RemoteHandle {
    let (tx, rx) = mpsc::channel(16);
    let initial_status = Saved::load(&root)
        .ok()
        .flatten()
        .map(|saved| RemoteStatus {
            state: if let Some(reason) = saved.stopped {
                remote::RemoteState::Stopped { reason }
            } else if saved.paused {
                remote::RemoteState::Paused
            } else {
                remote::RemoteState::Connecting { attempt: 0 }
            },
            origin: saved.origin,
            account: Some(saved.account),
            host_name: Some(saved.host_name),
            host_id: Some(saved.host_id),
            fingerprint: Some(saved.fingerprint),
            last_error: None,
        })
        .unwrap_or_default();
    let (status_tx, status) = watch::channel(initial_status);
    let (trust_tx, trust) = watch::channel(RemoteTrust::default());
    let status_tx = Snapshots {
        status: status_tx,
        trust: trust_tx,
    };
    let companion = Arc::new(std::sync::OnceLock::new());
    let rx = Arc::new(Inbox {
        companion: companion.clone(),
        commands: Mutex::new(rx),
        trust_writes: std::sync::OnceLock::new(),
    });
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let (stopped_tx, stopped) = watch::channel(false);
    #[cfg(test)]
    let (restart_tx, restarts) = watch::channel(0);
    let client_root = root.clone();
    let client_store = store.clone();
    let client_events = events.clone();
    tokio::spawn(async move {
        let mut attempt = 0;
        loop {
            let mut task = tokio::spawn(run(
                root.clone(),
                store.clone(),
                events.clone(),
                status_tx.clone(),
                rx.clone(),
                stop.clone(),
                clock.clone(),
            ));
            let result = tokio::select! {
                result = &mut task => Some(result),
                _ = stop.cancelled() => {
                    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut task).await.is_err() { task.abort(); }
                    None
                }
            };
            let error = match result {
                Some(Ok(Ok(()))) | None => break,
                Some(Ok(Err(error))) => remote::redact(&error),
                Some(Err(error)) => remote::redact(&format!("Remote worker failed: {error}")),
            };
            log::error!(
                "remote task ended unexpectedly: {error}; restarting independently of sessions"
            );
            let mut status = status_tx.status.borrow().clone();
            status.last_error = Some(error.clone());
            status_tx.status.send_replace(status.clone());
            events.emit("remote/status", &status);
            #[cfg(test)]
            restart_tx.send_modify(|count| *count += 1);
            let delay = machine::backoff(attempt, rand::random());
            attempt = attempt.saturating_add(1);
            let mut commands = rx.commands.lock().await;
            let delay = tokio::time::sleep(std::time::Duration::from_millis(delay as u64));
            tokio::pin!(delay);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    envelope = commands.recv() => {
                        let Some(envelope) = envelope else { break; };
                        let _ = envelope.reply.send(Err(error.clone()));
                    }
                    _ = &mut delay => break,
                }
            }
            if stop.is_cancelled() || commands.is_closed() {
                break;
            }
        }
        stopped_tx.send_replace(true);
    });
    let client = client::Client::new(
        client_root,
        client_store,
        status.clone(),
        client_events,
        cancel.clone(),
    );
    RemoteHandle {
        client: Some(client),
        companion,
        tx,
        status,
        trust,
        cancel,
        stopped,
        #[cfg(test)]
        restarts,
    }
}

#[derive(Clone)]
struct Snapshots {
    status: watch::Sender<RemoteStatus>,
    trust: watch::Sender<RemoteTrust>,
}

#[derive(Debug)]
enum ApplyError {
    Command(String),
    Persistence(&'static str),
}

async fn apply_input(
    machine: &mut Machine,
    input: Input,
    clock: &Clock,
    root: &std::path::Path,
    trust_writes: &std::sync::OnceLock<Arc<channels::TrustWrites>>,
) -> Result<Vec<Effect>, ApplyError> {
    let generation = machine.pairing_generation;
    // Reset may clear trust before its new identity is durably saved.
    let previous = machine.trust.clone();
    let (effects, _) = machine
        .step(input, clock.now(), &mut rand::random)
        .map_err(ApplyError::Command)?;
    if machine.pairing_generation != generation {
        if let Some(writes) = trust_writes.get() {
            let removed = writes
                .invalidate_pending(previous, root.to_path_buf())
                .await
                .map_err(ApplyError::Persistence)?;
            machine
                .trust
                .devices
                .retain(|device| !removed.contains(&device.fingerprint));
        }
    }
    Ok(effects)
}

async fn run(
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    events: crate::events::EventChannel,
    status_tx: Snapshots,
    inbox: Arc<Inbox>,
    cancel: CancellationToken,
    clock: Clock,
) -> Result<(), String> {
    let startup_error = status_tx.status.borrow().last_error.clone();
    let mut rx = inbox.commands.lock().await;
    let prepare_root = root.clone();
    let prepare_store = store.clone();
    let mut preparation = tokio::task::spawn_blocking(move || -> Result<_, &'static str> {
        let client = transport::client()?;
        let saved = Saved::load(&prepare_root)?;
        let (identity, secrets) = store::restore(
            prepare_store.as_ref(),
            saved.as_ref().map(|s| s.origin.as_str()),
        )?;
        let trust = trust::Trust::load(&prepare_root)?;
        Ok((client, saved, identity, secrets, trust))
    });
    let (client, saved, identity, secrets, trust) = loop {
        tokio::select! {
            prepared = &mut preparation => break prepared.map_err(|error| format!("Remote startup worker failed: {error}"))??,
            _ = cancel.cancelled() => return Ok(()),
            envelope = rx.recv(), if startup_error.is_some() => {
                let Some(envelope) = envelope else { return Ok(()); };
                let _ = envelope.reply.send(Err(startup_error.clone().unwrap()));
            }
        }
    };
    let (mut machine, initial) = Machine::restore(saved, secrets, clock.now());
    machine.set_identity(identity);
    machine.trust = trust;
    machine.host = remote::HostInfo::current();
    let mut socket = None;
    let trust_writes = &inbox.trust_writes;
    let initialized = {
        let initialization = perform(
            &mut machine,
            initial,
            &mut socket,
            &client,
            &store,
            &root,
            &events,
            &status_tx,
            &clock,
            trust_writes,
        );
        tokio::pin!(initialization);
        loop {
            tokio::select! {
                result = &mut initialization => break Some(result),
                _ = cancel.cancelled() => break None,
                envelope = rx.recv(), if startup_error.is_some() => {
                    let Some(envelope) = envelope else { return Ok(()); };
                    let _ = envelope.reply.send(Err(startup_error.clone().unwrap()));
                }
            }
        }
    };
    match initialized {
        Some(result) => {
            result?;
        }
        None => {
            close(&mut socket).await;
            return Ok(());
        }
    }
    if let Some(error) = startup_error {
        while let Ok(envelope) = rx.try_recv() {
            let _ = envelope.reply.send(Err(error.clone()));
        }
    }
    let mut pending_start: Option<oneshot::Sender<Result<Reply, String>>> = None;
    let mut channels: Option<channels::Channels> = None;
    publish(&machine, &events, &status_tx);
    loop {
        let deadline = machine.deadline.map(|at| {
            at.signed_duration_since(clock.now())
                .to_std()
                .unwrap_or_default()
        });
        let delay = async {
            match deadline {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        let writer = socket.as_ref().map(|s: &Relay| s.tx.clone());
        let writer_ready = writer
            .as_ref()
            .is_some_and(|tx| tx.capacity() > CONTROL_HEADROOM);
        let (stream, task) = match socket.as_mut() {
            Some(s) => (Some(&mut s.stream), Some(&mut s.task)),
            None => (None, None),
        };
        let read = async {
            match stream {
                Some(s) => s.next().await,
                None => std::future::pending().await,
            }
        };
        let (input, reply) = tokio::select! {
            _ = cancel.cancelled() => (Input::Command(Command::Shutdown), None),
            envelope = rx.recv() => {
                let Some(e) = envelope else { close(&mut socket).await; return Ok(()); };
                (Input::Command(e.command), Some(e.reply))
            }
            _ = delay => (Input::Deadline, None),
            _ = async { match task { Some(task) => { let _ = task.await; }, None => std::future::pending().await } } => (Input::Failure(transport::Failure::network()), None),
            _ = async { match &writer { Some(tx) => { let _ = tx.reserve_many(CONTROL_HEADROOM + 1).await; }, None => std::future::pending().await } }, if !writer_ready => continue,
            completion = async {
                match trust_writes.get() { Some(writes) => writes.next().await, None => std::future::pending().await }
            } => {
                trust_writes.get().unwrap().complete(completion, channels.as_mut(), &mut machine, &root, &events);
                if let Some(routes) = &channels { routes.sync(&mut machine); }
                publish(&machine, &events, &status_tx);
                continue;
            },
            event = async {
                match &mut channels { Some(routes) => routes.next().await, None => std::future::pending().await }
            }, if writer_ready => {
                let changed = !matches!(event, channels::Event::Send { .. });
                let output = channels.as_mut().unwrap().handle(event, &mut machine, &root, &clock, &events);
                if changed {
                    channels.as_ref().unwrap().sync(&mut machine);
                    publish(&machine, &events, &status_tx);
                }
                match output.map(|frame| send_frames(&socket, [frame])) {
                    Some(Err(f)) => (Input::Failure(f), None),
                    _ => continue,
                }
            },
            frame = read => {
                let input = match frame {
                    Some(Ok(Message::Text(text))) => {
                        let mut failure = None;
                        if text.len() <= remote::MAX_CONTROL_SIZE && matches!(machine.status.state, remote::RemoteState::Connected { .. }) {
                            match serde_json::from_str::<remote::Control>(&text) {
                                Ok(remote::Control::ChannelOpen { channel }) => {
                                    let writes=trust_writes.get_or_init(||Arc::new(channels::TrustWrites::default())).clone();
                                    let routes = channels.get_or_insert_with(||channels::Channels::new(writes, inbox.companion.clone()));
                                    if let Err(reason) = routes.open(channel, &machine) {
                                        let frames = [Message::Binary(runner_core::companion::channel::wrap(channel, &reason.frame()).into()), channels::close_message(channel)];
                                        failure = send_frames(&socket, frames).err();
                                    }
                                }
                                Ok(remote::Control::ChannelClosed { channel }) => {
                                    if let Some(routes) = &mut channels { routes.closed(channel); }
                                }
                                _ => {}
                            }
                        }
                        match failure { Some(failure) => Input::Failure(failure), None => Input::Text(text.to_string()) }
                    },
                    Some(Ok(Message::Close(frame))) => {
                        let code = frame.map(|f| u16::from(f.code)).unwrap_or(1006);
                        log::info!("remote close: {code}");
                        Input::Failure(transport::Failure { retry: remote::close_action(code), message: format!("Remote connection closed ({code})."), code: "unknown".into() })
                    }
                    Some(Ok(Message::Ping(_))) => {
                        if let Some(writer) = &writer { let _ = writer.try_send(Write::Flush); }
                        Input::Frame
                    },
                    Some(Ok(Message::Binary(bytes))) => {
                        let mut input = Input::Frame;
                        if let (Some(routes), Some((id, frame))) = (&mut channels, runner_core::companion::channel::unwrap(&bytes)) {
                            if !routes.feed(id, frame) {
                                routes.sync(&mut machine);
                                publish(&machine, &events, &status_tx);
                                if let Err(failure) = send_frames(&socket, [channels::close_message(id)]) { input = Input::Failure(failure); }
                            }
                        }
                        input
                    }
                    Some(Ok(_)) => Input::Frame,
                    _ => Input::Failure(transport::Failure::network()),
                };
                (input, None)
            }
        };
        let changed = !matches!(&input, Input::Frame);
        #[cfg(test)]
        if let Input::Command(Command::TestSocket(test)) = input {
            socket = Some(Relay::new(*test.0));
            machine.status.state = remote::RemoteState::Connected { since: clock.now() };
            machine.deadline = Some(clock.now() + chrono::Duration::seconds(75));
            if let Some(reply) = reply {
                let _ = reply.send(Ok(Reply::Status(Box::new(machine.status.clone()))));
            }
            publish(&machine, &events, &status_tx);
            continue;
        }
        let logout = matches!(&input, Input::Command(Command::Logout));
        let starting = matches!(&input, Input::Command(Command::Start { .. }));
        let trust_command = matches!(
            &input,
            Input::Command(
                Command::SetPin(_)
                    | Command::UnlockPin
                    | Command::RevokeDevice(_)
                    | Command::ResetIdentity(_)
                    | Command::RecordFailedPin
                    | Command::RecordPaired { .. }
            )
        );
        let needs_identity = starting || matches!(&input, Input::Command(Command::SetPin(_)));
        if needs_identity && machine.identity.is_none() {
            let store = store.clone();
            let identity = tokio::task::spawn_blocking(move || {
                let identity = Identity::generate()?;
                identity.save(store.as_ref())?;
                Ok::<_, &'static str>(identity)
            })
            .await
            .map_err(|_| "Host identity worker failed")?;
            match identity {
                Ok(identity) => machine.set_identity(Some(identity)),
                Err(error) => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(error.into()));
                    }
                    continue;
                }
            }
        }
        let revoke = match &input {
            Input::Command(Command::RevokeDevice(fingerprint)) => Some(fingerprint.clone()),
            _ => None,
        };
        let mut effects = match apply_input(&mut machine, input, &clock, &root, trust_writes).await
        {
            Ok(effects) => effects,
            Err(ApplyError::Command(error)) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(error));
                }
                continue;
            }
            Err(ApplyError::Persistence(error)) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(error.into()));
                }
                return Err(error.into());
            }
        };
        let changed = changed || !effects.is_empty();
        let frames = channels::transition(&mut channels, &mut machine, &effects, revoke.as_deref());
        if !frames.is_empty() {
            if let Err(failure) = send_frames(&socket, frames) {
                effects.extend(
                    apply_input(
                        &mut machine,
                        Input::Failure(failure),
                        &clock,
                        &root,
                        trust_writes,
                    )
                    .await
                    .map_err(|_| "Invalid remote transition")?,
                );
                channels::transition(&mut channels, &mut machine, &effects, None);
            }
        }
        if starting {
            if let Some(previous) = pending_start.take() {
                let _ = previous.send(Err("Enrollment replaced.".into()));
            }
        }
        let completed = tokio::select! {
            result = perform(
            &mut machine,
            effects,
            &mut socket,
            &client,
            &store,
            &root,
            &events,
            &status_tx,
            &clock,
            trust_writes,
        )
         => Some(result),
            _ = cancel.cancelled() => None,
        };
        let outcome = match completed {
            Some(result) => result?,
            None => {
                close(&mut socket).await;
                return Ok(());
            }
        };
        if socket.is_none() {
            channels = None;
        }
        if let Some(routes) = &channels {
            routes.sync(&mut machine);
        } else {
            for device in &mut machine.trust.devices {
                device.connected = false;
            }
        }
        if changed {
            publish(&machine, &events, &status_tx);
        }
        let waiting_for_code =
            matches!(machine.status.state, remote::RemoteState::Off) && machine.deadline.is_some();
        if starting && waiting_for_code {
            if let Some(previous) = pending_start.take() {
                let _ = previous.send(Err("Enrollment replaced.".into()));
            }
            pending_start = reply;
        } else {
            if !waiting_for_code {
                if let Some(pending) = pending_start.take() {
                    let _ = pending.send(Ok(Reply::Status(Box::new(machine.status.clone()))));
                }
            }
            if let Some(reply) = reply {
                let _ = reply.send(Ok(if logout {
                    Reply::Logout(outcome)
                } else if trust_command {
                    Reply::Trust(machine.trust())
                } else {
                    Reply::Status(Box::new(machine.status.clone()))
                }));
            }
        }

        if machine.shutting_down {
            return Ok(());
        }
    }
}
fn publish(machine: &Machine, events: &crate::events::EventChannel, tx: &Snapshots) {
    let trust = machine.trust();
    if *tx.trust.borrow() != trust {
        tx.trust.send_replace(trust.clone());
        events.emit("remote/trust", &trust);
    }
    publish_status(machine, events, tx);
}
fn publish_status(machine: &Machine, events: &crate::events::EventChannel, tx: &Snapshots) {
    if *tx.status.borrow() != machine.status {
        let state = match machine.status.state {
            remote::RemoteState::Off => "off",
            remote::RemoteState::Enrolling { .. } => "enrolling",
            remote::RemoteState::Confirming { .. } => "confirming",
            remote::RemoteState::Connecting { .. } => "connecting",
            remote::RemoteState::Connected { .. } => "connected",
            remote::RemoteState::Reconnecting { .. } => "reconnecting",
            remote::RemoteState::Paused => "paused",
            remote::RemoteState::Stopped { .. } => "stopped",
        };
        log::info!(
            "remote: state {state}, origin {}",
            remote::redact(&machine.status.origin)
        );
        tx.status.send_replace(machine.status.clone());
        events.emit("remote/status", &machine.status);
    }
}
#[allow(clippy::too_many_arguments)]
async fn perform(
    machine: &mut Machine,
    effects: Vec<Effect>,
    socket: &mut Option<Relay>,
    client: &reqwest::Client,
    store: &Arc<dyn SecretStore>,
    root: &std::path::Path,
    events: &crate::events::EventChannel,
    tx: &Snapshots,
    clock: &Clock,
    trust_writes: &std::sync::OnceLock<Arc<channels::TrustWrites>>,
) -> Result<RemoteLogout, &'static str> {
    let mut queue: std::collections::VecDeque<_> = effects.into();
    let mut outcome = RemoteLogout::Removed;
    while let Some(effect) = queue.pop_front() {
        let input = match effect {
            Effect::Log(message) => {
                log::info!("{}", remote::redact(&message));
                None
            }
            Effect::Start(request) => {
                let remaining = machine
                    .start_deadline()
                    .and_then(|at| at.signed_duration_since(clock.now()).to_std().ok())
                    .unwrap_or_default();
                Some(Input::Started(
                    tokio::time::timeout(
                        remaining,
                        transport::rpc(
                            client,
                            &machine.status.origin,
                            "StartEnrollment",
                            None,
                            &request,
                        ),
                    )
                    .await
                    .unwrap_or_else(|_| Err(transport::Failure::network())),
                ))
            }
            Effect::Poll(request) => Some(Input::Polled(
                transport::rpc(
                    client,
                    &machine.status.origin,
                    "PollEnrollment",
                    None,
                    &request,
                )
                .await,
            )),
            Effect::Connect => {
                publish_status(machine, events, tx);
                close(socket).await;
                if let (Some(s), Some(secrets)) = (&machine.saved, &machine.secrets) {
                    match transport::connect(
                        client,
                        &s.relay_url,
                        &secrets.credential,
                        &remote::HostInfo::current(),
                    )
                    .await
                    {
                        Ok(connection) => {
                            *socket = Some(Relay::new(connection));
                            machine.deadline = Some(
                                (clock.now() + chrono::Duration::seconds(10)).min(if s.activated {
                                    chrono::DateTime::<Utc>::MAX_UTC
                                } else {
                                    s.claim_expires_at
                                }),
                            );
                            None
                        }
                        Err(f) => Some(Input::Failure(f)),
                    }
                } else {
                    Some(Input::Failure(transport::Failure::protocol()))
                }
            }
            Effect::Close => {
                close(socket).await;
                None
            }
            Effect::Store => {
                let store = store.clone();
                let origin = machine.status.origin.clone();
                let secrets = machine.secrets.clone();
                let saved = machine.saved.clone();
                let root = root.to_path_buf();
                tokio::task::spawn_blocking(move || -> Result<(), &'static str> {
                    if let Some(secrets) = secrets {
                        let bytes = serde_json::to_vec(&secrets)
                            .map_err(|_| "Could not encode remote secrets")?;
                        store.put(&origin, &bytes)?;
                    }
                    if let Some(saved) = saved {
                        saved.save(&root)?;
                    }
                    Ok(())
                })
                .await
                .map_err(|_| "Remote secret-store worker failed")??;
                None
            }
            Effect::SaveTrust => {
                let trust = machine.trust.clone();
                let root = root.to_path_buf();
                if let Some(writes) = trust_writes.get() {
                    writes.save(trust, root).await?;
                } else {
                    tokio::task::spawn_blocking(move || trust.save(&root))
                        .await
                        .map_err(|_| "Remote trust worker failed")??;
                }
                None
            }
            Effect::SaveIdentity => {
                let store = store.clone();
                let identity = machine
                    .identity
                    .clone()
                    .ok_or("Host identity is unavailable")?;
                tokio::task::spawn_blocking(move || identity.save(store.as_ref()))
                    .await
                    .map_err(|_| "Host identity worker failed")??;
                None
            }
            Effect::Save => {
                if let Some(saved) = &machine.saved {
                    let saved = saved.clone();
                    let root = root.to_path_buf();
                    tokio::task::spawn_blocking(move || saved.save(&root))
                        .await
                        .map_err(|_| "Remote state worker failed")??;
                }
                None
            }
            Effect::Delete(origin) => {
                let store = store.clone();
                let root = root.to_path_buf();
                tokio::task::spawn_blocking(move || {
                    store.delete(&origin)?;
                    Saved::delete(&root)
                })
                .await
                .map_err(|_| "Remote cleanup worker failed")??;
                None
            }
            Effect::SignOut {
                origin,
                credential,
                name,
                account,
            } => {
                publish_status(machine, events, tx);
                let result: Result<remote::Empty, _> = transport::rpc(
                    client,
                    &origin,
                    "SignOutHost",
                    Some(&credential),
                    &remote::Empty {},
                )
                .await;
                if let Err(f) = result {
                    if f.code != "unauthenticated" {
                        let error = format!("Couldn't reach the service, so \"{name}\" is still listed on @{account}'s account. Remove it in the web panel.");
                        machine.status.last_error = Some(error.clone());
                        outcome = RemoteLogout::StillListed { error };
                    }
                }
                None
            }
        };
        if let Some(input) = input {
            let effects = apply_input(machine, input, clock, root, trust_writes)
                .await
                .map_err(|error| match error {
                    ApplyError::Command(_) => "Invalid remote transition",
                    ApplyError::Persistence(error) => error,
                })?;
            queue.extend(effects);
        }
    }
    Ok(outcome)
}
const WRITER_CAPACITY: usize = 64;
const CONTROL_HEADROOM: usize = channels::MAX_CHANNELS + 2;

enum Write {
    Frame(Message),
    Flush,
}
struct Relay {
    stream: futures_util::stream::SplitStream<transport::Socket>,
    tx: mpsc::Sender<Write>,
    task: tokio::task::JoinHandle<()>,
}
impl Relay {
    fn new(socket: transport::Socket) -> Self {
        let (mut sink, stream) = socket.split();
        let (tx, mut rx) = mpsc::channel::<Write>(WRITER_CAPACITY);
        let task = tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut next = Some(first);
                let mut closing = false;
                for index in 0..WRITER_CAPACITY {
                    let Some(frame) = next.take() else {
                        break;
                    };
                    if let Write::Frame(frame) = frame {
                        closing = matches!(frame, Message::Close(_));
                        if !matches!(
                            tokio::time::timeout(Duration::from_secs(10), sink.feed(frame)).await,
                            Ok(Ok(()))
                        ) {
                            return;
                        }
                    }
                    if closing || index + 1 == WRITER_CAPACITY {
                        break;
                    }
                    next = rx.try_recv().ok();
                }
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(10), sink.flush()).await,
                    Ok(Ok(()))
                ) || closing
                {
                    return;
                }
            }
        });
        Self { stream, tx, task }
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn send_frames(
    socket: &Option<Relay>,
    frames: impl IntoIterator<Item = Message>,
) -> Result<(), transport::Failure> {
    let socket = socket.as_ref().ok_or_else(transport::Failure::network)?;
    for frame in frames {
        socket
            .tx
            .try_send(Write::Frame(frame))
            .map_err(|_| transport::Failure::network())?;
    }
    Ok(())
}
async fn close(socket: &mut Option<Relay>) {
    if let Some(mut s) = socket.take() {
        let _ = s.tx.try_send(Write::Frame(Message::Close(Some(CloseFrame {
            code: CloseCode::Away,
            reason: "".into(),
        }))));
        if !s.task.is_finished() {
            let _ = tokio::time::timeout(Duration::from_secs(1), &mut s.task).await;
        }
    }
}

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
struct Clock {
    #[cfg(test)]
    wall: Option<Arc<std::sync::Mutex<chrono::DateTime<Utc>>>>,
}
impl Clock {
    fn now(&self) -> chrono::DateTime<Utc> {
        #[cfg(test)]
        if let Some(wall) = &self.wall {
            return *wall.lock().unwrap();
        }
        Utc::now()
    }
}

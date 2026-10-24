use std::collections::{HashMap, HashSet};

use crate::surfaces::front_page::{FrontPageClient, FrontPageTarget, RemoteAccess};
use futures::{channel::mpsc, future::BoxFuture, FutureExt, StreamExt};
use gpui::{AppContext, Context, Entity};
use runner_core::{
    companion::{NeedsYouReason, Overview, SessionStatus, Status, StatusPush},
    protocol::{ClientEvent, DaemonClient, EventError},
    remote::{Computer, PairedDevice, RemoteStatus, RemoteTrust},
};
use std::sync::Arc;

pub(crate) fn agent_status(status: &SessionStatus) -> runner_core::protocol::AgentStatus {
    use runner_core::protocol::{
        Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle, WaitReason,
    };
    // The overview has flags without timestamps; equal markers retain host pane order in rollups.
    let marker = i64::MAX;
    let kind = status.status.effective();
    AgentStatus {
        lifecycle: match kind {
            Status::Starting => Lifecycle::Starting,
            Status::Stopped => Lifecycle::Stopped,
            Status::Error => Lifecycle::Error,
            _ => Lifecycle::Running,
        },
        observation: AgentObservation {
            activity: match kind {
                Status::Working => Activity::Working,
                Status::NeedsYou | Status::Idle => Activity::Idle,
                _ => Activity::Unavailable,
            },
            interactions: (kind == Status::NeedsYou)
                .then(|| HumanInteraction {
                    id: String::new(),
                    reason: match status.reason {
                        Some(NeedsYouReason::Approval) => WaitReason::Approval,
                        Some(NeedsYouReason::Answer) => WaitReason::Answer,
                        _ => WaitReason::Unknown,
                    },
                    owners: Vec::new(),
                    since: marker,
                })
                .into_iter()
                .collect(),
            ..Default::default()
        },
        error_since: (kind == Status::Error).then_some(marker),
        unread_since: status.unread.then_some(marker),
        ..Default::default()
    }
}

pub(crate) fn this_computer(status: &RemoteStatus, show_host_name: bool) -> Computer {
    Computer {
        id: status.host_id.clone().unwrap_or_default(),
        name: status
            .host_name
            .as_ref()
            .filter(|_| show_host_name)
            .cloned()
            .unwrap_or_else(|| crate::i18n::text("client.this_computer").into()),
        is_self: true,
        ..Default::default()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Connection {
    #[default]
    Opening,
    Connected,
    Reconnecting,
    Unreachable,
    Revoked,
    Forgotten,
}

impl Connection {
    pub(crate) fn failure(error: &str) -> Self {
        if [
            "unknownDevice",
            "pinLocked",
            "pinned",
            "badHandshake",
            "Pair with",
            "identity does not match",
        ]
        .iter()
        .any(|reason| error.contains(reason))
        {
            Self::Revoked
        } else {
            Self::Unreachable
        }
    }
}

pub(crate) enum Command {
    Refresh,
    Acquire(String),
    Release(String),
    Retry(String),
    Forget(String),
    Reconnected,
    Paired(String),
}

pub(super) enum Update {
    Access(bool),
    Computers(Result<Vec<Computer>, ()>),
    Overview(String, Overview),
    Connection(String, Connection),
    Event(ClientEvent),
    Forgotten(String, bool),
}

pub(crate) struct Follow {
    computer: String,
    commands: mpsc::UnboundedSender<Command>,
}
impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self
            .commands
            .unbounded_send(Command::Release(self.computer.clone()));
    }
}

pub(crate) struct ClientState {
    pub(crate) computers: Vec<Computer>,
    pub(crate) overviews: HashMap<String, Overview>,
    pub(crate) overview_revisions: HashMap<String, u64>,
    pub(crate) connections: HashMap<String, Connection>,
    pub(crate) changed_identities: HashSet<String>,
    pub(crate) directory_error: bool,
    pub(crate) loading: bool,
    pub(crate) action_failed: bool,
    pub(crate) terminal_epoch: u64,
    client: DaemonClient,
    capabilities: HashMap<String, Arc<RemoteAccess>>,
    capability_generation: u64,
    pub(super) pair_epochs: HashMap<String, u64>,
    signed_in: Option<bool>,
    attention_follows: HashMap<String, Follow>,
    enabled: bool,
    commands: mpsc::UnboundedSender<Command>,
    _tasks: Vec<gpui::Task<()>>,
}

impl ClientState {
    pub(crate) fn new(client: DaemonClient, cx: &mut Context<Self>) -> Self {
        let (commands, requests) = mpsc::unbounded();
        let (updates, mut responses) = mpsc::unbounded();
        let event_updates = updates.clone();
        let mut events = client.subscribe();
        let reconnect = commands.clone();
        let event_task = cx.background_spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) if event.name == "daemon/reconnected" => {
                        if event_updates.unbounded_send(Update::Event(event)).is_err() {
                            break;
                        }
                        let _ = reconnect.unbounded_send(Command::Reconnected);
                    }
                    Ok(event)
                        if matches!(
                            event.name.as_str(),
                            "remote/overview"
                                | "remote/session-status"
                                | "remote/connection"
                                | "remote/status"
                                | "daemon/disconnected"
                        ) =>
                    {
                        if event_updates.unbounded_send(Update::Event(event)).is_err() {
                            break;
                        }
                    }
                    Err(EventError::Lagged(_)) => {
                        let _ = reconnect.unbounded_send(Command::Reconnected);
                    }
                    Ok(_) => {}
                    Err(EventError::Closed) => break,
                }
            }
        });
        let executor = cx.background_executor().clone();
        let command_task = cx.background_spawn(run_commands(
            client.clone(),
            requests,
            updates,
            move |job| {
                executor.spawn(job).detach();
            },
        ));
        let response_task = cx.spawn(async move |weak, cx| {
            while let Some(update) = responses.next().await {
                if weak
                    .update(cx, |state, cx| {
                        state.apply(update);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            computers: Vec::new(),
            overviews: HashMap::new(),
            overview_revisions: HashMap::new(),
            connections: HashMap::new(),
            changed_identities: HashSet::new(),
            directory_error: false,
            loading: false,
            action_failed: false,
            terminal_epoch: 0,
            client,
            capabilities: HashMap::new(),
            capability_generation: 0,
            pair_epochs: HashMap::new(),
            signed_in: None,
            attention_follows: HashMap::new(),
            enabled: true,
            commands,
            _tasks: vec![event_task, command_task, response_task],
        }
    }

    pub(crate) fn invalidate_capabilities(&mut self, computer: &str) {
        if let Some(access) = self.capabilities.remove(computer) {
            access.invalidate();
        }
    }

    pub(crate) fn front_page_access(
        &mut self,
        computer: &Computer,
        cx: &mut Context<Self>,
    ) -> Arc<RemoteAccess> {
        if !computer.paired {
            self.invalidate_capabilities(&computer.id);
        }
        if let Some(access) = self.capabilities.get(&computer.id) {
            if matches!(&access.target, FrontPageTarget::Remote { fingerprint, .. } if fingerprint == &computer.fingerprint)
            {
                return access.clone();
            }
        }
        self.invalidate_capabilities(&computer.id);
        self.capability_generation = self.capability_generation.wrapping_add(1);
        let access = RemoteAccess::new(FrontPageTarget::Remote {
            computer: computer.id.clone(),
            fingerprint: computer.fingerprint.clone(),
            generation: self.capability_generation,
        });
        self.capabilities
            .insert(computer.id.clone(), access.clone());
        if computer.paired && self.connection(&computer.id) == Connection::Connected {
            let client = FrontPageClient::remote(self.client.clone(), access.clone());
            let task = cx.background_spawn(async move { client.discover() });
            cx.spawn(async move |weak, cx| {
                let _ = task.await;
                let _ = weak.update(cx, |_, cx| cx.notify());
            })
            .detach();
        } else {
            self.invalidate_capabilities(&computer.id);
        }
        access
    }

    pub(crate) fn send(&self, command: Command) {
        let _ = self.commands.unbounded_send(command);
    }
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.loading {
            self.loading = true;
            self.send(Command::Refresh);
            cx.notify();
        }
    }
    pub(crate) fn follow(&self, computer: String) -> Follow {
        self.send(Command::Acquire(computer.clone()));
        Follow {
            computer,
            commands: self.commands.clone(),
        }
    }
    pub(crate) fn connection(&self, computer: &str) -> Connection {
        self.connections.get(computer).copied().unwrap_or_default()
    }
    fn apply(&mut self, update: Update) {
        match update {
            Update::Access(enabled) => self.signed_in = Some(enabled),
            Update::Computers(result) => {
                self.loading = false;
                if self.signed_in == Some(false) {
                    return;
                }
                self.directory_error = result.is_err();
                if let Ok(mut computers) = result {
                    for computer in &mut computers {
                        if self.connection(&computer.id) == Connection::Forgotten {
                            computer.paired = false;
                        }
                        if self.connection(&computer.id) != Connection::Forgotten
                            && self.computers.iter().any(|old| {
                                old.id == computer.id && old.fingerprint != computer.fingerprint
                            })
                        {
                            if let Some(access) = self.capabilities.remove(&computer.id) {
                                access.invalidate();
                            }
                            self.changed_identities.insert(computer.id.clone());
                            self.overviews.remove(&computer.id);
                            self.connections
                                .insert(computer.id.clone(), Connection::Revoked);
                        }
                    }
                    computers.sort_by_key(|computer| !computer.is_self);
                    self.computers = computers;
                }
            }
            Update::Overview(computer, overview) => {
                if self.connection(&computer) != Connection::Forgotten {
                    *self.overview_revisions.entry(computer.clone()).or_default() += 1;
                    self.overviews.insert(computer, overview);
                }
            }
            Update::Connection(computer, connection) => {
                if connection != Connection::Connected {
                    self.invalidate_capabilities(&computer);
                }
                if self.connection(&computer) != Connection::Forgotten
                    && (connection != Connection::Connected
                        || self.connection(&computer) != Connection::Revoked)
                {
                    self.connections.insert(computer, connection);
                }
            }
            Update::Event(event) => self.apply_event(event),
            Update::Forgotten(computer, success) => {
                self.action_failed = !success;
                if success {
                    self.invalidate_capabilities(&computer);
                    self.connections
                        .insert(computer.clone(), Connection::Forgotten);
                    self.overviews.remove(&computer);
                    self.changed_identities.remove(&computer);
                    if let Some(row) = self.computers.iter_mut().find(|row| row.id == computer) {
                        row.paired = false;
                    }
                }
            }
        }
        self.sync_attention_follows();
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.sync_attention_follows();
            cx.notify();
        }
    }

    fn sync_attention_follows(&mut self) {
        let desired = self
            .computers
            .iter()
            .filter(|computer| {
                self.enabled
                    && self.signed_in == Some(true)
                    && !computer.is_self
                    && computer.online
                    && computer.paired
                    && !self.changed_identities.contains(&computer.id)
                    && !matches!(
                        self.connection(&computer.id),
                        Connection::Revoked | Connection::Forgotten
                    )
            })
            .map(|computer| computer.id.clone())
            .collect::<HashSet<_>>();
        self.attention_follows.retain(|id, _| desired.contains(id));
        for id in desired {
            if !self.attention_follows.contains_key(&id) {
                self.attention_follows.insert(id.clone(), self.follow(id));
            }
        }
    }

    pub(crate) fn needs_attention(&self, computer: &str) -> bool {
        self.attention_follows.contains_key(computer)
            && self.connection(computer) == Connection::Connected
            && self.overviews.get(computer).is_some_and(|overview| {
                overview
                    .sessions
                    .values()
                    .any(|session| session.status.status.effective() == Status::NeedsYou)
                    || overview
                        .missions
                        .iter()
                        .any(|mission| !mission.asks.is_empty())
            })
    }

    pub(crate) fn apply_event(&mut self, event: ClientEvent) {
        if matches!(
            event.name.as_str(),
            "daemon/disconnected" | "daemon/reconnected"
        ) {
            for (_, access) in self.capabilities.drain() {
                access.invalidate();
            }
            self.terminal_epoch = self.terminal_epoch.wrapping_add(1);
            for connection in self.connections.values_mut() {
                if *connection != Connection::Forgotten {
                    *connection = Connection::Reconnecting;
                }
            }
            return;
        }
        if event.name == "remote/status" {
            if let Ok(status) =
                serde_json::from_value::<runner_core::remote::RemoteStatus>(event.payload.clone())
            {
                let signed_in = crate::surfaces::settings::remote::account::signed_in(&status);
                if signed_in && self.signed_in != Some(true) {
                    self.send(Command::Refresh);
                }
                self.signed_in = Some(signed_in);
                self.enabled = signed_in;
                if !signed_in {
                    for (_, access) in self.capabilities.drain() {
                        access.invalidate();
                    }
                    self.computers.clear();
                    self.overviews.clear();
                    for connection in self.connections.values_mut() {
                        if *connection != Connection::Forgotten {
                            *connection = Connection::Unreachable;
                        }
                    }
                }
            }
            return;
        }
        let Some(computer) = event
            .payload
            .get("computer")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            return;
        };
        if self.connection(&computer) == Connection::Forgotten {
            return;
        }
        match event.name.as_str() {
            "remote/overview" => {
                if let Ok(overview) = serde_json::from_value(event.payload["overview"].clone()) {
                    *self.overview_revisions.entry(computer.clone()).or_default() += 1;
                    self.overviews.insert(computer, overview);
                }
            }
            "remote/session-status" => {
                if let Ok(status) =
                    serde_json::from_value::<StatusPush>(event.payload["status"].clone())
                {
                    if let Some(session) = self
                        .overviews
                        .get_mut(&computer)
                        .and_then(|overview| overview.sessions.get_mut(&status.session))
                    {
                        session.status = status.status;
                    }
                }
            }
            "remote/connection" => {
                let state = match event.payload["state"].as_str() {
                    Some("connected") => Connection::Connected,
                    Some("reconnecting")
                        if Connection::failure(
                            event.payload["error"].as_str().unwrap_or_default(),
                        ) != Connection::Revoked =>
                    {
                        Connection::Reconnecting
                    }
                    _ => Connection::failure(event.payload["error"].as_str().unwrap_or_default()),
                };
                let state = if state == Connection::Unreachable
                    && self.connection(&computer) == Connection::Revoked
                {
                    Connection::Revoked
                } else {
                    state
                };
                if state != Connection::Connected {
                    self.invalidate_capabilities(&computer);
                }
                self.connections.insert(computer.clone(), state);
                if state == Connection::Revoked {
                    if let Some(row) = self.computers.iter_mut().find(|row| row.id == computer) {
                        row.paired = false;
                    }
                }
            }
            _ => {}
        }
    }
}

enum ComputerCommand {
    Acquire,
    Release,
    Retry,
    Reconnected,
    Forget,
}

pub(super) async fn run_commands(
    client: DaemonClient,
    mut requests: mpsc::UnboundedReceiver<Command>,
    updates: mpsc::UnboundedSender<Update>,
    mut spawn: impl FnMut(BoxFuture<'static, ()>),
) {
    let (directory, mut refreshes) = mpsc::unbounded::<()>();
    let directory_client = client.clone();
    let directory_updates = updates.clone();
    spawn(
        async move {
            while refreshes.next().await.is_some() {
                let computers = match directory_client.remote_status() {
                    Ok(status) => {
                        let enabled =
                            crate::surfaces::settings::remote::account::signed_in(&status);
                        let _ = directory_updates.unbounded_send(Update::Access(enabled));
                        if enabled {
                            directory_client.remote_computers().map_err(|_| ())
                        } else {
                            Ok(Vec::new())
                        }
                    }
                    Err(_) => Err(()),
                };
                let _ = directory_updates.unbounded_send(Update::Computers(computers));
            }
        }
        .boxed(),
    );
    let mut workers = HashMap::<String, mpsc::UnboundedSender<ComputerCommand>>::new();
    while let Some(command) = requests.next().await {
        let (computer, command) = match command {
            Command::Refresh => {
                let _ = directory.unbounded_send(());
                continue;
            }
            Command::Reconnected => {
                for worker in workers.values() {
                    let _ = worker.unbounded_send(ComputerCommand::Reconnected);
                }
                let _ = directory.unbounded_send(());
                continue;
            }
            Command::Acquire(id) => (id, ComputerCommand::Acquire),
            Command::Release(id) => (id, ComputerCommand::Release),
            Command::Retry(id) => (id, ComputerCommand::Retry),
            Command::Forget(id) => (id, ComputerCommand::Forget),
            Command::Paired(id) => (id, ComputerCommand::Reconnected),
        };
        let worker = workers.entry(computer.clone()).or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded();
            spawn(
                computer_requests(
                    client.clone(),
                    computer,
                    rx,
                    updates.clone(),
                    directory.clone(),
                )
                .boxed(),
            );
            tx
        });
        let _ = worker.unbounded_send(command);
    }
}

async fn computer_requests(
    client: DaemonClient,
    computer: String,
    mut requests: mpsc::UnboundedReceiver<ComputerCommand>,
    updates: mpsc::UnboundedSender<Update>,
    directory: mpsc::UnboundedSender<()>,
) {
    let mut follows = 0;
    while let Some(command) = requests.next().await {
        match command {
            ComputerCommand::Acquire => {
                follows += 1;
                if follows == 1 {
                    load(&client, &computer, &updates);
                }
            }
            ComputerCommand::Release => {
                follows -= 1;
                if follows == 0 {
                    let _ = client.remote_follow(computer.clone(), false);
                }
            }
            ComputerCommand::Retry => load(&client, &computer, &updates),
            ComputerCommand::Reconnected if follows > 0 => load(&client, &computer, &updates),
            ComputerCommand::Reconnected => {}
            ComputerCommand::Forget => {
                let success = client.remote_forget(computer.clone()).is_ok();
                let _ = updates.unbounded_send(Update::Forgotten(computer.clone(), success));
                let _ = directory.unbounded_send(());
            }
        }
    }
    if follows > 0 {
        let _ = client.remote_follow(computer, false);
    }
}

fn load(client: &DaemonClient, computer: &str, updates: &mpsc::UnboundedSender<Update>) {
    let result = client
        .remote_follow(computer.into(), true)
        .and_then(|()| client.remote_overview(computer.into()));
    match result {
        Ok(overview) => {
            let _ = updates.unbounded_send(Update::Overview(computer.into(), overview.0));
            let _ =
                updates.unbounded_send(Update::Connection(computer.into(), Connection::Connected));
        }
        Err(error) => {
            let _ = updates.unbounded_send(Update::Connection(
                computer.into(),
                Connection::failure(&error.message),
            ));
        }
    }
}

pub(crate) fn device_name(name: &str) -> &str {
    let mut end = name.len().min(64);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    This,
    Incoming,
    Outgoing,
    Both,
    Unpaired,
}
impl Access {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::This => "client.access_self",
            Self::Incoming => "client.access_incoming",
            Self::Outgoing => "client.access_outgoing",
            Self::Both => "client.access_both",
            Self::Unpaired => "client.access_none",
        }
    }
}

pub(crate) struct ComputerRow<'a> {
    pub(crate) computer: Option<&'a Computer>,
    pub(crate) devices: Vec<&'a PairedDevice>,
    pub(crate) access: Access,
}
pub(crate) fn incoming_devices<'a>(
    computer: &Computer,
    computers: &[Computer],
    trust: &'a RemoteTrust,
) -> Vec<&'a PairedDevice> {
    let name = device_name(&computer.name);
    if computer.is_self
        || computers
            .iter()
            .filter(|c| device_name(&c.name) == name)
            .count()
            != 1
    {
        return Vec::new();
    }
    trust
        .devices
        .iter()
        .filter(|device| device.name == name)
        .collect()
}

pub(crate) fn computer_rows<'a>(
    computers: &'a [Computer],
    trust: &'a RemoteTrust,
) -> Vec<ComputerRow<'a>> {
    let mut matched = HashSet::new();
    let mut rows = Vec::new();
    for computer in computers
        .iter()
        .filter(|c| c.is_self)
        .chain(computers.iter().filter(|c| !c.is_self))
    {
        let devices = incoming_devices(computer, computers, trust);
        for device in &devices {
            matched.insert(&device.fingerprint);
        }
        let access = if computer.is_self {
            Access::This
        } else {
            match (!devices.is_empty(), computer.paired) {
                (true, true) => Access::Both,
                (true, false) => Access::Incoming,
                (false, true) => Access::Outgoing,
                (false, false) => Access::Unpaired,
            }
        };
        rows.push(ComputerRow {
            computer: Some(computer),
            devices,
            access,
        });
    }
    rows.extend(
        trust
            .devices
            .iter()
            .filter(|device| !matched.contains(&device.fingerprint))
            .map(|device| ComputerRow {
                computer: None,
                devices: vec![device],
                access: Access::Incoming,
            }),
    );
    rows
}

pub(crate) fn state(store: &Entity<crate::AppStore>, cx: &gpui::App) -> Entity<ClientState> {
    store.read(cx).remote_clients.clone()
}

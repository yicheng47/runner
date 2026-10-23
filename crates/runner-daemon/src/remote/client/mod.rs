mod route;
mod terminal;

use super::{
    persistence::Saved,
    store::{ClientKey, ClientKeys, SecretStore, Secrets},
    transport,
};
use futures_util::future::BoxFuture;
use runner_core::{
    companion as c,
    protocol::terminal::TerminalAttachment,
    remote::{self, Computer, RemoteState, RemoteStatus, RemoteTerminal, Secret},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
struct Context {
    origin: String,
    host_id: String,
    name: String,
    credential: Secret,
}
trait Service: Send + Sync {
    fn list<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>>;
    fn open<'a>(
        &'a self,
        context: &'a Context,
        computer: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>>;
}
struct HttpService(reqwest::Client);
impl Service for HttpService {
    fn list<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<Vec<Computer>>> {
        Box::pin(async move {
            let response: remote::Computers = transport::rpc_service(
                &self.0,
                &context.origin,
                "HostClientService",
                "ListAccountHosts",
                Some(&context.credential),
                &remote::Empty {},
            )
            .await
            .map_err(|f| format!("Could not list computers: {}.", f.code))?;
            Ok(response.hosts)
        })
    }
    fn open<'a>(
        &'a self,
        context: &'a Context,
        computer: &'a str,
    ) -> BoxFuture<'a, std::result::Result<transport::Socket, transport::Failure>> {
        Box::pin(transport::connect_client(
            &self.0,
            &context.origin,
            computer,
            &context.credential,
        ))
    }
}
struct Directory {
    origin: String,
    host: String,
    expires: Instant,
    computers: Result<Vec<Computer>>,
}
struct Inner {
    root: PathBuf,
    store: Arc<dyn SecretStore>,
    writes: Arc<std::sync::Mutex<()>>,
    status: watch::Receiver<RemoteStatus>,
    service: tokio::sync::OnceCell<Arc<dyn Service>>,
    directory: Mutex<Option<Directory>>,
    routes: Mutex<HashMap<String, route::Handle>>,
    pairing: Mutex<()>,
    events: crate::events::EventChannel,
    cancel: CancellationToken,
}
#[derive(Clone)]
pub(super) struct Client(Arc<Inner>);
impl Client {
    pub fn new(
        root: PathBuf,
        store: Arc<dyn SecretStore>,
        status: watch::Receiver<RemoteStatus>,
        events: crate::events::EventChannel,
        cancel: CancellationToken,
    ) -> Self {
        Self(Arc::new(Inner {
            root,
            store,
            status,
            events,
            cancel,
            writes: Default::default(),
            service: Default::default(),
            directory: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            pairing: Mutex::new(()),
        }))
    }
    async fn service(&self) -> Result<Arc<dyn Service>> {
        self.0
            .service
            .get_or_try_init(|| async {
                let http = tokio::task::spawn_blocking(transport::client)
                    .await
                    .map_err(|_| "Remote HTTP worker failed")??;
                Ok::<Arc<dyn Service>, String>(Arc::new(HttpService(http)))
            })
            .await
            .cloned()
    }
    async fn context(&self) -> Result<Context> {
        let status = self.0.status.borrow().clone();
        if !signed_in(&status) {
            return Err("Sign in to your account first.".into());
        }
        let root = self.0.root.clone();
        let store = self.0.store.clone();
        let context = tokio::task::spawn_blocking(move || {
            let saved = Saved::load(&root)?
                .filter(|s| s.activated && s.stopped.is_none())
                .ok_or("This computer's enrollment is not activated.")?;
            let bytes = store
                .get(&saved.origin)?
                .ok_or("This computer has no enrollment credential.")?;
            let secrets: Secrets =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid enrollment credential")?;
            Ok::<_, &'static str>(Context {
                origin: saved.origin,
                host_id: saved.host_id,
                name: saved.host_name,
                credential: secrets.credential,
            })
        })
        .await
        .map_err(|_| "Remote credential worker failed")??;
        if context.origin != status.origin || Some(&context.host_id) != status.host_id.as_ref() {
            return Err("The signed-in computer changed; try again.".into());
        }
        Ok(context)
    }
    async fn keys(&self) -> Result<ClientKeys> {
        let store = self.0.store.clone();
        let writes = self.0.writes.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = writes.lock().map_err(|_| "Client keys lock failed")?;
            ClientKeys::load(store.as_ref())
        })
        .await
        .map_err(|_| "Client keys worker failed")?
        .map_err(Into::into)
    }
    async fn update_key(&self, fingerprint: String, key: Option<ClientKey>) -> Result<()> {
        let store = self.0.store.clone();
        let writes = self.0.writes.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = writes.lock().map_err(|_| "Client keys lock failed")?;
            let mut keys = ClientKeys::load(store.as_ref())?;
            if let Some(key) = key {
                keys.0.insert(fingerprint, key);
            } else {
                keys.0.remove(&fingerprint);
            }
            keys.save(store.as_ref())
        })
        .await
        .map_err(|_| "Client keys worker failed")?
        .map_err(Into::into)
    }
    pub async fn computers(&self) -> Result<Vec<Computer>> {
        let context = self.context().await?;
        let mut cache = self.0.directory.lock().await;
        let cached = cache.as_ref().filter(|entry| {
            entry.origin == context.origin
                && entry.host == context.host_id
                && entry.expires > Instant::now()
        });
        let mut computers = match cached {
            Some(entry) => entry.computers.clone()?,
            None => {
                let computers = self.service().await?.list(&context).await;
                let delay = if computers
                    .as_ref()
                    .is_err_and(|e| e.contains("resource_exhausted"))
                {
                    60
                } else {
                    5
                };
                *cache = Some(Directory {
                    origin: context.origin.clone(),
                    host: context.host_id.clone(),
                    expires: Instant::now() + Duration::from_secs(delay),
                    computers: computers.clone(),
                });
                computers?
            }
        };
        let keys = self.keys().await?;
        for computer in &mut computers {
            computer.paired = keys.0.contains_key(&computer.fingerprint);
        }
        Ok(computers)
    }
    async fn computer(&self, id: &str) -> Result<Computer> {
        self.computers()
            .await?
            .into_iter()
            .find(|c| c.id == id && !c.is_self)
            .ok_or_else(|| "No other computer has that id on this account.".into())
    }
    pub async fn forget(&self, id: String) -> Result<()> {
        let _pairing = self.0.pairing.lock().await;
        let computer = self.computer(&id).await?;
        if let Some(route) = self.0.routes.lock().await.remove(&computer.fingerprint) {
            route.cancel.cancel();
        }
        self.update_key(computer.fingerprint, None).await
    }
    pub async fn reset(&self) -> Result<()> {
        let _pairing = self.0.pairing.lock().await;
        for (_, route) in self.0.routes.lock().await.drain() {
            route.cancel.cancel();
        }
        let store = self.0.store.clone();
        let writes = self.0.writes.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = writes.lock().map_err(|_| "Client keys lock failed")?;
            store.delete(super::store::CLIENT_KEYS_NAME)
        })
        .await
        .map_err(|_| "Client keys worker failed")?
        .map_err(Into::into)
    }
    pub async fn pair(&self, id: String, pin: Secret) -> Result<()> {
        remote::validate_pin_format(&pin.0).map_err(|error| error.message())?;
        let computer = self.computer(&id).await?;
        let _pairing = self.0.pairing.lock().await;
        if self.keys().await?.0.contains_key(&computer.fingerprint) {
            return Err("This computer is already paired; Forget it before pairing again.".into());
        }
        if let Some(route) = self.0.routes.lock().await.get(&computer.fingerprint) {
            if !route.cancel.is_cancelled() {
                return Err("A channel to this computer is already open.".into());
            }
        }
        let context = self.context().await?;
        let key = remote::HostKey::generate().map_err(|_| "Could not generate a client key")?;
        let socket = self
            .service()
            .await?
            .open(&context, &id)
            .await
            .map_err(|f| f.message)?;
        let (socket, records, public) =
            route::pair(socket, &key, &pin, pairing_label(&context.name)).await?;
        if remote::fingerprint(&public) != computer.fingerprint {
            return Err("The computer's identity does not match the account listing.".into());
        }
        self.update_key(
            computer.fingerprint.clone(),
            Some(ClientKey {
                private: Secret(remote::encode_key(&key.private)),
                public,
                name: computer.name.clone(),
                paired_at: chrono::Utc::now(),
            }),
        )
        .await?;
        let handle = route::spawn(self.clone(), computer, context, Some((socket, records)));
        self.0
            .routes
            .lock()
            .await
            .insert(handle.fingerprint.clone(), handle);
        Ok(())
    }
    async fn route(&self, id: &str) -> Result<route::Handle> {
        let computer = self.computer(id).await?;
        let context = self.context().await?;
        let mut routes = self.0.routes.lock().await;
        if let Some(route) = routes
            .get(&computer.fingerprint)
            .filter(|r| !r.cancel.is_cancelled() && r.computer == computer.id)
        {
            return Ok(route.clone());
        }
        if let Some(old) = routes.remove(&computer.fingerprint) {
            old.cancel.cancel();
        }
        if !computer.paired {
            return Err("Pair with this computer first.".into());
        }
        let route = route::spawn(self.clone(), computer, context, None);
        routes.insert(route.fingerprint.clone(), route.clone());
        Ok(route)
    }
    pub async fn overview(&self, id: String) -> Result<c::Overview> {
        let route = self.route(&id).await?;
        let value = route
            .request(|id| c::ClientMessage::Overview(c::OverviewRequest { id }))
            .await?;
        serde_json::from_value(value).map_err(|_| "Invalid remote overview".into())
    }
    pub async fn follow(&self, id: String, owner: u64, follow: bool) -> Result<()> {
        let route = if follow {
            self.route(&id).await?
        } else {
            let routes = self.0.routes.lock().await;
            let Some(route) = routes
                .values()
                .find(|r| r.computer == id && !r.cancel.is_cancelled())
                .cloned()
            else {
                return Ok(());
            };
            route
        };
        route.send(route::Command::Follow(owner, follow)).await
    }
    pub async fn owner_gone(&self, owner: u64) {
        let routes: Vec<_> = self.0.routes.lock().await.values().cloned().collect();
        for route in routes {
            let _ = route.send(route::Command::Follow(owner, false)).await;
        }
    }
    pub async fn attach(&self, terminal: RemoteTerminal) -> Result<TerminalAttachment> {
        let route = self.route(&terminal.computer).await?;
        let target = terminal.session.clone();
        let (reply, receive) = oneshot::channel();
        route.send(route::Command::Attach(terminal, reply)).await?;
        let result = match tokio::time::timeout(Duration::from_secs(35), receive).await {
            Ok(reply) => reply.map_err(|_| route.failure())?,
            Err(_) => Err("Remote terminal attach timed out".into()),
        };
        if result.is_err() {
            tokio::spawn(async move {
                let _ = route.send(route::Command::Detach(target, 0)).await;
            });
        }
        result
    }
    pub async fn input(&self, computer: String, session: String, bytes: Vec<u8>) -> Result<()> {
        // Input uses an existing ready route only. Opening or reconnecting must never queue keystrokes.
        let routes = self.0.routes.lock().await;
        let route = routes
            .values()
            .find(|r| r.computer == computer)
            .ok_or("Remote computer is disconnected; input was discarded.")?
            .clone();
        let generation = route.generation();
        if !route.ready.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Remote computer is reconnecting; input was discarded.".into());
        }
        drop(routes);
        route.input(generation, session, bytes).await
    }

    pub async fn queue_viewport(
        &self,
        computer: String,
        target: String,
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        let routes = self.0.routes.lock().await;
        let route = routes
            .values()
            .find(|r| r.computer == computer && r.ready.load(std::sync::atomic::Ordering::Acquire))
            .ok_or("Remote computer is reconnecting; viewport was discarded.")?;
        let (reply, _) = oneshot::channel();
        route
            .tx
            .try_send(route::Command::Request(
                c::ClientMessage::Viewport(c::ViewportRequest {
                    id: route.next_id(),
                    session: target,
                    cols,
                    rows,
                }),
                reply,
            ))
            .map_err(|_| "Remote viewport queue is full or closed".into())
    }
    pub async fn viewport(
        &self,
        computer: String,
        session: String,
        cols: u16,
        rows: u16,
        take: bool,
    ) -> Result<()> {
        let route = self.route(&computer).await?;
        route
            .request(move |id| {
                let viewport = c::ViewportRequest {
                    id,
                    session,
                    cols,
                    rows,
                };
                if take {
                    c::ClientMessage::TakeControl(viewport)
                } else {
                    c::ClientMessage::Viewport(viewport)
                }
            })
            .await
            .map(|_| ())
    }
}
fn pairing_label(name: &str) -> &str {
    let mut end = name.len().min(64);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}
fn signed_in(status: &RemoteStatus) -> bool {
    matches!(
        status.state,
        RemoteState::Connected { .. }
            | RemoteState::Connecting { .. }
            | RemoteState::Reconnecting { .. }
            | RemoteState::Paused
    )
}

#[cfg(test)]
mod tests;

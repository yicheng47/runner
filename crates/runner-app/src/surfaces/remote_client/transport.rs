use runner_core::{
    protocol::{
        terminal::{TerminalAttachment, TerminalMetadata},
        ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
    },
    remote::RemoteTerminal,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

type Cancel = Arc<dyn Fn() + Send + Sync>;
#[derive(Default)]
struct AttachmentState {
    closed: bool,
    cancel: Option<Cancel>,
}
pub(crate) struct AttachmentLifetime(Arc<Mutex<AttachmentState>>);
impl Drop for AttachmentLifetime {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        if let Some(cancel) = state.cancel.take() {
            cancel();
        }
    }
}

pub(crate) struct RemoteTransport {
    attachment: Arc<Mutex<AttachmentState>>,
    client: DaemonClient,
    config: Mutex<RemoteTerminal>,
    title: String,
    first_attach: AtomicBool,
    pub(crate) connected: Arc<AtomicBool>,
}
impl RemoteTransport {
    pub(crate) fn client(
        client: DaemonClient,
        config: RemoteTerminal,
        title: String,
        connected: Arc<AtomicBool>,
    ) -> (DaemonClient, AttachmentLifetime) {
        let take_control = config.take_control;
        let attachment = Arc::new(Mutex::new(AttachmentState::default()));
        let lifetime = AttachmentLifetime(attachment.clone());
        (
            DaemonClient::new(Arc::new(Self {
                attachment,
                client,
                config: Mutex::new(config),
                title,
                first_attach: AtomicBool::new(take_control),
                connected,
            })),
            lifetime,
        )
    }
    fn check(&self, address: &str) -> Result<RemoteTerminal, ClientError> {
        let config = self.config.lock().unwrap().clone();
        if address != config.address() {
            return Err(ClientError::msg("invalid remote terminal address"));
        }
        Ok(config)
    }
}
impl Transport for RemoteTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        match request {
            Request::terminal_metadata { session_id } => {
                let config = self.check(&session_id)?;
                Ok(Response::terminal_metadata(Ok(TerminalMetadata {
                    title: self.title.clone(),
                    cols: config.cols,
                    rows: config.rows,
                    ..Default::default()
                })))
            }
            Request::terminal_take_back {
                session_id,
                cols,
                rows,
            } => {
                let config = self.check(&session_id)?;
                let result = if self.connected.load(Ordering::Acquire) {
                    self.client.remote_take_control(
                        config.computer,
                        config.session,
                        cols.max(20),
                        rows.max(8),
                    )
                } else {
                    Err(ClientError::msg("remote terminal is disconnected"))
                };
                if result.is_ok() {
                    let mut config = self.config.lock().unwrap();
                    config.cols = cols.max(20);
                    config.rows = rows.max(8);
                }
                Ok(Response::terminal_take_back(result))
            }
            Request::terminal_observe_input { session_id, .. } => {
                self.check(&session_id)?;
                Ok(Response::terminal_observe_input(Ok(())))
            }
            Request::terminal_input_reset_guard { session_id } => {
                self.check(&session_id)?;
                Ok(Response::terminal_input_reset_guard(Ok(0)))
            }
            Request::terminal_reset_input_state { session_id, .. } => {
                self.check(&session_id)?;
                Ok(Response::terminal_reset_input_state(Ok(())))
            }
            _ => Err(ClientError::msg(
                "operation is unavailable for a remote terminal",
            )),
        }
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.client.subscribe()
    }
    fn attach(&self, address: &str) -> Result<TerminalAttachment, ClientError> {
        let mut config = self.check(address)?;
        config.take_control = self.first_attach.swap(false, Ordering::AcqRel);
        let attachment = self.client.attach_remote(config)?;
        let mut state = self.attachment.lock().unwrap();
        let cancel = attachment.frames.cancellation();
        if state.closed {
            cancel();
        } else {
            state.cancel = Some(cancel);
        }
        Ok(attachment)
    }
    fn input(&self, address: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.check(address)?;
        if !self.connected.load(Ordering::Acquire) {
            return Ok(());
        }
        self.client.input(address, bytes)
    }
    fn resize(&self, address: &str, subscriber: u64, cols: u16, rows: u16) {
        if self.check(address).is_err() || !self.connected.load(Ordering::Acquire) {
            return;
        }
        let (cols, rows) = (cols.max(20), rows.max(8));
        {
            let mut config = self.config.lock().unwrap();
            config.cols = cols;
            config.rows = rows;
        }
        self.client.resize(address, subscriber, cols, rows);
    }
}

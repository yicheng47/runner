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
        let (daemon, lifetime, _) = Self::client_with_handle(client, config, title, connected);
        (daemon, lifetime)
    }

    pub(crate) fn client_with_handle(
        client: DaemonClient,
        config: RemoteTerminal,
        title: String,
        connected: Arc<AtomicBool>,
    ) -> (DaemonClient, AttachmentLifetime, Arc<Self>) {
        let take_control = config.take_control;
        let attachment = Arc::new(Mutex::new(AttachmentState::default()));
        let lifetime = AttachmentLifetime(attachment.clone());
        let transport = Arc::new(Self {
            attachment,
            client,
            config: Mutex::new(config),
            title,
            first_attach: AtomicBool::new(take_control),
            connected,
        });
        (DaemonClient::new(transport.clone()), lifetime, transport)
    }

    pub(crate) fn claim_if_free(
        &self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<(), ClientError> {
        let config = self.config.lock().unwrap().clone();
        let cols = cols.max(20);
        let rows = rows.max(8);
        let (cell_width, cell_height) =
            runner_core::protocol::terminal::normalize_cell_size(cell_width, cell_height)
                .unwrap_or((config.cell_width, config.cell_height));
        let result = if self.connected.load(Ordering::Acquire) {
            self.client.remote_claim_control(
                config.computer,
                config.session,
                cols,
                rows,
                cell_width,
                cell_height,
            )
        } else {
            Err(ClientError::msg("remote terminal is disconnected"))
        };
        if result.is_ok() {
            let mut config = self.config.lock().unwrap();
            config.cols = cols;
            config.rows = rows;
            if cell_width > 0 && cell_height > 0 {
                config.cell_width = cell_width;
                config.cell_height = cell_height;
            }
        }
        result
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
                cell_width,
                cell_height,
            } => {
                let config = self.check(&session_id)?;
                let result = if self.connected.load(Ordering::Acquire) {
                    self.client.remote_take_control(
                        config.computer,
                        config.session,
                        cols.max(20),
                        rows.max(8),
                        cell_width,
                        cell_height,
                    )
                } else {
                    Err(ClientError::msg("remote terminal is disconnected"))
                };
                if result.is_ok() {
                    let mut config = self.config.lock().unwrap();
                    config.cols = cols.max(20);
                    config.rows = rows.max(8);
                    if cell_width > 0 && cell_height > 0 {
                        config.cell_width = cell_width;
                        config.cell_height = cell_height;
                    }
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
    fn resize(
        &self,
        address: &str,
        subscriber: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) {
        if self.check(address).is_err() || !self.connected.load(Ordering::Acquire) {
            return;
        }
        let (cols, rows) = (cols.max(20), rows.max(8));
        {
            let mut config = self.config.lock().unwrap();
            config.cols = cols;
            config.rows = rows;
            if cell_width > 0 && cell_height > 0 {
                config.cell_width = cell_width;
                config.cell_height = cell_height;
            }
        }
        self.client
            .resize(address, subscriber, cols, rows, cell_width, cell_height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::terminal::{
        TerminalController, TerminalFrame, TerminalSnapshot, TerminalSubscription,
    };

    #[derive(Default)]
    struct CellRecordingTransport {
        attaches: Mutex<Vec<RemoteTerminal>>,
        takes: Mutex<Vec<(u16, u16, u16, u16)>>,
        claims: Mutex<Vec<(u16, u16, u16, u16)>>,
        resizes: Mutex<Vec<(u16, u16, u16, u16)>>,
    }

    struct ClosedFrames;
    impl TerminalSubscription for ClosedFrames {
        fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("closed"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
    }

    impl Transport for CellRecordingTransport {
        fn call(&self, request: Request) -> Result<Response, ClientError> {
            match request {
                Request::remote_take_control {
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                    ..
                } => {
                    self.takes
                        .lock()
                        .unwrap()
                        .push((cols, rows, cell_width, cell_height));
                    Ok(Response::remote_take_control(Ok(())))
                }
                Request::remote_claim_control {
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                    ..
                } => {
                    self.claims
                        .lock()
                        .unwrap()
                        .push((cols, rows, cell_width, cell_height));
                    Ok(Response::remote_claim_control(Ok(())))
                }
                _ => Err(ClientError::msg("unexpected request")),
            }
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach_remote(&self, config: RemoteTerminal) -> Result<TerminalAttachment, ClientError> {
            let (cols, rows, cell_width, cell_height) = (
                config.cols,
                config.rows,
                config.cell_width,
                config.cell_height,
            );
            self.attaches.lock().unwrap().push(config);
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 1,
                    activity: None,
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                    controller: TerminalController::Desktop,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                },
                frames: Box::new(ClosedFrames),
            })
        }
        fn resize(&self, _: &str, _: u64, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
            self.resizes
                .lock()
                .unwrap()
                .push((cols, rows, cell_width, cell_height));
        }
    }

    #[test]
    fn remote_attach_take_claim_and_resize_carry_cell_size() {
        let base = Arc::new(CellRecordingTransport::default());
        let config = RemoteTerminal {
            computer: "peer-1".into(),
            session: "sess-1".into(),
            cols: 96,
            rows: 32,
            cell_width: 16,
            cell_height: 36,
            take_control: true,
        };
        let address = config.address();
        let connected = Arc::new(AtomicBool::new(true));
        let (daemon, _lifetime, transport) = RemoteTransport::client_with_handle(
            DaemonClient::new(base.clone()),
            config,
            "Remote".into(),
            connected,
        );

        let attached = daemon.attach(&address).unwrap();
        assert_eq!(
            (attached.snapshot.cell_width, attached.snapshot.cell_height),
            (16, 36)
        );
        {
            let attaches = base.attaches.lock().unwrap();
            assert_eq!(attaches.len(), 1);
            assert_eq!((attaches[0].cell_width, attaches[0].cell_height), (16, 36));
        }

        transport.claim_if_free(100, 34, 18, 38).unwrap();
        assert_eq!(base.claims.lock().unwrap().as_slice(), &[(100, 34, 18, 38)]);

        daemon
            .terminal_take_back(&address, 110, 40, 20, 42)
            .unwrap();
        assert_eq!(base.takes.lock().unwrap().as_slice(), &[(110, 40, 20, 42)]);

        daemon.resize(&address, 1, 110, 40, 10, 21);
        assert_eq!(
            base.resizes.lock().unwrap().as_slice(),
            &[(110, 40, 10, 21)]
        );
    }
}

use super::{Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ClientError {
    pub message: String,
}
impl ClientError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClientEvent {
    pub name: String,
    pub payload: serde_json::Value,
}
#[derive(Debug)]
pub enum EventError {
    Lagged(u64),
    Closed,
}
pub trait EventSubscription: Send {
    fn recv(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<ClientEvent, EventError>> + Send + '_>>;
}
pub trait Transport: Send + Sync {
    fn call(&self, request: Request) -> Result<Response, ClientError>;
    fn subscribe(&self) -> Box<dyn EventSubscription>;
    fn attach(
        &self,
        _session_id: &str,
    ) -> Result<super::terminal::TerminalAttachment, ClientError> {
        Err(ClientError::msg("terminal attachment is unavailable"))
    }
    fn input(&self, _session_id: &str, _bytes: &[u8]) -> Result<(), ClientError> {
        Err(ClientError::msg("terminal input is unavailable"))
    }
    fn observe_terminals(
        &self,
        _observer: std::sync::Weak<dyn super::terminal::TerminalLifecycle>,
    ) {
    }
    fn resize(&self, _session_id: &str, _subscriber_id: u64, _cols: u16, _rows: u16) {}
}
#[derive(Clone)]
pub struct DaemonClient {
    pub(super) transport: Arc<dyn Transport>,
}
impl DaemonClient {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
    pub fn subscribe(&self) -> Box<dyn EventSubscription> {
        self.transport.subscribe()
    }
    pub fn attach(
        &self,
        session_id: &str,
    ) -> Result<super::terminal::TerminalAttachment, ClientError> {
        self.transport.attach(session_id)
    }
    pub fn input(&self, session_id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.transport.input(session_id, bytes)
    }
    pub fn observe_terminals(
        &self,
        observer: std::sync::Weak<dyn super::terminal::TerminalLifecycle>,
    ) {
        self.transport.observe_terminals(observer);
    }
    pub fn resize(&self, session_id: &str, subscriber_id: u64, cols: u16, rows: u16) {
        self.transport.resize(session_id, subscriber_id, cols, rows);
    }
}

pub type ClientResult<T> = Result<T, ClientError>;

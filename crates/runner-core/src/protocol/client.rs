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
}

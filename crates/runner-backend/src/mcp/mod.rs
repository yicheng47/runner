mod server;
pub(crate) mod tools;

use runner_core::app_paths::IpcEndpoint;
use std::sync::Mutex;

use crate::ipc::IpcListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::AppCore;

struct RunningListener {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
    endpoint: IpcEndpoint,
}

pub struct McpHandle {
    inner: Mutex<Option<RunningListener>>,
}

impl Default for McpHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl McpHandle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Start the socket listener. `rt` is the frontend's tokio runtime
    /// handle — the listener task and per-connection servers run there
    /// (the core owns no runtime of its own).
    pub fn start(
        &self,
        endpoint: &IpcEndpoint,
        state: AppCore,
        rt: &tokio::runtime::Handle,
    ) -> crate::error::Result<()> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let mut listener = {
            let _guard = rt.enter();
            IpcListener::bind(endpoint)?
        };
        log::info!("mcp: listening on {endpoint}");

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let endpoint_owned = endpoint.clone();

        let handle = rt.spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok(stream) => {
                                let conn_state = state.clone();
                                tokio::spawn(server::serve_connection(stream, conn_state));
                            }
                            Err(e) => {
                                log::error!("mcp: accept failed: {e}");
                            }
                        }
                    }
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                }
            }
        });

        *guard = Some(RunningListener {
            cancel,
            handle,
            endpoint: endpoint_owned,
        });
        Ok(())
    }

    pub fn stop(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(running) = guard.take() {
            log::info!("mcp: stopping listener");
            running.cancel.cancel();
            running.handle.abort();
            #[cfg(unix)]
            let _ = std::fs::remove_file(&running.endpoint.0);
        }
    }

    pub fn endpoint(&self) -> Option<IpcEndpoint> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().map(|r| r.endpoint.clone())
    }
}

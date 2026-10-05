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
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(windows)]
    pipe: std::os::windows::io::OwnedHandle,
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

        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::symlink_metadata(&endpoint.0)?;
            (meta.dev(), meta.ino())
        };
        #[cfg(windows)]
        let pipe = listener.duplicate_handle()?;
        let handle = rt.spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok(stream) => {
                                let conn_state = state.clone();
                                tokio::spawn(server::serve_connection(stream, conn_state, cancel_clone.clone()));
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
            #[cfg(unix)]
            identity,
            #[cfg(windows)]
            pipe,
        });
        Ok(())
    }

    #[cfg(windows)]
    pub fn duplicate_handle(&self) -> std::io::Result<std::os::windows::io::OwnedHandle> {
        let guard = self.inner.lock().unwrap();
        guard
            .as_ref()
            .ok_or_else(|| std::io::Error::other("MCP listener unavailable"))?
            .pipe
            .try_clone()
    }
    pub fn stop_accepting(&self) {
        if let Some(running) = self.inner.lock().unwrap().as_ref() {
            running.cancel.cancel();
            running.handle.abort();
        }
    }
    pub fn stop(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(running) = guard.take() {
            log::info!("mcp: stopping listener");
            running.cancel.cancel();
            running.handle.abort();
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if std::fs::symlink_metadata(&running.endpoint.0)
                    .is_ok_and(|meta| (meta.dev(), meta.ino()) == running.identity)
                {
                    let _ = std::fs::remove_file(&running.endpoint.0);
                }
            }
        }
    }

    pub fn endpoint(&self) -> Option<IpcEndpoint> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().map(|r| r.endpoint.clone())
    }
}

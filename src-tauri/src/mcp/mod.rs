mod server;
pub(crate) mod state;
pub(crate) mod tools;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::async_runtime::JoinHandle;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

use self::state::McpState;

struct RunningListener {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
    socket_path: PathBuf,
}

pub struct McpHandle {
    inner: Mutex<Option<RunningListener>>,
}

impl McpHandle {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub(crate) fn start(&self, socket_path: &Path, state: McpState) -> crate::error::Result<()> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        // Remove stale socket from a prior crash.
        let _ = std::fs::remove_file(socket_path);

        let listener = UnixListener::bind(socket_path).map_err(|e| {
            crate::error::Error::msg(format!(
                "mcp: failed to bind {}: {e}",
                socket_path.display()
            ))
        })?;
        log::info!("mcp: listening on {}", socket_path.display());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let socket_path_owned = socket_path.to_path_buf();

        let handle = tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _addr)) => {
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
            socket_path: socket_path_owned,
        });
        Ok(())
    }

    pub(crate) fn stop(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(running) = guard.take() {
            log::info!("mcp: stopping listener");
            running.cancel.cancel();
            running.handle.abort();
            let _ = std::fs::remove_file(&running.socket_path);
        }
    }

    #[allow(dead_code)] // used by Phase 3/4 settings UI
    pub(crate) fn socket_path(&self) -> Option<PathBuf> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().map(|r| r.socket_path.clone())
    }
}

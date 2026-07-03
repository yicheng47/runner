// The in-process MCP server speaks over a Unix-domain socket (`mcp.sock`)
// that the `runner-mcp` sidecar bridges stdio to. That's Unix-only; on
// the Windows host build `start` is a no-op stub (the WSL-hosted agents
// reach the app's tools from inside Linux — bringing a Windows-host MCP
// transport up is plan M4+). `state`/`tools` stay cross-platform because
// `AppState` and command handlers reference them. `server` also stays
// cross-platform — its `RunnerMcpHandler` is used by `tools`; only its
// Unix-socket `serve_connection` is gated, inside `server.rs`.
mod server;
pub(crate) mod state;
pub(crate) mod tools;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::net::UnixListener as StdUnixListener;
use tauri::async_runtime::JoinHandle;
#[cfg(unix)]
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

    #[cfg(unix)]
    pub(crate) fn start(&self, socket_path: &Path, state: McpState) -> crate::error::Result<()> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }

        let listener = bind_listener(socket_path)?;
        log::info!("mcp: listening on {}", socket_path.display());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let socket_path_owned = socket_path.to_path_buf();

        let handle = tauri::async_runtime::spawn(async move {
            let listener = match UnixListener::from_std(listener) {
                Ok(listener) => listener,
                Err(e) => {
                    log::error!("mcp: failed to attach listener to tokio runtime: {e}");
                    return;
                }
            };

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

    /// Windows host build: the Unix-socket MCP transport is unavailable.
    /// No-op so app startup proceeds; agents reach the app's tools from
    /// inside WSL (plan M4+). `_state` is consumed to match the Unix
    /// signature and to keep `RunningListener`/`CancellationToken`
    /// referenced types live.
    #[cfg(not(unix))]
    pub(crate) fn start(&self, socket_path: &Path, _state: McpState) -> crate::error::Result<()> {
        let _ = (socket_path, &self.inner);
        log::warn!(
            "mcp: in-process server is Unix-socket-based; disabled on the Windows host build"
        );
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

#[cfg(unix)]
fn bind_listener(socket_path: &Path) -> crate::error::Result<StdUnixListener> {
    // Remove stale socket from a prior crash.
    let _ = std::fs::remove_file(socket_path);

    let listener = StdUnixListener::bind(socket_path).map_err(|e| {
        crate::error::Error::msg(format!(
            "mcp: failed to bind {}: {e}",
            socket_path.display()
        ))
    })?;
    listener.set_nonblocking(true).map_err(|e| {
        crate::error::Error::msg(format!(
            "mcp: failed to set {} nonblocking: {e}",
            socket_path.display()
        ))
    })?;
    Ok(listener)
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    #[test]
    fn bind_listener_does_not_require_tokio_reactor() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("mcp.sock");

        let listener = bind_listener(&socket_path).unwrap();

        assert!(socket_path.exists());
        let err = listener.accept().expect_err("empty nonblocking listener");
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
    }
}

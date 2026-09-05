use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use runner_core::app_paths::IpcEndpoint;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf};

#[cfg(unix)]
use std::os::unix::net::UnixListener as StdUnixListener;
#[cfg(unix)]
use std::path::Path;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

pub struct IpcListener {
    #[cfg(unix)]
    listener: UnixListener,
    #[cfg(windows)]
    listener: NamedPipeServer,
    #[cfg(windows)]
    endpoint: IpcEndpoint,
}

impl IpcListener {
    pub fn bind(endpoint: &IpcEndpoint) -> crate::error::Result<Self> {
        #[cfg(unix)]
        {
            let listener = bind_unix_listener(&endpoint.0)?;
            let listener = UnixListener::from_std(listener).map_err(|e| {
                crate::error::Error::msg(format!(
                    "mcp: failed to attach listener to tokio runtime: {e}"
                ))
            })?;
            Ok(Self { listener })
        }
        #[cfg(windows)]
        {
            let listener = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&endpoint.0)
                .map_err(|e| {
                    crate::error::Error::msg(format!("mcp: failed to bind {endpoint}: {e}"))
                })?;
            Ok(Self {
                listener,
                endpoint: endpoint.clone(),
            })
        }
    }

    pub async fn accept(&mut self) -> io::Result<IpcStream> {
        #[cfg(unix)]
        {
            let (stream, _) = self.listener.accept().await?;
            Ok(IpcStream(stream))
        }
        #[cfg(windows)]
        {
            self.listener.connect().await?;
            let next = ServerOptions::new().create(&self.endpoint.0)?;
            Ok(IpcStream(std::mem::replace(&mut self.listener, next)))
        }
    }
}

#[cfg(unix)]
pub struct IpcStream(UnixStream);
#[cfg(windows)]
pub struct IpcStream(NamedPipeServer);

impl IpcStream {
    pub fn into_split(self) -> (ReadHalf<Self>, WriteHalf<Self>) {
        tokio::io::split(self)
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(unix)]
fn bind_unix_listener(socket_path: &Path) -> crate::error::Result<StdUnixListener> {
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

        let listener = bind_unix_listener(&socket_path).unwrap();

        assert!(socket_path.exists());
        let err = listener.accept().expect_err("empty nonblocking listener");
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn accepts_multiple_connections() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let endpoint = IpcEndpoint(dir.path().join("mcp.sock"));
        #[cfg(windows)]
        let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
            r"\\.\pipe\runner-test-{}",
            dir.path().file_name().unwrap().to_string_lossy()
        )));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut listener = IpcListener::bind(&endpoint).unwrap();
            #[cfg(windows)]
            assert!(IpcListener::bind(&endpoint).is_err());
            for byte in [42, 43] {
                #[cfg(unix)]
                let mut client = UnixStream::connect(&endpoint.0).await.unwrap();
                #[cfg(windows)]
                let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
                    .open(&endpoint.0)
                    .unwrap();
                let stream = listener.accept().await.unwrap();
                let (mut read, mut write) = stream.into_split();
                client.write_all(&[byte]).await.unwrap();
                assert_eq!(read.read_u8().await.unwrap(), byte);
                write.write_all(&[byte + 1]).await.unwrap();
                assert_eq!(client.read_u8().await.unwrap(), byte + 1);
            }
        });
    }
}

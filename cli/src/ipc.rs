use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use runner_core::app_paths::IpcEndpoint;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;
#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(unix)]
pub struct IpcStream(UnixStream);
#[cfg(windows)]
pub struct IpcStream(NamedPipeClient);

impl IpcStream {
    pub async fn connect(endpoint: &IpcEndpoint) -> io::Result<Self> {
        #[cfg(unix)]
        {
            UnixStream::connect(&endpoint.0).await.map(Self)
        }
        #[cfg(windows)]
        loop {
            match ClientOptions::new().open(&endpoint.0) {
                Ok(client) => return Ok(Self(client)),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::windows::named_pipe::ServerOptions;

    #[test]
    fn retries_busy_pipe_until_an_instance_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
            r"\\.\pipe\runner-client-test-{}",
            dir.path().file_name().unwrap().to_string_lossy()
        )));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .max_instances(1)
                .create(&endpoint.0)
                .unwrap();
            let first = ClientOptions::new().open(&endpoint.0).unwrap();
            server.connect().await.unwrap();
            assert_eq!(
                ClientOptions::new()
                    .open(&endpoint.0)
                    .unwrap_err()
                    .raw_os_error(),
                Some(ERROR_PIPE_BUSY)
            );
            let release = async {
                tokio::time::sleep(Duration::from_millis(60)).await;
                drop(first);
                server.disconnect().unwrap();
                server.connect().await.unwrap();
            };
            let connect = async {
                tokio::time::timeout(Duration::from_secs(2), IpcStream::connect(&endpoint))
                    .await
                    .unwrap()
                    .unwrap()
            };
            let (_client, ()) = tokio::join!(connect, release);
        });
    }
}

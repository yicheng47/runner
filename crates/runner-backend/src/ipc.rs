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
            let listener = secure_pipe(endpoint, true).map_err(|e| {
                crate::error::Error::msg(format!("mcp: failed to bind {endpoint}: {e}"))
            })?;
            Ok(Self {
                listener,
                endpoint: endpoint.clone(),
            })
        }
    }

    #[cfg(windows)]
    pub fn duplicate_handle(&self) -> io::Result<std::os::windows::io::OwnedHandle> {
        use std::os::windows::io::{AsRawHandle, BorrowedHandle};
        unsafe { BorrowedHandle::borrow_raw(self.listener.as_raw_handle()).try_clone_to_owned() }
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
            let next = secure_pipe(&self.endpoint, false)?;
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
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true).map_err(|e| {
        crate::error::Error::msg(format!(
            "mcp: failed to set {} nonblocking: {e}",
            socket_path.display()
        ))
    })?;
    Ok(listener)
}

#[cfg(windows)]
fn secure_pipe(endpoint: &IpcEndpoint, first: bool) -> io::Result<NamedPipeServer> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut user = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
        let got = GetTokenInformation(token, TokenUser, user.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if got == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW((*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sid_len = 0;
        while *sid.add(sid_len) != 0 {
            sid_len += 1;
        }
        let sid_string = String::from_utf16_lossy(std::slice::from_raw_parts(sid, sid_len));
        LocalFree(sid.cast());
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid_string})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                &endpoint.0,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        LocalFree(descriptor);
        result
    }
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

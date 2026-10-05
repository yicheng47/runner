use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, FILE_FLAG_OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

pub struct Stream {
    file: std::sync::Arc<File>,
    timeout: Option<Duration>,
}
impl Stream {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: std::sync::Arc::new(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(FILE_FLAG_OVERLAPPED)
                    .open(path)?,
            ),
            timeout: None,
        })
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.clone(),
            timeout: self.timeout,
        })
    }
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn interrupt(&self) {
        unsafe { CancelIoEx(self.file.as_raw_handle(), std::ptr::null()) };
    }
    fn transfer(&self, bytes: *mut u8, len: usize, write: bool) -> io::Result<usize> {
        unsafe {
            let event = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut overlapped: OVERLAPPED = std::mem::zeroed();
            overlapped.hEvent = event;
            let file = self.file.as_raw_handle();
            let mut transferred = 0;
            let started = if write {
                WriteFile(
                    file,
                    bytes,
                    len.min(u32::MAX as usize) as u32,
                    std::ptr::null_mut(),
                    &mut overlapped,
                )
            } else {
                ReadFile(
                    file,
                    bytes,
                    len.min(u32::MAX as usize) as u32,
                    std::ptr::null_mut(),
                    &mut overlapped,
                )
            };
            let result = if started == 0
                && io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING as i32)
            {
                Err(io::Error::last_os_error())
            } else {
                let wait = self
                    .timeout
                    .map(|timeout| timeout.as_millis().min((u32::MAX - 1) as u128) as u32)
                    .unwrap_or(INFINITE);
                if WaitForSingleObject(event, wait) == WAIT_TIMEOUT {
                    CancelIoEx(file, &overlapped);
                    GetOverlappedResult(file, &overlapped, &mut transferred, 1);
                    Err(io::ErrorKind::TimedOut.into())
                } else if GetOverlappedResult(file, &overlapped, &mut transferred, 1) == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(transferred as usize)
                }
            };
            CloseHandle(event);
            result
        }
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.transfer(bytes.as_mut_ptr(), bytes.len(), false)
    }
}
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(bytes.as_ptr() as *mut u8, bytes.len(), true)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

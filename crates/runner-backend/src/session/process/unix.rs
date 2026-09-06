use std::io::{self, ErrorKind};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::MasterPty;

use super::super::pty_runtime::{poll_until, STOP_POLL};
use super::super::runtime::RuntimeError;

/// How long descendants that outlived the agent get after SIGTERM before
/// SIGKILL. codex has no SIGHUP handler, so anything it was running (in its
/// own session via `setsid`) is still alive once codex is gone.
const DESCENDANT_TERM_GRACE: Duration = Duration::from_secs(1);

pub(crate) struct ProcessTree {
    pid: i32,
    foreground_fd: Option<std::os::fd::RawFd>,
}

impl ProcessTree {
    pub(crate) fn adopt(pid: u32) -> io::Result<Self> {
        Ok(Self {
            pid: pid as i32,
            foreground_fd: None,
        })
    }

    pub(crate) fn attach_pty(&mut self, master: &dyn MasterPty) {
        self.foreground_fd = master.as_raw_fd();
    }

    pub(crate) fn snapshot_descendants(&self) -> Vec<u32> {
        live_descendants(self.pid)
            .into_iter()
            .map(|pid| pid as u32)
            .collect()
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        signal_process_group(self.pid, libc::SIGKILL)
    }

    pub(crate) fn has_other_processes(&self) -> io::Result<bool> {
        let fd = self.foreground_fd.ok_or_else(|| {
            io::Error::new(
                ErrorKind::Unsupported,
                "PTY does not expose a file descriptor",
            )
        })?;
        let foreground_pid = match unsafe { libc::tcgetpgrp(fd) } {
            pid if pid > 0 => Some(pid),
            _ => None,
        };
        distinct_foreground_process(Some(self.pid), foreground_pid)
            .ok_or_else(io::Error::last_os_error)
    }
}

pub(crate) fn kill_process(pid: i32) -> io::Result<()> {
    signal_process(pid, libc::SIGKILL)
}

pub(crate) fn prepare_headless_fork(command: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

#[cfg(unix)]
pub(crate) fn distinct_foreground_process(
    shell_pid: Option<i32>,
    foreground_pid: Option<i32>,
) -> Option<bool> {
    shell_pid
        .zip(foreground_pid)
        .map(|(shell_pid, foreground_pid)| shell_pid != foreground_pid)
}

fn signal_process_group(pid: i32, signal: i32) -> std::io::Result<()> {
    if pid <= 1 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("refusing to signal unsafe pid {pid}"),
        ));
    }
    let group_result = unsafe { libc::kill(-pid, signal) };
    if group_result == 0 {
        return Ok(());
    }
    let group_error = std::io::Error::last_os_error();
    if group_error.raw_os_error() != Some(libc::ESRCH) {
        return Err(group_error);
    }
    signal_process(pid, signal)
}

fn signal_process(pid: i32, signal: i32) -> std::io::Result<()> {
    if pid <= 1 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("refusing to signal unsafe pid {pid}"),
        ));
    }
    let process_result = unsafe { libc::kill(pid, signal) };
    if process_result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Every live process below `pid`, depth-first, as of right now.
#[cfg(target_os = "macos")]
fn live_descendants(pid: i32) -> Vec<i32> {
    let mut found = Vec::new();
    let mut frontier = vec![pid];
    while let Some(parent) = frontier.pop() {
        let mut buf = [0 as libc::pid_t; 1024];
        // Returns the number of pids written, not bytes (unlike proc_listpids).
        let count = unsafe {
            libc::proc_listchildpids(
                parent,
                buf.as_mut_ptr().cast(),
                std::mem::size_of_val(&buf) as libc::c_int,
            )
        };
        if count <= 0 {
            continue;
        }
        for &child in &buf[..(count as usize).min(buf.len())] {
            if child > 1 && !found.contains(&child) {
                found.push(child);
                frontier.push(child);
            }
        }
    }
    found
}

#[cfg(not(target_os = "macos"))]
fn live_descendants(_pid: i32) -> Vec<i32> {
    Vec::new()
}

/// SIGTERM, then SIGKILL, whichever of the pre-stop `descendants` are still
/// running now that the agent itself has exited.
pub(crate) fn reap_descendants(session_id: &str, descendants: &[i32]) {
    let survivors: Vec<i32> = descendants
        .iter()
        .copied()
        .filter(|&pid| process_exists(pid))
        .collect();
    if survivors.is_empty() {
        return;
    }
    log::warn!(
        "session {session_id}: {} descendant(s) outlived the agent; SIGTERM {survivors:?}",
        survivors.len()
    );
    for &pid in &survivors {
        if let Err(error) = signal_process(pid, libc::SIGTERM) {
            log::warn!("session {session_id}: SIGTERM pid {pid} failed: {error}");
        }
    }
    let _ = poll_until(STOP_POLL, DESCENDANT_TERM_GRACE, || {
        Ok::<_, RuntimeError>(
            survivors
                .iter()
                .all(|&pid| !process_exists(pid))
                .then_some(()),
        )
    });
    let stubborn: Vec<i32> = survivors
        .into_iter()
        .filter(|&pid| process_exists(pid))
        .collect();
    if stubborn.is_empty() {
        return;
    }
    log::warn!("session {session_id}: SIGKILL descendants that ignored SIGTERM {stubborn:?}");
    for &pid in &stubborn {
        if let Err(error) = signal_process(pid, libc::SIGKILL) {
            log::warn!("session {session_id}: SIGKILL pid {pid} failed: {error}");
        }
    }
}

pub(crate) fn process_exists(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(target_os = "macos")]
pub(crate) fn process_command_line(pid: i32) -> std::io::Result<Option<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = 0;
    let size_result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if size_result != 0 {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let mut bytes = vec![0u8; size];
    let read_result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read_result != 0 {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    bytes.truncate(size);
    Ok(parse_macos_process_args(&bytes))
}

#[cfg(target_os = "macos")]
fn parse_macos_process_args(bytes: &[u8]) -> Option<String> {
    let argc_bytes: [u8; std::mem::size_of::<i32>()] =
        bytes.get(..std::mem::size_of::<i32>())?.try_into().ok()?;
    let argc = i32::from_ne_bytes(argc_bytes);
    if argc <= 0 {
        return None;
    }

    let mut cursor = std::mem::size_of::<i32>();
    cursor += bytes.get(cursor..)?.iter().position(|byte| *byte == 0)? + 1;
    while bytes.get(cursor) == Some(&0) {
        cursor += 1;
    }

    let mut args = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        let remaining = bytes.get(cursor..)?;
        let end = remaining.iter().position(|byte| *byte == 0)?;
        args.push(String::from_utf8_lossy(&remaining[..end]).into_owned());
        cursor += end + 1;
    }
    (!args.is_empty()).then(|| args.join(" "))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_command_line(pid: i32) -> std::io::Result<Option<String>> {
    let path = format!("/proc/{pid}/cmdline");
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let args: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect();
    Ok((!args.is_empty()).then(|| args.join(" ")))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_command_line(pid: i32) -> std::io::Result<Option<String>> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let command_line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!command_line.is_empty()).then_some(command_line))
}

pub(crate) fn wait_for_process_exit_until(pid: i32, deadline: Instant) -> bool {
    loop {
        if !process_exists(pid) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        thread::sleep(STOP_POLL.min(deadline.saturating_duration_since(now)));
    }
}

pub(crate) fn kill_headless_fork(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

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

#[cfg(not(target_os = "linux"))]
pub(crate) type Descendant = i32;

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcIdentity {
    pid: i32,
    started: u64,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(crate) struct Descendant {
    pid: i32,
    fd: std::os::fd::OwnedFd,
}

#[cfg(target_os = "linux")]
impl Descendant {
    fn capture(identity: ProcIdentity) -> Option<Self> {
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) };
        if fd < 0 {
            log::warn!(
                "cannot retain descendant {} identity: {}",
                identity.pid,
                io::Error::last_os_error()
            );
            return None;
        }
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) };
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", identity.pid)).ok()?;
        if !identity.matches(&stat) {
            return None;
        }
        Some(Self {
            pid: identity.pid,
            fd,
        })
    }

    fn alive(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut poll = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let result = unsafe { libc::poll(&mut poll, 1, 0) };
            if result >= 0 {
                return result == 0;
            }
            let error = io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                log::warn!("pidfd poll descendant {} failed: {error}", self.pid);
                return false;
            }
        }
    }

    fn signal(&self, signal: i32) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(target_os = "linux")]
impl ProcIdentity {
    fn matches(self, stat: &str) -> bool {
        stat.split_whitespace()
            .next()
            .and_then(|pid| pid.parse::<i32>().ok())
            == Some(self.pid)
            && proc_start_time(stat) == Some(self.started)
    }
}

pub(crate) struct ProcessTree {
    pid: i32,
    foreground_fd: Option<std::os::fd::RawFd>,
    #[cfg(target_os = "linux")]
    root: Descendant,
    #[cfg(target_os = "linux")]
    root_identity: ProcIdentity,
}

impl ProcessTree {
    pub(crate) fn adopt(pid: u32) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let root_identity = {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
            ProcIdentity {
                pid: pid as i32,
                started: proc_start_time(&stat)
                    .ok_or_else(|| io::Error::other("invalid root process stat"))?,
            }
        };
        #[cfg(target_os = "linux")]
        let root = Descendant::capture(root_identity).ok_or_else(|| {
            io::Error::other("cannot retain root process identity (Linux pidfds required)")
        })?;
        Ok(Self {
            pid: pid as i32,
            foreground_fd: None,
            #[cfg(target_os = "linux")]
            root,
            #[cfg(target_os = "linux")]
            root_identity,
        })
    }

    pub(crate) fn attach_pty(&mut self, master: &dyn MasterPty) {
        self.foreground_fd = master.as_raw_fd();
    }

    pub(crate) fn snapshot_descendants(&self) -> Vec<Descendant> {
        #[cfg(target_os = "linux")]
        {
            if !self.root.alive() {
                return Vec::new();
            }
            let found = live_descendants(self.root_identity)
                .into_iter()
                .filter_map(Descendant::capture)
                .collect();
            if self.root.alive() {
                found
            } else {
                Vec::new()
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            live_descendants(self.pid)
        }
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

#[cfg(target_os = "linux")]
fn live_descendants(identity: ProcIdentity) -> Vec<ProcIdentity> {
    descendants_for_identity(std::path::Path::new("/proc"), identity)
}

#[cfg(target_os = "linux")]
fn descendants_for_identity(root: &std::path::Path, identity: ProcIdentity) -> Vec<ProcIdentity> {
    let matches = || {
        std::fs::read_to_string(root.join(identity.pid.to_string()).join("stat"))
            .is_ok_and(|stat| identity.matches(&stat))
    };
    if !matches() {
        return Vec::new();
    }
    let found = descendants_in_proc(root, identity.pid);
    if matches() {
        found
    } else {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn descendants_in_proc(root: &std::path::Path, pid: i32) -> Vec<ProcIdentity> {
    if pid <= 1 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let parents: Vec<(ProcIdentity, i32)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let child = entry.file_name().to_str()?.parse::<i32>().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            Some((
                ProcIdentity {
                    pid: child,
                    started: proc_start_time(&stat)?,
                },
                proc_parent_pid(&stat)?,
            ))
        })
        .collect();
    let mut found = Vec::new();
    let mut frontier = vec![pid];
    while let Some(parent) = frontier.pop() {
        for &(child, owner) in &parents {
            if owner == parent && child.pid > 1 && child.pid != pid && !found.contains(&child) {
                found.push(child);
                frontier.push(child.pid);
            }
        }
    }
    found
}

#[cfg(target_os = "linux")]
fn proc_parent_pid(stat: &str) -> Option<i32> {
    // comm may contain spaces and parentheses; fields resume after its final ')'.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn proc_start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn live_descendants(_pid: i32) -> Vec<i32> {
    Vec::new()
}

/// SIGTERM, then SIGKILL, whichever of the pre-stop `descendants` are still
/// running now that the agent itself has exited.
#[cfg(not(target_os = "linux"))]
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

#[cfg(target_os = "linux")]
pub(crate) fn reap_descendants(session_id: &str, descendants: &[Descendant]) {
    for child in descendants.iter().filter(|child| child.alive()) {
        if let Err(error) = child.signal(libc::SIGTERM) {
            log::warn!(
                "session {session_id}: pidfd SIGTERM descendant {} failed: {error}",
                child.pid
            );
        }
    }
    let _ = poll_until(STOP_POLL, DESCENDANT_TERM_GRACE, || {
        Ok::<_, RuntimeError>(descendants.iter().all(|child| !child.alive()).then_some(()))
    });
    for child in descendants.iter().filter(|child| child.alive()) {
        if let Err(error) = child.signal(libc::SIGKILL) {
            log::warn!(
                "session {session_id}: pidfd SIGKILL descendant {} failed: {error}",
                child.pid
            );
        }
    }
}

pub(crate) fn process_exists(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        if stat
            .rsplit_once(')')
            .and_then(|(_, fields)| fields.split_whitespace().next())
            .is_some_and(|state| matches!(state, "Z" | "X"))
        {
            return false;
        }
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

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn proc_descendants_preserve_tree_boundaries_and_skip_vanished_entries() {
        let root = tempfile::tempdir().unwrap();
        for (pid, parent) in [(40, 2), (41, 40), (42, 41), (43, 2), (44, 44)] {
            let dir = root.path().join(pid.to_string());
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(
                dir.join("stat"),
                format!(
                    "{pid} (a ) tricky ( name) S {parent} {} {pid}",
                    vec!["0"; 17].join(" ")
                ),
            )
            .unwrap();
        }
        std::fs::create_dir(root.path().join("45")).unwrap();
        assert_eq!(
            descendants_in_proc(root.path(), 40),
            vec![
                ProcIdentity {
                    pid: 41,
                    started: 41
                },
                ProcIdentity {
                    pid: 42,
                    started: 42
                }
            ]
        );
        assert!(descendants_in_proc(root.path(), 1).is_empty());
        assert!(descendants_in_proc(root.path(), 99).is_empty());
        assert!(descendants_for_identity(
            root.path(),
            ProcIdentity {
                pid: 40,
                started: 39
            }
        )
        .is_empty());
        assert_eq!(
            descendants_for_identity(
                root.path(),
                ProcIdentity {
                    pid: 40,
                    started: 40
                }
            )
            .len(),
            2
        );

        assert!(descendants_in_proc(&root.path().join("missing"), 40).is_empty());
        assert_eq!(proc_parent_pid("malformed"), None);
        assert_eq!(proc_parent_pid("40 (name) S invalid"), None);
    }

    #[test]
    fn proc_identity_refuses_reused_or_malformed_pid() {
        let stat = format!("51 (same name) S 40 {} 123", vec!["0"; 17].join(" "));
        assert!(ProcIdentity {
            pid: 51,
            started: 123
        }
        .matches(&stat));
        assert!(!ProcIdentity {
            pid: 51,
            started: 122
        }
        .matches(&stat));
        assert!(!ProcIdentity {
            pid: 52,
            started: 123
        }
        .matches(&stat));
        assert!(!ProcIdentity {
            pid: 51,
            started: 123
        }
        .matches("51 (broken) S"));
    }

    #[test]
    fn proc_pidfd_retains_only_the_owned_child_lifetime() {
        let mut child = std::process::Command::new("sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        let identity = ProcIdentity {
            pid: child.id() as i32,
            started: proc_start_time(&stat).unwrap(),
        };
        let retained = Descendant::capture(identity).expect("Linux pidfds required");
        assert!(retained.alive());
        assert!(Descendant::capture(ProcIdentity {
            started: identity.started + 1,
            ..identity
        })
        .is_none());
        retained.signal(libc::SIGTERM).unwrap();
        child.wait().unwrap();
        assert!(!retained.alive());
        assert_eq!(
            retained.signal(libc::SIGKILL).unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn proc_snapshot_finds_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("10")
            .spawn()
            .unwrap();
        let pid = std::process::id() as i32;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let found = live_descendants(ProcIdentity {
            pid,
            started: proc_start_time(&stat).unwrap(),
        });
        let _ = child.kill();
        let _ = child.wait();
        assert!(found
            .iter()
            .any(|identity| identity.pid == child.id() as i32));
    }
}

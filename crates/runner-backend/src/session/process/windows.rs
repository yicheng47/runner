use std::io;
use std::time::Instant;

use portable_pty::MasterPty;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows process lifecycle is not implemented",
    )
}

pub(crate) struct ProcessTree;

impl ProcessTree {
    pub(crate) fn adopt(_pid: u32) -> io::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn attach_pty(&mut self, _master: &dyn MasterPty) {}

    pub(crate) fn snapshot_descendants(&self) -> Vec<u32> {
        Vec::new()
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn has_other_processes(&self) -> io::Result<bool> {
        Err(unsupported())
    }
}

pub(crate) fn kill_process(_pid: i32) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn process_exists(_pid: i32) -> bool {
    false
}

pub(crate) fn reap_descendants(_session_id: &str, _descendants: &[i32]) {}

pub(crate) fn process_command_line(_pid: i32) -> io::Result<Option<String>> {
    Err(unsupported())
}

pub(crate) fn wait_for_process_exit_until(_pid: i32, _deadline: Instant) -> bool {
    false
}

pub(crate) fn prepare_headless_fork(_command: &mut std::process::Command) {}

pub(crate) fn kill_headless_fork(_child: &mut std::process::Child) {}

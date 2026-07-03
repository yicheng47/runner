//! Install the Linux `runner` agent CLI into the WSL distro.
//!
//! Mission agents run inside WSL and emit signals/messages onto the
//! cross-boundary event bus by invoking `runner` (the bundled CLI). That
//! binary must therefore be a **Linux** ELF living somewhere the distro
//! can exec — not the Windows `runner.exe` sidecar `cli_install` stages
//! for the host.
//!
//! We embed the Linux ELF in the app binary (built from `cli/` for
//! `x86_64-unknown-linux-gnu`, staged at `binaries/`) and stream it into
//! the distro's ext4 at startup. ext4 (not `/mnt/c`) because drvfs does
//! not reliably persist the exec bit, and we need `chmod +x` to stick.
//!
//! Idempotent: rewrites every launch (cheap, ~1.4 MB), so a version bump
//! always ships the matching CLI.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The in-distro install location, relative to the WSL user's `$HOME`.
/// The mission launch script prepends `$HOME/RUNNER_BIN_SUBDIR` to PATH.
pub const RUNNER_BIN_SUBDIR: &str = ".local/share/runner/bin";

const WSL_EXE: &str = r"C:\Windows\System32\wsl.exe";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Linux ELF of the agent CLI, embedded at compile time. Staged by the
/// build/CI from `cargo build -p runner-cli --bin runner-agent-cli` on a
/// Linux target. (`include_bytes!` is compiled only on Windows since this
/// module is `cfg(windows)`.)
static LINUX_RUNNER_ELF: &[u8] =
    include_bytes!("../../../binaries/runner-agent-cli-linux-x86_64");

/// Stream the embedded Linux `runner` ELF into
/// `~/.local/share/runner/bin/runner` in the given distro and mark it
/// executable. Best-effort: logs and returns on failure (a mission that
/// then can't find `runner` degrades to a terminal that simply can't
/// emit bus signals — the rest of the app keeps working).
pub fn install_linux_runner(distro: &str) {
    let script = format!(
        "set -e; mkdir -p \"$HOME/{dir}\"; \
         cat > \"$HOME/{dir}/runner\"; \
         chmod +x \"$HOME/{dir}/runner\"",
        dir = RUNNER_BIN_SUBDIR
    );

    // `bash -c` (non-login): the script only needs `$HOME` (set by WSL
    // regardless) and mkdir/cat/chmod, so we skip `-l`/`-i` to avoid any
    // rc-file that might touch stdin before `cat` reads the ELF.
    let mut child = match Command::new(WSL_EXE)
        .args(["-d", distro, "--", "bash", "-c", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("wsl: failed to spawn runner-CLI install for distro {distro}: {e}");
            return;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(LINUX_RUNNER_ELF) {
            log::warn!("wsl: failed to stream runner ELF into {distro}: {e}");
            // Fall through to reap the child.
        }
        // Drop stdin so `cat` sees EOF.
    }

    // Bounded wait so a hung WSL can't stall startup.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(INSTALL_TIMEOUT) {
        Ok(Ok(out)) if out.status.success() => {
            log::info!(
                "wsl: installed Linux runner CLI into {distro}:~/{RUNNER_BIN_SUBDIR}/runner ({} bytes)",
                LINUX_RUNNER_ELF.len()
            );
        }
        Ok(Ok(out)) => {
            log::warn!(
                "wsl: runner-CLI install exited {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(Err(e)) => log::warn!("wsl: runner-CLI install wait failed: {e}"),
        Err(_) => log::warn!("wsl: runner-CLI install timed out after {INSTALL_TIMEOUT:?}"),
    }
}

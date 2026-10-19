//! Coordinator-only native systemd validation. Does not launch the GUI or agents.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("linux_service_check requires Linux and a working systemd user manager");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use runner_core::daemon_process::{self, Launch, NativePaths};
    use runner_core::protocol::socket::SocketTransport;
    use runner_core::protocol::wire::Hello;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn unit_value(unit: &str, property: &str) -> String {
        let output = Command::new("systemctl")
            .args(["--user", "show", unit, "--property", property, "--value"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "systemctl show {unit}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn connect(launch: &Launch, hash: &str) -> std::sync::Arc<SocketTransport> {
        let deadline = Instant::now() + daemon_process::DAEMON_START_TIMEOUT;
        loop {
            match SocketTransport::connect(
                &launch.daemon_endpoint,
                Hello {
                    exe_sha256: hash.into(),
                    client: "app".into(),
                },
            ) {
                Ok(client) => return client,
                Err(error) if Instant::now() >= deadline => panic!("service handshake: {error}"),
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }
    fn wait_inactive(unit: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = unit_value(unit, "ActiveState");
            if state == "inactive" || state == "failed" {
                break;
            }
            assert!(Instant::now() < deadline, "unit did not stop: {unit}");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(unit_value(unit, "NRestarts"), "0");
        assert_eq!(unit_value(unit, "ExecMainStatus"), "0");
    }
    fn daemon_args(launch: &Launch) -> Vec<std::ffi::OsString> {
        [
            ("--app-data-dir", launch.paths.app_data_dir.as_path()),
            ("--log-dir", launch.paths.log_dir.as_path()),
            ("--home-dir", launch.paths.home_dir.as_deref().unwrap()),
            ("--endpoint", launch.daemon_endpoint.0.as_path()),
            ("--mcp-endpoint", launch.mcp_endpoint.0.as_path()),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [flag.into(), path.as_os_str().to_owned()])
        .chain(["--isolated".into()])
        .collect()
    }
    fn start_unit(unit: &str, daemon: &Path, launch: &Launch) {
        let status = Command::new("systemd-run")
            .args([
                "--user",
                "--unit",
                unit,
                "--property=Restart=no",
                "--property=KillMode=mixed",
                "--property=TimeoutStopSec=20",
                "--property=UnsetEnvironment=SHELL",
                "--setenv=PATH=/usr/bin:/bin",
            ])
            .arg(format!(
                "--setenv=HOME={}",
                launch.paths.home_dir.as_ref().unwrap().display()
            ))
            .arg(daemon)
            .args(daemon_args(launch))
            .status()
            .unwrap();
        assert!(status.success(), "systemd-run failed");
    }
    struct Cleanup {
        units: Vec<String>,
        launch: Launch,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(client) = SocketTransport::connect(
                &self.launch.daemon_endpoint,
                Hello {
                    exe_sha256: String::new(),
                    client: "stop".into(),
                },
            ) {
                let _ = client.shutdown(true);
                let _ = daemon_process::wait_unlocked(
                    &self.launch.paths.app_data_dir,
                    daemon_process::DAEMON_STOP_TIMEOUT,
                );
            }
            for unit in &self.units {
                let _ = Command::new("systemctl")
                    .args(["--user", "stop", unit])
                    .status();
                let _ = Command::new("systemctl")
                    .args(["--user", "reset-failed", unit])
                    .status();
            }
        }
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: linux_service_check OLD_RELEASE_BIN_DIR DIFFERENT_RELEASE_BIN_DIR".into(),
        );
    }
    let old = PathBuf::from(&args[0]).canonicalize()?;
    let new = PathBuf::from(&args[1]).canonicalize()?;
    let source = old.join("runner-agent-cli");
    let replacement = new.join("runner-agent-cli");
    let old_hash = daemon_process::executable_hash(&source)?;
    let new_hash = daemon_process::executable_hash(&replacement)?;
    assert_ne!(old_hash, new_hash, "Use separately built versions/stamps; never mutate executable bytes to manufacture a mismatch");
    assert_eq!(
        old_hash,
        daemon_process::executable_hash(&old.join("runnerd"))?
    );
    let root = tempfile::Builder::new()
        .prefix("r823-svc-")
        .tempdir_in("/tmp")?;
    let paths = NativePaths {
        home_dir: Some(root.path().join("h")),
        app_data_dir: root.path().join("d"),
        log_dir: root.path().join("l"),
    };
    std::fs::create_dir_all(paths.home_dir.as_ref().unwrap())?;
    let mut launch = Launch::new(paths, source, true);
    launch.isolated = true;
    assert!(launch.daemon_endpoint.0.as_os_str().len() < 100);
    let units: Vec<String> = (0..3)
        .map(|index| format!("runner-823-{}-{index}.service", std::process::id()))
        .collect();
    let cleanup = Cleanup {
        units,
        launch: launch.clone(),
    };
    println!(
        "root={} old_sha256={old_hash} new_sha256={new_hash}",
        root.path().display()
    );

    let app_first = launch.connect_or_spawn(&old_hash)?;
    let app_pid = app_first.welcome.pid;
    start_unit(&cleanup.units[0], &old.join("runnerd"), &launch);
    wait_inactive(&cleanup.units[0]);
    assert_eq!(connect(&launch, &old_hash).welcome.pid, app_pid);
    app_first.shutdown(true)?;
    daemon_process::wait_unlocked(
        &launch.paths.app_data_dir,
        daemon_process::DAEMON_STOP_TIMEOUT,
    )?;
    println!("PASS app-first duplicate service exits without replacing pid={app_pid}");

    start_unit(&cleanup.units[1], &old.join("runnerd"), &launch);
    let service = connect(&launch, &old_hash);
    let pid = service.welcome.pid;
    let (same, restarted) = launch.connect_or_spawn_with_restart(&old_hash)?;
    assert!(!restarted);
    assert_eq!(same.welcome.pid, pid);
    assert_eq!(unit_value(&cleanup.units[1], "MainPID"), pid.to_string());
    let duplicate = Command::new(old.join("runnerd"))
        .args(daemon_args(&launch))
        .status()?;
    assert!(duplicate.success());
    assert_eq!(connect(&launch, &old_hash).welcome.pid, pid);
    same.shutdown(true)?;
    daemon_process::wait_unlocked(
        &launch.paths.app_data_dir,
        daemon_process::DAEMON_STOP_TIMEOUT,
    )?;
    wait_inactive(&cleanup.units[1]);
    println!("PASS service-first same-hash reuse/duplicate/normal Stop, pid={pid}, NRestarts=0");

    start_unit(&cleanup.units[2], &old.join("runnerd"), &launch);
    let before = connect(&launch, &old_hash).welcome.pid;
    let mut next = launch.clone();
    next.source = replacement;
    let (after, restarted) = next.connect_or_spawn_with_restart(&new_hash)?;
    assert!(restarted);
    assert_ne!(after.welcome.pid, before);
    wait_inactive(&cleanup.units[2]);
    assert_eq!(connect(&next, &new_hash).welcome.pid, after.welcome.pid);
    assert_eq!(
        daemon_process::executable_hash(&launch.paths.app_data_dir.join("bin/runnerd"))?,
        new_hash
    );
    println!(
        "PASS different-hash app replacement {before}->{}; old unit inactive, NRestarts=0",
        after.welcome.pid
    );
    after.shutdown(true)?;
    daemon_process::wait_unlocked(
        &launch.paths.app_data_dir,
        daemon_process::DAEMON_STOP_TIMEOUT,
    )?;
    assert!(!launch.daemon_endpoint.0.exists());
    assert!(!launch.mcp_endpoint.0.exists());
    drop(cleanup);
    println!("PASS owned services, daemon and endpoints cleaned up");
    Ok(())
}

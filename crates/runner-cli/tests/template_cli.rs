use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use runner_core::daemon_process::{self, Launch, NativePaths};
use runner_core::protocol::socket::{ConnectError, SocketTransport};
use runner_core::protocol::wire::Hello;
use serde_json::Value;

struct TestDaemon {
    root: tempfile::TempDir,
    launch: Launch,
    exe: PathBuf,
    path_env: String,
    child: Option<Child>,
}

impl TestDaemon {
    fn start(installed_binaries: &[&str]) -> Self {
        #[cfg(unix)]
        let root = tempfile::Builder::new()
            .prefix("804-cli-")
            .tempdir_in("/tmp")
            .unwrap();
        #[cfg(windows)]
        let root = tempfile::Builder::new()
            .prefix("804-cli-")
            .tempdir()
            .unwrap();
        let root_canon = root.path().canonicalize().unwrap();
        let bin = root_canon.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let source = PathBuf::from(env!("CARGO_BIN_EXE_runner-agent-cli"));
        let daemon_bin = bin.join(runner_core::cli_install::DAEMON_DEST_BIN_NAME);
        let cli_bin = bin.join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
        std::fs::copy(&source, &daemon_bin).unwrap();
        std::fs::copy(&source, &cli_bin).unwrap();
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            let companion = source.parent().unwrap().join(name);
            if companion.exists() {
                std::fs::copy(companion, bin.join(name)).unwrap();
            }
        }

        for binary in installed_binaries {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let stub = bin.join(binary);
                std::fs::write(&stub, "#!/bin/sh\necho 99.0.0\n").unwrap();
                let mut perms = std::fs::metadata(&stub).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&stub, perms).unwrap();
            }
            #[cfg(windows)]
            {
                let stub = bin.join(format!("{binary}.cmd"));
                std::fs::write(&stub, "@echo off\r\necho 99.0.0\r\n").unwrap();
            }
        }

        let home = root_canon.join("home");
        std::fs::create_dir_all(&home).unwrap();
        #[cfg(target_os = "macos")]
        let app_data_dir = home.join("Library/Application Support/com.wycstudios.runner-dev");
        #[cfg(all(unix, not(target_os = "macos")))]
        let app_data_dir = home.join(".local/share/com.wycstudios.runner-dev");
        #[cfg(windows)]
        let app_data_dir = home.join("AppData/Roaming/com.wycstudios.runner-dev");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let paths = NativePaths {
            home_dir: Some(home.clone()),
            app_data_dir,
            log_dir: root_canon.join("logs"),
        };
        let mut launch = Launch::new(paths, cli_bin.clone(), false);
        launch.isolated = true;
        #[cfg(windows)]
        {
            use runner_core::app_paths::IpcEndpoint;
            let key = root.path().file_name().unwrap().to_string_lossy();
            launch.daemon_endpoint =
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runnerd-cli-804-{key}")));
            launch.mcp_endpoint =
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runner-mcp-cli-804-{key}")));
        }

        let path_env = bin.display().to_string();
        let mut cmd = Command::new(&daemon_bin);
        cmd.args(["--app-data-dir"])
            .arg(&launch.paths.app_data_dir)
            .arg("--log-dir")
            .arg(&launch.paths.log_dir)
            .arg("--home-dir")
            .arg(&home)
            .arg("--endpoint")
            .arg(&launch.daemon_endpoint.0)
            .arg("--mcp-endpoint")
            .arg(&launch.mcp_endpoint.0)
            .arg("--isolated")
            .env("PATH", &path_env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(root_canon.join("daemon-stderr.log")).unwrap());
        #[cfg(unix)]
        cmd.env("SHELL", "/bin/sh");
        let child = cmd.spawn().unwrap();
        let hash = daemon_process::executable_hash(&daemon_bin).unwrap();
        let deadline = Instant::now() + daemon_process::DAEMON_START_TIMEOUT;
        loop {
            match SocketTransport::connect(
                &launch.daemon_endpoint,
                Hello {
                    exe_sha256: hash.clone(),
                    client: "template-cli-test".into(),
                },
            ) {
                Ok(_) => break,
                Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => panic!("daemon failed to start: {error}"),
            }
        }

        Self {
            root,
            launch,
            exe: cli_bin,
            path_env,
            child: Some(child),
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(&self.exe);
        cmd.args(args)
            .env("HOME", self.launch.paths.home_dir.as_ref().unwrap())
            .env("USERPROFILE", self.launch.paths.home_dir.as_ref().unwrap())
            .env(
                "XDG_DATA_HOME",
                self.launch
                    .paths
                    .home_dir
                    .as_ref()
                    .unwrap()
                    .join(".local/share"),
            )
            .env("PATH", &self.path_env)
            .env("SSH_CONNECTION", "1")
            .env(
                "RUNNER_TEST_DAEMON_ENDPOINT",
                &self.launch.daemon_endpoint.0,
            )
            .env("RUNNER_TEST_MCP_ENDPOINT", &self.launch.mcp_endpoint.0);
        #[cfg(windows)]
        cmd.env("APPDATA", self.launch.paths.app_data_dir.parent().unwrap());
        for key in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
            "RUNNER_SESSION_ID",
            "RUNNER_SESSION_CLI",
        ] {
            cmd.env_remove(key);
        }
        cmd.output().unwrap()
    }

    fn path(&self) -> &Path {
        self.root.path()
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn crew_and_role_template_export_import_dry_run_and_mapping_round_trip() {
    let daemon = TestDaemon::start(&["claude"]);

    // Use `trae` as the uninstalled runtime unless somehow installed on the host.
    let crew_file = daemon.path().join("trio.crew.toml");
    std::fs::write(
        &crew_file,
        r#"
version = 1
kind = "crew"

[meta]
description = "Pair test crew"
author = "Test"

[crew]
name = "Test Trio"
conventions = """
Line one.
Line two."""

[[slots]]
handle = "coder"
role = "impl"

[[slots]]
handle = "reviewer"
role = "review"
runtime = "trae"
model = "trae-model"
effort = "high"

[[roles]]
handle = "impl"
display_name = "Implementer"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
codex_speed = "fast"
system_prompt = "Build features."

[[roles]]
handle = "review"
display_name = "Reviewer"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
system_prompt = "Review diffs."
"#,
    )
    .unwrap();
    let crew_path = crew_file.to_str().unwrap();

    // 1. --dry-run writes nothing, prints plan, warns about any uninstalled runtimes.
    let dry = daemon.run(&["crew", "import", crew_path, "--dry-run", "--json"]);
    assert!(
        dry.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let dry_val: Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(dry_val["dry_run"], true);
    assert_eq!(dry_val["crew_name"], "Test Trio");
    assert_eq!(dry_val["roles"].as_array().unwrap().len(), 2);
    if !dry_val["uninstalled_runtimes"]
        .as_array()
        .unwrap()
        .is_empty()
    {
        assert!(
            String::from_utf8_lossy(&dry.stderr).contains("not installed"),
            "expected uninstalled warning on stderr"
        );
    }
    let list = daemon.run(&["crew", "list", "--json"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&list.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0,
        "--dry-run must not create any crew"
    );

    // 2. Import with --map-runtime codex=claude-code --map-runtime trae=claude-code --name Custom.
    let imported = daemon.run(&[
        "crew",
        "import",
        crew_path,
        "--name",
        "Custom Trio",
        "--map-runtime",
        "codex=claude-code",
        "--map-runtime",
        "trae=claude-code",
        "--json",
    ]);
    assert!(
        imported.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert!(
        String::from_utf8_lossy(&imported.stderr).trim().is_empty(),
        "no uninstalled warning expected once mapped to installed claude-code: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let imp_val: Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(imp_val["crew"]["name"], "Custom Trio");
    assert_eq!(imp_val["roles"][0]["runtime"], "claude-code");
    assert!(imp_val["roles"][0]["model"].is_null());
    assert!(imp_val["roles"][0]["effort"].is_null());
    assert!(imp_val["roles"][0]["codex_speed"].is_null());
    assert_eq!(imp_val["slots"][1]["runtime_override"], Value::Null);
    assert_eq!(imp_val["slots"][1]["model_override"], Value::Null);

    // 3. Export to stdout, --json, and -o <path>.
    let exported_stdout = daemon.run(&["crew", "export", "Custom Trio"]);
    assert!(exported_stdout.status.success());
    let stdout_toml = String::from_utf8(exported_stdout.stdout).unwrap();
    assert!(stdout_toml.contains("kind = \"crew\""));
    assert!(stdout_toml.contains("name = \"Custom Trio\""));
    runner_core::template::parse_template(&stdout_toml).unwrap();

    let exported_json = daemon.run(&["crew", "export", "Custom Trio", "--json"]);
    assert!(exported_json.status.success());
    let json_val: Value = serde_json::from_slice(&exported_json.stdout).unwrap();
    assert_eq!(json_val["filename"], "custom-trio.crew.toml");
    assert_eq!(json_val["toml"], stdout_toml);

    let out_file = daemon.path().join("exported.crew.toml");
    let exported_file = daemon.run(&[
        "crew",
        "export",
        "Custom Trio",
        "-o",
        out_file.to_str().unwrap(),
        "--json",
    ]);
    assert!(exported_file.status.success());
    let file_val: Value = serde_json::from_slice(&exported_file.stdout).unwrap();
    assert_eq!(file_val["path"], out_file.display().to_string());
    assert_eq!(std::fs::read_to_string(&out_file).unwrap(), stdout_toml);

    // 4. Standalone role export and import with --handle and --runtime.
    let role_file = daemon.path().join("impl.role.toml");
    let role_export = daemon.run(&["role", "export", "impl", "-o", role_file.to_str().unwrap()]);
    assert!(
        role_export.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&role_export.stderr)
    );
    let role_toml = std::fs::read_to_string(&role_file).unwrap();
    assert!(role_toml.contains("kind = \"role\""));

    let role_import = daemon.run(&[
        "role",
        "import",
        role_file.to_str().unwrap(),
        "--handle",
        "impl-copy",
        "--runtime",
        "trae",
        "--json",
    ]);
    assert!(
        role_import.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&role_import.stderr)
    );
    let role_imp_val: Value = serde_json::from_slice(&role_import.stdout).unwrap();
    assert_eq!(role_imp_val["roles"][0]["handle"], "impl-copy");
    assert_eq!(role_imp_val["roles"][0]["runtime"], "trae");
    if !role_imp_val["uninstalled_runtimes"]
        .as_array()
        .unwrap()
        .is_empty()
    {
        assert!(
            String::from_utf8_lossy(&role_import.stderr).contains("trae"),
            "expected uninstalled trae warning"
        );
    }

    // 5. Wrong kind & invalid files produce proper non-zero exit codes.
    let wrong_kind = daemon.run(&["role", "import", crew_path]);
    assert_eq!(wrong_kind.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_kind.stderr).contains("runner crew import"));

    let bad_file = daemon.path().join("bad.crew.toml");
    std::fs::write(
        &bad_file,
        "version = 1\nkind = \"role\"\n[role]\nhandle = \"x\"\ndisplay_name = \"X\"\nruntime = \"codex\"\nenv = { A = \"1\" }\n",
    )
    .unwrap();
    let bad_import = daemon.run(&["role", "import", bad_file.to_str().unwrap()]);
    assert_eq!(bad_import.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad_import.stderr).contains("env"));

    // 6. --dry-run --handle rejects an already-occupied explicit handle, while
    // omitting --handle preserves auto-suffix behavior.
    let occupied_dry = daemon.run(&[
        "role",
        "import",
        role_file.to_str().unwrap(),
        "--handle",
        "impl-copy",
        "--dry-run",
    ]);
    assert_eq!(occupied_dry.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&occupied_dry.stderr).contains("already taken"),
        "stderr: {}",
        String::from_utf8_lossy(&occupied_dry.stderr)
    );
    let autosuffix_dry = daemon.run(&[
        "role",
        "import",
        role_file.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert!(autosuffix_dry.status.success());
    let autosuffix_val: Value = serde_json::from_slice(&autosuffix_dry.stdout).unwrap();
    assert_eq!(autosuffix_val["roles"][0]["handle"], "impl-2");

    // 7. Oversized files and oversized stdin fail cleanly before crossing IPC.
    let huge_file = daemon.path().join("huge.role.toml");
    std::fs::write(
        &huge_file,
        " ".repeat(runner_core::template::MAX_TEMPLATE_BYTES + 1024),
    )
    .unwrap();
    let huge_out = daemon.run(&["role", "import", huge_file.to_str().unwrap()]);
    assert_eq!(huge_out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&huge_out.stderr).contains("256 KiB"),
        "stderr: {}",
        String::from_utf8_lossy(&huge_out.stderr)
    );

    let run_with_stdin = |args: &[&str], stdin_bytes: &[u8]| -> std::process::Output {
        use std::io::Write as _;
        let mut cmd = Command::new(&daemon.exe);
        cmd.args(args)
            .env("HOME", daemon.launch.paths.home_dir.as_ref().unwrap())
            .env(
                "USERPROFILE",
                daemon.launch.paths.home_dir.as_ref().unwrap(),
            )
            .env(
                "XDG_DATA_HOME",
                daemon
                    .launch
                    .paths
                    .home_dir
                    .as_ref()
                    .unwrap()
                    .join(".local/share"),
            )
            .env("PATH", &daemon.path_env)
            .env("SSH_CONNECTION", "1")
            .env(
                "RUNNER_TEST_DAEMON_ENDPOINT",
                &daemon.launch.daemon_endpoint.0,
            )
            .env("RUNNER_TEST_MCP_ENDPOINT", &daemon.launch.mcp_endpoint.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.env(
            "APPDATA",
            daemon.launch.paths.app_data_dir.parent().unwrap(),
        );
        for key in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
            "RUNNER_SESSION_ID",
            "RUNNER_SESSION_CLI",
        ] {
            cmd.env_remove(key);
        }
        let mut child = cmd.spawn().unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin_bytes);
        child.wait_with_output().unwrap()
    };

    let huge_stdin = vec![b' '; runner_core::template::MAX_TEMPLATE_BYTES + 1024];
    let huge_stdin_out = run_with_stdin(&["crew", "import", "-"], &huge_stdin);
    assert_eq!(huge_stdin_out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&huge_stdin_out.stderr).contains("256 KiB"),
        "stderr: {}",
        String::from_utf8_lossy(&huge_stdin_out.stderr)
    );

    let ok_stdin_out = run_with_stdin(
        &["role", "import", "-", "--dry-run", "--json"],
        role_toml.as_bytes(),
    );
    assert!(
        ok_stdin_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&ok_stdin_out.stderr)
    );
}

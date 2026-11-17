use runner_core::app_paths::{normalize_checkout_path, same_checkout_path};
use runner_core::daemon_process::{self, Launch, NativePaths};
use runner_core::protocol::socket::{ConnectError, SocketTransport};
use runner_core::protocol::wire::Hello;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct TestDaemon {
    root: tempfile::TempDir,
    launch: Launch,
    exe: PathBuf,
    path_env: String,
    child: Option<Child>,
}

impl TestDaemon {
    fn start() -> Self {
        #[cfg(unix)]
        let root = tempfile::Builder::new()
            .prefix("1213-m1-")
            .tempdir_in("/tmp")
            .unwrap();
        #[cfg(windows)]
        let root = tempfile::Builder::new()
            .prefix("1213-m1-")
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

        // Provide a minimal `codex` stub in PATH so `chat start` and `mission start` spawn
        // real PTY processes against our git worktrees without invoking an external LLM CLI.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stub = bin.join("codex");
            std::fs::write(&stub, "#!/bin/sh\nsleep 30\n").unwrap();
            let mut perms = std::fs::metadata(&stub).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&stub, perms).unwrap();
        }
        #[cfg(windows)]
        {
            let stub = bin.join("codex.cmd");
            std::fs::write(&stub, "@echo off\r\nping -n 31 127.0.0.1 >nul\r\n").unwrap();
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
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runnerd-cli-wt-{key}")));
            launch.mcp_endpoint =
                IpcEndpoint(PathBuf::from(format!(r"\\.\pipe\runner-mcp-cli-wt-{key}")));
        }
        let host_path = std::env::var("PATH").unwrap_or_default();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path_env = format!("{}{sep}{host_path}", bin.display());

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
                    client: "worktree-cli-test".into(),
                },
            ) {
                Ok(_) => break,
                Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!(
                    "daemon failed to start: {error}; stderr={}",
                    std::fs::read_to_string(root_canon.join("daemon-stderr.log"))
                        .unwrap_or_default()
                ),
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

    fn cli(&self, cwd: &Path, args: &[&str]) -> std::process::Output {
        let home = self.launch.paths.home_dir.as_ref().unwrap();
        let mut cmd = Command::new(&self.exe);
        cmd.current_dir(cwd)
            .args(args)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("APPDATA", home.join("AppData/Roaming"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("PATH", &self.path_env)
            .env("NO_COLOR", "1")
            .env("SSH_CONNECTION", "test")
            .env(
                "RUNNER_TEST_DAEMON_ENDPOINT",
                &self.launch.daemon_endpoint.0,
            )
            .env("RUNNER_TEST_MCP_ENDPOINT", &self.launch.mcp_endpoint.0)
            .stdin(Stdio::null());
        for var in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
            "RUNNER_SESSION_ID",
            "RUNNER_SESSION_CLI",
        ] {
            cmd.env_remove(var);
        }
        cmd.output().unwrap()
    }

    fn cli_ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.cli(cwd, args);
        assert!(
            out.status.success(),
            "runner {args:?} failed (code={:?}):\nstdout: {}\nstderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Ok(client) = SocketTransport::connect(
            &self.launch.daemon_endpoint,
            Hello {
                exe_sha256: String::new(),
                client: "worktree-cli-cleanup".into(),
            },
        ) {
            let _ = client.shutdown(true);
            let _ = daemon_process::wait_unlocked(
                &self.launch.paths.app_data_dir,
                Duration::from_secs(10),
            );
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn worktree_subcommands_and_start_flags_work_end_to_end() {
    let daemon = TestDaemon::start();
    let repo = daemon.root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("README.md"), "init\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    // Add a non-Runner external linked worktree to verify list distinguishes managed vs `-`.
    let external = daemon.root.path().join("external-wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/external",
            &external.to_string_lossy(),
            "main",
        ],
    );

    daemon.cli_ok(&repo, &["project", "create", "Repo"]);
    daemon.cli_ok(&repo, &["role", "create", "coder", "--runtime", "codex"]);
    daemon.cli_ok(&repo, &["crew", "create", "Trio"]);
    daemon.cli_ok(&repo, &["crew", "add", "Trio", "coder", "--as", "coder"]);

    // 1. `runner worktree create` prints path and retry is idempotent.
    let created_path = daemon
        .cli_ok(
            &repo,
            &["worktree", "create", "feat/cli-wt", "--project", "Repo"],
        )
        .trim()
        .to_owned();
    let expected_wt = normalize_checkout_path(&repo)
        .join(".worktrees")
        .join("feat-cli-wt");
    assert!(
        same_checkout_path(Path::new(&created_path), &expected_wt),
        "created_path={created_path:?} expected_wt={expected_wt:?}"
    );
    assert!(expected_wt.is_dir());

    let retry_json: Value = serde_json::from_str(&daemon.cli_ok(
        &repo,
        &[
            "worktree",
            "create",
            "feat/cli-wt",
            "--project",
            "Repo",
            "--json",
        ],
    ))
    .unwrap();
    assert_eq!(retry_json["created"], false);
    assert!(same_checkout_path(
        Path::new(retry_json["path"].as_str().unwrap()),
        &expected_wt
    ));

    // 2. `runner worktree list` text & `--json` distinguish managed vs manual/external.
    let table_out = daemon.cli_ok(&repo, &["worktree", "list", "--project", "Repo"]);
    assert!(table_out.contains("feat/cli-wt"));
    assert!(table_out.contains("managed"));
    assert!(table_out.contains("feat/external"));
    let list_json: Value = serde_json::from_str(
        &daemon.cli_ok(&repo, &["worktree", "list", "--project", "Repo", "--json"]),
    )
    .unwrap();
    let items = list_json["worktrees"].as_array().unwrap();
    assert!(items
        .iter()
        .any(|w| w["branch"] == "feat/cli-wt" && w["managed"] == true));
    assert!(items
        .iter()
        .any(|w| w["branch"] == "feat/external" && w["managed"] == false));

    // 3. `runner worktree remove` refuses active worktrees, succeeds once free, and rejects outside.
    let fail_ext = daemon.cli(&repo, &["worktree", "remove", "feat/external"]);
    assert!(!fail_ext.status.success());

    let chat_json: Value = serde_json::from_str(&daemon.cli_ok(
        &repo,
        &[
            "chat",
            "start",
            "--runtime",
            "codex",
            "--project",
            "Repo",
            "--worktree",
            "feat/cli-wt",
            "--json",
        ],
    ))
    .unwrap();
    let chat_id = chat_json["id"].as_str().unwrap();
    assert!(same_checkout_path(
        Path::new(chat_json["cwd"].as_str().unwrap()),
        &expected_wt
    ));
    assert!(!daemon
        .cli(&repo, &["worktree", "remove", "feat/cli-wt"])
        .status
        .success());
    daemon.cli_ok(&repo, &["session", "archive", chat_id]);
    let rm_json: Value = serde_json::from_str(
        &daemon.cli_ok(&repo, &["worktree", "remove", "feat/cli-wt", "--json"]),
    )
    .unwrap();
    assert!(same_checkout_path(
        Path::new(rm_json["path"].as_str().unwrap()),
        &expected_wt
    ));
    assert!(!expected_wt.exists());

    // 4. `runner mission start --worktree --base` creates worktree, infers project, retries idempotently.
    let mission_json: Value = serde_json::from_str(&daemon.cli_ok(
        &repo,
        &[
            "mission",
            "start",
            "--crew",
            "Trio",
            "--title",
            "Worktree mission",
            "--goal",
            "Run in wt",
            "--worktree",
            "feat/cli-mission",
            "--base",
            "main",
            "--json",
        ],
    ))
    .unwrap();
    let mission_id = mission_json["mission"]["id"].as_str().unwrap();
    let expected_m_wt = normalize_checkout_path(&repo)
        .join(".worktrees")
        .join("feat-cli-mission");
    assert!(expected_m_wt.is_dir());
    assert!(same_checkout_path(
        Path::new(mission_json["mission"]["cwd"].as_str().unwrap()),
        &expected_m_wt
    ));
    assert!(!mission_json["mission"]["project_id"].is_null());

    // Retrying with the same `--worktree` reuses the worktree without failing.
    let retry_mission_json: Value = serde_json::from_str(&daemon.cli_ok(
        &repo,
        &[
            "mission",
            "start",
            "--crew",
            "Trio",
            "--title",
            "Second mission in same wt",
            "--goal",
            "Reuse wt",
            "--worktree",
            "feat/cli-mission",
            "--json",
        ],
    ))
    .unwrap();
    let retry_mission_id = retry_mission_json["mission"]["id"].as_str().unwrap();
    assert!(same_checkout_path(
        Path::new(retry_mission_json["mission"]["cwd"].as_str().unwrap()),
        &expected_m_wt
    ));

    daemon.cli_ok(&repo, &["mission", "stop", mission_id]);
    daemon.cli_ok(&repo, &["mission", "stop", retry_mission_id]);
    daemon.cli_ok(&repo, &["mission", "archive", mission_id]);
    daemon.cli_ok(&repo, &["mission", "archive", retry_mission_id]);
    daemon.cli_ok(&repo, &["worktree", "remove", "feat/cli-mission"]);
    assert!(!expected_m_wt.exists());

    // `--base` without `--worktree` fails with exit 2.
    let bad_cli = daemon.cli(
        &repo,
        &["mission", "start", "--title", "Bad", "--base", "main"],
    );
    assert_eq!(bad_cli.status.code(), Some(2));
}

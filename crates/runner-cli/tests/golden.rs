use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use runner_core::app_paths::IpcEndpoint;
use runner_core::protocol::socket::{ConnectError, SocketTransport};
use runner_core::protocol::wire::Hello;

const CREW: &str = "crew-fixture";
const MISSION: &str = "mission-fixture";
const SESSION: &str = "session-fixture";
const ARCHIVED: &str = "01K5ZCJNP00000000000000010";
const EVENTS: [&str; 3] = [
    "01K5ZCJNP00000000000000001",
    "01K5ZCJNP00000000000000002",
    "01K5ZCJNP00000000000000003",
];
const STAMP: &str = "2026-09-18T10:20:30Z";
const HEADER: &str = "# Built runner CLI, isolated runnerd, fixed SQLite and event-log fixtures.\n# Each block records argv, exit status, stdout, then non-empty stderr after [stderr].\n# Only unfixed values are normalized: temporary paths (<ROOT>, <HOME>, <APP_DATA>,\n# <SIDECAR>, including platform-native and truncated paths), generated ULIDs (<ULID_n>),\n# generated RFC3339 times (<TIMESTAMP>), daemon PID (<PID>) and build hash (<EXE_SHA256>).\n# Fixture IDs, timestamps, event offsets, whitespace and JSON ordering stay verbatim.\n# Start and resume use a missing executable or unusable cwd. No agents run.\n# Windows status has its own block because command installation is unsupported in debug.\n\n";

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    home: PathBuf,
    data: PathBuf,
    cli: PathBuf,
    daemon: PathBuf,
    endpoint: IpcEndpoint,
    mcp: IpcEndpoint,
    log: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        #[cfg(unix)]
        let root = tempfile::Builder::new()
            .prefix("821-")
            .tempdir_in("/tmp")
            .unwrap();
        #[cfg(windows)]
        let root = tempfile::Builder::new()
            .prefix("821-")
            .tempdir_in(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("Temp"))
            .unwrap();
        let directory = root.path().canonicalize().unwrap();
        let home = directory.join("home");
        #[cfg(target_os = "macos")]
        let data = home.join("Library/Application Support/com.wycstudios.runner-dev");
        #[cfg(all(unix, not(target_os = "macos")))]
        let data = home.join(".local/share/com.wycstudios.runner-dev");
        #[cfg(windows)]
        let data = home.join("AppData/Roaming/com.wycstudios.runner-dev");
        let bin = data.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cli = bin.join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
        let daemon = bin.join(runner_core::cli_install::DAEMON_DEST_BIN_NAME);
        let source = Path::new(env!("CARGO_BIN_EXE_runner-agent-cli"));
        for destination in [&cli, &daemon] {
            std::fs::hard_link(source, destination)
                .or_else(|_| std::fs::copy(source, destination).map(|_| ()))
                .unwrap();
        }
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            let companion = source.parent().unwrap().join(name);
            if companion.exists() {
                std::fs::copy(companion, bin.join(name)).unwrap();
            }
        }
        #[cfg(unix)]
        let (endpoint, mcp) = (
            IpcEndpoint(data.join("runnerd.sock")),
            IpcEndpoint(data.join("mcp.sock")),
        );
        #[cfg(windows)]
        let (endpoint, mcp) = {
            let key = root.path().file_name().unwrap().to_string_lossy();
            (
                IpcEndpoint(format!(r"\\.\pipe\runnerd-golden-{key}").into()),
                IpcEndpoint(format!(r"\\.\pipe\runner-mcp-golden-{key}").into()),
            )
        };
        let log = data
            .join("crews")
            .join(CREW)
            .join("missions")
            .join(MISSION)
            .join("events.ndjson");
        let fixture = Self {
            _root: root,
            directory,
            home,
            data,
            cli,
            daemon,
            endpoint,
            mcp,
            log,
            child: None,
        };
        fixture.seed();
        fixture
    }

    fn seed(&self) {
        std::fs::write(self.directory.as_path().join("not-a-directory"), b"fixture").unwrap();
        let pool = runner_daemon::db::open_pool(&self.data.join("runner.db")).unwrap();
        let conn = pool.get().unwrap();
        conn.execute_batch("DELETE FROM sessions; DELETE FROM missions; DELETE FROM slots; DELETE FROM crews; DELETE FROM roles; DELETE FROM nodes; DELETE FROM projects;").unwrap();
        conn.execute(
            "INSERT INTO projects (id,name,cwd,position,created_at) VALUES ('project-fixture','Runner',?1,0,?2)",
            [self.directory.as_path().to_str().unwrap(), STAMP],
        ).unwrap();
        for (id, handle, name) in [
            ("role-coder", "coder", "Coder"),
            ("role-reviewer", "reviewer", "Reviewer"),
            ("role-unused", "unused", "Unused"),
        ] {
            conn.execute(
                "INSERT INTO roles (id,handle,display_name,runtime,command,args_json,env_json,system_prompt,model,effort,runtime_options_json,created_at,updated_at) VALUES (?1,?2,?3,'codex','missing-fixture-agent','[]','{}','Fixture prompt','fixture-model','high','{\"codex\":{\"speed\":\"fast\"}}',?4,?4)",
                [id,handle,name,STAMP],
            ).unwrap();
        }
        conn.execute("INSERT INTO crews (id,name,system_prompt_addendum,created_at,updated_at) VALUES (?1,'Peer','Fixture crew conventions',?2,?2)", [CREW,STAMP]).unwrap();
        conn.execute(
            "INSERT INTO crews (id,name,created_at,updated_at) VALUES ('crew-empty','Empty',?1,?1)",
            [STAMP],
        )
        .unwrap();
        for (id, role, handle, position, lead) in [
            ("slot-coder", "role-coder", "coder", 0, 1),
            ("slot-reviewer", "role-reviewer", "reviewer", 1, 0),
        ] {
            conn.execute("INSERT INTO slots (id,crew_id,role_id,slot_handle,position,lead,added_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", (id,CREW,role,handle,position,lead,STAMP)).unwrap();
        }
        conn.execute("INSERT INTO missions (id,crew_id,project_id,title,status,goal_override,cwd,started_at,stopped_at) VALUES (?1,?2,'project-fixture','Fixture mission','completed','Fixture goal',?3,?4,?4)", [MISSION,CREW,self.directory.as_path().to_str().unwrap(),STAMP]).unwrap();
        conn.execute("INSERT INTO missions (id,crew_id,title,status,cwd,started_at) VALUES ('mission-running',?1,'Active fixture','running',?2,?3)", [CREW,self.directory.as_path().to_str().unwrap(),STAMP]).unwrap();
        conn.execute("INSERT INTO missions (id,crew_id,title,status,started_at,stopped_at,archived_at) VALUES (?1,?2,'Archived mission','completed',?3,?3,?3)", [ARCHIVED,CREW,STAMP]).unwrap();
        conn.execute("INSERT INTO sessions (id,role_id,project_id,cwd,status,started_at,stopped_at,title,agent_runtime,agent_command) VALUES (?1,'role-coder','project-fixture',?2,'stopped',?3,?3,'Fixture chat','codex','missing-fixture-agent')", [SESSION,self.directory.join("not-a-directory").to_str().unwrap(),STAMP]).unwrap();
        conn.execute("INSERT INTO sessions (id,mission_id,role_id,slot_id,cwd,status,started_at,stopped_at,agent_runtime,agent_command) VALUES ('session-mission',?1,'role-coder','slot-coder',?2,'stopped',?3,?3,'codex','missing-fixture-agent')", [MISSION,self.directory.as_path().to_str().unwrap(),STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-project',NULL,0,'project','project-fixture',?1)", [STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-mission','node-project',0,'mission',?1,?2)", [MISSION,STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-active',NULL,1,'mission','mission-running',?1)", [STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,name,layout,created_at) VALUES ('node-chat','node-project',1,'tab','Fixture chat',?1,?2)", [r#"{"preset":"single","slots":["session-fixture"],"sizes":{}}"#,STAMP]).unwrap();
        std::fs::create_dir_all(self.log.parent().unwrap()).unwrap();
        std::fs::write(
            self.log.parent().unwrap().join("roster.json"),
            r#"[{"handle":"coder","lead":true},{"handle":"reviewer","lead":false}]"#,
        )
        .unwrap();
        let events = [
            serde_json::json!({"id":EVENTS[0],"ts":STAMP,"crew_id":CREW,"mission_id":MISSION,"kind":"message","from":"reviewer","to":"coder","payload":{"text":"Ready for review\nChecks passed"}}),
            serde_json::json!({"id":EVENTS[1],"ts":STAMP,"crew_id":CREW,"mission_id":MISSION,"kind":"signal","from":"coder","type":"human_question","payload":{"prompt":"Ship?","choices":["yes","no"],"on_behalf_of":"coder"}}),
            serde_json::json!({"id":EVENTS[2],"ts":STAMP,"crew_id":CREW,"mission_id":MISSION,"kind":"signal","from":"coder","type":"session_status","payload":{"state":"idle","source":"fixture"}}),
        ];
        let bytes = events
            .iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>();
        std::fs::write(&self.log, bytes).unwrap();
        let active = self
            .log
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("mission-running");
        std::fs::create_dir_all(&active).unwrap();
        let question = serde_json::json!({"id":EVENTS[1],"ts":STAMP,"crew_id":CREW,"mission_id":"mission-running","kind":"signal","from":"human","type":"human_question","payload":{"prompt":"Ship?","choices":["yes","no"],"on_behalf_of":"coder"}});
        std::fs::write(active.join("events.ndjson"), format!("{question}\n")).unwrap();
        std::fs::write(self.data.join("ui-settings.json"), r#"{"resumeOnLaunch":false,"disabledAgents":["codex","claude-code","antigravity","pi","copilot","trae"]}"#).unwrap();
    }

    fn start(&mut self) {
        let stderr = std::fs::File::create(self.directory.as_path().join("daemon-stderr")).unwrap();
        let mut cmd = Command::new(&self.daemon);
        cmd.args(["--app-data-dir"])
            .arg(&self.data)
            .arg("--log-dir")
            .arg(self.directory.as_path().join("logs"))
            .arg("--home-dir")
            .arg(&self.home)
            .arg("--endpoint")
            .arg(&self.endpoint.0)
            .arg("--mcp-endpoint")
            .arg(&self.mcp.0)
            .arg("--isolated")
            .env("PATH", self.data.join("bin"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        #[cfg(unix)]
        cmd.env("SHELL", "/bin/sh");
        self.child = Some(cmd.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match SocketTransport::connect(&self.endpoint, hello()) {
                Ok(_) => break,
                Err(ConnectError::NotRunning) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!(
                    "golden daemon: {error}; {}",
                    std::fs::read_to_string(self.directory.as_path().join("daemon-stderr"))
                        .unwrap()
                ),
            }
        }
    }

    fn command(&self, inside: bool) -> Command {
        let mut cmd = Command::new(&self.cli);
        cmd.current_dir(self.directory.as_path())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", self.home.join("AppData/Roaming"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("PATH", self.data.join("bin"))
            .env("NO_COLOR", "1")
            .env("SSH_CONNECTION", "fixture: never autostart")
            .env("RUNNER_TEST_MCP_ENDPOINT", &self.mcp.0)
            .env("RUNNER_TEST_DAEMON_ENDPOINT", &self.endpoint.0)
            .stdin(Stdio::null());
        for name in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
        ] {
            cmd.env_remove(name);
        }
        if inside {
            cmd.env("RUNNER_CREW_ID", CREW)
                .env("RUNNER_MISSION_ID", MISSION)
                .env("RUNNER_HANDLE", "coder")
                .env("RUNNER_EVENT_LOG", &self.log);
        }
        cmd
    }

    fn normalize(&self, text: &str) -> String {
        let mut text = text.to_owned();
        let paths = [
            (
                self.directory.join("not-a-directory"),
                "<ROOT>/not-a-directory".to_owned(),
            ),
            (
                self.home.join(".claude/skills").join("runner-dev"),
                "<HOME>/.claude/skills/runner-dev".to_owned(),
            ),
            (
                self.home.join(".agents/skills").join("runner-dev"),
                "<HOME>/.agents/skills/runner-dev".to_owned(),
            ),
            (
                self.home.join(".trae/skills").join("runner-dev"),
                "<HOME>/.trae/skills/runner-dev".to_owned(),
            ),
            (
                self.home
                    .join(".gemini/antigravity-cli/skills")
                    .join("runner-dev"),
                "<HOME>/.gemini/antigravity-cli/skills/runner-dev".to_owned(),
            ),
            (
                self.home.join(".claude/skills"),
                "<HOME>/.claude/skills".to_owned(),
            ),
            (
                self.home.join(".agents/skills"),
                "<HOME>/.agents/skills".to_owned(),
            ),
            (
                self.home.join(".trae/skills"),
                "<HOME>/.trae/skills".to_owned(),
            ),
            (
                self.home.join(".gemini/antigravity-cli/skills"),
                "<HOME>/.gemini/antigravity-cli/skills".to_owned(),
            ),
        ];
        for (path, name) in paths {
            text = replace_path(text, &path, &name);
        }
        let paths = [
            (&self.cli, "<SIDECAR>"),
            (&self.mcp.0, "<APP_DATA>/mcp.sock"),
            (&self.endpoint.0, "<APP_DATA>/runnerd.sock"),
            (&self.data, "<APP_DATA>"),
            (&self.home, "<HOME>"),
            (&self.directory.as_path().to_path_buf(), "<ROOT>"),
        ];
        for (path, name) in paths {
            text = replace_path(text, path, name);
        }
        let ulids = regex::Regex::new(r"\b[0-9A-HJKMNP-TV-Z]{26}\b").unwrap();
        let mut names = BTreeMap::new();
        text = ulids
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                if EVENTS.contains(&&caps[0]) || caps[0] == *ARCHIVED {
                    return caps[0].to_owned();
                }
                let next = names.len() + 1;
                names
                    .entry(caps[0].to_owned())
                    .or_insert_with(|| format!("<ULID_{next}>"))
                    .clone()
            })
            .into_owned();
        let times =
            regex::Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|\+00:00)")
                .unwrap();
        text = times
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                if caps[0] == *STAMP || caps[0] == *"2026-09-18T10:20:30+00:00" {
                    caps[0].to_owned()
                } else {
                    "<TIMESTAMP>".into()
                }
            })
            .into_owned();
        let pid = regex::Regex::new(r#"(?m)("pid":\s*|^pid\s+)\d+"#).unwrap();
        text = pid.replace_all(&text, "${1}<PID>").into_owned();
        let hash =
            regex::Regex::new(r#"(?m)("exe_sha256":\s*"|^exe_sha256\s+)[a-f0-9]{64}"#).unwrap();
        hash.replace_all(&text, "${1}<EXE_SHA256>").into_owned()
    }
}

fn replace_path(mut text: String, path: &Path, name: &str) -> String {
    let raw = path.to_str().unwrap();
    for raw in [raw, raw.strip_prefix(r"\\?\").unwrap_or(raw)] {
        let escaped = serde_json::to_string(raw).unwrap();
        text = text
            .replace(&escaped[1..escaped.len() - 1], name)
            .replace(raw, name);
        for width in [48, 96] {
            if raw.chars().count() > width {
                let truncated = raw
                    .chars()
                    .take(width - 1)
                    .chain(std::iter::once('…'))
                    .collect::<String>();
                text = text.replace(&truncated, name);
            }
        }
    }
    text
}

fn hello() -> Hello {
    Hello {
        exe_sha256: String::new(),
        client: "golden".into(),
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.child.is_some() {
            if let Ok(client) = SocketTransport::connect(&self.endpoint, hello()) {
                let _ = client.shutdown(true);
                let _ =
                    runner_core::daemon_process::wait_unlocked(&self.data, Duration::from_secs(10));
            }
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn cases() -> Vec<Vec<&'static str>> {
    vec![
        vec!["status"],
        vec!["daemon", "status"],
        vec!["daemon", "stop"],
        vec!["project", "list"],
        vec!["project", "show", "Runner"],
        vec!["project", "create", "New", "--path", "."],
        vec!["project", "rename", "Runner", "Renamed"],
        vec!["project", "delete", "Runner", "--force"],
        vec!["role", "list"],
        vec!["role", "show", "coder"],
        vec!["role", "create", "new", "--runtime", "codex"],
        vec![
            "role", "update", "coder", "--model", "updated", "--speed", "standard",
        ],
        vec!["role", "delete", "unused"],
        vec!["crew", "list"],
        vec!["crew", "show", "Peer"],
        vec!["crew", "create", "New"],
        vec!["crew", "update", "Peer", "--name", "Renamed"],
        vec!["crew", "delete", "Empty"],
        vec!["crew", "add", "Peer", "unused", "--as", "impl"],
        vec!["crew", "set", "Peer", "reviewer", "--effort", "medium"],
        vec!["crew", "remove", "Peer", "reviewer"],
        vec!["crew", "lead", "Peer", "reviewer"],
        vec!["crew", "order", "Peer", "reviewer", "coder"],
        vec!["mission", "list", "--crew", "Peer"],
        vec!["mission", "show", MISSION],
        vec![
            "mission",
            "start",
            "--crew",
            "Peer",
            "--goal",
            "ship",
            "--cwd",
            "not-a-directory",
        ],
        vec!["mission", "stop", MISSION],
        vec!["mission", "resume", MISSION],
        vec!["mission", "archive", MISSION],
        vec!["mission", "unarchive", ARCHIVED],
        vec!["mission", "rename", MISSION, "Renamed"],
        vec!["mission", "pin", MISSION],
        vec!["mission", "unpin", MISSION],
        vec!["mission", "move", MISSION, "--unfile"],
        vec!["mission", "feed", MISSION, "--oldest-first"],
        vec!["mission", "answer", MISSION, EVENTS[1], "yes"],
        vec!["chat", "start", "coder", "--cwd", "not-a-directory"],
        vec![
            "chat",
            "start",
            "--runtime",
            "codex",
            "--cwd",
            "not-a-directory",
        ],
        vec!["session", "list"],
        vec!["session", "show", SESSION],
        vec!["session", "stop", SESSION],
        vec!["session", "archive", SESSION],
        vec!["session", "resume", SESSION],
        vec!["session", "restart", SESSION],
        vec![
            "msg",
            "post",
            "Hello",
            "--mission",
            MISSION,
            "--to",
            "reviewer",
        ],
        vec!["msg", "read"],
        vec![
            "signal",
            "ask_lead",
            "--mission",
            MISSION,
            "--as",
            "coder",
            "--payload",
            r#"{"question":"Review?"}"#,
        ],
        vec![
            "ask",
            "Review?",
            "--context",
            "Checks pass",
            "--mission",
            MISSION,
            "--as",
            "coder",
        ],
        vec![
            "ask",
            "--human",
            "Ship?",
            "--choices",
            "yes,no",
            "--mission",
            MISSION,
            "--as",
            "coder",
        ],
        vec!["call", "role_get", r#"{"id":"role-coder"}"#],
        vec!["msg", "post", "Hello", "--mission", "mission-running"],
        vec![
            "signal",
            "ask_lead",
            "--mission",
            "mission-running",
            "--as",
            "coder",
            "--payload",
            r#"{"question":"Review?"}"#,
        ],
        vec![
            "ask",
            "Review?",
            "--mission",
            "mission-running",
            "--as",
            "coder",
        ],
        vec![
            "ask",
            "--human",
            "Ship?",
            "--choices",
            "yes,no",
            "--mission",
            "mission-running",
            "--as",
            "coder",
        ],
        vec!["mission", "answer", "mission-running", EVENTS[1], "yes"],
        vec!["project", "show", "missing"],
        vec!["role", "show", "missing"],
        vec!["crew", "show", "missing"],
        vec!["mission", "show", "missing"],
        vec!["session", "show", "missing"],
        vec![
            "msg",
            "post",
            "Hello",
            "--mission",
            "mission-running",
            "--as",
            "ghost",
        ],
        vec!["signal", "ask_lead", "--mission", MISSION, "--as", "ghost"],
        vec!["mission", "feed", MISSION, "--since", "0", "--limit", "1"],
        vec![
            "mission", "feed", MISSION, "--types", "message", "--from", "reviewer",
        ],
        vec!["mission", "move", MISSION, "--project", "Runner"],
        vec!["role", "create", "coder"],
        vec!["role", "create", "new", "--runtime", "shell"],
        vec!["mission", "move", MISSION],
        vec!["signal", "unknown", "--mission", MISSION],
    ]
}

fn quote(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_/.:".contains(c))
    {
        arg.into()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

fn record(result: &mut String, fixture: &Fixture, args: &[&str], inside: bool, running: bool) {
    let mut child = fixture
        .command(inside)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "CLI exceeded 10 seconds: {args:?}; stderr: {}",
                String::from_utf8_lossy(&err.join().unwrap())
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = std::process::Output {
        status: child.wait().unwrap(),
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    };
    let context = if inside {
        "# context: inside mission\n"
    } else if !running {
        "# context: not running\n"
    } else {
        ""
    };
    result.push_str(&format!(
        "{context}$ runner {}\nexit {}\n",
        args.iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
        output.status.code().expect("CLI exited by signal")
    ));
    result.push_str(&fixture.normalize(std::str::from_utf8(&output.stdout).unwrap()));
    if !output.stderr.is_empty() {
        result.push_str("[stderr]\n");
        result.push_str(&fixture.normalize(std::str::from_utf8(&output.stderr).unwrap()));
    }
    result.push('\n');
}

#[test]
fn built_cli_goldens() {
    let platform = if cfg!(windows) { "windows" } else { "unix" };
    let mut result = format!("{HEADER}# platform {platform}\n");
    for args in [vec!["status"], vec!["status", "--json"]] {
        let mut fixture = Fixture::new();
        fixture.start();
        record(&mut result, &fixture, &args, false, true);
    }
    result.push_str("# platform all\n");
    for args in cases().into_iter().filter(|args| args[0] != "status") {
        for json in [false, true] {
            let mut fixture = Fixture::new();
            fixture.start();
            let mut args = args.clone();
            if json {
                args.push("--json");
            }
            record(&mut result, &fixture, &args, false, true);
        }
    }
    for args in [
        vec!["msg", "read"],
        vec!["msg", "post", "Direct message", "--to", "reviewer"],
        vec![
            "signal",
            "ask_lead",
            "--payload",
            r#"{"question":"Review?"}"#,
        ],
        vec!["ask", "Review?"],
        vec!["ask", "--human", "Ship?", "--choices", "yes,no"],
    ] {
        for json in [false, true] {
            let fixture = Fixture::new();
            let mut args = args.clone();
            if json {
                args.push("--json");
            }
            record(&mut result, &fixture, &args, true, false);
        }
    }
    for args in [
        vec!["status"],
        vec!["status", "--json"],
        vec!["role", "list"],
        vec!["role", "list", "--json"],
        vec!["daemon", "status"],
        vec!["daemon", "stop"],
        vec!["unknown-command"],
    ] {
        let fixture = Fixture::new();
        record(&mut result, &fixture, &args, false, false);
    }
    let mut help_cases = vec![
        vec!["help"],
        vec!["help", "agents"],
        vec!["help", "mission"],
    ];
    for args in cases().into_iter().take(50) {
        let count = if matches!(args.first(), Some(&"status" | &"signal" | &"ask" | &"call")) {
            1
        } else {
            2
        };
        let mut help = args.into_iter().take(count).collect::<Vec<_>>();
        help.push("--help");
        if !help_cases.contains(&help) {
            help_cases.push(help);
        }
    }
    for args in help_cases {
        let fixture = Fixture::new();
        record(&mut result, &fixture, &args, false, false);
    }
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/cli.txt");
    if std::env::var_os("UPDATE_RUNNER_GOLDENS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        let previous = std::fs::read_to_string(&golden).unwrap_or_default();
        let other = if cfg!(windows) { "unix" } else { "windows" };
        let marker = format!("# platform {other}\n");
        let other_section = previous
            .split_once(&marker)
            .map(|(_, rest)| rest.split("# platform ").next().unwrap())
            .unwrap_or("");
        let updated = result.replacen(
            "# platform all\n",
            &format!("{marker}{other_section}# platform all\n"),
            1,
        );
        std::fs::write(&golden, &updated).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap();
    if select_platform(&result, platform) != select_platform(&expected, platform) {
        let actual =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/cli-golden-actual.txt");
        std::fs::write(&actual, &result).unwrap();
        panic!(
            "CLI output changed; diff {} {}",
            golden.display(),
            actual.display()
        );
    }
}

fn select_platform(text: &str, platform: &str) -> String {
    let mut include = true;
    text.split_inclusive('\n')
        .filter(|line| {
            if let Some(selected) = line.strip_prefix("# platform ") {
                include = selected.trim() == "all" || selected.trim() == platform;
                false
            } else {
                include
            }
        })
        .collect()
}

#[test]
fn normalization_preserves_fixed_offsets_and_non_build_hashes() {
    let fixture = Fixture::new();
    let hash = "a".repeat(64);
    let original =
        format!(r#"{{"pid":692,"next_offset":692,"exe_sha256":"{hash}","sha256":"{hash}"}}"#);
    assert_eq!(
        fixture.normalize(&original),
        format!(
            r#"{{"pid":<PID>,"next_offset":692,"exe_sha256":"<EXE_SHA256>","sha256":"{hash}"}}"#
        )
    );
    assert_eq!(
        fixture.normalize(&format!(
            "pid         692\nexe_sha256  {hash}\nnext_offset  692\n"
        )),
        "pid         <PID>\nexe_sha256  <EXE_SHA256>\nnext_offset  692\n"
    );
}

#[test]
fn built_follower_receives_appends_and_reports_disconnect_with_its_last_cursor() {
    use runner_core::event_log::EventLog;
    use runner_core::model::{EventDraft, SignalType};
    use std::io::BufRead;

    struct Follower(Child);
    impl Drop for Follower {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut fixture = Fixture::new();
    fixture.start();
    let mut follower = Follower(
        fixture
            .command(false)
            .args([
                "mission",
                "feed",
                "mission-running",
                "--follow",
                "--json",
                "--oldest-first",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = follower.0.stdout.take().unwrap();
    let (sender, lines) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    let first: serde_json::Value = serde_json::from_str(
        &lines
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["id"], EVENTS[1]);
    let mission_dir = fixture
        .data
        .join("crews")
        .join(CREW)
        .join("missions/mission-running");
    let log = EventLog::open(&mission_dir).unwrap();
    let appended = log
        .append(EventDraft::signal(
            CREW,
            "mission-running",
            "human",
            SignalType::new("fixture_event"),
            serde_json::json!({"text": "pushed fixture message"}),
        ))
        .unwrap();
    let second: serde_json::Value =
        serde_json::from_str(&lines.recv_timeout(Duration::from_secs(5)).unwrap().unwrap())
            .unwrap();
    assert_eq!(second["id"], appended.id);
    assert_eq!(second["payload"]["text"], "pushed fixture message");
    let cursor = log
        .read_from_lossy(0)
        .unwrap()
        .0
        .iter()
        .find(|entry| entry.event.id == appended.id)
        .unwrap()
        .next_offset;
    assert_eq!(second["next_offset"], cursor);
    assert!(cursor > first["next_offset"].as_u64().unwrap());
    fixture.child.as_mut().unwrap().kill().unwrap();
    fixture.child.as_mut().unwrap().wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = follower.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "follower did not report daemon disconnect"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(3));
    let mut stderr = String::new();
    follower
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        stderr.contains("watch ended: Runner is not running"),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("--since {cursor} --oldest-first --follow --json")),
        "{stderr}"
    );
    reader.join().unwrap();
    assert!(
        lines.try_iter().next().is_none(),
        "duplicate event after the initial drain"
    );
}

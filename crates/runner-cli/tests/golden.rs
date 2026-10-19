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
const HEADER: &str = "# Built runner CLI, isolated runnerd, fixed SQLite and event-log fixtures.\n# Each block records argv, exit status, stdout, then non-empty stderr after [stderr].\n# Only unfixed values are normalized: temporary paths (<ROOT>, <HOME>, <APP_DATA>,\n# <SIDECAR>, including platform-native and truncated paths), generated ULIDs (<ULID_n>),\n# generated RFC3339 times (<TIMESTAMP>), daemon PID (<PID>), build hash (<EXE_SHA256>) and crate version (<VERSION>).\n# Fixture IDs, timestamps, event offsets, whitespace and JSON ordering stay verbatim.\n# Start and resume use a missing executable or unusable cwd. No agents run.\n# Windows status has its own block because command installation is unsupported in debug.\n\n";

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
        let definition = runner_daemon::model::LaunchDefinition {
            claude_account: None,
            command: "missing-fixture-agent".into(),
            args: vec![],
            env: Default::default(),
            model: Some("fixture-model".into()),
            effort: Some("high".into()),
            codex_speed: Some(runner_daemon::model::CodexSpeed::Fast),
            system_prompt: Some("Fixture prompt".into()),
        };
        for id in [SESSION, "session-mission"] {
            runner_daemon::repo::session::set_launch(&conn, id, "codex", &definition).unwrap();
        }
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-project',NULL,0,'project','project-fixture',?1)", [STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-mission','node-project',0,'mission',?1,?2)", [MISSION,STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,ref_id,created_at) VALUES ('node-active',NULL,1,'mission','mission-running',?1)", [STAMP]).unwrap();
        conn.execute("INSERT INTO nodes (id,parent_id,position,type,name,layout,created_at) VALUES ('node-chat','node-project',1,'tab','Fixture chat',?1,?2)", [r#"{"preset":"single","slots":["session-fixture"],"sizes":{}}"#,STAMP]).unwrap();
        std::fs::create_dir_all(self.log.parent().unwrap()).unwrap();
        std::fs::write(
            self.log.parent().unwrap().join("roster.json"),
            r#"{"version":2,"crew_name":"Peer","crew_addendum":"Fixture crew conventions","slots":[{"id":"slot-coder","role_id":"role-coder","position":0,"handle":"coder","lead":true},{"id":"slot-reviewer","role_id":"role-reviewer","position":1,"handle":"reviewer","lead":false}]}"#,
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
        text = hash.replace_all(&text, "${1}<EXE_SHA256>").into_owned();
        let version = regex::Regex::new(&format!(
            r#"(?m)("cli_version":"|"app_version":"|^CLI VERSION\s+|^APP VERSION\s+){}"#,
            regex::escape(env!("CARGO_PKG_VERSION"))
        ))
        .unwrap();
        version.replace_all(&text, "${1}<VERSION>").into_owned()
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

macro_rules! cli_cases {
    ($($name:ident => $args:expr),+ $(,)?) => {
        fn cases() -> Vec<Vec<&'static str>> {
            vec![$($args),+]
        }

        mod built_cli_goldens {
            use super::*;

            $(#[test]
            fn $name() {
                check_cases(variants($args, false, true));
            })+
        }
    };
}

cli_cases! {
    status => vec!["status"],
    daemon_status => vec!["daemon", "status"],
    daemon_stop => vec!["daemon", "stop"],
    project_list => vec!["project", "list"],
    project_show => vec!["project", "show", "Runner"],
    project_create => vec!["project", "create", "New", "--path", "."],
    project_rename => vec!["project", "rename", "Runner", "Renamed"],
    project_delete => vec!["project", "delete", "Runner", "--force"],
    role_list => vec!["role", "list"],
    role_show => vec!["role", "show", "coder"],
    role_create => vec!["role", "create", "new", "--runtime", "codex"],
    role_update => vec!["role", "update", "coder", "--model", "updated", "--speed", "standard"],
    role_delete => vec!["role", "delete", "unused"],
    crew_list => vec!["crew", "list"],
    crew_show => vec!["crew", "show", "Peer"],
    crew_create => vec!["crew", "create", "New"],
    crew_update => vec!["crew", "update", "Peer", "--name", "Renamed"],
    crew_delete => vec!["crew", "delete", "Empty"],
    crew_add => vec!["crew", "add", "Peer", "unused", "--as", "impl"],
    crew_set => vec!["crew", "set", "Peer", "reviewer", "--effort", "medium"],
    crew_remove => vec!["crew", "remove", "Peer", "reviewer"],
    crew_lead => vec!["crew", "lead", "Peer", "reviewer"],
    crew_order => vec!["crew", "order", "Peer", "reviewer", "coder"],
    mission_list => vec!["mission", "list", "--crew", "Peer"],
    mission_show => vec!["mission", "show", MISSION],
    mission_start => vec!["mission", "start", "--crew", "Peer", "--goal", "ship", "--cwd", "not-a-directory"],
    mission_start_empty => vec!["mission", "start", "--title", "Empty mission"],
    mission_start_goal_title => vec!["mission", "start", "--goal", "Shared goal"],
    mission_start_empty_untitled => vec!["mission", "start"],
    mission_add_role => vec!["mission", "add-role", "mission-running", "reviewer", "--handle", "reviewer-2", "--task", "Review the change"],
    mission_remove_role => vec!["mission", "remove-role", "mission-running", "reviewer"],
    mission_add_role_stopped => vec!["mission", "add-role", MISSION, "reviewer"],
    mission_remove_role_lead => vec!["mission", "remove-role", "mission-running", "coder"],
    mission_add_role_as => vec!["mission", "add-role", "mission-running", "reviewer", "--as", "coder"],
    mission_remove_role_as => vec!["mission", "remove-role", "mission-running", "reviewer", "--as", "coder"],
    mission_stop => vec!["mission", "stop", MISSION],
    mission_resume => vec!["mission", "resume", MISSION],
    mission_archive => vec!["mission", "archive", MISSION],
    mission_unarchive => vec!["mission", "unarchive", ARCHIVED],
    mission_rename => vec!["mission", "rename", MISSION, "Renamed"],
    mission_pin => vec!["mission", "pin", MISSION],
    mission_unpin => vec!["mission", "unpin", MISSION],
    mission_unfile => vec!["mission", "move", MISSION, "--unfile"],
    mission_feed => vec!["mission", "feed", MISSION, "--oldest-first"],
    mission_answer => vec!["mission", "answer", MISSION, EVENTS[1], "yes"],
    chat_start => vec!["chat", "start", "coder", "--cwd", "not-a-directory"],
    chat_start_runtime => vec!["chat", "start", "--runtime", "codex", "--cwd", "not-a-directory"],
    session_list => vec!["session", "list"],
    session_show => vec!["session", "show", SESSION],
    session_stop => vec!["session", "stop", SESSION],
    session_archive => vec!["session", "archive", SESSION],
    session_resume => vec!["session", "resume", SESSION],
    session_restart => vec!["session", "restart", SESSION],
    msg_post => vec!["msg", "post", "Hello", "--mission", MISSION, "--to", "reviewer"],
    msg_read => vec!["msg", "read"],
    signal_ask_lead => vec![
        "signal",
        "ask_lead",
        "--mission",
        MISSION,
        "--as",
        "coder",
        "--payload",
        r#"{"question":"Review?"}"#
    ],
    ask_lead => vec!["ask", "Review?", "--context", "Checks pass", "--mission", MISSION, "--as", "coder"],
    ask_human => vec![
        "ask",
        "--human",
        "Ship?",
        "--choices",
        "yes,no",
        "--mission",
        MISSION,
        "--as",
        "coder"
    ],
    call => vec!["call", "role_get", r#"{"id":"role-coder"}"#],
    msg_post_running => vec!["msg", "post", "Hello", "--mission", "mission-running"],
    signal_running => vec![
        "signal",
        "ask_lead",
        "--mission",
        "mission-running",
        "--as",
        "coder",
        "--payload",
        r#"{"question":"Review?"}"#
    ],
    ask_running => vec!["ask", "Review?", "--mission", "mission-running", "--as", "coder"],
    ask_human_running => vec![
        "ask",
        "--human",
        "Ship?",
        "--choices",
        "yes,no",
        "--mission",
        "mission-running",
        "--as",
        "coder"
    ],
    mission_answer_running => vec!["mission", "answer", "mission-running", EVENTS[1], "yes"],
    project_missing => vec!["project", "show", "missing"],
    role_missing => vec!["role", "show", "missing"],
    crew_missing => vec!["crew", "show", "missing"],
    mission_missing => vec!["mission", "show", "missing"],
    session_missing => vec!["session", "show", "missing"],
    msg_unknown_sender => vec!["msg", "post", "Hello", "--mission", "mission-running", "--as", "ghost"],
    signal_unknown_sender => vec!["signal", "ask_lead", "--mission", MISSION, "--as", "ghost"],
    mission_feed_cursor => vec!["mission", "feed", MISSION, "--since", "0", "--limit", "1"],
    mission_feed_filters => vec!["mission", "feed", MISSION, "--types", "message", "--from", "reviewer"],
    mission_move => vec!["mission", "move", MISSION, "--project", "Runner"],
    role_duplicate => vec!["role", "create", "coder"],
    role_shell => vec!["role", "create", "new", "--runtime", "shell"],
    mission_move_invalid => vec!["mission", "move", MISSION],
    signal_unknown => vec!["signal", "unknown", "--mission", MISSION],
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

#[derive(Debug, PartialEq)]
struct Case {
    args: Vec<&'static str>,
    inside: bool,
    running: bool,
}

fn variants(args: Vec<&'static str>, inside: bool, running: bool) -> Vec<Case> {
    [false, true]
        .into_iter()
        .map(|json| {
            let mut args = args.clone();
            if json {
                args.push("--json");
            }
            Case {
                args,
                inside,
                running,
            }
        })
        .collect()
}

fn inside_cases() -> Vec<Case> {
    let args = [
        vec!["mission", "add-role", "mission-running", "reviewer"],
        vec!["mission", "remove-role", "mission-running", "reviewer"],
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
    ];
    args.into_iter()
        .flat_map(|args| variants(args, true, false))
        .collect()
}

fn offline_cases() -> Vec<Case> {
    let args = [
        vec!["status"],
        vec!["status", "--json"],
        vec!["role", "list"],
        vec!["role", "list", "--json"],
        vec!["daemon", "status"],
        vec!["daemon", "stop"],
        vec!["unknown-command"],
    ];
    args.into_iter()
        .map(|args| Case {
            args,
            inside: false,
            running: false,
        })
        .collect()
}

fn help_cases() -> Vec<Case> {
    let mut help_cases = vec![
        vec!["help"],
        vec!["help", "agents"],
        vec!["help", "mission"],
        vec!["mission", "add-role", "--help"],
        vec!["mission", "remove-role", "--help"],
        vec!["--help"],
        vec!["daemon", "--help"],
        vec!["remote", "--help"],
        vec!["remote", "computers", "--help"],
        vec!["remote", "pair", "--help"],
        vec!["remote", "forget", "--help"],
        vec!["remote", "overview", "--help"],
        vec!["remote", "status", "--help"],
        vec!["remote", "login", "--help"],
        vec!["remote", "logout", "--help"],
        vec!["remote", "pin", "--help"],
        vec!["remote", "devices", "--help"],
        vec!["remote", "revoke", "--help"],
        vec!["remote", "reset-identity", "--help"],
        vec!["project", "--help"],
        vec!["role", "--help"],
        vec!["crew", "--help"],
        vec!["mission", "--help"],
        vec!["chat", "--help"],
        vec!["session", "--help"],
        vec!["msg", "--help"],
        vec!["help", "--help"],
    ];
    for args in cases()
        .into_iter()
        .filter(|args| !matches!(args.get(1), Some(&"add-role" | &"remove-role")))
    {
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
    help_cases
        .into_iter()
        .map(|args| Case {
            args,
            inside: false,
            running: false,
        })
        .collect()
}

fn golden_cases() -> Vec<Case> {
    cases()
        .into_iter()
        .flat_map(|args| variants(args, false, true))
        .chain(inside_cases())
        .chain(offline_cases())
        .chain(help_cases())
        .collect()
}

fn heading(case: &Case) -> String {
    let context = if case.inside {
        "# context: inside mission\n"
    } else if !case.running {
        "# context: not running\n"
    } else {
        ""
    };
    format!(
        "{context}$ runner {}",
        case.args
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn blocks(text: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut offset = 0;
    let mut context = None;
    for line in text.split_inclusive('\n') {
        if line.starts_with("# context: ") {
            context = Some(offset);
        } else if line.starts_with("$ runner ") {
            starts.push(context.take().unwrap_or(offset));
        }
        offset += line.len();
    }
    starts.push(text.len());
    starts
        .windows(2)
        .map(|pair| &text[pair[0]..pair[1]])
        .collect()
}

fn assert_manifest(expected: &str, cases: &[Case]) {
    assert_eq!(expected, format!("{HEADER}{}", blocks(expected).concat()));
    assert_eq!(
        blocks(expected)
            .iter()
            .map(|block| block.split_once("\nexit ").unwrap().0)
            .collect::<Vec<_>>(),
        cases.iter().map(heading).collect::<Vec<_>>(),
        "golden cases are missing, extra, or reordered"
    );
}

fn render_case(case: &Case) -> String {
    let mut fixture = Fixture::new();
    if case.args.starts_with(&["mission", "remove-role"]) {
        let pool = runner_daemon::db::open_pool(&fixture.data.join("runner.db")).unwrap();
        let conn = pool.get().unwrap();
        conn.execute("INSERT INTO sessions (id,mission_id,role_id,slot_id,status,started_at) VALUES ('session-remove','mission-running','role-reviewer','slot-reviewer','stopped',?1)", [STAMP]).unwrap();
    }
    if case.running {
        fixture.start();
    }
    let mut result = String::new();
    record(&mut result, &fixture, &case.args, case.inside, case.running);
    result
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/cli.txt")
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else {
        "unix"
    }
}

fn check_cases(selected: Vec<Case>) {
    // The manifest/update test is the only writer, including under nextest's separate processes.
    if std::env::var_os("UPDATE_RUNNER_GOLDENS").is_some() {
        return;
    }
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let expected = select_platform(&std::fs::read_to_string(golden_path()).unwrap(), platform());
    let all = golden_cases();
    assert_manifest(&expected, &all);
    let expected = blocks(&expected);
    for case in selected {
        let index = all.iter().position(|candidate| *candidate == case).unwrap();
        assert_eq!(render_case(&case), expected[index], "{}", heading(&case));
    }
}

#[test]
fn built_cli_goldens_inside_mission() {
    check_cases(inside_cases());
}

#[test]
fn built_cli_goldens_not_running() {
    check_cases(offline_cases());
}

#[test]
fn built_cli_goldens_help() {
    check_cases(help_cases());
}

fn visible_help_cases(fixture: &Fixture, args: Vec<String>) -> Vec<Vec<String>> {
    let output = fixture.command(false).args(&args).output().unwrap();
    assert!(output.status.success(), "help failed for {args:?}");
    let text = std::str::from_utf8(&output.stdout).unwrap();
    let command = regex::Regex::new(r"^  ([a-z][a-z0-9-]*)(?:\s|$)").unwrap();
    let mut cases = vec![args.clone()];
    for line in text
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' '))
    {
        if let Some(captures) = command.captures(line) {
            let mut child = args[..args.len() - 1].to_vec();
            child.extend([captures[1].to_owned(), "--help".to_owned()]);
            cases.extend(visible_help_cases(fixture, child));
        }
    }
    cases
}

fn assert_help_coverage(visible: &[Vec<String>], cases: &[Case]) {
    for args in visible {
        assert!(
            cases.iter().any(|case| case.args == *args),
            "missing help golden: runner {}",
            args.join(" ")
        );
    }
}

#[test]
fn help_goldens_cover_every_visible_command() {
    let fixture = Fixture::new();
    let visible = visible_help_cases(&fixture, vec!["--help".to_owned()]);
    assert!(visible.contains(&vec!["ask".to_owned(), "--help".to_owned()]));
    assert!(visible.contains(&vec![
        "mission".to_owned(),
        "start".to_owned(),
        "--help".to_owned()
    ]));
    assert_help_coverage(&visible, &help_cases());
}

#[test]
fn help_coverage_rejects_missing_commands() {
    for args in [vec!["ask", "--help"], vec!["mission", "start", "--help"]] {
        let visible = vec![args.iter().map(|arg| (*arg).to_owned()).collect()];
        let mut cases = help_cases();
        cases.retain(|case| case.args != args);
        assert!(std::panic::catch_unwind(|| assert_help_coverage(&visible, &cases)).is_err());
    }
}

#[test]
fn built_cli_goldens_manifest_and_update() {
    let cases = golden_cases();
    let golden = golden_path();
    if std::env::var_os("UPDATE_RUNNER_GOLDENS").is_some() {
        let mut result = format!("{HEADER}# platform {}\n", platform());
        for (index, case) in cases.iter().enumerate() {
            if index == 2 {
                result.push_str("# platform all\n");
            }
            result.push_str(&render_case(case));
        }
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
        std::fs::write(&golden, updated).unwrap();
    }
    let expected = std::fs::read_to_string(golden).unwrap();
    assert!(expected.starts_with(HEADER));
    assert_manifest(&select_platform(&expected, platform()), &cases);
}

#[test]
fn golden_manifest_rejects_missing_and_reordered_cases() {
    let cases = offline_cases();
    let blocks = cases
        .iter()
        .map(|case| format!("{}\nexit 0\n\n", heading(case)))
        .collect::<String>();
    let expected = format!("{HEADER}{blocks}");
    assert_manifest(&expected, &cases);
    assert!(std::panic::catch_unwind(|| assert_manifest(&expected, &cases[1..])).is_err());
    let mut reordered = cases;
    reordered.swap(0, 1);
    assert!(std::panic::catch_unwind(|| assert_manifest(&expected, &reordered)).is_err());
}

fn select_platform(text: &str, platform: &str) -> String {
    let mut include = true;
    let selected: String = text
        .split_inclusive('\n')
        .filter(|line| {
            if let Some(selected) = line.strip_prefix("# platform ") {
                include = selected.trim() == "all" || selected.trim() == platform;
                false
            } else {
                include
            }
        })
        .collect();
    // Linux has an actionable systemd remedy; the common command/output corpus stays shared.
    #[cfg(target_os = "linux")]
    let selected = selected.replace(
        "Runner is not running. Open Runner and retry.",
        runner_cli::client::NOT_RUNNING_MESSAGE,
    );
    selected
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
fn remote_revoke_unknown_fingerprint_reports_refusal_and_exits_one() {
    let mut fixture = Fixture::new();
    fixture.start();
    for inside in [false, true] {
        let output = fixture
            .command(inside)
            .args(["remote", "revoke", "typo"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap().trim(),
            "No paired device has fingerprint typo."
        );
    }
    assert!(!fixture.data.join("remote-trust.json").exists());
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
            Some(CREW),
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

#[test]
fn session_show_text_and_json_keep_launch_values_after_role_edit() {
    const MISSION_SESSION: &str = "01M4R000000000000000000000";
    let mut fixture = Fixture::new();
    {
        let pool = runner_daemon::db::open_pool(&fixture.data.join("runner.db")).unwrap();
        let conn = pool.get().unwrap();
        conn.execute("UPDATE roles SET runtime = 'pi', command = 'edited-command', model = 'edited-model', effort = 'max', runtime_options_json = NULL WHERE id = 'role-coder'", []).unwrap();
        conn.execute(
            "UPDATE sessions SET id = ?1 WHERE id = 'session-mission'",
            [MISSION_SESSION],
        )
        .unwrap();
        for id in [SESSION, MISSION_SESSION] {
            let mut definition = runner_daemon::repo::session::get_row(&conn, id)
                .unwrap()
                .unwrap()
                .launch_definition
                .unwrap();
            definition
                .env
                .insert("API_KEY".into(), "SECRET_878_SENTINEL".into());
            runner_daemon::repo::session::set_launch(&conn, id, "codex", &definition).unwrap();
        }
    }
    fixture.start();
    for id in [SESSION, MISSION_SESSION] {
        let text = fixture
            .command(false)
            .args(["session", "show", id])
            .output()
            .unwrap();
        assert!(
            text.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&text.stderr)
        );
        let text = String::from_utf8(text.stdout).unwrap();
        for expected in ["missing-fixture-agent", "fixture-model", "high", "fast"] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("edited"));
        let output = fixture
            .command(false)
            .args(["session", "show", id, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["agent_runtime"], "codex");
        assert_eq!(json["agent_command"], "missing-fixture-agent");
        assert_eq!(json["agent_model"], "fixture-model");
        assert_eq!(json["agent_effort"], "high");
        assert_eq!(json["agent_speed"], "fast");
        assert!(json.get("launch_definition").is_none());
        assert!(json.get("launch_definition_json").is_none());
        assert!(!String::from_utf8(output.stdout)
            .unwrap()
            .contains("SECRET_878_SENTINEL"));
    }
}

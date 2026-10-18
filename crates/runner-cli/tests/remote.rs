use std::process::{Command, Stdio};

#[test]
fn enrollment_refused_before_daemon_without_terminals_or_inside_agents() {
    for agent in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"));
        command
            .args(["remote", "login"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("RUNNER_HANDLE");
        if agent {
            command.env("RUNNER_HANDLE", "remote-guard-test");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("interactive terminal outside an agent session"));
    }
}
#[test]
fn remote_has_clap_help_but_is_absent_from_the_agent_guide() {
    let binary = env!("CARGO_BIN_EXE_runner-agent-cli");
    let help = Command::new(binary)
        .args(["remote", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("login"));
    for args in [vec!["help"], vec!["help", "agents"]] {
        let guide = Command::new(binary).args(args).output().unwrap();
        assert!(guide.status.success());
        assert!(!String::from_utf8(guide.stdout)
            .unwrap()
            .contains("runner remote"));
    }
}
